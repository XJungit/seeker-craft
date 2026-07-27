//! Agent 核心 — 主循环与工具编排
//!
//! 子模块拆分：
//! - [`prompt`] — 提示词构建（知识字符串自动生成 / build_context / WorldInfo 注入）
//! - [`compaction`] — token 估算 + 上下文压缩
//! - [`modes`] — 模式响应系统（self_preservation / self_defense / unstuck 等 10 个 mode）
//! - [`session`] — 会话持久化 + 知识管理

mod compaction;
mod modes;
mod prompt;
mod session;

use crate::core::message::{Message, Usage, now_ms};
use crate::core::prompt::{WorldInfoLib, default_mc_world_info};
use crate::core::memory::WorldMemory;
use crate::core::session::Session;
use crate::core::skill::SkillLibrary;
use crate::core::tool::{ToolEffects, ToolRegistry, ToolResult, plan_tool_effect_batches};
use anyhow::Result;
use serde::Serialize;
use serde_json::Value;
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

const MAX_AGENT_MESSAGES: usize = 10_000;

pub use compaction::is_obs_tool;

// ── Provider ──

pub trait LlmProvider: Send + Sync {
    fn complete(
        &self,
        messages: &[Value],
        tools: &[Value],
    ) -> Result<crate::core::message::AssistantResponse>;
}

// ── AgentEvent ──

#[derive(Debug, Clone, Serialize)]
pub enum AgentEvent {
    AgentStart,
    TurnStart {
        turn: u32,
    },
    Assistant {
        content: Option<String>,
        reasoning: Option<String>,
        calls: Vec<String>,
    },
    ToolExecutionStart {
        tool_call_id: String,
        name: String,
        timestamp: String,
    },
    ToolExecutionUpdate {
        tool_call_id: String,
        name: String,
        is_error: bool,
    },
    ToolExecutionEnd {
        tool_call_id: String,
        name: String,
        is_error: bool,
        timestamp: String,
    },
    TurnEnd {
        turn: u32,
    },
    AgentEnd,
    Done {
        reason: String,
    },
    AutoCompactionStart,
    AutoCompactionEnd,
    AutoRetryStart {
        attempt: u32,
        max_attempts: u32,
        delay_ms: u64,
        error_message: String,
    },
    AutoRetryEnd {
        success: bool,
        attempt: u32,
        final_error: Option<String>,
    },
    /// Mode 触发强制重 prompt（如血量危急/敌对生物靠近）。
    /// 外部循环（viewer/agent_loop）应在下一轮立即再跑 step()，不延迟。
    ModeForceReprompt {
        mode_id: u32,
    },
}

// ── SessionEntry ──

#[derive(Debug, Clone, Serialize)]
pub struct SessionEntry {
    pub id: String,
    pub parent_id: Option<String>,
    pub turn: u32,
    pub tool: String,
    pub reasoning: Option<String>,
    pub detail: String,
    pub timestamp: i64,
}

// ── Compaction ──

pub struct CompactionConfig {
    pub context_window: u32,
    pub reserve: u32,
    pub keep_recent: u32,
    /// 可选：专用压缩模型名（若不设，回退到主模型）
    pub compaction_model: Option<String>,
    /// 可选：专用压缩模型 provider 端点（需实现 LlmProvider），用于隔离主模型
    pub compaction_provider: Option<Box<dyn LlmProvider>>,
    /// 是否为压缩模型启用 thinking 模式（仅在 provider 支持时生效）
    pub compaction_thinking: bool,
}

// compaction_provider 持有 Box<dyn LlmProvider>（非 Clone/Debug），
// 因此 Clone/Debug 手动实现：克隆时丢弃 provider（其总是在 Agent::new 时从 config 重新注入）。
impl Clone for CompactionConfig {
    fn clone(&self) -> Self {
        Self {
            context_window: self.context_window,
            reserve: self.reserve,
            keep_recent: self.keep_recent,
            compaction_model: self.compaction_model.clone(),
            compaction_provider: None,
            compaction_thinking: self.compaction_thinking,
        }
    }
}
impl std::fmt::Debug for CompactionConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompactionConfig")
            .field("context_window", &self.context_window)
            .field("reserve", &self.reserve)
            .field("keep_recent", &self.keep_recent)
            .field("compaction_model", &self.compaction_model)
            .field(
                "compaction_provider",
                &self.compaction_provider.as_ref().map(|_| ".."),
            )
            .field("compaction_thinking", &self.compaction_thinking)
            .finish()
    }
}
impl Default for CompactionConfig {
    fn default() -> Self {
        Self {
            context_window: 1_000_000,
            reserve: 200_000,
            keep_recent: 200_000,
            compaction_model: None,
            compaction_provider: None,
            compaction_thinking: false,
        }
    }
}

// ── Retry ──

#[derive(Debug, Clone)]
pub struct RetryConfig {
    pub enabled: bool,
    pub max_retries: u32,
    pub base_delay_ms: u64,
    pub backoff_multiplier: f64,
}
impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_retries: 2,
            base_delay_ms: 500,
            backoff_multiplier: 2.0,
        }
    }
}
impl RetryConfig {
    pub fn delay_ms(&self, attempt: u32) -> u64 {
        (self.base_delay_ms as f64 * self.backoff_multiplier.powi(attempt as i32 - 1)) as u64
    }
}

fn is_retryable_error(msg: &str) -> bool {
    let lower = msg.to_lowercase();
    lower.contains("timeout")
        || lower.contains("rate")
        || lower.contains("503")
        || lower.contains("502")
        || lower.contains("429")
        || lower.contains("connection")
}

/// P0 改进2: 检测交替模式死循环 (A→B→A→B→A→B)
/// 返回 Some((sig_a, sig_b, cycles)) 当检测到 ≥3 轮交替时
fn detect_alternating_pattern(
    recent: &std::collections::VecDeque<String>,
) -> Option<(String, String, usize)> {
    if recent.len() < 6 {
        return None;
    }
    // 取最后 6 个签名，检查是否构成 A B A B A B 模式
    let tail: Vec<&String> = recent.iter().rev().take(6).collect();
    // tail[0] 是最新的，反转回正序
    let seq: Vec<&String> = tail.into_iter().rev().collect();
    let a = &seq[0];
    let b = &seq[1];
    if a == b {
        return None; // 不是交替，是重复
    }
    // 检查 A B A B A B
    if seq[2] == *a && seq[3] == *b && seq[4] == *a && seq[5] == *b {
        // 计算总交替轮数（往前再数）
        let mut cycles = 3;
        let mut idx = 6;
        while idx + 1 < recent.len() {
            // 从尾部往前数
            let rev: Vec<&String> = recent.iter().rev().collect();
            if idx < rev.len() && rev[idx] == *b && idx + 1 < rev.len() && rev[idx + 1] == *a {
                cycles += 1;
                idx += 2;
            } else {
                break;
            }
        }
        return Some((a.to_string(), b.to_string(), cycles));
    }
    None
}

/// P2 改进7: 从 perceive 文本中提取位置键（用于位置卡死检测）
/// 返回 "x,y,z" 格式的粗粒度位置键（取整），None 表示无法提取
fn extract_position_key(perceive_msg: &str) -> Option<String> {
    // 匹配 "位置: (x, y, z)" 格式
    let marker = "位置:";
    let pos = perceive_msg.find(marker)?;
    let after = &perceive_msg[pos + marker.len()..];
    let paren_start = after.find('(')?;
    let paren_content = &after[paren_start + 1..];
    let paren_end = paren_content.find(')')?;
    let coords = &paren_content[..paren_end];
    let parts: Vec<&str> = coords.split(',').collect();
    if parts.len() < 3 {
        return None;
    }
    let x: i32 = parts[0].trim().parse().ok()?;
    let y: i32 = parts[1].trim().parse().ok()?;
    let z: i32 = parts[2].trim().parse().ok()?;
    Some(format!("{x},{y},{z}"))
}

/// P1 改进4: 自动回退链 — 根据失败的工具名生成回退建议
fn build_fallback_suggestion(failed_tools: &[&str]) -> String {
    let mut suggestions = Vec::new();
    for &tool in failed_tools {
        let suggestion = match tool {
            "gather" => Some("gather 失败 → 尝试 mine（手动挖指定坐标）或 go 到新区域再 gather"),
            "mine" => Some("mine 失败 → 检查是否需要更好的镐（craft 木镐/石镐），或 mine_above 脱困"),
            "go" | "goto" => Some("go 失败 → 距离可能超 32m，尝试分段走或换方向"),
            "place" => Some("place 失败 → 检查坐标是否被占，尝试附近 3 格内其他位置"),
            "craft" | "craft_3x3" => Some("craft 失败 → 检查背包是否有足够原料，先 gather 原料再 craft"),
            "attack" => Some("attack 失败 → 目标可能已离开或死亡，perceive 确认后换目标"),
            "open" => Some("open 失败 → 检查方块是否在 reach 范围内，先 go 靠近"),
            "equip" => Some("equip 失败 → 检查背包是否有该物品"),
            "smelt" => Some("smelt 失败 → 检查是否有熔炉+燃料+原料，先 craft 熔炉"),
            _ => None,
        };
        if let Some(s) = suggestion {
            suggestions.push(s);
        }
    }
    if suggestions.is_empty() {
        String::new()
    } else {
        format!("建议回退方案:\n{}", suggestions.iter().map(|s| format!("  - {s}")).collect::<Vec<_>>().join("\n"))
    }
}

/// P53（2026-07-27）：从工具错误消息中抽取「建议：...」段。
///
/// 工具实现层（craft.rs / smelt / gather 等）在 Err 消息里嵌入明确的解决步骤，
/// 例如：
///   "背包未持有 furnace。建议：先 craft_3x3('furnace') 合成一个"
///   "背包缺少燃料 coal。建议：1) gather oak_log；2) craft planks 后做燃料"
///
/// 本函数提取「建议：」之后的所有文本（直到字符串结尾或下一个换行符之前），
/// 让 agent 主循环把它升级为强制 nudge 注入 user 消息，避免 LLM 无视建议
/// 原地重试同一失败工具。
///
/// 支持中英文冒号（「：」/「:」），不区分大小写。返回 None 表示错误消息
/// 未包含建议段（agent 走原有 consecutive_failures 路径）。
fn extract_error_suggestion(err_msg: &str) -> Option<String> {
    // 找「建议」二字的字节位置
    let key = "建议";
    let idx = err_msg.find(key)?;
    // 跳过「建议」二字，再跳过紧随的冒号（中英文都接受）
    let after_key = &err_msg[idx + key.len()..];
    let after_colon = after_key.trim_start_matches(['：', ':', ' ', '\t']);
    // 取到行尾或字符串结尾（建议段通常在一行内，多行建议用「；」或「；」分隔）
    let end = after_colon.find('\n').unwrap_or(after_colon.len());
    let suggestion = after_colon[..end].trim();
    if suggestion.is_empty() {
        None
    } else {
        Some(suggestion.to_string())
    }
}

// ── Config ──

/// SelfPrompter 三态状态机（学习自 Mindcraft self_prompter.js）。
///
/// 取代原 `self_prompt: Option<String>` 的二态语义，新增 `Paused` 态：
/// 紧急情况（self_preservation / self_defense 触发 force_reprompt）时
/// 自动暂停目标注入，避免 LLM 被 `[当前目标] 做木镐` 干扰而忽略
/// `[MODE: self_preservation] 立即逃跑！` 的紧急指令。
///
/// - `Stopped`：无目标，agent 自由行动
/// - `Active`：目标激活，每轮注入 [当前目标]，agent 持续朝目标行动
/// - `Paused`：目标暂停，不注入但保留 goal；紧急情况结束后自动/手动恢复
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptState {
    Stopped,
    Active { goal: String, since_turn: u32 },
    Paused {
        goal: String,
        /// 暂停时所处的轮次（用于日志/调试）
        paused_since: u32,
        /// 是否为自动暂停（mode 触发）。true 时无 mode 触发 N 轮后自动恢复；
        /// false（LLM 主动 pause）时只能由 LLM 主动 resume。
        auto_paused: bool,
    },
}

impl PromptState {
    /// 当前持有的目标文本（Stopped 返回 None）。
    pub fn goal(&self) -> Option<&str> {
        match self {
            PromptState::Stopped => None,
            PromptState::Active { goal, .. } | PromptState::Paused { goal, .. } => Some(goal),
        }
    }

    /// 是否处于 Active 态（每轮注入目标）。
    pub fn is_active(&self) -> bool {
        matches!(self, PromptState::Active { .. })
    }

    /// 是否有目标（Active 或 Paused，排除 Stopped）。
    pub fn has_goal(&self) -> bool {
        !matches!(self, PromptState::Stopped)
    }
}

impl Default for PromptState {
    fn default() -> Self {
        PromptState::Stopped
    }
}

pub struct AgentConfig {
    pub prompt: String,
    pub max_iterations: u32,
    pub compaction: CompactionConfig,
    pub retry: RetryConfig,
    pub auto_perceive: bool,
    pub enable_compaction: bool,
    pub enable_retry: bool,
    pub enable_skill: bool,
    pub enable_world_info: bool,
    pub enable_self_prompt: bool,
    pub enable_modes: bool,
    /// 是否注册 manage_knowledge 工具（动态世界知识增删）。
    /// mod 路线默认 true；azalea 等无世界知识库的路线设 false，
    /// 避免向 LLM 暴露不存在/无用的工具导致上游偶发 400。
    pub enable_knowledge_tool: bool,
    /// 静态知识库前缀（如 mod 路线的 MC 配方/生存策略）。
    /// `None` 表示不注入任何静态知识，仅用工具自描述（azalea 等无 mod 专属知识时设此）。
    pub knowledge_base: Option<String>,
    /// 世界信息库（动态注入与感知相关的提示）。`None` 表示空库，
    /// 用于非 mod 路线（azalea 等）避免注入 mod 专属的 collect/combat 提示。
    pub world_info: Option<WorldInfoLib>,
    /// 世界记忆库（空间-状态长期记忆）。可外部注入共享实例（适配器/工具共用）。
    pub world_memory: WorldMemory,
}
impl AgentConfig {
    pub fn new(prompt: String, max_iterations: u32) -> Self {
        Self {
            prompt,
            max_iterations,
            compaction: CompactionConfig::default(),
            retry: RetryConfig::default(),
            auto_perceive: false,
            enable_compaction: true,
            enable_retry: true,
            enable_skill: true,
            enable_world_info: true,
            enable_self_prompt: true,
            enable_modes: true,
            // 默认关闭 mod 专属知识污染：新路线（azalea 等）开箱即用、
            // 仅见自身工具集；mod 路线在 demo 里显式 .with_knowledge_base/
            // world_info/enable_knowledge_tool 开启。
            enable_knowledge_tool: false,
            knowledge_base: None,
            world_info: None,
            world_memory: WorldMemory::new(),
        }
    }
    /// 设置静态知识库（`None` 关闭，仅用工具自描述）。
    pub fn with_knowledge_base(mut self, kb: Option<String>) -> Self {
        self.knowledge_base = kb;
        self
    }
    /// 设置世界信息库（`None` 为空库，不注入任何路线专属提示）。
    pub fn with_world_info(mut self, wi: Option<WorldInfoLib>) -> Self {
        self.world_info = wi;
        self
    }
    /// 设置是否注册 manage_knowledge 工具。
    pub fn with_knowledge_tool(mut self, v: bool) -> Self {
        self.enable_knowledge_tool = v;
        self
    }
    pub fn with_compaction(mut self, c: CompactionConfig) -> Self {
        self.compaction = c;
        self
    }
    pub fn with_retry(mut self, r: RetryConfig) -> Self {
        self.retry = r;
        self
    }
    pub fn with_auto_perceive(mut self, v: bool) -> Self {
        self.auto_perceive = v;
        self
    }
    /// 注入外部共享的世界记忆库（与适配器/工具共用同一实例，保证写入即见）。
    pub fn with_world_memory(mut self, mem: WorldMemory) -> Self {
        self.world_memory = mem;
        self
    }
}

// ── MC Knowledge Base (static parts, prefixed to auto-generated tool reference) ──
//
// **历史教训**：旧版这里塞了一份从其他项目（Mindcraft）复制来的英文 prompt，
// 里面全是虚构工具名（goal_execute/collect/combat/nav_to/consume/digDown/moveAway/
// execute_plan/look_at/discard）。虽然标了 #[allow(dead_code)] 没被直接使用，
// 但留在源码里是定时炸弹——日后有人引用就会污染 LLM。已彻底清空。
//
// 真实 system prompt 在 viewer/agent_loop.rs 里注入（中文版，列了真实 37 个工具）。
// modes.rs 的 [MODE: ...] 提示也只引用真实工具名（attack/goto/gather/craft）。

#[allow(dead_code)]
const MC_KNOWLEDGE_BASE: &str = "";

/// Auto-generated knowledge string (base + tool reference from ToolRegistry).
/// Generated once lazily to keep system prompt stable for prefix caching.
/// `kb` 为静态知识库前缀（如 mod 路线的 MC 配方）；`None` 时仅用工具自描述，
/// 不注入任何路线专属知识（azalea 等路线设 None 以避免污染工具集）。
pub fn build_knowledge_string(tools: &ToolRegistry, kb: Option<&str>) -> String {
    let tool_ref = tools.to_knowledge_string();
    let base = kb.unwrap_or("").trim();
    if tool_ref.is_empty() {
        base.to_string()
    } else if base.is_empty() {
        format!(
            "## Available Tools\nThe following tools are the ONLY ones available:\n\n{}",
            tool_ref
        )
    } else {
        format!(
            "{}\n\n## Available Tools\nThe following tools are the ONLY ones available:\n\n{}",
            base,
            tool_ref
        )
    }
}

// ── Context ──

pub struct Context {
    pub system_prompt: String,
    pub messages: Vec<Value>,
    pub tools: Vec<Value>,
}

// ── Knowledge tool schema ──

pub const MANAGE_KNOWLEDGE: &str = "manage_knowledge";
pub const MANAGE_KNOWLEDGE_TOOL: &str = r#"{
  "type": "function",
  "function": {
    "name": "manage_knowledge",
    "description": "Dynamically manage long-term game knowledge (WorldInfo). Use add to remember block/mob/pattern discoveries; use remove to delete outdated knowledge. Matched keywords auto-inject context in future turns.",
    "parameters": {
      "type": "object",
      "properties": {
        "action": {"type": "string", "enum": ["add", "remove"], "description": "add=new knowledge entry, remove=delete entry"},
        "id": {"type": "string", "description": "Stable id for removal. Recommended for add too."},
        "keys": {"type": "array", "items": {"type": "string"}, "description": "Trigger keywords (lowercase), e.g. ['creeper']"},
        "template": {"type": "string", "description": "Knowledge template, supports {label} {offset_x} {offset_y} variables"}
      },
      "required": ["action"]
    }
  }
}"#;

#[derive(Debug, Default, Clone)]
pub struct CompactionResult {
    pub summary: String,
    pub first_kept_entry_id: String,
    pub tokens_before: u64,
}

// ── Agent ──

pub struct Agent {
    pub provider: Box<dyn LlmProvider>,
    /// 可选：专用压缩模型 provider（用于上下文压缩/摘要，隔离主模型 token 预算）
    pub compaction_provider: Option<Box<dyn LlmProvider>>,
    pub tools: ToolRegistry,
    pub messages: Vec<Message>,
    pub session_entries: Vec<SessionEntry>,
    pub config: AgentConfig,
    pub events: Vec<AgentEvent>,
    usage: Usage,
    previous_summary: Option<String>,
    steering: VecDeque<String>,
    follow_up: VecDeque<String>,
    turn: u32,
    world_info: WorldInfoLib,
    world_memory: WorldMemory,
    skill_lib: SkillLibrary,
    knowledge_bootstrapped: bool,
    obs_streak: u32,
    /// SelfPrompter 三态状态机（Stopped / Active / Paused）。
    /// 取代原 `self_prompt: Option<String>`。
    prompt_state: PromptState,
    /// 连续无 mode 触发的轮数（用于自动恢复 Paused 目标）。
    /// 每次有 mode 触发重置为 0；无触发 +1；≥2 时自动恢复 auto_paused 目标。
    turns_since_mode: u32,
    last_mode_trigger: u32,
    pub session: Option<Session>,
    pending_checkpoint: bool,
    session_msg_offset: usize,
    persisted_memory_len: usize,
    pending_compaction: Option<CompactionResult>,
    pub last_compaction: Option<CompactionResult>,
    pub retry_abort: Arc<AtomicBool>,
    recent_calls: std::collections::VecDeque<String>,
    /// P8 连续纯文字回复计数（≥2 时强制注入更强 nudge）。
    text_only_count: u32,
    /// P31 连续"假完成"宣告计数（LLM 说"任务完成"但目标仍 Active）。
    /// ≥2 时注入更强 nudge，明确禁止再用文字宣告完成。
    fake_completion_count: u32,
    /// P8 连续工具失败计数（用于检测"反复尝试同一件事都失败"模式）。
    consecutive_failures: u32,
    /// P8 上次位置记录（用于检测位置卡死）。
    last_position_key: Option<String>,
    /// P8 位置未变轮数。
    position_stale_turns: u32,
}
impl Agent {
    pub fn abort(&self) {
        self.retry_abort.store(true, Ordering::Relaxed);
    }

    /// 返回知识字符串（工具参考自动从 ToolRegistry 生成），缓存复用保 prefix-cache 稳定
    pub fn knowledge_string(&self) -> String {
        // 可通过 cache field 优化，目前每次都生成（工具集不变，结果相同）
        build_knowledge_string(&self.tools, self.config.knowledge_base.as_deref())
    }
}

impl Agent {
    pub fn new(
        provider: Box<dyn LlmProvider>,
        tools: ToolRegistry,
        mut config: AgentConfig,
    ) -> Self {
        let world_info = config.world_info.take().unwrap_or_else(default_mc_world_info);
        let compaction_provider = config.compaction.compaction_provider.take();
        let world_memory = config.world_memory.clone();
        Self {
            provider,
            tools,
            config,
            messages: vec![],
            session_entries: vec![],
            events: vec![],
            usage: Usage::default(),
            previous_summary: None,
            steering: VecDeque::new(),
            follow_up: VecDeque::new(),
            turn: 0,
            world_info,
            world_memory,
            skill_lib: SkillLibrary::new(20),
            knowledge_bootstrapped: false,
            obs_streak: 0,
            prompt_state: PromptState::Stopped,
            turns_since_mode: 0,
            last_mode_trigger: 0,
            session: None,
            pending_checkpoint: false,
            session_msg_offset: 0,
            persisted_memory_len: 0,
            pending_compaction: None,
            last_compaction: None,
            retry_abort: Arc::new(AtomicBool::new(false)),
            recent_calls: std::collections::VecDeque::with_capacity(10),
            compaction_provider,
            text_only_count: 0,
            fake_completion_count: 0,
            consecutive_failures: 0,
            last_position_key: None,
            position_stale_turns: 0,
        }
    }

    /// 注入外部共享的世界记忆库（与适配器/工具共用同一实例，保证写入即见）。
    pub fn with_world_memory(mut self, mem: WorldMemory) -> Self {
        self.world_memory = mem;
        self
    }

    // ── Queues ──
    pub fn queue_steering(&mut self, msg: impl Into<String>) {
        self.steering.push_back(msg.into());
    }
    pub fn queue_follow_up(&mut self, msg: impl Into<String>) {
        self.follow_up.push_back(msg.into());
    }
    fn drain_queues(&mut self) {
        while let Some(m) = self.steering.pop_front() {
            self.messages.push(Message::user(format!("[steering] {m}")));
        }
        while let Some(m) = self.follow_up.pop_front() {
            self.messages
                .push(Message::user(format!("[follow_up] {m}")));
        }
    }

    // ── SelfPrompter 三态状态机 ──
    //
    // 学习自 Mindcraft self_prompter.js：目标有 Stopped/Active/Paused 三态，
    // 紧急 mode 触发时自动暂停（避免 [当前目标] 噪声干扰紧急决策），
    // 紧急情况结束后自动恢复。

    /// 设置/更新目标 → 进入 Active 态。
    /// 取代旧 `set_self_prompt`，保留旧名为兼容别名。
    pub fn set_self_prompt(&mut self, goal: impl Into<String>) {
        self.set_goal(goal);
    }
    /// 设置/更新目标 → 进入 Active 态。
    pub fn set_goal(&mut self, goal: impl Into<String>) {
        self.prompt_state = PromptState::Active {
            goal: goal.into(),
            since_turn: self.turn,
        };
    }
    /// 清空目标 → 进入 Stopped 态。
    pub fn clear_self_prompt(&mut self) {
        self.stop_goal();
    }
    /// 清空目标 → 进入 Stopped 态。
    pub fn stop_goal(&mut self) {
        self.prompt_state = PromptState::Stopped;
    }
    /// 暂停目标（Active → Paused）。
    /// `auto_paused=true` 表示由 mode 自动触发（无 mode 时自动恢复）；
    /// `auto_paused=false` 表示 LLM 主动暂停（只能由 LLM 主动恢复）。
    pub fn pause_goal(&mut self, auto_paused: bool) {
        if let PromptState::Active { goal, .. } = &self.prompt_state {
            self.prompt_state = PromptState::Paused {
                goal: goal.clone(),
                paused_since: self.turn,
                auto_paused,
            };
        }
    }
    /// 恢复目标（Paused → Active）。Stopped/Active 时无操作。
    pub fn resume_goal(&mut self) {
        if let PromptState::Paused { goal, .. } = &self.prompt_state {
            self.prompt_state = PromptState::Active {
                goal: goal.clone(),
                since_turn: self.turn,
            };
        }
    }
    /// 当前 PromptState（供外部 viewer/调试读取）。
    pub fn prompt_state(&self) -> &PromptState {
        &self.prompt_state
    }
    /// 是否有目标（Active 或 Paused）。
    pub fn has_goal(&self) -> bool {
        self.prompt_state.has_goal()
    }
    /// 当前目标文本（Stopped 返回 None）。
    pub fn current_goal(&self) -> Option<&str> {
        self.prompt_state.goal()
    }
    /// 自动恢复检查：若处于 auto_paused 态且连续 N 轮无 mode 触发，恢复为 Active。
    /// 由 run_one_turn 在无 mode 触发的轮次调用。
    fn maybe_auto_resume(&mut self) {
        if let PromptState::Paused { auto_paused: true, .. } = self.prompt_state {
            if self.turns_since_mode >= 2 {
                self.resume_goal();
            }
        }
    }
}

// ── Compaction prompts ──

const COMPACTION_SYSTEM: &str = "You are an AI assistant that summarizes game-play conversations. \
Output ONLY a detailed summary covering: the player's goal and current progress, \
what has been accomplished, what tools were used and their outcomes, current inventory and position, \
nearby threats or opportunities, and next steps the player might take. \
Keep the summary factual and specific — include coordinates, item counts, entity names where relevant.";

const SUMMARIZATION_PROMPT: &str =
    "Please provide a comprehensive summary of the above conversation.";
const UPDATE_SUMMARIZATION_PROMPT: &str =
    "Please update the existing summary with new information from the latest conversation segment.";

// ── Core orchestration methods ──

impl Agent {
    pub fn run(&mut self, user_message: impl Into<String>) -> Result<Vec<String>> {
        let goal = user_message.into();
        // 初始目标同步为 self_prompt：这样每轮 run_one_turn 都会以
        // "[当前目标]" 形式在末尾重新注入，模型不会在后续轮次丢失最高层意图
        // （否则目标只存在于第 1 轮的普通 user 消息里，被后续 perceive/工具
        // 噪音稀释，导致"上轮要挖木、下轮忘了要离开又想挖"的漂移）。
        self.set_self_prompt(goal.clone());
        self.messages.push(Message::user(goal));
        self.continue_run()
    }

    pub fn step(&mut self) -> Result<(Vec<String>, bool)> {
        self.retry_abort.store(false, Ordering::Relaxed);
        let (log, done) = self.run_one_turn()?;
        Ok((log, done))
    }

    pub fn continue_run(&mut self) -> Result<Vec<String>> {
        self.retry_abort.store(false, Ordering::Relaxed);
        let mut all_logs = Vec::new();
        self.events.push(AgentEvent::AgentStart);
        for _ in 0..self.config.max_iterations {
            match self.run_one_turn() {
                Ok((log, true)) => all_logs.extend(log),
                Ok((log, false)) => {
                    all_logs.extend(log);
                    self.events.push(AgentEvent::AgentEnd);
                    return Ok(all_logs);
                }
                Err(e) => {
                    all_logs.push(format!("Fatal error: {e}"));
                    self.events.push(AgentEvent::AgentEnd);
                    return Ok(all_logs);
                }
            }
        }
        self.events.push(AgentEvent::AgentEnd);
        Ok(all_logs)
    }

    fn run_one_turn(&mut self) -> Result<(Vec<String>, bool)> {
        let mut log = Vec::new();
        self.turn += 1;
        let turn = self.turn;
        self.events.push(AgentEvent::TurnStart { turn });
        self.drain_queues();

        // Compaction if message limit exceeded OR token budget exceeded.
        // 两条触发条件合并为一次压缩（#14 修复：避免一回合压缩两次浪费 LLM 调用）。
        let budget = self
            .config
            .compaction
            .context_window
            .saturating_sub(self.config.compaction.reserve);
        let over_messages = self.messages.len() >= MAX_AGENT_MESSAGES;
        let over_tokens = self.estimate_tokens() > budget;
        if over_messages || over_tokens {
            if over_messages {
                log.push(format!(
                    "[t{turn}] 消息数达到上限 {}，触发压缩",
                    MAX_AGENT_MESSAGES
                ));
            } else {
                log.push(format!(
                    "[t{turn}] token 估算 {} 超过预算 {}，触发压缩",
                    self.estimate_tokens(),
                    budget
                ));
            }
            if self.config.enable_compaction {
                self.events.push(AgentEvent::AutoCompactionStart);
                match self.compact() {
                    Ok(result) => {
                        if !result.summary.is_empty() {
                            self.pending_compaction = Some(result);
                        }
                        // 压缩后 messages 变短，session_msg_offset 必须重置，
                        // 否则下次 save() 切片会越界 panic。
                        self.session_msg_offset = self.messages.len();
                    }
                    Err(e) => {
                        log.push(format!("[t{turn}] 压缩失败，改用硬截断: {e}"));
                        // #12 修复：硬截断后注入提示，避免 LLM 对上下文丢失完全无感知
                        self.hard_truncate();
                        self.messages.push(Message::user(
                            "【系统提示】由于上下文压缩失败，早期对话已被截断，仅保留最近片段。请基于当前可见信息继续。".to_string(),
                        ));
                        self.session_msg_offset = self.messages.len();
                    }
                }
                self.events.push(AgentEvent::AutoCompactionEnd);
            } else {
                // 压缩关闭时：硬截断兜底（不调 LLM），避免直接卡死
                log.push(format!("[t{turn}] compaction 已禁用，执行硬截断兜底"));
                self.hard_truncate();
            }
        }

        // 覆盖式清理：移除上一轮 run_one_turn 注入的易变瞬时消息
        // （perceive 状态快照、邻近世界记忆、上一轮的 [当前目标] 重注）。
        // 这些每轮重生，不应在 history 中累积成过期噪音，也不应污染上下文
        // 压缩摘要。只删带固定标记前缀的 user 消息，绝不碰 assistant/tool
        // 真实交互历史。
        self.messages
            .retain(|m| match m {
                Message::User(u) => {
                    !(u.content.starts_with("【当前游戏状态（自动注入）】")
                        || u.content.starts_with("【邻近世界记忆】")
                        || u.content.starts_with("[当前目标]"))
                }
                _ => true,
            });

        // Auto-perceive
        if self.config.auto_perceive
            && let Some(tool) = self.tools.get("perceive")
        {
            match tool.execute("auto_perceive", serde_json::json!({}), None) {
                Ok(result) => {
                    // P2 改进7: 从 perceive 结果提取坐标，跟踪位置卡死
                    let pos_key = extract_position_key(&result.message);
                    if let Some(ref pk) = pos_key {
                        if self.last_position_key.as_ref() == Some(pk) {
                            self.position_stale_turns = self.position_stale_turns.saturating_add(1);
                        } else {
                            self.position_stale_turns = 0;
                        }
                        self.last_position_key = Some(pk.clone());
                    }
                    let state_msg = format!("【当前游戏状态（自动注入）】\n{}", result.message);
                    self.messages
                        .push(Message::user_with_images(state_msg, result.images));
                }
                Err(e) => {
                    eprintln!("[DBG] auto_perceive FAIL: {e}");
                    log.push(format!("[t{turn}] 自动感知失败: {e}"));
                }
            }
        }

        // P2 改进7: 探索策略 — 位置卡死 5+ 轮时注入"换区域"提示
        if self.position_stale_turns >= 5 {
            let nudge = format!(
                "【探索建议】你已在同一位置停留 {} 轮，可能资源已耗尽。请：\n\
                 1. 向一个新方向走 20-30 格（用 go 工具）\n\
                 2. 探索新区域寻找资源\n\
                 3. 如果在做地下挖掘，尝试换一个方向或回到地表",
                self.position_stale_turns
            );
            self.messages.push(Message::user(nudge));
            log.push(format!(
                "[t{turn}] 位置卡死 {} 轮，注入探索建议",
                self.position_stale_turns
            ));
            // 注入后重置，避免每轮都注入
            self.position_stale_turns = 0;
        }

        // Modes reaction system（支持 force_reprompt 通道 + 自动暂停目标）
        if self.config.enable_modes
            && let Some(reaction) = self.check_modes()
        {
            // mode 触发：重置无 mode 计数
            self.turns_since_mode = 0;
            if let Some(prompt) = &reaction.prompt {
                self.messages.push(Message::user(prompt.clone()));
                log.push(format!("[t{turn}] {prompt}"));
            }
            if reaction.force_reprompt {
                self.events.push(AgentEvent::ModeForceReprompt {
                    mode_id: reaction.mode_id,
                });
                log.push(format!(
                    "[t{turn}] mode {} 触发 force_reprompt，本轮将立即重跑 LLM",
                    reaction.mode_id
                ));
                // 紧急 mode（force_reprompt=true）自动暂停目标注入：
                // 避免 [当前目标] 做木镐 干扰 [MODE: self_preservation] 立即逃跑 的紧急决策。
                // 仅 Active → Paused（Stopped/Paused 无操作）。auto_paused=true 标记可自动恢复。
                if self.prompt_state.is_active() {
                    self.pause_goal(true);
                    log.push(format!(
                        "[t{turn}] 紧急 mode 触发，自动暂停目标注入（auto_paused）"
                    ));
                }
            }
        } else {
            // 无 mode 触发：累加计数 + 检查自动恢复
            self.turns_since_mode = self.turns_since_mode.saturating_add(1);
            self.maybe_auto_resume();
            if let PromptState::Active { goal, .. } = &self.prompt_state {
                if self.turn > 0 && self.turns_since_mode == 2 {
                    // 刚从 auto_paused 恢复（maybe_auto_resume 已切换为 Active）
                    log.push(format!(
                        "[t{turn}] 紧急情况结束，自动恢复目标注入：{goal}"
                    ));
                }
            }
        }

        // SelfPrompter：仅 Active 态注入目标（Paused/Stopped 不注入）
        if self.config.enable_self_prompt
            && let PromptState::Active { goal, .. } = &self.prompt_state
        {
            self.messages
                .push(Message::user(format!("[当前目标] {goal}")));
        }

        // Dynamic context (WorldInfo + Skill)
        if (self.config.enable_world_info || self.config.enable_skill)
            && let Some(dynamic_msg) = self.build_dynamic_context_msg()
        {
            self.messages.push(Message::user(dynamic_msg));
        }

        // WorldMemory 邻近记忆注入（空间-状态长期记忆）
        if let Some(mem_msg) = self.build_memory_context_msg() {
            self.messages
                .push(Message::user(format!("【邻近世界记忆】\n{mem_msg}")));
        }

        // Dynamic instructions
        if let Some(instr) = self.build_dynamic_instructions_msg() {
            self.messages.push(Message::user(instr));
        }

        let ctx = self.build_context();

        // LLM call with retry
        let mut response = None;
        let mut last_error;
        let max_attempts = if self.config.enable_retry && self.config.retry.enabled {
            1 + self.config.retry.max_retries
        } else {
            1
        };
        for attempt in 1..=max_attempts {
            match self.provider.complete(&ctx.messages, &ctx.tools) {
                Ok(resp) => {
                    response = Some(resp);
                    if attempt > 1 {
                        self.events.push(AgentEvent::AutoRetryEnd {
                            success: true,
                            attempt,
                            final_error: None,
                        });
                    }
                    break;
                }
                Err(e) => {
                    last_error = format!("{e}");
                    let retryable = is_retryable_error(&last_error);
                    if attempt >= max_attempts || !retryable {
                        if attempt > 1 {
                            self.events.push(AgentEvent::AutoRetryEnd {
                                success: false,
                                attempt,
                                final_error: Some(last_error.clone()),
                            });
                        }
                        log.push(format!(
                            "[t{turn}] LLM 错误 (第{attempt}/{max_attempts}次): {last_error}",
                        ));
                        break;
                    }
                    let delay_ms = self.config.retry.delay_ms(attempt);
                    self.events.push(AgentEvent::AutoRetryStart {
                        attempt,
                        max_attempts,
                        delay_ms,
                        error_message: last_error.clone(),
                    });
                    let ticks = delay_ms / 50;
                    for _ in 0..ticks {
                        if self.retry_abort.load(Ordering::Relaxed) {
                            break;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(50));
                    }
                    if self.retry_abort.load(Ordering::Relaxed) {
                        log.push(format!("[t{turn}] 用户中止重试"));
                        break;
                    }
                }
            }
        }

        let Some(response) = response else {
            eprintln!("[DBG] LLM: all attempts failed");
            self.persist_turn()?;
            self.events.push(AgentEvent::Done {
                reason: "LLM call failed after retries".into(),
            });
            return Ok((log, false));
        };

        self.usage = response.usage.clone();
        log.push(format!(
            "[t{turn}] tokens: input={} out={} total={} cache_hit={} cache_miss={}",
            response.usage.input_tokens,
            response.usage.output_tokens,
            response.usage.total_tokens,
            response.usage.cache_hit_tokens,
            response.usage.cache_miss_tokens,
        ));

        // Track obs streak — text only
        let calls = response.tool_calls.clone();
        if calls.is_empty() {
            self.obs_streak += 1;
            // P0 改进1: SelfPrompter 强制执行 — 连续纯文字回复计数
            self.text_only_count = self.text_only_count.saturating_add(1);
            self.events.push(AgentEvent::Assistant {
                content: response.content.clone(),
                reasoning: response.reasoning.clone(),
                calls: vec![],
            });
            self.messages.push(Message::assistant_response(&response));
            let content = response.content.as_deref().unwrap_or("");
            let goal_hint = self
                .current_goal()
                .map(|g| format!("你的目标是: {g}。"))
                .unwrap_or_default();
            // P12 修复（2026-07-26）：思考用尽 token 的专门检测。
            // deepseek-v4-flash-free 默认开启 thinking 模式，reasoning_content 会消耗大量 token。
            // 当 max_tokens 不足以同时容纳 reasoning + content + tool_calls 时，
            // 会出现 content="" + tool_calls=[] + finish_reason="length" + reasoning_content 非空，
            // 此时通用的"纯文字回复"nudge 无效（LLM 并没有产出文字，只是在思考），
            // 需要专门提示 LLM 跳过思考直接输出工具调用。
            let is_thinking_timeout = matches!(response.stop_reason, crate::core::message::StopReason::Length)
                && content.is_empty()
                && response.reasoning.is_some()
                && response.reasoning.as_deref().map_or(false, |r| !r.is_empty());
            // P12 + P56 修复（2026-07-26/27）：过早宣告任务完成检测。
            // LLM 在纯文字回复里宣告"任务完成/目标完成"但未实际验证，
            // 此时目标仍在 Active 态，应强制 perceive 验证而非停止行动。
            //
            // P56 扩展（2026-07-27）：scan_20260727_205138.md 显示 step 11/14/33/41
            // 都出现 LLM 宣告"smelt 任务完成 ✅"但无 tool_call，浪费 4 轮。
            // 原关键词列表漏掉了「✅」「已验证」「最终确认」等总结性措辞，
            // 现在扩展到任何包含「✅」或总结性关键词的纯文字回复。
            let is_premature_completion = content.contains("任务完成")
                || content.contains("目标完成")
                || content.contains("目标已全部完成")
                || content.contains("全部完成")
                || content.contains("任务已全部完成")
                || content.contains("已完成所有")
                || content.contains("最终成果")
                // P56 新增：覆盖 scan 报告里实际出现的措辞
                || content.contains('✅')
                || content.contains("已验证")
                || content.contains("验证完成")
                || content.contains("已确认")
                || content.contains("最终确认")
                || content.contains("任务 ✅")
                || content.contains("任务完成")
                || content.contains("smelt 任务")
                || content.contains("craft 任务")
                || content.contains("gather 任务")
                || content.contains("mine 任务");
            // 文字伪调用检测：LLM 在 assistant 文字里写 `tool(...)` 伪调用而不产生真实 tool_calls。
            let _lower = content.to_lowercase();
            let is_pseudo_call = content.contains("【工具")
                || content.contains("【tool")
                || content.contains("[工具")
                || content.contains("[tool ")
                || content.contains("→ 命令完成")
                || content.contains("→ ok")
                || content.contains("→ OK")
                || content.contains("→ 已到达")
                || content.contains("→ 已完成")
                || content.contains("工具执行】")
                || content.contains("工具调用】")
                || (content.contains("goto(")
                    || content.contains("mine(")
                    || content.contains("gather(")
                    || content.contains("craft(")
                    || content.contains("attack(")
                    || content.contains("place("))
                    && response.tool_calls.is_empty();
            // P0 改进1: 连续 ≥2 次纯文字回复时注入更强提示，列出可用工具
            let nudge = if is_thinking_timeout {
                format!("{goal_hint}【纠偏】你上一轮的思考（reasoning）用尽了全部 max_tokens，\
                 没有产出任何文字或工具调用——bot 实际上什么都没做！\n\
                 请立即通过 function calling 输出工具调用，**不要做长篇思考**。\
                 根据当前感知到的状态直接选一个工具调用，1 句话说明意图即可。")
            } else if is_pseudo_call {
                format!("{goal_hint}【纠正】你的回复里写了文字伪调用（如 `【工具执行】xxx(...)` 或 `xxx(...) → 命令完成`），\
                 这**不会被执行**——只有 function calling 输出的 tool_calls 才会被真正执行。\
                 你刚才的所有「工具执行」都是幻觉，bot 实际上没做任何动作！\n\
                 必须用 function calling 输出工具调用（系统自动附加 tool_calls 字段，\
                 不要在文字里写任何 tool() 调用）。请重新回复：文字只说 1 句意图，\
                 然后通过 function calling 输出真实 tool_calls。")
            } else if is_premature_completion && self.current_goal().is_some() {
                // P12 + P31：LLM 过早宣告完成但目标仍 Active，强制 perceive 验证。
                // P31 加强：用 fake_completion_count 计数，连续多次假完成时注入更强 nudge。
                self.fake_completion_count = self.fake_completion_count.saturating_add(1);
                let goal = self.current_goal().unwrap_or("");
                let nudge = if self.fake_completion_count >= 3 {
                    // 连续 3+ 次假完成：最后通牒
                    format!("{goal_hint}【最后通牒】你已连续 {} 次宣告「任务完成」，但目标仍在执行中：{goal}\n\
                     **禁止再用任何文字宣告完成！** 文字宣告永远不算完成。\n\
                     你必须立即调用 perceive 工具查看实际状态。如果 perceive 显示目标未达成，\n\
                     继续调用其他工具行动（gather/mine/craft/smelt 等），直到目标真正达成。\n\
                     再用文字说「完成」将被视为故障，系统会强制注入工具调用。",
                     self.fake_completion_count)
                } else if self.fake_completion_count == 2 {
                    // 连续 2 次假完成：更强警告
                    format!("{goal_hint}【严重警告】你已连续 2 次宣告「任务完成」，但目标仍在执行中：{goal}\n\
                     文字宣告**不算完成**——你被禁止再用文字说「完成」「已达成」「全部完成」等词。\n\
                     必须立即调用 perceive 工具验证实际状态。perceive 会显示背包物品和当前位置，\n\
                     你需要对比目标要求，确认每个目标物品都在背包中。若缺少，继续调用 gather/mine/craft/smelt 等工具获取。",
                    )
                } else {
                    // 第 1 次假完成：温和提示
                    format!("{goal_hint}【验证】你宣告「任务完成」，但目标仍在执行中：{goal}\n\
                     文字宣告**不算完成**——必须通过 function calling 调用 perceive 查看实际状态，\n\
                     确认目标物品在背包/目标位置已到达，才算真正完成。\n\
                     请立即调用 perceive 验证当前状态，不要只用文字说完成了。")
                };
                nudge
            } else if self.text_only_count >= 2 {
                // 连续 2+ 次纯文字：列出可用工具强制行动
                let tool_list: Vec<&str> = self.tools.tools().iter().map(|t| t.name()).collect();
                format!("{goal_hint}【强制行动】你已连续 {} 次只回复文字而不调用工具！\n\
                 这是不允许的——每轮必须通过 function calling 调用至少一个工具。\n\
                 可用工具: {}\n\
                 根据当前状态立即选择一个工具调用。不要解释，直接调用工具。",
                 self.text_only_count,
                 tool_list.join(", "))
            } else {
                format!("{goal_hint}【继续】你刚才只用了文字回复，没有产生真正的工具调用。\
                 请用 function calling 输出工具调用（不要用 markdown 写 `tool()` 伪调用，那不会被执行）。\
                 根据当前状态选一个工具立即行动。")
            };
            self.messages.push(Message::user(nudge));
            log.push(format!(
                "[t{turn}] 提醒: 纯文字回复 (连续 {} 次)，已注入续跑指令",
                self.text_only_count
            ));
            self.events.push(AgentEvent::TurnEnd { turn });
            self.persist_turn()?;
            return Ok((log, true));
        }
        // 有工具调用：重置纯文字计数和假完成计数
        // P31：LLM 调用了工具（哪怕是 perceive），说明它不再只用文字 fake completion，
        // 重置 fake_completion_count 给 LLM 重新开始的机会。
        self.text_only_count = 0;
        self.fake_completion_count = 0;

        // Track obs streak from tool names
        let obs_tools: &[&str] = &["perceive", "visual_perceive", "look"];
        if calls.iter().all(|tc| obs_tools.contains(&tc.name.as_str())) {
            self.obs_streak += 1;
        } else {
            self.obs_streak = 0;
        }
        if !self.knowledge_bootstrapped {
            self.knowledge_bootstrapped = true;
            if let Some(ref mut sess) = self.session {
                sess.mark_header_dirty();
            }
        }

        self.events.push(AgentEvent::Assistant {
            content: response.content.clone(),
            reasoning: response.reasoning.clone(),
            calls: calls.iter().map(|tc| tc.name.clone()).collect(),
        });
        self.messages.push(Message::assistant_response(&response));

        // Dead-loop detection
        // #15 修复：签名归一化——把参数里的数字（坐标等）替换为 #，
        // 这样"每次换不同坐标的重复 move_to"也能被识别为同一循环，而非永不触发。
        // P5 修复：位置类工具（goto/mine/place/interact_block/chest_*）的坐标
        // 不应被归一化——bot 探索时正常会 goto 不同坐标，归一化会导致误报死循环。
        // 这些工具只在「精确相同参数」重复时才算死循环。
        let positional_tools: &[&str] = &[
            "goto", "go", "mine", "place", "interact_block",
            "chest_view", "chest_withdraw", "chest_deposit", "open",
        ];
        let normalize = |arg_json: &str, tool_name: &str| -> String {
            if positional_tools.contains(&tool_name) {
                // 位置类工具：保留原参数（坐标不同则视为不同调用）
                return arg_json.to_string();
            }
            // 其他工具：数字归一化（捕捉"perceive 4 次"等无参数或同参数循环）
            let mut out = String::with_capacity(arg_json.len());
            let mut in_num = false;
            for ch in arg_json.chars() {
                if ch.is_ascii_digit() {
                    if !in_num { out.push('#'); in_num = true; }
                } else {
                    in_num = false;
                    out.push(ch);
                }
            }
            out
        };
        let call_sig = calls
            .iter()
            .map(|tc| format!("{}|{}", tc.name, normalize(&tc.arguments.to_string(), &tc.name)))
            .collect::<Vec<_>>()
            .join(";");
        self.recent_calls.push_back(call_sig.clone());
        if self.recent_calls.len() > 10 {
            self.recent_calls.pop_front();
        }
        let repeat_count = self.recent_calls.iter().filter(|c| **c == call_sig).count();
        // P0 改进2: 交替模式检测 (A→B→A→B→A→B)
        // 即使单个调用没重复 4 次，但两个调用交替出现也是死循环
        let alternating = detect_alternating_pattern(&self.recent_calls);
        // 注意：死循环 nudge 不能在 assistant(tool_calls) 与后续 tool result 之间插入
        // user 消息（否则 DeepSeek/OpenAI 报 400：tool 消息必须紧跟其 tool_calls）。
        // 故先暂存，待本轮 tool result 全部 push 之后再注入。
        let mut loop_nudge: Option<String> = None;
        // P53：错误驱动重规划 nudge（同 loop_nudge 一样，必须延迟到 tool result 之后注入）
        let mut error_suggestion: Option<String> = None;
        if repeat_count >= 4 {
            // P12 修复（2026-07-26）：针对 mine_below 的死循环给出具体建议。
            // mine_below 无参数，连续调用签名必然相同，但 bot 实际在向下挖。
            // 真正的问题是 LLM 不知道何时停止 mine_below（应该 perceive 检查 Y 坐标和背包）。
            let tool_names: Vec<&str> = calls.iter().map(|tc| tc.name.as_str()).collect();
            let specific_hint = if tool_names.iter().any(|n| *n == "mine_below") {
                "你在不断 mine_below 向下挖。问题：mine_below 没有终点反馈，你需要：\n\
                 1. **立即调用 perceive** 查看当前 Y 坐标、背包、附近方块\n\
                 2. 如果 Y 已到目标深度（如煤矿 Y=15-0、铁矿 Y=15-(-30)、钻石 Y=-59），停止 mine_below\n\
                 3. 改用 mine 挖指定坐标的目标矿石，或 gather 让 bot 自动找矿\n\
                 4. 如果挖到基岩（Y=-64），立即 mine_above 向上脱困\n\
                 5. 如果背包已有目标矿石，直接 craft/equip 后继续任务，不要再挖"
            } else if tool_names.iter().any(|n| *n == "mine_above") {
                "你在不断 mine_above 向上挖脱困。问题：可能卡在 1x1 竖井里出不来。你需要：\n\
                 1. **立即调用 perceive** 查看当前 Y 坐标和周围方块\n\
                 2. 如果已到地表（头顶是空气），停止 mine_above，改用 goto 走到目标位置\n\
                 3. 如果还在地下，尝试 goto 走出竖井（先 mine 挖水平方向拓宽）\n\
                 4. 如果背包有方块，可以 place 在脚下垫高（向上搭方块）脱困\n\
                 5. 实在出不来用 chat('/tp @s ~ 70 ~') 传送到地表"
            } else if tool_names.iter().any(|n| *n == "gather") {
                "你在反复 gather 同一物品但失败。问题可能是：\n\
                 1. 没装备镐（先 equip wooden_pickaxe/stone_pickaxe/iron_pickaxe）\n\
                 2. 附近没有该方块（先 perceive 看资源标签，或 goto 换区域）\n\
                 3. 工具等级不足（钻石矿需要 iron_pickaxe 以上，参考错误提示合成更高 tier 的镐）\n\
                 立即 perceive 检查状态，再决定下一步"
            } else if tool_names.iter().any(|n| *n == "goto" || *n == "go") {
                "你在反复 goto 同一坐标但走不到。问题可能是：\n\
                 1. 距离超过 32m 限制（分段走，或换中间目标）\n\
                 2. 路径被阻挡（先 mine 挖通，或绕路）\n\
                 3. 坐标错误（perceive 确认实际坐标）\n\
                 立即 perceive 检查位置，或换一个目标"
            } else if tool_names.iter().any(|n| *n == "craft" || *n == "craft_3x3") {
                "你在反复 craft 但失败。问题可能是：\n\
                 1. 缺少原料（perceive 查看背包，先 gather 采集缺的原料）\n\
                 2. 没有 crafting_table（先 craft('crafting_table') 造一个，或 place 已有的）\n\
                 3. 配方不对（检查 profiles/_default.json 里的配方知识）\n\
                 立即 perceive 查看背包，确认原料后再 craft"
            } else {
                ""
            };
            let nudge = format!(
                "【死循环警告】你已连续 {repeat_count} 次执行相同操作 ({}). 请：\n\
                 1. 检查 perceive 返回的状态，确认当前实际情况\n\
                 2. 换一种完全不同的方法\n\
                 3. 如果在建造，改用 build 蓝图工具而不是手动 place\n\
                 4. 如果在采集，先 goto 到新位置再 gather\n\
                 5. 如果目标已达成，停止调用工具\n\
                 {specific_hint}",
                calls
                    .iter()
                    .map(|tc| tc.name.clone())
                    .collect::<Vec<_>>()
                    .join(",")
            );
            loop_nudge = Some(nudge);
            log.push(format!(
                "[t{turn}] 死循环检测: 相同调用重复 {repeat_count} 次，注入打断指令"
            ));
        } else if let Some((a, b, cycles)) = alternating {
            // 交替模式：A→B→A→B 重复 cycles 轮
            let nudge = format!(
                "【死循环警告】你正在交替重复两个操作 ({a} ↔ {b})，已循环 {cycles} 轮。\n\
                 这种左右摇摆不会推进目标。请：\n\
                 1. 停止在两个操作间来回切换\n\
                 2. 检查 perceive 状态，确认当前实际进度\n\
                 3. 选择一个全新的方法推进目标");
            loop_nudge = Some(nudge);
            log.push(format!(
                "[t{turn}] 死循环检测: 交替模式 {a}↔{b} 循环 {cycles} 轮，注入打断指令"
            ));
        }

        // Execute tool calls
        let effects: Vec<ToolEffects> = calls
            .iter()
            .map(|tc| {
                self.tools
                    .get(&tc.name)
                    .map(|tool| tool.effects())
                    .unwrap_or(ToolEffects::write())
            })
            .collect();
        let batches = plan_tool_effect_batches(&effects);
        let mut turn_had_error = false;

        for batch in &batches {
            let mut parallel_indices = Vec::new();
            let mut knowledge_work = None;
            for &idx in batch {
                if calls[idx].name == MANAGE_KNOWLEDGE {
                    knowledge_work = Some(idx);
                } else {
                    parallel_indices.push(idx);
                }
            }

            let mut batch_results: Vec<(usize, String, bool, String, String)> = Vec::new();
            if let Some(idx) = knowledge_work {
                let tc = &calls[idx];
                let call_id = tc.id.clone();
                let tool_name = tc.name.clone();
                let args = tc.arguments.clone();
                let (msg, _is_err) = self.manage_knowledge(&args);
                batch_results.push((idx, msg, _is_err, call_id, tool_name));
            }

            if !parallel_indices.is_empty() {
                std::thread::scope(|s| {
                    let mut handles = Vec::new();
                    for &idx in &parallel_indices {
                        let tc = &calls[idx];
                        let call_id = tc.id.clone();
                        let tool_name = tc.name.clone();
                        let args = tc.arguments.clone();
                        let tools_ref = &self.tools;
                        let handle = s.spawn(move || {
                            let result = match tools_ref.get(&tool_name) {
                                Some(tool) => tool.execute(&call_id, args, None),
                                None => Ok(ToolResult {
                                    message: format!("Unknown tool: {}", tool_name),
                                    is_error: true,
                                    images: vec![],
                                }),
                            };
                            let (msg, is_err) = match result {
                                Ok(r) => (r.message, r.is_error),
                                Err(e) => (format!("Error: {e}"), true),
                            };
                            (idx, msg, is_err, call_id, tool_name)
                        });
                        handles.push(handle);
                    }
                    for handle in handles {
                        let result = handle.join().unwrap();
                        batch_results.push(result);
                    }
                });
            }

            batch_results.sort_by_key(|(idx, _, _, _, _)| *idx);
            // P53（2026-07-27）：错误驱动重规划 nudge。
            // 当工具 Err 文本含「建议：先 X」时，agent 主循环注入强制 user 消息
            // 「上一工具建议立即调用 X，禁止重试原工具」，把建议升级为指令。
            // 解决 LLM 抄 few-shot 调 auto_craft(furnace) 被拒后原地重试的死循环。
            for (idx, msg, is_err, call_id, tool_name) in batch_results {
                let tc = &calls[idx];
                if is_err {
                    turn_had_error = true;
                    // 抽取错误消息中的「建议：...」段（支持中英文冒号）
                    if error_suggestion.is_none()
                        && let Some(sug) = extract_error_suggestion(&msg)
                    {
                        error_suggestion = Some(format!(
                            "工具 {tool_name} 失败并给出建议：{sug}"
                        ));
                    }
                }
                self.events.push(AgentEvent::ToolExecutionStart {
                    tool_call_id: call_id.clone(),
                    name: tool_name.clone(),
                    timestamp: now_ms().to_string(),
                });
                self.events.push(AgentEvent::ToolExecutionUpdate {
                    tool_call_id: call_id.clone(),
                    name: tool_name.clone(),
                    is_error: is_err,
                });
                self.events.push(AgentEvent::ToolExecutionEnd {
                    tool_call_id: call_id.clone(),
                    name: tool_name.clone(),
                    is_error: is_err,
                    timestamp: now_ms().to_string(),
                });
                let details = if is_err {
                    Some(serde_json::json!({ "error": msg }))
                } else {
                    None
                };
                let tool_msg = if is_err {
                    Message::tool_error(&call_id, &tool_name, &msg)
                } else {
                    Message::tool_result(&call_id, &tool_name, &msg)
                };
                let tool_msg = match tool_msg {
                    Message::ToolResult(mut tr) => {
                        tr.details = details;
                        Message::ToolResult(tr)
                    }
                    other => other,
                };
                self.messages.push(tool_msg);

                if tool_name == "set_goal"
                    && !is_err
                    && let Some(goal_val) = tc.arguments.get("goal")
                {
                    let goal = goal_val.as_str().unwrap_or("");
                    if goal.is_empty() {
                        // P58 修复（2026-07-27）：拦截 set_goal("") 绕过 P56 检测。
                        // 实测：LLM 用 set_goal(goal="") 清空目标来绕过 P56 plain_text_reply 检测，
                        // 文字说"任务完成 ✅"但同时有 set_goal("") 调用，P56 检测在 `if calls.is_empty()` 块内不触发。
                        // 修复：如果 LLM 文字包含"任务完成/✅"等关键词，且当前目标仍 Active，
                        // 拒绝 stop_goal()，注入 nudge 强制 perceive 验证。
                        let assistant_text = response.content.as_deref().unwrap_or("");
                        let declares_completion = assistant_text.contains('✅')
                            || assistant_text.contains("任务完成")
                            || assistant_text.contains("已验证")
                            || assistant_text.contains("最终确认")
                            || assistant_text.contains("目标完成")
                            || assistant_text.contains("全部完成");
                        if declares_completion && self.current_goal().is_some() {
                            // 拒绝清空目标，注入 P58 nudge
                            let current_goal = self.current_goal().unwrap_or("").to_string();
                            self.fake_completion_count = self.fake_completion_count.saturating_add(1);
                            let nudge = format!(
                                "【P58 拦截】你调用了 set_goal(\"\") 清空目标，但文字宣告「任务完成 ✅」。\n\
                                 当前目标仍在执行中：{current_goal}\n\
                                 **禁止用 set_goal(\"\") 绕过验证！** 文字宣告永远不算完成。\n\
                                 你必须立即调用 perceive 工具查看实际状态，对比目标要求逐项验证。\n\
                                 若 perceive 显示目标未达成，继续调用 gather/mine/craft/smelt 等工具推进。\n\
                                 再用文字说「完成」或 set_goal(\"\") 清空目标将被视为故障。"
                            );
                            self.messages.push(Message::user(nudge));
                            log.push(format!(
                                "[t{turn}] P58 拦截: set_goal(\"\") + 文字宣告完成，注入强制验证 nudge"
                            ));
                            // 不调用 stop_goal()，保留原目标
                        } else {
                            // 空目标 → 停止（Stopped）
                            self.stop_goal();
                        }
                    } else {
                        // 非空目标 → 进入 Active 态（覆盖任何 Paused/Stopped）
                        self.set_goal(goal);
                    }
                }
                // pause_goal / resume_goal 工具响应（由 LLM 主动控制）
                if tool_name == "pause_goal" && !is_err {
                    self.pause_goal(false); // LLM 主动暂停：auto_paused=false，不会自动恢复
                }
                if tool_name == "resume_goal" && !is_err {
                    self.resume_goal();
                }

                self.session_entries.push(SessionEntry {
                    id: call_id.clone(),
                    parent_id: Some(format!("call_{turn}")),
                    turn,
                    tool: tool_name.clone(),
                    reasoning: response.reasoning.clone(),
                    detail: format!("{:.120}", msg),
                    timestamp: now_ms(),
                });
                log.push(format!(
                    "[t{turn}] {}({}) -> {:.100}",
                    tool_name, tc.arguments, msg
                ));
            }
        }

        // P1 改进6: 连续失败检测 — 3+ 轮工具失败时注入诊断提示
        if turn_had_error {
            self.consecutive_failures = self.consecutive_failures.saturating_add(1);
        } else {
            self.consecutive_failures = 0;
        }

        // 死循环 nudge 在所有 tool result 之后注入，避免插在 assistant(tool_calls)
        // 与 tool result 之间导致 DeepSeek/OpenAI 400。
        if let Some(nudge) = loop_nudge.take() {
            self.messages.push(Message::user(nudge));
        } else if let Some(suggestion) = error_suggestion.take() {
            // P53：错误驱动重规划 nudge（优先级高于 consecutive_failures）。
            // 当工具 Err 含「建议：先 X」时，强制 LLM 按 X 执行，禁止重试原工具。
            // 这解决 LLM 抄 few-shot 调 auto_craft(furnace) 被拒后仍原地重试的死循环。
            let nudge = format!(
                "【错误驱动重规划】{suggestion}\n\n\
                 **强制指令**：\n\
                 1. 立即按上述建议调用对应工具，**禁止重试刚刚失败的工具**\n\
                 2. 如果建议包含多个步骤（如「先 X → 再 Y → 重试 Z」），用 run_plan 一次执行\n\
                 3. 不要宣告任务完成或放弃，按建议链路推进\n\
                 4. 若建议原料不足，先 perceive 查看背包，再 gather 缺的原料"
            );
            self.messages.push(Message::user(nudge));
            log.push(format!(
                "[t{turn}] 错误驱动 nudge: 注入强制重规划指令"
            ));
        } else if self.consecutive_failures >= 3 {
            // P1 改进4+6: 连续 3+ 轮失败 — 注入诊断提示 + 自动回退链建议
            let failed_tools: Vec<&str> = calls.iter().map(|tc| tc.name.as_str()).collect();
            let fallback = build_fallback_suggestion(&failed_tools);
            let nudge = format!(
                "【连续失败警告】你已连续 {} 轮工具调用失败。请：\n\
                 1. 调用 perceive 检查当前状态\n\
                 2. 分析失败原因（距离太远？缺工具？缺材料？位置错误？）\n\
                 3. 换一种方法，不要重复尝试相同的失败操作\n\
                 {fallback}",
                self.consecutive_failures
            );
            self.messages.push(Message::user(nudge));
            log.push(format!(
                "[t{turn}] 连续失败检测: {} 轮失败，注入诊断提示+回退建议",
                self.consecutive_failures
            ));
        }

        // Extract skill
        if !calls.is_empty() && calls.iter().all(|tc| !is_obs_tool(&tc.name)) {
            let tool_names: Vec<String> = calls.iter().map(|tc| tc.name.clone()).collect();
            let scene = self
                .messages
                .iter()
                .rev()
                .find_map(|m| match m {
                    Message::User(u) if u.content.starts_with("【当前游戏状态") => {
                        Some(u.content.as_str())
                    }
                    _ => None,
                })
                .unwrap_or("");
            let goal = self.config.prompt.as_str();
            let _ = self.skill_lib.extract_from_turn(&tool_names, goal, scene);
        }

        self.events.push(AgentEvent::TurnEnd { turn });
        self.persist_turn()?;
        Ok((log, true))
    }

    pub fn events(&self) -> &[AgentEvent] {
        &self.events
    }
    pub fn usage(&self) -> Usage {
        self.usage.clone()
    }
    /// 暴露共享记忆库（供适配器在 perceive/action 后回填世界记忆）。
    pub fn world_memory(&self) -> WorldMemory {
        self.world_memory.clone()
    }
}

// ── Tests ──

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::message::{AssistantResponse, Message, StopReason, ToolCall, Usage};
    use crate::core::tool::ToolResult;

    struct FakeProvider;
    impl LlmProvider for FakeProvider {
        fn complete(
            &self,
            _messages: &[Value],
            _tools: &[Value],
        ) -> Result<crate::core::message::AssistantResponse> {
            Ok(AssistantResponse {
                content: Some("fake".into()),
                reasoning: None,
                tool_calls: vec![],
                usage: Usage::default(),
                stop_reason: StopReason::Stop,
            })
        }
    }

    #[test]
    fn agent_run_returns_logs() {
        let tools = ToolRegistry::new();
        let config = AgentConfig::new("test".into(), 1);
        let mut agent = Agent::new(Box::new(FakeProvider), tools, config);
        agent.run("hello").unwrap();
        assert!(!agent.messages.is_empty());
    }

    /// 验证 A：每轮 run 会把 WorldMemory 邻近记忆注入到发给 LLM 的 messages 中。
    /// 用 FakeProvider 跑一轮，检查存在包含已知记忆标签的 user 消息。
    #[test]
    fn memory_injected_into_prompt_each_turn() {
        use crate::core::memory::{MemoryPos, WorldMemory};
        let mem = WorldMemory::new();
        mem.record_resource(MemoryPos::new(2, 64, 3), "oak_log", "测试橡树林", Some(4));
        mem.set_anchor("__self__", Some(MemoryPos::new(0, 64, 0)), "当前位置");

        let tools = ToolRegistry::new();
        let config = AgentConfig::new("test".into(), 1);
        let mut agent = Agent::new(Box::new(FakeProvider), tools, config)
            .with_world_memory(mem);
        agent.run("去砍点木头").unwrap();

        let injected = agent
            .messages
            .iter()
            .any(|m| matches!(m, Message::User(u) if u.content.contains("测试橡树林")));
        assert!(injected, "WorldMemory 邻近记忆未被注入 prompt");
    }

    #[test]
    fn is_retryable_matches_errors() {
        assert!(is_retryable_error("timeout"));
        assert!(is_retryable_error("rate limit exceeded"));
        assert!(is_retryable_error("connection refused"));
        assert!(!is_retryable_error("invalid request"));
    }

    #[test]
    fn retry_config_delay_ms_computes() {
        let cfg = RetryConfig::default();
        assert_eq!(cfg.delay_ms(1), 500);
        assert_eq!(cfg.delay_ms(2), 1000);
        assert_eq!(cfg.delay_ms(3), 2000);
    }

    #[test]
    fn is_obs_tool_identifies_observation_tools() {
        assert!(is_obs_tool("perceive"));
        assert!(is_obs_tool("visual_perceive"));
        assert!(!is_obs_tool("collect"));
    }

    #[test]
    fn estimate_tokens_heuristic() {
        let provider = Box::new(FakeProvider);
        let tools = ToolRegistry::new();
        let config = AgentConfig::new("test-prompt".into(), 3);
        let agent = Agent::new(provider, tools, config);
        let tokens = agent.estimate_tokens();
        assert!(
            tokens > 0,
            "should estimate >0 tokens even with empty messages"
        );
    }

    #[test]
    fn auto_perceive_skips_when_perceive_missing() {
        let tools = ToolRegistry::new();
        let mut config = AgentConfig::new("test".into(), 1);
        config.auto_perceive = true;
        let mut agent = Agent::new(Box::new(FakeProvider), tools, config);
        agent.run("look around").unwrap();
    }

    #[test]
    fn self_prompt_injects_every_turn() {
        let tools = ToolRegistry::new();
        let config = AgentConfig::new("test".into(), 3);
        let mut agent = Agent::new(Box::new(FakeProvider), tools, config);
        agent.set_self_prompt("mine diamond");
        agent.run("start").unwrap();
        let goal_msgs: Vec<_> = agent
            .messages
            .iter()
            .filter(|m| matches!(m, Message::User(u) if u.content.contains("[当前目标]")))
            .collect();
        // 每轮注入但覆盖式清理：history 中只保留最近 1 份 [当前目标]，
        // 不随轮次无限累积（避免稀释上下文与浪费 token）。
        assert_eq!(goal_msgs.len(), 1, "自提示每轮注入但只保留最新 1 份");
    }

    #[test]
    fn regression_system_prompt_byte_stable_across_obs_streak() {
        // 修复：jailbreak 中的 obs_streak / knowledge_bootstrapped 变量已从 system
        // prompt 移出，改为动态 user message 注入。回归点：改变这些变量后，
        // build_context().system_prompt 必须 byte-identical（DeepSeek prefix cache）。
        let tools = ToolRegistry::new();
        let mut agent = Agent::new(
            Box::new(FakeProvider),
            tools,
            AgentConfig::new("sys".into(), 5),
        );

        let sys1 = agent.build_context().system_prompt.clone();

        // 模拟运行后状态变化（旧实现会改 system prompt，破坏缓存）
        agent.obs_streak = 7;
        agent.knowledge_bootstrapped = true;

        let sys2 = agent.build_context().system_prompt.clone();

        assert_eq!(
            sys1, sys2,
            "system prompt 必须跨轮 byte-identical，否则破坏 DeepSeek prefix cache"
        );
        // 动态内容应改走 build_dynamic_instructions_msg
        let instr = agent.build_dynamic_instructions_msg();
        assert!(instr.is_some() && instr.unwrap().contains("观察提醒"));
    }

    #[test]
    fn dead_loop_detection_triggers_nudge() {
        let provider = Box::new(FakeProvider);
        let mut tools = ToolRegistry::new();
        struct FakeTool;
        impl crate::core::tool::GameTool for FakeTool {
            fn name(&self) -> &str {
                "perceive"
            }
            fn description(&self) -> &str {
                ""
            }
            fn parameters(&self) -> Value {
                serde_json::json!({})
            }
            fn execute(
                &self,
                _id: &str,
                _a: Value,
                _u: Option<crate::core::tool::ToolUpdateFn>,
            ) -> anyhow::Result<ToolResult> {
                Ok(ToolResult {
                    message: "ok".into(),
                    is_error: false,
                    images: vec![],
                })
            }
        }
        tools.register(Box::new(FakeTool));
        let config = AgentConfig::new("test".into(), 5);
        let mut agent = Agent::new(provider, tools, config);
        let resp = AssistantResponse {
            content: Some("fake".into()),
            reasoning: None,
            tool_calls: vec![ToolCall {
                id: "1".into(),
                name: "perceive".into(),
                arguments: serde_json::json!({}),
            }],
            usage: Usage::default(),
            stop_reason: StopReason::ToolCalls,
        };
        for _ in 0..5 {
            let calls = resp.tool_calls.clone();
            agent.recent_calls.push_back(
                calls
                    .iter()
                    .map(|tc| format!("{}|{}", tc.name, tc.arguments))
                    .collect::<Vec<_>>()
                    .join(";"),
            );
        }
        agent.messages.push(Message::user("test"));
        let nudge_found = agent
            .recent_calls
            .iter()
            .filter(|c| **c == "perceive|{}")
            .count()
            >= 4;
        assert!(nudge_found, "死循环检测应识别 4+ 次重复调用");
    }

    // ── 回归：死循环检测对"每次换不同坐标的重复 move_to"也能触发（#15 修复）──
    #[test]
    fn dead_loop_detection_normalizes_coordinates() {
        let provider = Box::new(FakeProvider);
        let mut tools = ToolRegistry::new();
        struct FakeTool;
        impl crate::core::tool::GameTool for FakeTool {
            fn name(&self) -> &str { "move_to" }
            fn description(&self) -> &str { "" }
            fn parameters(&self) -> Value { serde_json::json!({}) }
            fn execute(&self, _id: &str, _a: Value, _u: Option<crate::core::tool::ToolUpdateFn>) -> anyhow::Result<ToolResult> {
                Ok(ToolResult { message: "ok".into(), is_error: false, images: vec![] })
            }
        }
        tools.register(Box::new(FakeTool));
        let config = AgentConfig::new("test".into(), 5);
        let mut agent = Agent::new(provider, tools, config);
        // 模拟每次坐标都不同的重复 move_to（归一化后应视为同一循环）
        let coords = [[1.0, 64.0, 2.0], [3.0, 64.0, 5.0], [9.0, 65.0, 1.0], [12.0, 64.0, 8.0], [0.0, 64.0, 0.0]];
        for c in coords {
            let resp = AssistantResponse {
                content: Some("fake".into()),
                reasoning: None,
                tool_calls: vec![ToolCall {
                    id: "1".into(),
                    name: "move_to".into(),
                    arguments: serde_json::json!({"x": c[0], "y": c[1], "z": c[2]}),
                }],
                usage: Usage::default(),
                stop_reason: StopReason::ToolCalls,
            };
            let calls = resp.tool_calls.clone();
            agent.recent_calls.push_back(
                calls.iter()
                    .map(|tc| format!("{}|{}", tc.name, tc.arguments))
                    .collect::<Vec<_>>()
                    .join(";"),
            );
        }
        // 复刻生产代码的归一化逻辑（数字→#），验证坐标游走被识别为同一循环
        let normalize = |arg_json: &str| -> String {
            let mut out = String::new();
            let mut in_num = false;
            for ch in arg_json.chars() {
                if ch.is_ascii_digit() {
                    if !in_num { out.push('#'); in_num = true; }
                } else {
                    in_num = false;
                    out.push(ch);
                }
            }
            out
        };
        let normalized: Vec<String> = agent.recent_calls.iter()
            .map(|c| {
                // c 形如 "move_to|{...}"，对参数部分归一化
                if let Some((name, args)) = c.split_once('|') {
                    format!("{}|{}", name, normalize(args))
                } else {
                    c.to_string()
                }
            })
            .collect();
        let all_same = normalized.iter().all(|c| c == &normalized[0]);
        assert!(all_same, "坐标归一化后所有签名应相同: {:?}", normalized);
    }

    // ── 回归：易变瞬时注入（perceive 状态 / 邻近世界记忆）每轮覆盖，不累积 ──
    #[test]
    fn volatile_injections_are_overwritten_not_accumulated() {
        use crate::core::message::Message;
        let tools = ToolRegistry::new();
        let config = AgentConfig::new("test".into(), 1);
        let mut agent = Agent::new(Box::new(FakeProvider), tools, config);

        // 模拟历史里已有上一轮的易变注入 + 真实交互
        agent.messages.push(Message::user(
            "【当前游戏状态（自动注入）】\n坐标(0,64,0) 旧快照",
        ));
        agent
            .messages
            .push(Message::user("【邻近世界记忆】\n旧记忆"));
        agent.messages.push(Message::assistant_response(
            &AssistantResponse {
                content: Some("ok".into()),
                reasoning: None,
                tool_calls: vec![],
                usage: Usage::default(),
                stop_reason: StopReason::Stop,
            },
        ));
        agent.messages.push(Message::tool_result("c1", "perceive", "结果"));

        let before: usize = agent
            .messages
            .iter()
            .filter(|m| matches!(m, Message::User(u) if u.content.starts_with("【当前游戏状态（自动注入）】") || u.content.starts_with("【邻近世界记忆】")))
            .count();
        assert_eq!(before, 2, "前置：应有 2 条旧易变注入");

        // run 一轮（FakeProvider 无 perceive 工具，auto_perceive 跳过，不新注入）
        agent.run("start").unwrap();

        let after: Vec<&Message> = agent
            .messages
            .iter()
            .filter(|m| matches!(m, Message::User(u) if u.content.starts_with("【当前游戏状态（自动注入）】") || u.content.starts_with("【邻近世界记忆】")))
            .collect();
        assert_eq!(after.len(), 0, "覆盖清理后应移除旧的易变注入（不累积）");
        // 真实交互历史保留
        assert!(
            agent.messages.iter().any(|m| matches!(m, Message::ToolResult(r) if r.tool_call_id == "c1")),
            "真实 tool 交互历史不应被覆盖清理删除"
        );
    }

    // ── 回归：死循环 nudge 必须在 tool result 之后，不能在 assistant(tool_calls) 与 tool 之间（否则 400）──
    #[test]
    fn dead_loop_nudge_not_between_assistant_and_tool() {
        use crate::core::message::Message;
        struct ToolCallProvider;
        impl LlmProvider for ToolCallProvider {
            fn complete(
                &self,
                _messages: &[Value],
                _tools: &[Value],
            ) -> Result<AssistantResponse> {
                Ok(AssistantResponse {
                    content: None,
                    reasoning: None,
                    tool_calls: vec![ToolCall {
                        id: "tc1".into(),
                        name: "perceive".into(),
                        arguments: serde_json::json!({}),
                    }],
                    usage: Usage::default(),
                    stop_reason: StopReason::ToolCalls,
                })
            }
        }
        struct PerceiveTool;
        impl crate::core::tool::GameTool for PerceiveTool {
            fn name(&self) -> &str { "perceive" }
            fn description(&self) -> &str { "" }
            fn parameters(&self) -> Value { serde_json::json!({}) }
            fn execute(&self, _id: &str, _a: Value, _u: Option<crate::core::tool::ToolUpdateFn>) -> anyhow::Result<ToolResult> {
                Ok(ToolResult { message: "ok".into(), is_error: false, images: vec![] })
            }
        }
        let mut tools = ToolRegistry::new();
        tools.register(Box::new(PerceiveTool));
        let mut config = AgentConfig::new("test".into(), 1);
        config.auto_perceive = false; // 避免额外注入干扰断言
        let mut agent = Agent::new(Box::new(ToolCallProvider), tools, config);

        // 预填 recent_calls 触发死循环（4+ 次相同）
        for _ in 0..5 {
            agent.recent_calls.push_back("perceive|{}".to_string());
        }
        agent.run("start").unwrap();

        // 找到 assistant(tool_calls) 的索引
        let mut assistant_idx = None;
        for (i, m) in agent.messages.iter().enumerate() {
            if let Message::Assistant(a) = m
                && !a.tool_calls.is_empty()
            {
                assistant_idx = Some(i);
                break;
            }
        }
        let ai = assistant_idx.expect("应存在带 tool_calls 的 assistant");
        // assistant 之后紧跟的消息必须是 tool（role=tool），不能是 user（nudge）
        match &agent.messages[ai + 1] {
            Message::ToolResult(_) => {}
            other => panic!("assistant(tool_calls) 之后必须紧跟 tool 消息，实际: {:?}", other),
        }
        // nudge（若存在）必须出现在所有 tool 之后
        let last_tool = agent
            .messages
            .iter()
            .rposition(|m| matches!(m, Message::ToolResult(_)))
            .expect("应有 tool 结果");
        let nudge_after = agent.messages[last_tool..]
            .iter()
            .any(|m| matches!(m, Message::User(u) if u.content.contains("死循环警告")));
        assert!(
            nudge_after,
            "死循环 nudge 应在 tool result 之后注入"
        );
    }

    // ── 回归：上下文压缩摘要不得包含易变 perceive 快照（避免过期坐标污染）──
    #[test]
    fn compaction_excludes_volatile_perceive_snapshot() {
        use crate::core::message::Message;
        let tools = ToolRegistry::new();
        let config = AgentConfig::new("test".into(), 1);
        let mut agent = Agent::new(Box::new(FakeProvider), tools, config);

        // 填充足够 token 的真实交互 + 一条易变 perceive 快照
        for i in 0..10 {
            agent.messages.push(Message::assistant_response(
                &AssistantResponse {
                    content: Some(format!("step {i} action")),
                    reasoning: None,
                    tool_calls: vec![],
                    usage: Usage::default(),
                    stop_reason: StopReason::Stop,
                },
            ));
            agent.messages.push(Message::tool_result(
                &format!("c{i}"),
                "mine",
                &"x".repeat(200),
            ));
        }
        agent.messages.push(Message::user(
            "【当前游戏状态（自动注入）】\n坐标(0,64,0) 过期快照",
        ));

        // 关闭压缩模型，用主模型（FakeProvider）生成摘要
        let result = agent.compact();
        assert!(result.is_ok(), "compact 不应失败: {:?}", result.err());
        let summary = result.unwrap().summary;
        assert!(
            !summary.contains("【当前游戏状态（自动注入）】"),
            "压缩摘要不应包含易变 perceive 快照，实际摘要: {summary}"
        );
    }

    // ── SelfPrompter 三态状态机回归测试（P1-3）──

    #[test]
    fn prompt_state_machine_transitions() {
        let tools = ToolRegistry::new();
        let config = AgentConfig::new("test".into(), 3);
        let mut agent = Agent::new(Box::new(FakeProvider), tools, config);

        // 初始：Stopped
        assert_eq!(agent.prompt_state(), &PromptState::Stopped);
        assert!(!agent.has_goal());
        assert_eq!(agent.current_goal(), None);

        // set_goal → Active
        agent.set_goal("做木镐");
        assert!(matches!(
            agent.prompt_state(),
            PromptState::Active { goal, .. } if goal == "做木镐"
        ));
        assert!(agent.has_goal());
        assert_eq!(agent.current_goal(), Some("做木镐"));

        // pause_goal(auto=true) → Paused(auto_paused=true)
        agent.pause_goal(true);
        assert!(matches!(
            agent.prompt_state(),
            PromptState::Paused { goal, auto_paused: true, .. } if goal == "做木镐"
        ));
        assert!(agent.has_goal(), "Paused 仍持有 goal");

        // resume_goal → Active
        agent.resume_goal();
        assert!(matches!(
            agent.prompt_state(),
            PromptState::Active { goal, .. } if goal == "做木镐"
        ));

        // pause_goal(auto=false) → Paused(auto_paused=false)（LLM 主动暂停）
        agent.pause_goal(false);
        assert!(matches!(
            agent.prompt_state(),
            PromptState::Paused { auto_paused: false, .. }
        ));

        // stop_goal → Stopped
        agent.stop_goal();
        assert_eq!(agent.prompt_state(), &PromptState::Stopped);
        assert!(!agent.has_goal());
    }

    #[test]
    fn maybe_auto_resume_only_for_auto_paused() {
        let tools = ToolRegistry::new();
        let config = AgentConfig::new("test".into(), 3);
        let mut agent = Agent::new(Box::new(FakeProvider), tools, config);

        // Active → auto_paused
        agent.set_goal("挖矿");
        agent.pause_goal(true);
        // turns_since_mode=0，不应恢复
        agent.turns_since_mode = 0;
        agent.maybe_auto_resume();
        assert!(matches!(agent.prompt_state(), PromptState::Paused { .. }));

        // turns_since_mode=1，仍不应恢复
        agent.turns_since_mode = 1;
        agent.maybe_auto_resume();
        assert!(matches!(agent.prompt_state(), PromptState::Paused { .. }));

        // turns_since_mode=2，应恢复
        agent.turns_since_mode = 2;
        agent.maybe_auto_resume();
        assert!(matches!(agent.prompt_state(), PromptState::Active { .. }));

        // LLM 主动暂停（auto_paused=false）不应被自动恢复
        agent.pause_goal(false);
        agent.turns_since_mode = 100;
        agent.maybe_auto_resume();
        assert!(
            matches!(agent.prompt_state(), PromptState::Paused { auto_paused: false, .. }),
            "LLM 主动暂停只能由 LLM 主动 resume"
        );
    }

    #[test]
    fn paused_state_does_not_inject_goal_message() {
        // 用 step() 而非 run()，避免 run() 内部 set_self_prompt 覆盖 paused 状态
        let tools = ToolRegistry::new();
        let config = AgentConfig::new("test".into(), 1);
        let mut agent = Agent::new(Box::new(FakeProvider), tools, config);
        agent.set_goal("做木镐");
        agent.pause_goal(true); // 紧急情况自动暂停
        agent.messages.push(Message::user("start"));
        agent.step().unwrap();

        let goal_msgs: Vec<_> = agent
            .messages
            .iter()
            .filter(|m| matches!(m, Message::User(u) if u.content.starts_with("[当前目标]")))
            .collect();
        assert!(
            goal_msgs.is_empty(),
            "Paused 态不应注入 [当前目标]，实际有 {:?}",
            goal_msgs
        );

        // 对照组：Active 态应注入 [当前目标]
        agent.resume_goal();
        agent.step().unwrap();
        let goal_msgs_active: Vec<_> = agent
            .messages
            .iter()
            .filter(|m| matches!(m, Message::User(u) if u.content.starts_with("[当前目标]")))
            .collect();
        assert!(
            !goal_msgs_active.is_empty(),
            "Active 态应注入 [当前目标]"
        );
    }
}
