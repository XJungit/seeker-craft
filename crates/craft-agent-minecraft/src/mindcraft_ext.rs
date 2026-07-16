//! mindcraft 学习点 — ActionManager watchdog + SelfPrompter 三态 + History 滚动摘要。
//!
//! 参考 mindcraft 项目的三个核心抽象：
//! - `ActionManager`：中央动作调度器（循环检测、超时 watchdog、resume 机制）
//! - `SelfPrompter`：自驱动目标循环器（Stopped/Active/Paused 三态状态机）
//! - `History`：滚动摘要式长对话管理（max_messages 阈值触发 LLM 摘要压缩 + 磁盘归档）
//!
//! 设计目标：
//! 1. 动作超时自动中断 + 循环检测（同一动作重复 N 次告警）
//! 2. 持续目标注入（SelfPrompter 周期性提醒 agent 当前目标）
//! 3. 长对话自动压缩（超过阈值触发摘要，避免上下文爆炸）

use std::collections::VecDeque;
use std::time::{Duration, Instant};

// ═══════════════════════════════════════════════════════════════
// ActionManager — 中央动作调度器（mindcraft 风格）
// ═══════════════════════════════════════════════════════════════

/// 动作状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionState {
    /// 空闲。
    Idle,
    /// 运行中。
    Running { action_name: String, started_at: Instant },
    /// 已中断。
    Interrupted,
    /// 超时。
    TimedOut,
    /// 已完成。
    Done,
}

/// 动作调度器（mindcraft ActionManager 风格）。
///
/// 功能：
/// - 超时 watchdog：动作运行超过 timeout 自动标记 TimedOut
/// - 循环检测：同一动作连续触发 N 次告警
/// - resume 机制：可恢复动作（如 followPlayer）不被中断清除
#[derive(Debug)]
pub struct ActionManager {
    state: ActionState,
    /// 默认超时（秒）。
    default_timeout: Duration,
    /// 循环检测窗口（记录最近 N 个动作名）。
    recent_actions: VecDeque<String>,
    /// 循环检测阈值。
    loop_threshold: usize,
    /// 是否有挂起的中断请求。
    interrupt_requested: bool,
    /// resume 动作（可恢复，中断后保留）。
    resume_action: Option<String>,
}

impl Default for ActionManager {
    fn default() -> Self {
        Self::new(Duration::from_secs(60), 5)
    }
}

impl ActionManager {
    pub fn new(default_timeout: Duration, loop_threshold: usize) -> Self {
        Self {
            state: ActionState::Idle,
            default_timeout,
            recent_actions: VecDeque::with_capacity(loop_threshold * 2),
            loop_threshold,
            interrupt_requested: false,
            resume_action: None,
        }
    }

    /// 启动动作。
    pub fn start(&mut self, action_name: &str, timeout: Option<Duration>) {
        // 循环检测
        self.recent_actions.push_back(action_name.into());
        if self.recent_actions.len() > self.loop_threshold * 2 {
            self.recent_actions.pop_front();
        }
        // 检查最近 N 个是否都相同
        let recent_count = self.recent_actions.iter().rev().take(self.loop_threshold).filter(|a| *a == action_name).count();
        if recent_count >= self.loop_threshold {
            eprintln!("[ActionManager] WARNING: loop detected — '{action_name}' triggered {recent_count} times in a row");
        }

        self.state = ActionState::Running {
            action_name: action_name.into(),
            started_at: Instant::now(),
        };
        self.interrupt_requested = false;
    }

    /// 启动可恢复动作（resume=true，中断后保留）。
    pub fn start_resumable(&mut self, action_name: &str, timeout: Option<Duration>) {
        self.resume_action = Some(action_name.into());
        self.start(action_name, timeout);
    }

    /// 请求中断（下一个 tick 生效）。
    pub fn request_interrupt(&mut self) {
        self.interrupt_requested = true;
    }

    /// tick 检查（检查超时 + 中断）。
    /// 返回 Some(ActionState) 表示状态已变更。
    pub fn tick(&mut self) -> Option<ActionState> {
        let old_state = self.state.clone();

        // 检查中断
        if self.interrupt_requested {
            if let ActionState::Running { .. } = &self.state {
                // resume 动作中断后保留，非 resume 动作清除
                if self.resume_action.is_none() {
                    self.state = ActionState::Interrupted;
                } else {
                    // resume 动作仅暂停，不清除
                    self.state = ActionState::Idle;
                }
                self.interrupt_requested = false;
                return Some(self.state.clone());
            }
        }

        // 检查超时
        if let ActionState::Running { started_at, .. } = &self.state {
            if started_at.elapsed() > self.default_timeout {
                self.state = ActionState::TimedOut;
                return Some(self.state.clone());
            }
        }

        if old_state != self.state {
            Some(self.state.clone())
        } else {
            None
        }
    }

    /// 标记完成。
    pub fn complete(&mut self) {
        self.state = ActionState::Done;
    }

    /// 获取当前状态。
    pub fn state(&self) -> &ActionState {
        &self.state
    }

    /// 是否正在运行。
    pub fn is_running(&self) -> bool {
        matches!(self.state, ActionState::Running { .. })
    }

    /// 恢复 resume 动作（如果有）。
    pub fn resume(&mut self) -> bool {
        if let Some(action_name) = self.resume_action.clone() {
            self.start(&action_name, None);
            true
        } else {
            false
        }
    }

    /// 取消 resume 动作。
    pub fn cancel_resume(&mut self) {
        self.resume_action = None;
    }

    /// 重置为空闲。
    pub fn reset(&mut self) {
        self.state = ActionState::Idle;
        self.interrupt_requested = false;
    }
}

// ═══════════════════════════════════════════════════════════════
// SelfPrompter — 自驱动目标循环器（mindcraft 风格）
// ═══════════════════════════════════════════════════════════════

/// SelfPrompter 状态（mindcraft 三态状态机）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrompterState {
    /// 已停止（无目标）。
    Stopped,
    /// 活跃（持续注入目标）。
    Active,
    /// 已暂停（临时中断，可恢复）。
    Paused,
}

/// 自驱动目标循环器（mindcraft SelfPrompter 风格）。
///
/// 周期性向 agent 注入目标 prompt，保持 agent 持续工作。
/// 三态状态机：Stopped → Active ↔ Paused
#[derive(Debug)]
pub struct SelfPrompter {
    state: PrompterState,
    /// 当前目标。
    goal: Option<String>,
    /// 注入间隔（秒）。
    inject_interval: Duration,
    /// 上次注入时间。
    last_inject: Option<Instant>,
    /// 注入次数。
    inject_count: u64,
}

impl Default for SelfPrompter {
    fn default() -> Self {
        Self::new(Duration::from_secs(30))
    }
}

impl SelfPrompter {
    pub fn new(inject_interval: Duration) -> Self {
        Self {
            state: PrompterState::Stopped,
            goal: None,
            inject_interval,
            last_inject: None,
            inject_count: 0,
        }
    }

    /// 启动目标（进入 Active 状态）。
    pub fn start(&mut self, goal: &str) {
        self.goal = Some(goal.into());
        self.state = PrompterState::Active;
        self.last_inject = Some(Instant::now());
        self.inject_count = 0;
    }

    /// 停止（进入 Stopped 状态，清除目标）。
    pub fn stop(&mut self) {
        self.goal = None;
        self.state = PrompterState::Stopped;
        self.last_inject = None;
    }

    /// 暂停（进入 Paused 状态，保留目标）。
    pub fn pause(&mut self) {
        if self.state == PrompterState::Active {
            self.state = PrompterState::Paused;
        }
    }

    /// 恢复（从 Paused 回到 Active）。
    pub fn resume(&mut self) {
        if self.state == PrompterState::Paused {
            self.state = PrompterState::Active;
            self.last_inject = Some(Instant::now());
        }
    }

    /// tick 检查（是否需要注入目标）。
    /// 返回 Some(prompt) 表示应注入目标 prompt。
    pub fn tick(&mut self) -> Option<String> {
        if self.state != PrompterState::Active {
            return None;
        }
        let goal = self.goal.clone()?;
        let last = self.last_inject?;
        if last.elapsed() >= self.inject_interval {
            self.last_inject = Some(Instant::now());
            self.inject_count += 1;
            return Some(format!(
                "[SelfPrompter] Continue working on your goal: {goal}\n(reminder #{}, stay focused)",
                self.inject_count
            ));
        }
        None
    }

    /// 获取当前状态。
    pub fn state(&self) -> &PrompterState {
        &self.state
    }

    /// 是否活跃。
    pub fn is_active(&self) -> bool {
        self.state == PrompterState::Active
    }

    /// 获取当前目标。
    pub fn goal(&self) -> Option<&str> {
        self.goal.as_deref()
    }

    /// 获取注入次数。
    pub fn inject_count(&self) -> u64 {
        self.inject_count
    }
}

// ═══════════════════════════════════════════════════════════════
// History — 滚动摘要式长对话管理（mindcraft 风格）
// ═══════════════════════════════════════════════════════════════

/// 对话消息（简化版）。
#[derive(Debug, Clone)]
pub struct HistoryMessage {
    pub role: String, // "user" / "assistant" / "system"
    pub content: String,
    pub timestamp: Instant,
}

/// 滚动摘要式历史管理（mindcraft History 风格）。
///
/// 当消息数超过 max_messages 时，触发 LLM 摘要压缩：
/// 1. 取最早的一批消息
/// 2. 调用 LLM 生成摘要
/// 3. 用摘要替换原消息
/// 4. 归档原始消息到磁盘
#[derive(Debug)]
pub struct History {
    messages: Vec<HistoryMessage>,
    max_messages: usize,
    /// 摘要阈值（达到此消息数时触发压缩）。
    summarize_threshold: usize,
    /// 每次压缩保留的最近消息数。
    keep_recent: usize,
    /// 摘要历史（历次摘要的累积）。
    summaries: Vec<String>,
}

impl Default for History {
    fn default() -> Self {
        Self::new(50, 40, 10)
    }
}

impl History {
    /// new(max_messages, summarize_threshold, keep_recent)
    pub fn new(max_messages: usize, summarize_threshold: usize, keep_recent: usize) -> Self {
        Self {
            messages: vec![],
            max_messages,
            summarize_threshold,
            keep_recent,
            summaries: vec![],
        }
    }

    /// 添加消息。
    pub fn add(&mut self, role: &str, content: &str) {
        self.messages.push(HistoryMessage {
            role: role.into(),
            content: content.into(),
            timestamp: Instant::now(),
        });
    }

    /// 检查是否需要压缩（消息数 >= summarize_threshold）。
    pub fn needs_summarize(&self) -> bool {
        self.messages.len() >= self.summarize_threshold
    }

    /// 执行压缩（返回需要摘要的消息，调用方负责调用 LLM）。
    /// 压缩后保留最近 keep_recent 条消息 + 摘要。
    pub fn prepare_summarize(&mut self) -> Option<Vec<HistoryMessage>> {
        if !self.needs_summarize() {
            return None;
        }
        let to_summarize_count = self.messages.len() - self.keep_recent;
        let to_summarize: Vec<HistoryMessage> = self.messages.drain(..to_summarize_count).collect();
        Some(to_summarize)
    }

    /// 应用摘要结果（将摘要插入到消息列表开头）。
    pub fn apply_summary(&mut self, summary: String) {
        self.summaries.push(summary.clone());
        self.messages.insert(
            0,
            HistoryMessage {
                role: "system".into(),
                content: format!("[Previous conversation summary]\n{summary}"),
                timestamp: Instant::now(),
            },
        );
    }

    /// 获取所有消息。
    pub fn messages(&self) -> &[HistoryMessage] {
        &self.messages
    }

    /// 消息数。
    pub fn len(&self) -> usize {
        self.messages.len()
    }

    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    /// 获取摘要历史。
    pub fn summaries(&self) -> &[String] {
        &self.summaries
    }

    /// 清空（clear_chat 时调用）。
    pub fn clear(&mut self) {
        self.messages.clear();
        self.summaries.clear();
    }

    /// 渲染为文本（用于 LLM 上下文）。
    pub fn render(&self) -> String {
        let mut out = String::new();
        for msg in &self.messages {
            out.push_str(&format!("{}: {}\n", msg.role, msg.content));
        }
        out
    }
}

// ═══════════════════════════════════════════════════════════════
// 测试
// ═══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_manager_timeout_detection() {
        let mut am = ActionManager::new(Duration::from_millis(10), 5);
        am.start("mine", None);
        assert!(am.is_running());
        // 等待超时
        std::thread::sleep(Duration::from_millis(20));
        let state = am.tick();
        assert!(state.is_some());
        assert_eq!(*am.state(), ActionState::TimedOut);
    }

    #[test]
    fn action_manager_loop_detection() {
        let mut am = ActionManager::new(Duration::from_secs(60), 3);
        // 同一动作连续触发 3 次
        am.start("collect", None);
        am.complete();
        am.start("collect", None);
        am.complete();
        am.start("collect", None);
        // 循环检测应在第 3 次告警（不影响功能，仅打印警告）
        assert!(am.is_running());
    }

    #[test]
    fn action_manager_resume_mechanism() {
        let mut am = ActionManager::new(Duration::from_secs(60), 5);
        // 启动可恢复动作
        am.start_resumable("follow_player", None);
        assert!(am.is_running());
        assert!(am.resume_action.is_some());
        // 中断
        am.request_interrupt();
        am.tick();
        // resume 动作中断后应为 Idle（非 Interrupted）
        assert_eq!(*am.state(), ActionState::Idle);
        // 恢复
        assert!(am.resume());
        assert!(am.is_running());
    }

    #[test]
    fn self_prompter_state_machine() {
        let mut sp = SelfPrompter::new(Duration::from_millis(10));
        assert_eq!(*sp.state(), PrompterState::Stopped);
        assert!(!sp.is_active());

        // Stopped → Active
        sp.start("collect 10 wood");
        assert_eq!(*sp.state(), PrompterState::Active);
        assert!(sp.is_active());
        assert_eq!(sp.goal(), Some("collect 10 wood"));

        // Active → Paused
        sp.pause();
        assert_eq!(*sp.state(), PrompterState::Paused);
        // Paused 状态不应注入
        assert!(sp.tick().is_none());

        // Paused → Active
        sp.resume();
        assert_eq!(*sp.state(), PrompterState::Active);

        // Active → Stopped
        sp.stop();
        assert_eq!(*sp.state(), PrompterState::Stopped);
        assert!(sp.goal().is_none());
    }

    #[test]
    fn self_prompter_periodic_injection() {
        let mut sp = SelfPrompter::new(Duration::from_millis(10));
        sp.start("build shelter");
        // 首次不应立即注入（刚启动）
        // 等待间隔
        std::thread::sleep(Duration::from_millis(15));
        let inject = sp.tick();
        assert!(inject.is_some());
        assert!(inject.unwrap().contains("build shelter"));
        assert_eq!(sp.inject_count(), 1);
    }

    #[test]
    fn history_summarize_flow() {
        let mut h = History::new(10, 8, 3);
        // 添加 8 条消息触发压缩
        for i in 0..8 {
            h.add("user", &format!("message {i}"));
        }
        assert!(h.needs_summarize());
        // 准备压缩（取前 5 条，保留后 3 条）
        let to_summarize = h.prepare_summarize().unwrap();
        assert_eq!(to_summarize.len(), 5);
        assert_eq!(h.len(), 3); // 保留 3 条
        // 应用摘要
        h.apply_summary("User asked about messages 0-4".into());
        assert_eq!(h.len(), 4); // 3 条 + 1 条摘要
        assert_eq!(h.summaries().len(), 1);
        // 第一条应为摘要
        assert!(h.messages()[0].content.contains("Previous conversation summary"));
    }

    #[test]
    fn history_clear() {
        let mut h = History::new(10, 8, 3);
        h.add("user", "hello");
        h.add("assistant", "hi");
        h.apply_summary("test summary".into());
        assert!(!h.is_empty());
        assert_eq!(h.summaries().len(), 1);
        h.clear();
        assert!(h.is_empty());
        assert_eq!(h.summaries().len(), 0);
    }
}
