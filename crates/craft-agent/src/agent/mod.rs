//! Agent 核心 — 主循环与工具编排
//!
//! 子模块拆分：
//! - [`prompt`] — 提示词构建（知识字符串自动生成 / build_context / WorldInfo 注入）
//! - [`compaction`] — token 估算 + 上下文压缩
//! - [`modes`] — 模式响应系统（self_preservation / self_defense / unstuck）
//! - [`session`] — 会话持久化 + 知识管理

mod compaction;
mod modes;
mod prompt;
mod session;

use crate::core::message::{Message, Usage, now_ms};
use crate::core::prompt::{WorldInfoLib, default_mc_world_info};
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

// ── Config ──

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
        }
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
}

// ── MC Knowledge Base (static parts, prefixed to auto-generated tool reference) ──

const MC_KNOWLEDGE_BASE: &str = r#"
## Your Role
You are a Minecraft bot. Each turn you receive game state (STATS, HOTBAR, INVENTORY, NEARBY BLOCKS, NEARBY ENTITIES) and must call exactly one tool. Never text-only.

## Crafting Recipes (craft handles automatically)
1 log→4 planks | 2 planks→4 sticks | 4 planks→1 crafting_table
3 planks+2 sticks→wooden_pickaxe/axe/hoe | 2 planks+1 stick→wooden_sword
1 planks+2 sticks→wooden_shovel | 1 stick+1 coal→4 torches
8 cobblestone→1 furnace | 3 cobblestone+2 sticks→stone_pickaxe/axe
8 planks→1 chest | 6 planks→3 door

## Survival Strategy

### Daytime: gather wood, craft tools, find food
1. goal_execute(type="get", param="oak_log", count=8) → goal_execute(type="craft", param="crafting_table") → place("crafting_table", x, y, z)
2. goal_execute(type="craft", param="stone_pickaxe") → goal_execute(type="get", param="stone", count=20)
3. goal_execute(type="craft", param="stone_sword")
4. Hunt animals: goal_execute(type="hunt") — auto finds, kills, collects meat
5. Eat when hungry: consume("beef", 32) or consume("porkchop", 32)

### Evening: build shelter before night
1. PREFER build("dirt_shelter", x, y, z, 0) — auto-builds 3x3 shelter at your position.
2. Or: digDown(3) → look_at(above) → place("dirt") — hide underground
3. If you have wood, craft("torch", 16) → look_at(ground) → place("torch")

### Night: stay safe or fight
1. If shelter built: stay inside, craft items (tools, torches, furnace, chest) using goal_execute(type="craft")
2. Zombies drop only rotten_flesh (worthless + poisonous). Prefer to avoid/flee zombies — not worth fighting.
3. Only fight if cornered/no escape: combat("melee", 200) for zombies/spiders, combat("kite", 200) for skeletons/creeper
4. If health < 8: combat("retreat", 100) to flee, then consume food
5. Light prevents spawns: place torches every 5 blocks in dark areas

### Mining cave exploration
1. Find a cave entrance or digDown(5) to create shaft
2. Place torches as you go down
3. goal_execute(type="get", param="coal", count=10) for fuel → goal_execute(type="get", param="iron_ore", count=20) for iron
4. goal_execute(type="smelt", param="raw_iron", count=20) → goal_execute(type="craft", param="iron_pickaxe")
5. goal_execute(type="craft", param="iron_sword")
6. craft armor: goal_execute(type="craft", param="iron_helmet"), etc.

### Food & health management
1. Hungry (hunger<15): check inventory for edible food → consume("food_name", 32)
2. NEVER eat rotten_flesh: it causes hunger effect (food poisoning) for 30s, making things worse
3. Good food: cooked_beef, cooked_porkchop, cooked_mutton, cooked_chicken, bread, apple, baked_potato, carrot
4. Rotten flesh: WORTHLESS garbage. Discard immediately (do NOT save it). If inventory has rotten_flesh, discard("rotten_flesh", all) right away.
5. No food: goal_execute(type="hunt") — auto hunts and collects meat
6. cook raw meat: goal_execute(type="smelt", param="beef", count=N) → cooked_beef
7. Low health: run away with moveAway(10), eat to regen

## Decision Rules
1. Read auto-injected STATS+HOTBAR: know position, health, hunger, what's in quick-access slots, nearby blocks
2. PREFER goal_execute() for compound tasks (crafting, gathering, smelting) — it handles all sub-steps automatically
3. For complex multi-step tasks, use execute_plan() with a JSON plan array — supports tool calls, if-then-else conditions, loops, and wait. Example: execute_plan(plan='[{"tool":"nav_to","args":{"x":12,"y":64,"z":8}},{"if":{"state":"has_item","args":{"item":"iron_ore","count":3}},"then":[{"tool":"goal_execute","args":{"type":"smelt","param":"raw_iron","count":3}}],"else":[{"tool":"goal_execute","args":{"type":"get","param":"iron_ore","count":3}}]}]')
4. Use collect() for simple single-block gathering
5. Use craft() for simple single-item crafting (goal_execute for complex chains)
6. Place with place() — call look_at(x,y,z) first to aim at surface
7. Navigate with nav_to(x,y,z) — use NEARBY BLOCKS coords
8. Fight with combat(mode, ticks) or attack(ticks); flee with moveAway() if health<8
9. Eat with consume() when hunger<15 — but NEVER eat rotten_flesh (causes hunger effect)
10. Every response MUST end with a tool call. Tool error→retry with adjusted params. No faking success.
11. If a tool returns "Unknown tool", STOP using that tool name — switch to one listed.

## Response Format
  execute_plan(plan='[{"tool":"nav_to","args":{"x":12,"y":64,"z":8}}]') — GOOD (complex plan)
  goal_execute(type="craft", param="iron_pickaxe") — GOOD (compound)
  collect("oak_log", 4) — GOOD
  craft("oak_planks", 8) — GOOD
  nav_to(120, 64, -45) — GOOD
  "I should collect wood" — BAD (text-only)
  "Need to look around first" — BAD (text-only)
"#;

/// Auto-generated knowledge string (base + tool reference from ToolRegistry).
/// Generated once lazily to keep system prompt stable for prefix caching.
pub fn build_knowledge_string(tools: &ToolRegistry) -> String {
    let tool_ref = tools.to_knowledge_string();
    if tool_ref.is_empty() {
        MC_KNOWLEDGE_BASE.to_string()
    } else {
        format!(
            "{}\n\n## Available Tools\nThe following tools are the ONLY ones available:\n\n{}",
            MC_KNOWLEDGE_BASE.trim(),
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
    skill_lib: SkillLibrary,
    knowledge_bootstrapped: bool,
    obs_streak: u32,
    self_prompt: Option<String>,
    last_mode_trigger: u32,
    pub session: Option<Session>,
    pending_checkpoint: bool,
    session_msg_offset: usize,
    pending_compaction: Option<CompactionResult>,
    pub last_compaction: Option<CompactionResult>,
    pub retry_abort: Arc<AtomicBool>,
    recent_calls: std::collections::VecDeque<String>,
}
impl Agent {
    pub fn abort(&self) {
        self.retry_abort.store(true, Ordering::Relaxed);
    }

    /// 返回知识字符串（工具参考自动从 ToolRegistry 生成），缓存复用保 prefix-cache 稳定
    pub fn knowledge_string(&self) -> String {
        // 可通过 cache field 优化，目前每次都生成（工具集不变，结果相同）
        build_knowledge_string(&self.tools)
    }
}

impl Agent {
    pub fn new(
        provider: Box<dyn LlmProvider>,
        tools: ToolRegistry,
        mut config: AgentConfig,
    ) -> Self {
        let world_info = default_mc_world_info();
        let compaction_provider = config.compaction.compaction_provider.take();
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
            skill_lib: SkillLibrary::new(20),
            knowledge_bootstrapped: false,
            obs_streak: 0,
            self_prompt: None,
            last_mode_trigger: 0,
            session: None,
            pending_checkpoint: false,
            session_msg_offset: 0,
            pending_compaction: None,
            last_compaction: None,
            retry_abort: Arc::new(AtomicBool::new(false)),
            recent_calls: std::collections::VecDeque::with_capacity(10),
            compaction_provider,
        }
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

    // ── SelfPrompter ──
    pub fn set_self_prompt(&mut self, goal: impl Into<String>) {
        self.self_prompt = Some(goal.into());
    }
    pub fn clear_self_prompt(&mut self) {
        self.self_prompt = None;
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
        self.messages.push(Message::user(user_message));
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
                    }
                    Err(e) => {
                        log.push(format!("[t{turn}] 压缩失败，改用硬截断: {e}"));
                        // #12 修复：硬截断后注入提示，避免 LLM 对上下文丢失完全无感知
                        self.hard_truncate();
                        self.messages.push(Message::user(
                            "【系统提示】由于上下文压缩失败，早期对话已被截断，仅保留最近片段。请基于当前可见信息继续。".to_string(),
                        ));
                    }
                }
                self.events.push(AgentEvent::AutoCompactionEnd);
            } else {
                // 压缩关闭时：硬截断兜底（不调 LLM），避免直接卡死
                log.push(format!("[t{turn}] compaction 已禁用，执行硬截断兜底"));
                self.hard_truncate();
            }
        }

        // Auto-perceive
        if self.config.auto_perceive
            && let Some(tool) = self.tools.get("perceive")
        {
            match tool.execute("auto_perceive", serde_json::json!({}), None) {
                Ok(result) => {
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

        // Modes reaction system
        if self.config.enable_modes
            && let Some(mode_msg) = self.check_modes()
        {
            self.messages.push(Message::user(mode_msg.clone()));
            log.push(format!("[t{turn}] {mode_msg}"));
        }

        // SelfPrompter
        if self.config.enable_self_prompt
            && let Some(prompt) = &self.self_prompt
        {
            self.messages
                .push(Message::user(format!("[当前目标] {prompt}")));
        }

        // Dynamic context (WorldInfo + Skill)
        if (self.config.enable_world_info || self.config.enable_skill)
            && let Some(dynamic_msg) = self.build_dynamic_context_msg()
        {
            self.messages.push(Message::user(dynamic_msg));
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

        // Track obs streak — text only
        let calls = response.tool_calls.clone();
        if calls.is_empty() {
            self.obs_streak += 1;
            self.events.push(AgentEvent::Assistant {
                content: response.content.clone(),
                reasoning: response.reasoning.clone(),
                calls: vec![],
            });
            self.messages.push(Message::assistant_response(&response));
            let nudge = "【继续】你刚才只用了文字回复。必须调用一个工具。根据当前状态选一个工具立即行动，不要只用文字回复。".to_string();
            self.messages.push(Message::user(nudge));
            log.push(format!("[t{turn}] 提醒: 纯文字回复，已注入续跑指令"));
            self.events.push(AgentEvent::TurnEnd { turn });
            self.persist_turn()?;
            return Ok((log, true));
        }

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
        let call_sig = calls
            .iter()
            .map(|tc| format!("{}|{}", tc.name, tc.arguments))
            .collect::<Vec<_>>()
            .join(";");
        self.recent_calls.push_back(call_sig.clone());
        if self.recent_calls.len() > 10 {
            self.recent_calls.pop_front();
        }
        let repeat_count = self.recent_calls.iter().filter(|c| **c == call_sig).count();
        if repeat_count >= 4 {
            let nudge = format!(
                "【死循环警告】你已连续 {repeat_count} 次执行相同操作 ({}). 请：\n\
                 1. 检查 perceive 返回的状态，确认当前实际情况\n\
                 2. 换一种完全不同的方法\n\
                 3. 如果在建造，改用 build 蓝图工具而不是手动 place\n\
                 4. 如果在采集，先 nav_to 到新位置再 collect\n\
                 5. 如果目标已达成，停止调用工具",
                calls
                    .iter()
                    .map(|tc| tc.name.clone())
                    .collect::<Vec<_>>()
                    .join(",")
            );
            self.messages.push(Message::user(nudge));
            log.push(format!(
                "[t{turn}] 死循环检测: 相同调用重复 {repeat_count} 次，注入打断指令"
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
            for (idx, msg, is_err, call_id, tool_name) in batch_results {
                let tc = &calls[idx];
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
                        self.self_prompt = None;
                    } else {
                        self.self_prompt = Some(goal.to_string());
                    }
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
        assert_eq!(goal_msgs.len(), 3, "自提示应在每轮注入");
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
}
