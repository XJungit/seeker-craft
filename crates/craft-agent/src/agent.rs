//! Agent 核心

use crate::core::message::{Message, Usage, system_chatml};
use crate::core::prompt::{PromptBuilder, WorldInfo, WorldInfoLib, default_mc_world_info};
use crate::core::session::{AgentSnapshot, Session};
use crate::core::tool::{ToolEffects, ToolRegistry, ToolResult, plan_tool_effect_batches};
use anyhow::Result;
use serde::Serialize;
use serde_json::Value;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

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
        name: String,
    },
    ToolExecutionEnd {
        name: String,
        is_error: bool,
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

#[derive(Debug, Clone)]
pub struct CompactionConfig {
    pub context_window: u32,
    pub reserve: u32,
    pub keep_recent: u32,
}
impl Default for CompactionConfig {
    fn default() -> Self {
        Self {
            context_window: 1_000_000,
            reserve: 200_000,
            keep_recent: 200_000,
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
}
impl AgentConfig {
    pub fn new(prompt: String, max_iterations: u32) -> Self {
        Self {
            prompt,
            max_iterations,
            compaction: CompactionConfig::default(),
            retry: RetryConfig::default(),
            auto_perceive: false,
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

// ── MC Knowledge (injected as role_desc, prefix-cached) ──

pub const MC_KNOWLEDGE: &str = r#"
## Your Role
You are a Minecraft bot controlling a player via tools. Each turn you receive game state (STATS, INVENTORY, NEARBY BLOCKS, NEARBY ENTITIES) and must decide the next action. Always respond with exactly one tool call — no text-only responses ever.

## Tool Reference (call exactly as shown)

--- High-Level Tools (preferred) ---

collect(target, count)
  Auto: find nearest target block → aim at it → walk to it → mine it. Your primary gathering tool.
  - target: block ID string. Examples: "oak_log", "birch_log", "stone", "coal_ore", "iron_ore"
  - count: how many to collect (integer, default 1)
  - Usage: collect("oak_log", 4)
  - Returns actual count collected.

craft(item, count)
  Craft items from inventory materials. Mod handles recipe automatically.
  - item: "oak_planks", "stick", "crafting_table", "wooden_pickaxe", "wooden_axe", "wooden_sword"
  - count: how many to craft (integer, default 1)
  - Usage: craft("oak_planks", 8)

place(item)
  Place a block from your hotbar at the targeted position (right-click).
  - item: "crafting_table", "torch", "furnace"
  - Usage: place("crafting_table")
  - Automatically switches to the hotbar slot containing the item, then right-clicks.
  - Look at the ground or surface where you want to place before calling.

--- Utility Tools ---

equip(slot)
  Switch active hotbar slot.
  - slot: number 1-9
  - Usage: equip(3)

use_item(ticks)
  Right-click to eat food, drink potions, open doors, interact with blocks/entities.
  - ticks: hold duration (20 ~ 1 second). Default 20 for eating, 5 for quick use.
  - Usage: use_item(20)

attack(ticks)
  Hold left-click to attack the nearest entity in your crosshair direction.
  - ticks: duration (30 ~ 1.5 seconds), default 30
  - Usage: attack(30)

move_to(x, y, z)
  Navigate to exact world coordinates. Mod handles aiming+movement per tick. Use coordinates from NEARBY BLOCKS section.
  - x/y/z: target position (block y + 0.5 for center)
  - Usage: move_to(-35.0, 68.5, 56.0)

look_at(x, y, z)
  Instantly face a specific world coordinate. More precise than look(dx,dy). Use for accurate aiming before mining.
  - x/y/z: block coordinates from NEARBY BLOCKS
  - Usage: look_at(-35.0, 68.0, 56.0)

--- Fine Control Tools ---

look(dx, dy)
  Rotate camera precisely. dx>0 turns right (~300 units ~ 90 deg), dy>0 looks down.
  - dx: horizontal rotation amount (integer)
  - dy: vertical rotation amount (integer)
  - Usage: look(150, 0)

press(keys, ticks)
  Hold keyboard keys for movement/interaction.
  - keys: "w"/"a"/"s"/"d" (movement), "space" (jump), "shift" (sneak), "e" (inventory)
  - ticks: hold duration (20 ~ 1s), default 20
  - Usage: press("w", 30), press("space", 5)

mine(ticks)
  Hold left-click to mine the targeted block.
  - ticks: 60 for wood/leaves (~3s), 120 for stone (~6s), 120+ for ores
  - Usage: mine(60)

## Crafting Recipes (craft tool handles these automatically)
- 1 oak_log -> 4 oak_planks (any log type works)
- 2 oak_planks -> 4 sticks (any plank type works)
- 4 oak_planks -> 1 crafting_table
- 3 planks + 2 sticks -> 1 wooden_pickaxe
- 3 planks + 2 sticks -> 1 wooden_axe
- 2 planks + 1 stick -> 1 wooden_sword
- 1 stick + 1 coal -> 4 torches

## Decision Rules
1. Read STATS data: position, health, hunger, nearby blocks and entities
2. Gather resources with collect() - it handles aim+walk+mine automatically
3. Craft items with craft() when you have enough materials
4. Place blocks with place() - first ensure you're looking at the target surface
5. If hostile mobs attack you -> attack() to fight back; if health < 8 -> press("w", 40) to flee
6. Every response MUST end with a tool call. Never text-only.
7. Tool error -> retry with adjusted parameters. Don't fake success.

## Response Format
ALWAYS end with a tool call:
  GOOD: collect("oak_log", 4)
  GOOD: craft("oak_planks", 8)
  BAD: "I should collect some wood first"
  BAD: "Now I need to mine the oak log that is nearby"
"#;

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

// ── Agent ──

pub struct Agent {
    pub provider: Box<dyn LlmProvider>,
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
    /// WorldInfo for dynamic knowledge injection
    world_info: WorldInfoLib,
    knowledge_bootstrapped: bool,
    obs_streak: u32,
    /// Session persistence
    pub session: Option<Session>,
    pending_checkpoint: bool,
    session_msg_offset: usize,
    /// Retry abort signal
    pub retry_abort: AtomicBool,
}

impl Agent {
    pub fn new(provider: Box<dyn LlmProvider>, tools: ToolRegistry, config: AgentConfig) -> Self {
        let world_info = default_mc_world_info();
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
            knowledge_bootstrapped: false,
            obs_streak: 0,
            session: None,
            pending_checkpoint: false,
            session_msg_offset: 0,
            retry_abort: AtomicBool::new(false),
        }
    }

    /// Attach session for persistence
    pub fn with_session(mut self, sess: Session) -> Self {
        self.session = Some(sess);
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

    /// Build system prompt with layered prompt pipeline: identity -> role_desc -> scenario -> jailbreak
    fn build_context(&self) -> Context {
        let recent_perception = self
            .messages
            .iter()
            .rev()
            .find_map(|message| match message {
                Message::ToolResult(result) if result.tool_name == "perceive" => {
                    Some(result.content.as_str())
                }
                _ => None,
            })
            .unwrap_or("");
        let dynamic_hints = self.world_info.scan_text(recent_perception, 4_000);

        let mut jailbreak =
            "Act autonomously. If a tool fails, adjust parameters and retry - never pretend success.".to_string();
        if !self.knowledge_bootstrapped {
            jailbreak
                .push_str(" Start executing tasks directly, no need to re-enter game knowledge.");
        }
        if self.obs_streak >= 5 {
            jailbreak.push_str(&format!(
                " [Hint: {} steps observing without action. Pick any tool and act now.]",
                self.obs_streak
            ));
        }

        let mut builder = PromptBuilder::new()
            .identity(&self.config.prompt)
            .role_desc(MC_KNOWLEDGE)
            .jailbreak(jailbreak);
        if !dynamic_hints.is_empty() {
            builder.set_scenario(dynamic_hints.join("\n"));
        }
        let full_prompt = builder.build();
        let system = system_chatml(&full_prompt);
        let mut chatml = vec![system];
        chatml.extend(self.messages.iter().map(Message::to_chatml));
        let mut tool_defs = self.tools.to_openai_defs();
        if let Ok(def) = serde_json::from_str::<Value>(MANAGE_KNOWLEDGE_TOOL) {
            tool_defs.push(def);
        }
        Context {
            system_prompt: full_prompt,
            messages: chatml,
            tools: tool_defs,
        }
    }

    fn estimate_tokens(&self) -> u32 {
        let chars = self.config.prompt.len()
            + MC_KNOWLEDGE.len()
            + self.messages.iter().map(Self::msg_chars).sum::<usize>();
        let estimated = u32::try_from(chars / 3).unwrap_or(u32::MAX);
        let measured = u32::try_from(self.usage.total_tokens).unwrap_or(u32::MAX);
        estimated.max(measured)
    }

    fn msg_chars(m: &Message) -> usize {
        match m {
            Message::User(u) => u.content.len() + u.images.iter().map(|i| i.len()).sum::<usize>(),
            Message::Assistant(a) => {
                a.reasoning.as_deref().map_or(0, |s| s.len())
                    + a.content.as_deref().map_or(0, |s| s.len())
                    + a.tool_calls
                        .iter()
                        .map(|tc| tc.name.len() + tc.arguments.to_string().len())
                        .sum::<usize>()
            }
            Message::ToolResult(r) => {
                r.content.len() + r.images.iter().map(|i| i.len()).sum::<usize>()
            }
        }
    }

    /// Run one turn: push goal message, then execute all iterations.
    pub fn run(&mut self, user_message: impl Into<String>) -> Result<Vec<String>> {
        self.messages.push(Message::user(user_message));
        self.continue_run()
    }

    /// Single-step: execute exactly one turn (LLM call + tool execution).
    /// Returns (log_lines, should_continue). `should_continue` is false when
    /// the agent has reached max iterations or hit a terminal stop reason.
    /// Call this in a loop from external runners (e.g. the control panel viewer).
    pub fn step(&mut self) -> Result<(Vec<String>, bool)> {
        self.retry_abort.store(false, Ordering::Relaxed);
        let (log, done) = self.run_one_turn()?;
        Ok((log, done))
    }

    /// Continue from current state (used by `run` and session recovery).
    pub fn continue_run(&mut self) -> Result<Vec<String>> {
        self.retry_abort.store(false, Ordering::Relaxed);
        let mut all_logs = Vec::new();
        self.events.push(AgentEvent::AgentStart);

        for _ in 0..self.config.max_iterations {
            match self.run_one_turn() {
                Ok((log, true)) => {
                    all_logs.extend(log);
                    // turn completed, continue loop
                }
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

    /// Core: execute a single turn. Returns (log_lines, should_continue).
    /// `should_continue = true` means more iterations are possible;
    /// `false` means terminal (max reached, LLM fatal error, or explicit stop).
    fn run_one_turn(&mut self) -> Result<(Vec<String>, bool)> {
        let mut log = Vec::new();
        self.turn += 1;
        let turn = self.turn;
            self.events.push(AgentEvent::TurnStart { turn });
            self.drain_queues();

            // Compaction check
            let budget = self
                .config
                .compaction
                .context_window
                .saturating_sub(self.config.compaction.reserve);
            if self.estimate_tokens() > budget {
                self.events.push(AgentEvent::AutoCompactionStart);
                if let Err(e) = self.compact() {
                    log.push(format!("[t{turn}] compaction failed: {e}"));
                }
                self.events.push(AgentEvent::AutoCompactionEnd);
            }

            // Auto-perceive: inject latest game state as user message (Mindcraft style, replaced each turn)
            if self.config.auto_perceive {
                if let Some(tool) = self.tools.get("perceive") {
                    match tool.execute("auto_perceive", serde_json::json!({}), None) {
                        Ok(result) => {
                            let state_msg = format!(
                                "【Current Game State (auto-injected)】\n{}",
                                result.message
                            );
                            self.messages.retain(|m| {
                                !matches!(m, Message::User(u) if u.content.starts_with("【Current Game State"))
                            });
                            self.messages.push(Message::user(state_msg));
                        }
                        Err(e) => {
                            log.push(format!("[t{turn}] auto-perceive failed: {e}"));
                        }
                    }
                }
            }

            let ctx = self.build_context();

            // LLM call with retry
            let mut response = None;
            let mut last_error = String::new();
            let max_attempts = if self.config.retry.enabled {
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
                                "[t{turn}] LLM error (attempt {attempt}): {last_error}"
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
                    }
                }
            }

            let Some(response) = response else {
                self.persist_turn()?;
                self.events.push(AgentEvent::Done {
                    reason: "LLM call failed after retries".into(),
                });
                return Ok((log, false));
            };

            self.usage = response.usage.clone();

            // Track obs streak
            let calls = response.tool_calls.clone();
            if calls.is_empty() {
                // LLM returned text-only — nudge if not at max
                self.obs_streak += 1;
                self.events.push(AgentEvent::Assistant {
                    content: response.content.clone(),
                    reasoning: response.reasoning.clone(),
                    calls: vec![],
                });
                self.messages.push(Message::assistant_response(&response));

                if self.turn >= self.config.max_iterations {
                    self.events.push(AgentEvent::TurnEnd { turn });
                    self.persist_turn()?;
                    return Ok((log, false));
                } else {
                    let nudge = "【Continue】You responded with text only. You MUST call a tool. Pick any tool based on the current state and act now. Never end a turn with text-only.".to_string();
                    self.messages.push(Message::user(nudge));
                    log.push(format!(
                        "[t{turn}] nudge: text-only response, injected continue prompt"
                    ));
                    self.events.push(AgentEvent::TurnEnd { turn });
                    self.persist_turn()?;
                    return Ok((log, true));
                }
            }

            // Track obs streak from tool names
            let obs_tools: &[&str] = &["perceive", "visual_perceive", "look"];
            if calls.iter().all(|tc| obs_tools.contains(&tc.name.as_str())) {
                self.obs_streak += 1;
            } else {
                self.obs_streak = 0;
            }
            // Update knowledge bootstrap flag after first successful tool call
            if !self.knowledge_bootstrapped {
                self.knowledge_bootstrapped = true;
            }

            self.events.push(AgentEvent::Assistant {
                content: response.content.clone(),
                reasoning: response.reasoning.clone(),
                calls: calls.iter().map(|tc| tc.name.clone()).collect(),
            });
            self.messages.push(Message::assistant_response(&response));

            // Execute each tool call
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
                for &idx in batch {
                    let tc = &calls[idx];
                    // Handle meta-tool: manage_knowledge
                    if tc.name == MANAGE_KNOWLEDGE {
                        let args = tc.arguments.clone();
                        let (msg, _is_err) = self.manage_knowledge(&args);
                        self.messages.push(Message::tool_result(
                            &format!("call_{turn}_{idx}"),
                            &tc.name,
                            &msg,
                        ));
                        log.push(format!("[t{turn}] manage_knowledge -> {:.100}", msg));
                        continue;
                    }

                    self.events.push(AgentEvent::ToolExecutionStart {
                        name: tc.name.clone(),
                    });
                    let args = tc.arguments.clone();
                    let result = match self.tools.get(&tc.name) {
                        Some(tool) => tool.execute(&format!("call_{turn}_{idx}"), args, None),
                        None => Ok(ToolResult {
                            message: format!("Unknown tool: {}", tc.name),
                            is_error: true,
                            images: vec![],
                        }),
                    };
                    let (msg, is_err) = match result {
                        Ok(r) => (r.message, r.is_error),
                        Err(e) => (format!("Error: {e}"), true),
                    };
                    self.events.push(AgentEvent::ToolExecutionEnd {
                        name: tc.name.clone(),
                        is_error: is_err,
                    });
                    self.messages.push(Message::tool_result(
                        &format!("call_{turn}_{idx}"),
                        &tc.name,
                        &msg,
                    ));
                    self.session_entries.push(SessionEntry {
                        id: format!("call_{turn}_{idx}"),
                        parent_id: Some(format!("call_{turn}")),
                        turn,
                        tool: tc.name.clone(),
                        reasoning: response.reasoning.clone(),
                        detail: format!("{:.120}", msg),
                        timestamp: SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap()
                            .as_millis() as i64,
                    });
                    log.push(format!(
                        "[t{turn}] {}({}) -> {:.100}",
                        tc.name, tc.arguments, msg
                    ));
                }
            }
            self.events.push(AgentEvent::TurnEnd { turn });
            return Ok((log, true));
    }

    fn persist_turn(&mut self) -> Result<()> {
        let Some(sess) = &mut self.session else {
            return Ok(());
        };
        if self.pending_checkpoint {
            let snapshot = AgentSnapshot {
                messages: self.messages.clone(),
                previous_summary: self.previous_summary.clone(),
                usage: self.usage.clone(),
                turn: self.turn,
            };
            sess.append_checkpoint("compaction", snapshot);
            self.pending_checkpoint = false;
            self.session_msg_offset = self.messages.len();
        } else {
            let new_msgs: Vec<Message> = self.messages[self.session_msg_offset..].to_vec();
            for m in new_msgs {
                sess.append_message(m);
            }
            self.session_msg_offset = self.messages.len();
        }
        sess.save()?;
        Ok(())
    }

    fn manage_knowledge(&mut self, args: &Value) -> (String, bool) {
        let action = match args["action"].as_str() {
            Some(a) => a,
            None => {
                return (
                    "manage_knowledge missing 'action' parameter (add/remove)".into(),
                    true,
                );
            }
        };
        match action {
            "add" => {
                let keys: Vec<String> = args["keys"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_lowercase()))
                            .collect()
                    })
                    .unwrap_or_default();
                let template = args["template"].as_str().unwrap_or("Detected {label}");
                let id = args["id"].as_str().map(|s| s.to_string());
                let wi = WorldInfo {
                    keys,
                    template: template.to_string(),
                    priority: 0,
                    id: id.clone(),
                };
                self.world_info.add(wi);
                (
                    format!(
                        "Added knowledge entry (id={:?}). Total entries: {}",
                        id,
                        self.world_info.len()
                    ),
                    false,
                )
            }
            "remove" => {
                let before = self.world_info.len();
                if let Some(id) = args["id"].as_str() {
                    self.world_info.remove_by_id(id);
                } else if let Some(keys) = args["keys"].as_array() {
                    let ks: Vec<String> = keys
                        .iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_lowercase()))
                        .collect();
                    self.world_info.remove_by_keys(&ks);
                } else {
                    return ("remove needs 'id' or 'keys' parameter".into(), true);
                }
                let removed = before - self.world_info.len();
                (
                    format!(
                        "Removed {removed} entries. Total: {}",
                        self.world_info.len()
                    ),
                    false,
                )
            }
            other => (format!("Unknown action: {other}"), true),
        }
    }

    /// Compaction: LLM-generated structured summary
    fn compact(&mut self) -> Result<()> {
        let keep_tokens = self.config.compaction.keep_recent;
        let mut kept: u32 = 0;
        let mut cut = self.messages.len();
        for (i, msg) in self.messages.iter().enumerate().rev() {
            let t = Self::msg_chars(msg) as u32 / 3;
            if kept + t > keep_tokens {
                cut = i + 1;
                break;
            }
            kept += t;
        }
        if cut == 0 || cut >= self.messages.len() {
            return Ok(());
        }

        let old: Vec<String> = self.messages[..cut]
            .iter()
            .map(Self::serialize_msg)
            .collect();
        let mut prompt = format!("<conversation>\n{}\n</conversation>\n\n", old.join("\n\n"));
        let system = if let Some(prev) = &self.previous_summary {
            prompt.push_str(&format!(
                "<previous-summary>\n{prev}\n</previous-summary>\n\n"
            ));
            prompt.push_str(UPDATE_SUMMARIZATION_PROMPT);
            COMPACTION_SYSTEM
        } else {
            prompt.push_str(SUMMARIZATION_PROMPT);
            COMPACTION_SYSTEM
        };

        let cm = vec![system_chatml(system), Message::user(prompt).to_chatml()];
        // Retry summarization too
        let summary = {
            let mut result = None;
            for attempt in 1..=3 {
                match self.provider.complete(&cm, &[]) {
                    Ok(resp) => {
                        result = Some(
                            resp.content
                                .filter(|t| !t.trim().is_empty())
                                .or_else(|| resp.reasoning.filter(|t| !t.trim().is_empty()))
                                .unwrap_or_else(|| format!("{cut} messages compacted")),
                        );
                        break;
                    }
                    Err(_) if attempt < 3 => {
                        std::thread::sleep(std::time::Duration::from_millis(500 * attempt as u64));
                    }
                    Err(_) => {
                        result = Some(format!("{cut} messages compacted"));
                    }
                }
            }
            result.unwrap_or_else(|| format!("{cut} messages compacted"))
        };

        let recent: Vec<_> = self.messages.drain(cut..).collect();
        let summary_msg = Message::user(format!(
            "Previous conversation compacted:\n\n<summary>\n{}\n</summary>",
            summary
        ));
        self.messages = vec![summary_msg];
        self.messages.extend(recent);
        self.previous_summary = Some(summary);
        self.pending_checkpoint = true;
        Ok(())
    }

    fn serialize_msg(m: &Message) -> String {
        match m {
            Message::User(u) => format!("user: {}", u.content),
            Message::Assistant(a) => {
                let mut s = String::new();
                if let Some(r) = &a.reasoning {
                    s.push_str(&format!("[Think] {r}\n"));
                }
                if let Some(c) = &a.content {
                    if !c.is_empty() {
                        s.push_str(&format!("{c}\n"));
                    }
                }
                for tc in &a.tool_calls {
                    s.push_str(&format!("-> {}({})\n", tc.name, tc.arguments));
                }
                s.trim().to_string()
            }
            Message::ToolResult(r) => format!("result({}): {}", r.tool_name, r.content),
        }
    }

    pub fn events(&self) -> &[AgentEvent] {
        &self.events
    }
    pub fn usage(&self) -> Usage {
        self.usage.clone()
    }
}

// ── Compaction prompts ──

const COMPACTION_SYSTEM: &str = "You are a context summarization assistant.";

const SUMMARIZATION_PROMPT: &str = "Summarize the Minecraft gameplay conversation:\n\n## Goal\n[What is the agent trying to accomplish?]\n\n## Progress\n### Done\n- [x] [accomplished]\n### In Progress\n- [ ] [current]\n\n## Key Decisions\n- **[Decision]**: [why]\n\n## Next Steps\n1. [recommended]\n\n## Critical Context\n- [inventory, position, nearby blocks, threats]";

const UPDATE_SUMMARIZATION_PROMPT: &str =
    "Update the existing summary with new information from the messages above.";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::message::AssistantResponse;
    use crate::core::message::StopReason;
    use crate::core::tool::ToolUpdateFn;

    struct TextProvider;
    impl LlmProvider for TextProvider {
        fn complete(&self, _m: &[Value], _t: &[Value]) -> Result<AssistantResponse> {
            Ok(AssistantResponse {
                content: Some("ok".into()),
                reasoning: None,
                tool_calls: vec![],
                usage: Usage::default(),
                stop_reason: StopReason::Stop,
            })
        }
    }

    struct DummyTool {
        name: &'static str,
        effect: ToolEffects,
    }
    impl crate::core::tool::GameTool for DummyTool {
        fn name(&self) -> &str {
            self.name
        }
        fn description(&self) -> &str {
            "dummy"
        }
        fn parameters(&self) -> Value {
            serde_json::json!({})
        }
        fn effects(&self) -> ToolEffects {
            self.effect
        }
        fn execute(
            &self,
            _id: &str,
            _args: Value,
            _on: Option<ToolUpdateFn>,
        ) -> Result<ToolResult> {
            Ok(ToolResult {
                message: format!("ran {}", self.name),
                is_error: false,
                images: vec![],
            })
        }
    }

    fn response(
        content: Option<&str>,
        reasoning: Option<&str>,
        calls: Vec<crate::core::message::ToolCall>,
        stop: StopReason,
    ) -> AssistantResponse {
        AssistantResponse {
            content: content.map(|s| s.to_string()),
            reasoning: reasoning.map(|s| s.to_string()),
            tool_calls: calls,
            usage: Usage::default(),
            stop_reason: stop,
        }
    }

    #[test]
    fn mc_knowledge_loaded() {
        assert!(MC_KNOWLEDGE.contains("collect"));
        assert!(MC_KNOWLEDGE.contains("craft"));
        assert!(!MC_KNOWLEDGE.contains("Survival Strategy"));
    }

    #[test]
    fn build_context_includes_role_desc() {
        let agent = Agent::new(
            Box::new(TextProvider),
            ToolRegistry::new(),
            AgentConfig::new("You are a test bot.".into(), 5),
        );
        let ctx = agent.build_context();
        assert!(ctx.system_prompt.contains("You are a test bot"));
        assert!(ctx.system_prompt.contains("Tool Reference"));
    }

    #[test]
    fn jailbreak_english_consistent() {
        let agent = Agent::new(
            Box::new(TextProvider),
            ToolRegistry::new(),
            AgentConfig::new("sys".into(), 5),
        );
        let ctx = agent.build_context();
        assert!(
            !ctx.system_prompt.contains("保持自主"),
            "jailbreak must be English"
        );
        assert!(
            ctx.system_prompt.contains("autonomously"),
            "jailbreak should say 'autonomously'"
        );
    }

    #[test]
    fn knowledge_bootstrap_injects_once() {
        let mut agent = Agent::new(
            Box::new(TextProvider),
            ToolRegistry::new(),
            AgentConfig::new("sys".into(), 5),
        );
        let before = agent.build_context();
        assert!(
            before.system_prompt.contains("no need to re-enter"),
            "fresh agent should bootstrap"
        );

        agent.knowledge_bootstrapped = true;
        let after = agent.build_context();
        assert!(
            !after.system_prompt.contains("no need to re-enter"),
            "bootstrapped agent should not repeat"
        );
    }

    #[test]
    fn obs_streak_hint_at_threshold() {
        let mut agent = Agent::new(
            Box::new(TextProvider),
            ToolRegistry::new(),
            AgentConfig::new("sys".into(), 5),
        );
        let fresh = agent.build_context();
        assert!(
            !fresh.system_prompt.contains("observing without action"),
            "obs_streak=0, no hint"
        );

        agent.obs_streak = 5;
        let guarded = agent.build_context();
        assert!(
            guarded.system_prompt.contains("observing without action"),
            "obs_streak>=5 should inject hint"
        );
    }

    #[test]
    fn continue_run_with_one_tool_call() {
        use std::sync::atomic::AtomicU32;
        struct OneCallProvider {
            n: AtomicU32,
        }
        impl LlmProvider for OneCallProvider {
            fn complete(&self, _m: &[Value], _t: &[Value]) -> Result<AssistantResponse> {
                let k = self.n.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if k == 0 {
                    Ok(AssistantResponse {
                        content: None,
                        reasoning: Some("I will collect wood".into()),
                        tool_calls: vec![crate::core::message::ToolCall {
                            id: "call_1".into(),
                            name: "perceive".into(),
                            arguments: serde_json::json!({}),
                        }],
                        usage: Usage::default(),
                        stop_reason: StopReason::ToolCalls,
                    })
                } else {
                    Ok(AssistantResponse {
                        content: None,
                        reasoning: None,
                        tool_calls: vec![],
                        usage: Usage::default(),
                        stop_reason: StopReason::Stop,
                    })
                }
            }
        }
        let mut reg = ToolRegistry::new();
        reg.register(Box::new(DummyTool {
            name: "perceive",
            effect: ToolEffects::read(),
        }));
        let cfg = AgentConfig::new("test".into(), 5).with_auto_perceive(false);
        let mut agent = Agent::new(
            Box::new(OneCallProvider {
                n: AtomicU32::new(0),
            }),
            reg,
            cfg,
        );
        let log = agent.run("collect wood").unwrap();
        assert_eq!(
            agent
                .messages
                .iter()
                .filter(|m| matches!(m, Message::ToolResult(_)))
                .count(),
            1,
            "should have exactly 1 tool result, but got {}",
            agent
                .messages
                .iter()
                .filter(|m| matches!(m, Message::ToolResult(_)))
                .count()
        );
        assert!(
            log.iter().any(|l| l.contains("perceive")),
            "perceive should be in log"
        );
    }

    #[test]
    fn text_only_response_gets_nudged() {
        struct TextOnlyProvider;
        impl LlmProvider for TextOnlyProvider {
            fn complete(&self, _m: &[Value], _t: &[Value]) -> Result<AssistantResponse> {
                Ok(AssistantResponse {
                    content: Some("I should look around first".into()),
                    reasoning: None,
                    tool_calls: vec![],
                    usage: Usage::default(),
                    stop_reason: StopReason::Stop,
                })
            }
        }
        let cfg = AgentConfig::new("test".into(), 3)
            .with_compaction(CompactionConfig {
                context_window: 1_000_000,
                reserve: 200_000,
                keep_recent: 200_000,
            })
            .with_auto_perceive(false);
        let mut agent = Agent::new(Box::new(TextOnlyProvider), ToolRegistry::new(), cfg);
        let _log = agent.run("goal").unwrap();
        let has_nudge = agent
            .messages
            .iter()
            .any(|m| matches!(m, Message::User(u) if u.content.contains("Continue")));
        assert!(
            has_nudge,
            "text-only responses should trigger nudge message"
        );
    }
}
