//! Numen 生存自治层 — 零额外 LLM 调用的自主行为通知 + 结构化失败 + 反垂直挖坑评分。
//!
//! 参考 Numen 项目的核心抽象（深度对齐）：
//! - `SurvivalJournal`：有界环形缓冲（6 条），drain 消费式读取，零额外 LLM 调用通知自主行为
//! - `SurvivalConfig`：全局使能门（AtomicBool），关闭时生存层等价于不存在
//! - `MiningEconomics`：反垂直挖坑评分（distance + 3.0 × depth_penalty）
//! - `FailureType`：14 个结构化失败类型（对齐 Numen 14 变体，in-ladder vs kick-back-to-LLM）
//! - `append_survival_notes`：把 journal 拼到 TaskResult.message（Numen withSurvivalNotes）
//!
//! 设计原则（Numen 边界红线）：
//! 1. reactive 层只恢复**同目标执行**，永不扩大目标范围或自动获取前置条件
//! 2. NO_MATERIAL / WRONG_TOOL / TARGET_LOST / MINED_OUT 是 prerequisite gap，直接踢回 LLM
//! 3. OCCLUDED / NO_PATH / OUT_OF_REACH / HAZARD / NO_SUPPORT / BOXED_IN 是 alternative execution，可 ladder 内重试
//! 4. 生存层通知 LLM "身体自己做了什么"，**从不询问**（informational, not consultative）

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

// ═══════════════════════════════════════════════════════════════
// SurvivalConfig — 全局使能门（Numen SurvivalConfig 风格）
// ═══════════════════════════════════════════════════════════════

/// 生存层全局使能开关（默认关，graduating 后再开）。
static SURVIVAL_ENABLED: AtomicBool = AtomicBool::new(false);

/// 查询生存层是否启用。
pub fn survival_enabled() -> bool {
    SURVIVAL_ENABLED.load(Ordering::Relaxed)
}

/// 设置生存层使能开关。
pub fn set_survival_enabled(v: bool) {
    SURVIVAL_ENABLED.store(v, Ordering::Relaxed);
}

// ═══════════════════════════════════════════════════════════════
// SurvivalJournal — 有界环形缓冲（Numen 风格，含 drain）
// ═══════════════════════════════════════════════════════════════

/// 生存事件类型（自主行为通知，零 LLM 调用）。
#[derive(Debug, Clone, PartialEq)]
pub enum SurvivalEvent {
    /// 自我保护触发（血量低于阈值）
    SelfPreservation { health: f32, trigger: String },
    /// 卡住自救（move_stuck 检测）
    Unstuck { position: (f64, f64, f64), attempt: u32 },
    /// 自动进食（饥饿值低于阈值）
    AutoEat { hunger: u32, item: String },
    /// 自动防御（敌对实体接近）
    SelfDefense { entity_type: String, distance: f64 },
    /// 自动撤退（濒死）
    Retreat { health: f32, from: (f64, f64, f64) },
    /// 自动拾取（掉落物接近）
    ItemCollecting { item: String, count: u32 },
    /// 自动放火把（亮度低于阈值）
    TorchPlacing { position: (i32, i32, i32), light: u32 },
    /// 模式切换通知
    ModeSwitch { from: String, to: String, reason: String },
}

impl SurvivalEvent {
    /// 渲染为简洁的单行文本（注入到 LLM 上下文）。
    pub fn to_brief(&self) -> String {
        match self {
            Self::SelfPreservation { health, trigger } => {
                format!("[survival] self_preservation: health={health:.1} trigger={trigger}")
            }
            Self::Unstuck { position, attempt } => {
                format!(
                    "[survival] unstuck: pos=({:.1},{:.1},{:.1}) attempt={attempt}",
                    position.0, position.1, position.2
                )
            }
            Self::AutoEat { hunger, item } => {
                format!("[survival] auto_eat: hunger={hunger} item={item}")
            }
            Self::SelfDefense { entity_type, distance } => {
                format!("[survival] self_defense: entity={entity_type} dist={distance:.1}m")
            }
            Self::Retreat { health, from } => {
                format!(
                    "[survival] retreat: health={health:.1} from=({:.1},{:.1},{:.1})",
                    from.0, from.1, from.2
                )
            }
            Self::ItemCollecting { item, count } => {
                format!("[survival] item_collecting: {item} x{count}")
            }
            Self::TorchPlacing { position, light } => {
                format!(
                    "[survival] torch_placed: ({},{},{}) light_was={light}",
                    position.0, position.1, position.2
                )
            }
            Self::ModeSwitch { from, to, reason } => {
                format!("[survival] mode_switch: {from}→{to} reason={reason}")
            }
        }
    }
}

/// 有界环形缓冲日志（Numen SurvivalJournal 风格）。
///
/// 容量 6 条（Numen 原版值），超出后丢弃最旧。零额外 LLM 调用——
/// 仅在下一轮 perceive 时作为上下文前缀注入，避免每条事件单独唤醒 LLM。
///
/// **drain 语义**：消费式读取，读后清空，保证"通知一次就忘"。
#[derive(Debug, Clone)]
pub struct SurvivalJournal {
    events: VecDeque<SurvivalEvent>,
    capacity: usize,
    /// 事件计数（不重置，用于统计自主行为频率）。
    pub total_events: u64,
    /// 最近一次事件时间（用于退避判断）。
    pub last_event_at: Option<Instant>,
}

impl Default for SurvivalJournal {
    fn default() -> Self {
        Self::new(6)
    }
}

impl SurvivalJournal {
    pub fn new(capacity: usize) -> Self {
        Self {
            events: VecDeque::with_capacity(capacity),
            capacity,
            total_events: 0,
            last_event_at: None,
        }
    }

    /// 记录事件（环形覆盖最旧）。
    pub fn record(&mut self, event: SurvivalEvent) {
        if self.events.len() >= self.capacity {
            self.events.pop_front();
        }
        self.events.push_back(event);
        self.total_events += 1;
        self.last_event_at = Some(Instant::now());
    }

    /// 渲染为多行文本（注入 LLM 上下文前缀，**不消费**）。
    pub fn render(&self) -> String {
        if self.events.is_empty() {
            return String::new();
        }
        let mut out = String::from("[Recent autonomous events]\n");
        for e in &self.events {
            out.push_str(&format!("  {}\n", e.to_brief()));
        }
        out
    }

    /// **消费式读取**（Numen drain）：返回所有事件并清空缓冲。
    /// 用于"通知一次就忘"语义，避免同一事件被重复注入 LLM 上下文。
    pub fn drain(&mut self) -> Vec<SurvivalEvent> {
        self.events.drain(..).collect()
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// 当前条数。
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// 清空（clear_chat 时调用）。
    pub fn clear(&mut self) {
        self.events.clear();
    }

    /// 距上次事件的间隔（用于退避判断，避免同 tick 连续触发）。
    pub fn elapsed_since_last(&self) -> Option<Duration> {
        self.last_event_at.map(|t| t.elapsed())
    }

    /// 是否在退避期内（默认 2 秒，避免抖动）。
    pub fn in_backoff(&self, backoff: Duration) -> bool {
        self.elapsed_since_last()
            .map(|d| d < backoff)
            .unwrap_or(false)
    }
}

/// 把 journal 的 drain 内容拼接到工具结果消息（Numen withSurvivalNotes 风格）。
///
/// 这是 Numen "零额外 LLM 调用通知自主行为"设计的核心注入点：
/// 生存层做了一件事 → drain 出事件 → 拼到当前 tool result → LLM 下一轮被动看到。
pub fn append_survival_notes(result_msg: &mut String, journal: &mut SurvivalJournal) {
    let events = journal.drain();
    if events.is_empty() {
        return;
    }
    let notes: Vec<String> = events.iter().map(|e| e.to_brief()).collect();
    result_msg.push_str(&format!(
        " [meanwhile, my body handled on its own: {}]",
        notes.join("; ")
    ));
}

// ═══════════════════════════════════════════════════════════════
// MiningEconomics — 反垂直挖坑评分（Numen 风格）
// ═══════════════════════════════════════════════════════════════

/// 挖掘候选评分（越低越优先）。
///
/// Numen 公式：score = distance + 3.0 × depth_penalty
/// depth_penalty = 垂直挖坑深度（player.y - block.y），惩罚"越挖越深无法回程"。
#[derive(Debug, Clone)]
pub struct MiningCandidate {
    pub block_id: String,
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub distance: f64,
    pub depth: i32,
    pub score: f64,
}

impl MiningCandidate {
    /// 计算评分（越低越优先）。
    pub fn score(distance: f64, depth: i32) -> f64 {
        let depth_penalty = if depth > 0 { depth as f64 } else { 0.0 };
        distance + 3.0 * depth_penalty
    }

    pub fn new(
        block_id: String,
        x: i32,
        y: i32,
        z: i32,
        player_x: f64,
        player_y: f64,
        player_z: f64,
    ) -> Self {
        let dx = x as f64 - player_x;
        let dz = z as f64 - player_z;
        let distance = (dx * dx + dz * dz).sqrt();
        let depth = (player_y - y as f64).round() as i32;
        let score = Self::score(distance, depth);
        Self { block_id, x, y, z, distance, depth, score }
    }
}

/// 从候选列表中选出最优（评分最低）的方块。
pub fn pick_best_candidate(candidates: Vec<MiningCandidate>) -> Option<MiningCandidate> {
    candidates.into_iter().min_by(|a, b| {
        a.score.partial_cmp(&b.score).unwrap_or(std::cmp::Ordering::Equal)
    })
}

/// 过滤掉评分过高的候选（避免挖太深的坑）。
pub fn filter_by_score(candidates: Vec<MiningCandidate>, max_score: f64) -> Vec<MiningCandidate> {
    candidates.into_iter().filter(|c| c.score <= max_score).collect()
}

// ═══════════════════════════════════════════════════════════════
// FailureType — 14 个结构化失败类型（对齐 Numen FailureType.java）
// ═══════════════════════════════════════════════════════════════

/// 结构化失败类型（对齐 Numen 14 变体）。
///
/// **边界红线**（Numen recovery boundary）：
/// - reactive 层只恢复**同有界目标的执行**，永不扩大目标范围或自动获取前置条件
/// - `is_in_ladder()`：6 类 alternative execution，可在 RecoveryLadder 内换策略重试
/// - `is_kick_back()`：4 类 prerequisite gap，直接踢回 LLM 重新决策
/// - INTERRUPTED / TIMED_OUT / UNSUPPORTED / UNKNOWN 是终态但不属于两类
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FailureType {
    // ═══ InLadder（alternative execution，阶梯内换策略重试） ═══
    /// 视线被墙挡（可换 stance / 清遮挡）
    Occluded,
    /// body 出不去（可换出发点）
    BoxedIn,
    /// 路径不可达（可换路径 / 重规划）
    NoPath,
    /// 目标太远（可走近 / 换近目标）
    OutOfReach,
    /// 流体/岩浆/虚空危害（拒绝挖——infinite cost, not large cost）
    Hazard,
    /// 无支撑面（可换 stance 找支撑）
    NoSupport,

    // ═══ KickBack（prerequisite gap，直接踢回 LLM） ═══
    /// 缺材料（需要 LLM 规划采集）
    NoMaterial,
    /// 工具不对（需要 LLM 规划制作）
    WrongTool,
    /// 目标已消失（需要 LLM 换目标）
    TargetLost,
    /// 扫描半径内无目标（需要 LLM widen or stop）
    MinedOut,

    // ═══ 终态（不属于 in-ladder 也不属于 kick-back） ═══
    /// 被抢占中断（生存链抢 body）
    Interrupted,
    /// deadline 用完（进度租约耗尽）
    TimedOut,
    /// 记录类型无 runner（注册缺失）
    Unsupported,
    /// 未知错误（需要 LLM 诊断）
    Unknown,
}

impl FailureType {
    /// 是否可以阶梯内重试（alternative execution，不踢回 LLM）。
    pub fn is_in_ladder(&self) -> bool {
        matches!(
            self,
            FailureType::Occluded
                | FailureType::BoxedIn
                | FailureType::NoPath
                | FailureType::OutOfReach
                | FailureType::Hazard
                | FailureType::NoSupport
        )
    }

    /// 是否需要踢回 LLM 重新决策（prerequisite gap）。
    pub fn is_kick_back(&self) -> bool {
        matches!(
            self,
            FailureType::NoMaterial
                | FailureType::WrongTool
                | FailureType::TargetLost
                | FailureType::MinedOut
        )
    }

    /// 是否为终态（不属于 in-ladder 也不属于 kick-back）。
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            FailureType::Interrupted | FailureType::TimedOut | FailureType::Unsupported | FailureType::Unknown
        )
    }

    /// 渲染为 LLM 友好的文本。
    pub fn to_llm_text(&self, context: &str) -> String {
        match self {
            Self::Occluded => format!("View blocked ({context}) — trying alternate stance"),
            Self::BoxedIn => format!("Can't leave current spot ({context}) — finding exit"),
            Self::NoPath => format!("No path to target ({context}) — replanning route"),
            Self::OutOfReach => format!("Target too far ({context}) — moving closer"),
            Self::Hazard => format!("HAZARD: fluid/void near ({context}) — refusing to dig here"),
            Self::NoSupport => format!("No support face ({context}) — trying alternate stance"),
            Self::NoMaterial => format!("CRITICAL: Missing material ({context}) — needs LLM to plan gathering"),
            Self::WrongTool => format!("CRITICAL: Wrong tool ({context}) — needs LLM to plan crafting"),
            Self::TargetLost => format!("CRITICAL: Target gone ({context}) — needs LLM to pick new target"),
            Self::MinedOut => format!("CRITICAL: No targets in scan radius ({context}) — needs LLM to widen or stop"),
            Self::Interrupted => format!("Interrupted by survival chain ({context})"),
            Self::TimedOut => format!("Timed out ({context}) — progress lease exhausted"),
            Self::Unsupported => format!("Unsupported task type ({context})"),
            Self::Unknown => format!("CRITICAL: Unknown error ({context}) — needs LLM diagnosis"),
        }
    }
}

/// 失败记录（含上下文 + 时间戳 + 重试次数）。
#[derive(Debug, Clone)]
pub struct FailureRecord {
    pub failure_type: FailureType,
    pub context: String,
    pub timestamp: Instant,
    pub retry_count: u32,
}

impl FailureRecord {
    pub fn new(failure_type: FailureType, context: String) -> Self {
        Self { failure_type, context, timestamp: Instant::now(), retry_count: 0 }
    }

    /// 增加重试次数。
    pub fn increment_retry(&mut self) {
        self.retry_count += 1;
    }

    /// 是否超过最大重试次数（in_ladder 类型的上限是 3 次）。
    pub fn should_kick_back(&self) -> bool {
        if self.failure_type.is_kick_back() {
            return true;
        }
        self.retry_count > 3
    }

    /// 渲染为 LLM 友好的文本。
    pub fn to_llm_text(&self) -> String {
        self.failure_type.to_llm_text(&self.context)
    }
}

/// 失败追踪器（同目标多次失败后踢回 LLM，跨任务统计）。
#[derive(Debug, Default)]
pub struct FailureTracker {
    recent: std::collections::HashMap<String, FailureRecord>,
}

impl FailureTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// 记录失败。返回 true 表示应踢回 LLM。
    pub fn record(&mut self, target_key: &str, failure_type: FailureType, context: String) -> bool {
        let record = self
            .recent
            .entry(target_key.to_string())
            .or_insert_with(|| FailureRecord::new(failure_type.clone(), context.clone()));
        if record.failure_type != failure_type {
            *record = FailureRecord::new(failure_type.clone(), context);
        } else {
            record.increment_retry();
        }
        record.should_kick_back()
    }

    /// 清除目标的失败记录（成功后调用）。
    pub fn clear(&mut self, target_key: &str) {
        self.recent.remove(target_key);
    }

    /// 清空所有失败记录。
    pub fn clear_all(&mut self) {
        self.recent.clear();
    }

    /// 获取目标的失败记录。
    pub fn get(&self, target_key: &str) -> Option<&FailureRecord> {
        self.recent.get(target_key)
    }
}

// ═══════════════════════════════════════════════════════════════
// 测试
// ═══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn survival_config_global_toggle() {
        set_survival_enabled(false);
        assert!(!survival_enabled());
        set_survival_enabled(true);
        assert!(survival_enabled());
        set_survival_enabled(false); // 恢复默认
    }

    #[test]
    fn survival_journal_ring_buffer() {
        let mut j = SurvivalJournal::new(3);
        j.record(SurvivalEvent::AutoEat { hunger: 6, item: "bread".into() });
        j.record(SurvivalEvent::SelfDefense { entity_type: "zombie".into(), distance: 3.0 });
        j.record(SurvivalEvent::Unstuck { position: (1.0, 64.0, 2.0), attempt: 1 });
        assert_eq!(j.len(), 3);
        j.record(SurvivalEvent::Retreat { health: 4.0, from: (1.0, 64.0, 2.0) });
        assert_eq!(j.len(), 3);
        assert_eq!(j.total_events, 4);
        let rendered = j.render();
        assert!(!rendered.contains("auto_eat"));
        assert!(rendered.contains("self_defense"));
    }

    #[test]
    fn survival_journal_drain_consumes() {
        let mut j = SurvivalJournal::new(6);
        j.record(SurvivalEvent::AutoEat { hunger: 6, item: "bread".into() });
        j.record(SurvivalEvent::SelfDefense { entity_type: "zombie".into(), distance: 3.0 });
        assert_eq!(j.len(), 2);
        let drained = j.drain();
        assert_eq!(drained.len(), 2);
        assert!(j.is_empty());
        // drain 后 render 应为空
        assert!(j.render().is_empty());
    }

    #[test]
    fn append_survival_notes_injects_and_drains() {
        let mut msg = String::from("collected 3 oak_log");
        let mut j = SurvivalJournal::new(6);
        j.record(SurvivalEvent::SelfDefense { entity_type: "zombie".into(), distance: 3.0 });
        append_survival_notes(&mut msg, &mut j);
        assert!(msg.contains("meanwhile, my body handled on its own"));
        assert!(msg.contains("self_defense"));
        // journal 应被 drain 清空
        assert!(j.is_empty());
        // 再次调用不应追加空内容（msg 长度不变）
        let len_before = msg.len();
        append_survival_notes(&mut msg, &mut j);
        assert_eq!(msg.len(), len_before, "第二次调用（空 journal）不应追加任何内容");
    }

    #[test]
    fn mining_economics_penalizes_depth() {
        let shallow = MiningCandidate::new("coal_ore".into(), 0, 60, 0, 5.0, 64.0, 0.0);
        let deep = MiningCandidate::new("coal_ore".into(), 0, 40, 0, 5.0, 64.0, 0.0);
        assert!(shallow.score < deep.score, "shallow should have lower score");
        assert!(
            deep.score - shallow.score > 3.0 * 20.0 - 0.1,
            "depth penalty should be ~3.0 * depth_diff"
        );
    }

    #[test]
    fn failure_type_numen_14_variants_alignment() {
        // 6 in-ladder (alternative execution)
        assert!(FailureType::Occluded.is_in_ladder());
        assert!(FailureType::BoxedIn.is_in_ladder());
        assert!(FailureType::NoPath.is_in_ladder());
        assert!(FailureType::OutOfReach.is_in_ladder());
        assert!(FailureType::Hazard.is_in_ladder());
        assert!(FailureType::NoSupport.is_in_ladder());
        // 4 kick-back (prerequisite gap)
        assert!(FailureType::NoMaterial.is_kick_back());
        assert!(FailureType::WrongTool.is_kick_back());
        assert!(FailureType::TargetLost.is_kick_back());
        assert!(FailureType::MinedOut.is_kick_back());
        // 4 terminal (neither)
        assert!(FailureType::Interrupted.is_terminal());
        assert!(FailureType::TimedOut.is_terminal());
        assert!(FailureType::Unsupported.is_terminal());
        assert!(FailureType::Unknown.is_terminal());
        // in_ladder 和 kick_back 互斥
        for v in [
            FailureType::Occluded, FailureType::BoxedIn, FailureType::NoPath,
            FailureType::OutOfReach, FailureType::Hazard, FailureType::NoSupport,
        ] {
            assert!(!v.is_kick_back() && !v.is_terminal());
        }
        for v in [
            FailureType::NoMaterial, FailureType::WrongTool,
            FailureType::TargetLost, FailureType::MinedOut,
        ] {
            assert!(!v.is_in_ladder() && !v.is_terminal());
        }
    }

    #[test]
    fn failure_tracker_kicks_back_after_3_retries() {
        let mut t = FailureTracker::new();
        // 前 3 次 in_ladder 失败不应踢回
        for _ in 0..3 {
            assert!(!t.record("oak_log", FailureType::NoPath, "blocked by dirt".into()));
        }
        // 第 4 次应踢回
        assert!(t.record("oak_log", FailureType::NoPath, "blocked by dirt".into()));
        // kick_back 类型应立即踢回
        assert!(t.record("diamond_ore", FailureType::NoMaterial, "need iron pickaxe".into()));
    }

    #[test]
    fn pick_best_candidate_returns_lowest_score() {
        let candidates = vec![
            MiningCandidate::new("coal_ore".into(), 0, 60, 0, 5.0, 64.0, 0.0),
            MiningCandidate::new("coal_ore".into(), 3, 62, 0, 2.0, 64.0, 0.0),
            MiningCandidate::new("coal_ore".into(), 0, 40, 0, 5.0, 64.0, 0.0),
        ];
        let best = pick_best_candidate(candidates).unwrap();
        assert_eq!(best.y, 62, "should pick shallow + close candidate");
    }
}
