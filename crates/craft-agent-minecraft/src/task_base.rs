//! Numen 任务骨架 — RecoveryLadder + Precondition + CountedProgress + TargetSet + 任务基类。
//!
//! 对齐 Numen 项目的核心任务架构（base/*.java）：
//! - `TaskState` / `TaskRecord` / `TaskQueue`：任务状态机 + FIFO 队列 + freeze-aware deadline
//! - `Suspendable` trait：抢占时不丢逻辑状态（nav plan / dig 进度 / phase / blacklist）
//! - `CompanionTask` trait：start → tick → buildResult 生命周期契约
//! - `Precondition`：开始时前置门（prerequisite gap 直接踢回 LLM）
//! - `RecoveryLadder`：同目标多策略阶梯（alternative execution 内重试）
//! - `CountedProgress`：基线-增量进度模型（采集 N 个"更多"而非"持有 N 个"）
//! - `TargetSet`：黑名单/跳过 + 选最优候选
//! - `AbstractCompanionTask`：reactive 任务统一骨架（final lifecycle 模板）
//! - `GoToThenDoTask`：走过去再做一件事
//!
//! 设计原则（Numen 边界红线）：
//! 1. reactive 层只恢复**同有界目标的执行**，永不扩大目标范围
//! 2. prerequisite gap（NO_MATERIAL/WRONG_TOOL/TARGET_LOST/MINED_OUT）直接踢回 LLM
//! 3. alternative execution（OCCLUDED/NO_PATH/OUT_OF_REACH/HAZARD/NO_SUPPORT/BOXED_IN）在 ladder 内重试
//! 4. suspend 只释放 body，保留 nav plan（resume 能直接继续）

use crate::survival::FailureType;
use std::collections::{HashMap, HashSet};
use std::time::Instant;

// ═══════════════════════════════════════════════════════════════
// TaskState — 任务状态机（Numen TaskState.java）
// ═══════════════════════════════════════════════════════════════

/// 任务状态（对齐 Numen TaskState）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskState {
    Pending,
    Running,
    Success,
    Failed,
    Timeout,
    Cancelled,
}

impl TaskState {
    /// 是否为终态。
    pub fn is_terminal(self) -> bool {
        !matches!(self, Self::Pending | Self::Running)
    }
}

// ═══════════════════════════════════════════════════════════════
// TaskRecord — 任务记录基类（Numen TaskRecord.java）
// ═══════════════════════════════════════════════════════════════

/// 任务记录（对齐 Numen TaskRecord）。
#[derive(Debug, Clone)]
pub struct TaskRecord {
    /// 单调递增 ID。
    pub id: u64,
    /// 工具名（LLM tool call name）。
    pub tool_name: String,
    /// 工具调用 ID（必须原样回显给 OpenAI）。
    pub tool_call_id: String,
    /// deadline（game time，freeze-aware）。
    pub deadline_game_time: i64,
    /// 当前状态。
    pub state: TaskState,
    /// 创建时间。
    pub created_at: Instant,
}

impl TaskRecord {
    pub fn new(id: u64, tool_name: String, tool_call_id: String, deadline_game_time: i64) -> Self {
        Self {
            id,
            tool_name,
            tool_call_id,
            deadline_game_time,
            state: TaskState::Pending,
            created_at: Instant::now(),
        }
    }

    /// 推后 deadline（只允许往后推，Numen extendDeadlineTo）。
    pub fn extend_deadline_to(&mut self, gt: i64) {
        if gt > self.deadline_game_time {
            self.deadline_game_time = gt;
        }
    }
}

// ═══════════════════════════════════════════════════════════════
// TaskQueue — FIFO + 冻结 + 取消（Numen TaskQueue.java）
// ═══════════════════════════════════════════════════════════════

/// 任务队列（对齐 Numen TaskQueue）。
///
/// FIFO pending + completed outbox。支持：
/// - `freeze_pending_deadlines`：生存链持有 body 时把 pending deadline 推后 1 tick
/// - `cancel_all`：实体移除/死亡时把所有 pending 标 CANCELLED
#[derive(Debug, Default)]
pub struct TaskQueue {
    pending: Vec<TaskRecord>,
    completed: Vec<TaskRecord>,
}

impl TaskQueue {
    pub fn new() -> Self {
        Self::default()
    }

    /// 入队。
    pub fn enqueue(&mut self, record: TaskRecord) {
        self.pending.push(record);
    }

    /// 取队首。
    pub fn poll_head(&mut self) -> Option<TaskRecord> {
        if self.pending.is_empty() {
            None
        } else {
            Some(self.pending.remove(0))
        }
    }

    /// 完成（移到 outbox）。
    pub fn complete(&mut self, mut record: TaskRecord) {
        record.state = if record.state.is_terminal() {
            record.state
        } else {
            TaskState::Success
        };
        self.completed.push(record);
    }

    /// 排出已完成。
    pub fn drain_completed(&mut self) -> Vec<TaskRecord> {
        self.completed.drain(..).collect()
    }

    /// 是否有 pending。
    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    /// 冻结 pending deadline（每项推后 1 tick）。
    pub fn freeze_pending_deadlines(&mut self) {
        for r in &mut self.pending {
            r.deadline_game_time += 1;
        }
    }

    /// 取消所有 pending（标 CANCELLED 移到 outbox）。
    pub fn cancel_all(&mut self, _reason: &str) {
        let mut to_cancel: Vec<_> = self.pending.drain(..).collect();
        for r in &mut to_cancel {
            r.state = TaskState::Cancelled;
        }
        self.completed.extend(to_cancel);
    }

    /// pending 数量。
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }
}

// ═══════════════════════════════════════════════════════════════
// Suspendable — 抢占时不丢逻辑状态（Numen Suspendable.java）
// ═══════════════════════════════════════════════════════════════

/// 可挂起 trait（对齐 Numen Suspendable）。
///
/// **关键性质**：suspend 释放 BODY 但保留 nav plan / dig 进度 / phase / blacklist。
/// **故意不调用 nav.stop()**——plan 保留是 resume 能直接继续的关键。
pub trait Suspendable {
    /// 释放 body（halt 输入），保留逻辑状态。
    fn suspend(&mut self);

    /// 恢复（默认 no-op，子类按需实现）。
    fn resume(&mut self) {}
}

// ═══════════════════════════════════════════════════════════════
// CompanionTask — 任务生命周期契约（Numen CompanionTask.java）
// ═══════════════════════════════════════════════════════════════

/// 任务执行上下文（简化版，实际应包含 player/nav/inventory 等）。
pub struct TaskContext {
    pub game_time: i64,
    pub player_health: f32,
    pub player_food: u32,
    pub player_pos: (f64, f64, f64),
}

/// 反应式任务 trait（对齐 Numen CompanionTask）。
pub trait CompanionTask: Suspendable {
    /// 启动（返回首态）。
    fn start(&mut self, ctx: &TaskContext) -> TaskState;

    /// 每 tick 推进（返回当前状态）。
    fn tick(&mut self, ctx: &mut TaskContext) -> TaskState;

    /// 构建最终结果消息。
    fn build_result(&self, final_state: TaskState) -> TaskResult;
}

/// 任务执行结果。
#[derive(Debug, Clone)]
pub struct TaskResult {
    pub success: bool,
    pub message: String,
    pub timeout: bool,
    pub cancelled: bool,
    pub failure_type: Option<FailureType>,
}

impl TaskResult {
    pub fn ok(message: String) -> Self {
        Self { success: true, message, timeout: false, cancelled: false, failure_type: None }
    }

    pub fn fail(message: String, failure_type: FailureType) -> Self {
        Self {
            success: false,
            message,
            timeout: false,
            cancelled: false,
            failure_type: Some(failure_type),
        }
    }

    pub fn timeout(message: String) -> Self {
        Self { success: false, message, timeout: true, cancelled: false, failure_type: Some(FailureType::TimedOut) }
    }

    pub fn cancelled(message: String) -> Self {
        Self {
            success: false,
            message,
            timeout: false,
            cancelled: true,
            failure_type: Some(FailureType::Interrupted),
        }
    }
}

// ═══════════════════════════════════════════════════════════════
// Precondition — 开始时前置门（Numen Precondition.java）
// ═══════════════════════════════════════════════════════════════

/// 前置条件失败。
#[derive(Debug, Clone)]
pub struct PreconditionFailure {
    pub message: String,
    pub failure_type: FailureType,
}

/// 前置条件 trait（对齐 Numen Precondition）。
///
/// **边界红线**：prerequisite gap 不在 reactive 层自动获取，直接踢回 LLM。
pub trait Precondition {
    fn check(&self, ctx: &TaskContext) -> Option<PreconditionFailure>;
}

// ═══════════════════════════════════════════════════════════════
// RecoveryLadder — 同目标多策略阶梯（Numen RecoveryLadder.java）
// ═══════════════════════════════════════════════════════════════

/// 阶梯的一个 rung（策略 + 能处理的失败类型 + 最大尝试次数）。
pub struct Rung<F> {
    pub name: String,
    pub strategy: F,
    pub handles: HashSet<FailureType>,
    pub max_attempts: u32,
    pub attempts: u32,
}

impl<F> std::fmt::Debug for Rung<F> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Rung")
            .field("name", &self.name)
            .field("handles", &self.handles)
            .field("max_attempts", &self.max_attempts)
            .field("attempts", &self.attempts)
            .finish()
    }
}

/// 同目标多策略恢复阶梯（对齐 Numen RecoveryLadder）。
///
/// **核心规则**：所有 rung 必须围绕**同一有界目标**+ 不同策略。
/// 不能"目标改成去拿更多方块"——那是 scope creep。
///
/// 行为：
/// - `advance(last_fail)`：当前 rung `handles` 包含 last_fail 且 `attempts < max_attempts` → retry 同 rung；
///   否则找下一个 `handles` 包含 last_fail 的 rung；都不行 → 返回 false（exhausted）
pub struct RecoveryLadder<F> {
    rungs: Vec<Rung<F>>,
    index: usize,
}

impl<F> std::fmt::Debug for RecoveryLadder<F> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecoveryLadder")
            .field("rungs", &self.rungs)
            .field("index", &self.index)
            .finish()
    }
}

impl<F> RecoveryLadder<F> {
    pub fn new(rungs: Vec<Rung<F>>) -> Self {
        Self { rungs, index: 0 }
    }

    /// 当前 rung 名称。
    pub fn current_name(&self) -> Option<&str> {
        self.rungs.get(self.index).map(|r| r.name.as_str())
    }

    /// 是否已耗尽。
    pub fn is_exhausted(&self) -> bool {
        self.index >= self.rungs.len()
    }

    /// 记录一次尝试。
    pub fn record_attempt(&mut self) {
        if let Some(r) = self.rungs.get_mut(self.index) {
            r.attempts += 1;
        }
    }

    /// 推进到下一个能处理 last_fail 的 rung。
    /// 返回 true 表示成功推进（或同 rung retry），false 表示已耗尽。
    pub fn advance(&mut self, last_fail: FailureType) -> bool {
        // 先检查当前 rung 是否还能 retry
        if let Some(r) = self.rungs.get(self.index) {
            if r.handles.contains(&last_fail) && r.attempts < r.max_attempts {
                // 同 rung retry
                return true;
            }
        }
        // 找下一个能处理的 rung
        let start = self.index + 1;
        for i in start..self.rungs.len() {
            if self.rungs[i].handles.contains(&last_fail) {
                self.index = i;
                return true;
            }
        }
        // 都不行 → exhausted
        self.index = self.rungs.len();
        false
    }

    /// 重置（新目标时调用）。
    pub fn reset(&mut self) {
        self.index = 0;
        for r in &mut self.rungs {
            r.attempts = 0;
        }
    }

    /// rung 数量。
    pub fn len(&self) -> usize {
        self.rungs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rungs.is_empty()
    }
}

// ═══════════════════════════════════════════════════════════════
// CountedProgress — 基线-增量进度模型（Numen CountedProgress.java）
// ═══════════════════════════════════════════════════════════════

/// 基线-增量进度（对齐 Numen CountedProgress）。
///
/// "采集 N 个**更多**"而非"持有 N 个"。
/// 构造时快照 baseline，gained = max(0, current - baseline)。
pub struct CountedProgress<F: Fn() -> u32> {
    target: u32,
    current: F,
    baseline: u32,
}

impl<F: Fn() -> u32> CountedProgress<F> {
    pub fn new(target: u32, current: F) -> Self {
        let baseline = current();
        Self { target, current, baseline }
    }

    /// 已获得数量（相对基线）。
    pub fn gained(&self) -> u32 {
        (self.current)().saturating_sub(self.baseline)
    }

    /// 剩余数量。
    pub fn remaining(&self) -> u32 {
        self.target.saturating_sub(self.gained())
    }

    /// 是否完成。
    pub fn done(&self) -> bool {
        self.gained() >= self.target
    }

    /// 基线值。
    pub fn baseline(&self) -> u32 {
        self.baseline
    }

    /// 目标值。
    pub fn target(&self) -> u32 {
        self.target
    }
}

// ═══════════════════════════════════════════════════════════════
// TargetSet — 黑名单/跳过 + 选最优候选（Numen TargetSet.java）
// ═══════════════════════════════════════════════════════════════

/// 目标集合 + 黑名单（对齐 Numen TargetSet）。
///
/// mine 用 blacklist（不可达矿石），hunt 用 skip（不可达 mob）。
#[derive(Debug)]
pub struct TargetSet<T: Eq + std::hash::Hash + Clone> {
    excluded: HashSet<T>,
}

impl<T: Eq + std::hash::Hash + Clone> Default for TargetSet<T> {
    fn default() -> Self {
        Self { excluded: HashSet::new() }
    }
}

impl<T: Eq + std::hash::Hash + Clone> TargetSet<T> {
    pub fn new() -> Self {
        Self::default()
    }

    /// 加入黑名单/跳过列表。
    pub fn blacklist(&mut self, t: T) {
        self.excluded.insert(t);
    }

    pub fn skip(&mut self, t: T) {
        self.blacklist(t);
    }

    /// 是否被排除。
    pub fn is_excluded(&self, t: &T) -> bool {
        self.excluded.contains(t)
    }

    /// 从候选中选最优（未被排除 + 评分最低）。
    pub fn pick<'a, U, S: Fn(&U) -> (&T, f64)>(&self, candidates: &'a [U], score_fn: S) -> Option<&'a U> {
        candidates
            .iter()
            .filter(|c| {
                let (t, _) = score_fn(c);
                !self.is_excluded(t)
            })
            .min_by(|a, b| {
                let (_, sa) = score_fn(a);
                let (_, sb) = score_fn(b);
                sa.partial_cmp(&sb).unwrap_or(std::cmp::Ordering::Equal)
            })
    }

    /// 清空黑名单。
    pub fn clear(&mut self) {
        self.excluded.clear();
    }

    /// 黑名单大小。
    pub fn excluded_count(&self) -> usize {
        self.excluded.len()
    }
}

// ═══════════════════════════════════════════════════════════════
// 测试
// ═══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_state_terminal_check() {
        assert!(!TaskState::Pending.is_terminal());
        assert!(!TaskState::Running.is_terminal());
        assert!(TaskState::Success.is_terminal());
        assert!(TaskState::Failed.is_terminal());
        assert!(TaskState::Timeout.is_terminal());
        assert!(TaskState::Cancelled.is_terminal());
    }

    #[test]
    fn task_record_extend_deadline_only_forward() {
        let mut r = TaskRecord::new(1, "mine".into(), "call-1".into(), 100);
        r.extend_deadline_to(150);
        assert_eq!(r.deadline_game_time, 150);
        // 不能往前推
        r.extend_deadline_to(80);
        assert_eq!(r.deadline_game_time, 150);
    }

    #[test]
    fn task_queue_fifo_and_freeze() {
        let mut q = TaskQueue::new();
        q.enqueue(TaskRecord::new(1, "a".into(), "c1".into(), 100));
        q.enqueue(TaskRecord::new(2, "b".into(), "c2".into(), 200));
        assert!(q.has_pending());
        assert_eq!(q.pending_count(), 2);

        // freeze 推后 deadline
        q.freeze_pending_deadlines();
        let head = q.poll_head().unwrap();
        assert_eq!(head.id, 1);
        assert_eq!(head.deadline_game_time, 101); // 100 + 1

        // cancel_all
        q.cancel_all("entity removed");
        assert!(!q.has_pending());
        let completed = q.drain_completed();
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].state, TaskState::Cancelled);
    }

    #[test]
    fn recovery_ladder_retry_same_rung() {
        let rungs = vec![
            Rung {
                name: "direct".into(),
                strategy: (),
                handles: [FailureType::Occluded, FailureType::NoPath].into(),
                max_attempts: 2,
                attempts: 0,
            },
            Rung {
                name: "alternate_stance".into(),
                strategy: (),
                handles: [FailureType::Occluded].into(),
                max_attempts: 1,
                attempts: 0,
            },
        ];
        let mut ladder = RecoveryLadder::new(rungs);
        assert_eq!(ladder.current_name(), Some("direct"));
        ladder.record_attempt();
        // 第 1 次失败 Occluded，当前 rung 还能 retry（attempts=1 < max=2）
        assert!(ladder.advance(FailureType::Occluded));
        assert_eq!(ladder.current_name(), Some("direct"));
        ladder.record_attempt();
        // 第 2 次失败 Occluded，当前 rung 已用完（attempts=2 >= max=2），应推进到下一个
        assert!(ladder.advance(FailureType::Occluded));
        assert_eq!(ladder.current_name(), Some("alternate_stance"));
    }

    #[test]
    fn recovery_ladder_exhausted() {
        let rungs = vec![Rung {
            name: "only".into(),
            strategy: (),
            handles: [FailureType::Occluded].into(),
            max_attempts: 1,
            attempts: 0,
        }];
        let mut ladder = RecoveryLadder::new(rungs);
        ladder.record_attempt();
        // 失败 NoMaterial（kick-back，不在 handles 中）→ 应耗尽
        assert!(!ladder.advance(FailureType::NoMaterial));
        assert!(ladder.is_exhausted());
        assert_eq!(ladder.current_name(), None);
    }

    #[test]
    fn recovery_ladder_reset() {
        let rungs = vec![Rung {
            name: "r1".into(),
            strategy: (),
            handles: [FailureType::NoPath].into(),
            max_attempts: 1,
            attempts: 1,
        }];
        let mut ladder = RecoveryLadder::new(rungs);
        ladder.advance(FailureType::NoPath); // 应耗尽
        assert!(ladder.is_exhausted());
        ladder.reset();
        assert!(!ladder.is_exhausted());
        assert_eq!(ladder.current_name(), Some("r1"));
    }

    #[test]
    fn counted_progress_baseline_increment() {
        let counter = std::cell::Cell::new(10u32);
        let progress = CountedProgress::new(5, || counter.get());
        assert_eq!(progress.baseline(), 10);
        assert_eq!(progress.gained(), 0);
        assert!(!progress.done());

        counter.set(13);
        assert_eq!(progress.gained(), 3);
        assert_eq!(progress.remaining(), 2);
        assert!(!progress.done());

        counter.set(15);
        assert_eq!(progress.gained(), 5);
        assert!(progress.done());
    }

    #[test]
    fn target_set_blacklist_and_pick() {
        let mut ts = TargetSet::<String>::new();
        ts.blacklist("far_ore".into());

        let candidates = vec![
            ("near_ore".to_string(), 5.0_f64),
            ("far_ore".to_string(), 50.0),
            ("mid_ore".to_string(), 15.0),
        ];
        let best = ts.pick(&candidates, |c| (&c.0, c.1));
        assert!(best.is_some());
        assert_eq!(best.unwrap().0, "near_ore"); // 最低分且未被排除

        ts.blacklist("near_ore".into());
        let best2 = ts.pick(&candidates, |c| (&c.0, c.1));
        assert_eq!(best2.unwrap().0, "mid_ore"); // near_ore 被排除后选 mid_ore
    }

    #[test]
    fn task_result_constructors() {
        let ok = TaskResult::ok("done".into());
        assert!(ok.success && !ok.timeout && !ok.cancelled);

        let fail = TaskResult::fail("no mat".into(), FailureType::NoMaterial);
        assert!(!fail.success && fail.failure_type == Some(FailureType::NoMaterial));

        let to = TaskResult::timeout("slow".into());
        assert!(to.timeout);

        let can = TaskResult::cancelled("stopped".into());
        assert!(can.cancelled);
    }
}
