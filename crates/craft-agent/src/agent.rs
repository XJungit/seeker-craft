//! Agent 核心

use crate::core::message::{Message, Usage, system_chatml};
use crate::core::prompt::{PromptBuilder, WorldInfo, WorldInfoLib, default_mc_world_info};
use crate::core::session::SessionEntry as SessionFileEntry;
use crate::core::session::{AgentSnapshot, Session};
use crate::core::skill::SkillLibrary;
use crate::core::tool::{ToolEffects, ToolRegistry, ToolResult, plan_tool_effect_batches};
use anyhow::Result;
use serde::Serialize;
use serde_json::Value;
use std::collections::VecDeque;
use std::sync::Arc;
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
You are a Minecraft bot. Each turn you receive game state (STATS, HOTBAR, INVENTORY, NEARBY BLOCKS, NEARBY ENTITIES) and must call exactly one tool. Never text-only.

## Tool Reference — High-Level (prefer these)

collect(target, count)
  Auto find→walk→mine. target: block ID substring ("oak_log","stone","coal_ore"). count: 1-64. Returns actual collected count. Uses mod move_to (no camera oscillation) and auto-break detection.

craft(item, count)
  Craft via inventory. item: "oak_planks","stick","crafting_table","wooden_pickaxe","torch",etc. Check craftable() first. Mod handles recipe automatically.

place(item)
  Place block from hotbar at crosshair. Finds item in slots 1-9, switches, right-clicks. Look at target surface BEFORE calling.

build(blueprint, x, y, z, orientation)
  Build a structure from blueprint at world coords. Auto-generates steps layer-by-layer, calls place_at for each block. blueprint: "dirt_shelter"(3x3), "wood_house"(5x5), "stone_house"(5x5), "wall_3x3". orientation: 0-3. Check blueprints() for materials needed.

blueprints()
  List all available blueprints with materials requirements and current inventory status. Call before build().

combat(mode, ticks)
  Autonomous combat AI. mode: "melee"(zombies/spiders), "kite"(skeletons/creeper), "retreat"(flee). Auto-equips best weapon, auto-retreats when health<6 or creeper nearby. ticks: 200≈10s. Returns killed/retreated/timeout/no_target.

## Tool Reference — Utility

equip(slot)
  Switch to hotbar slot 1-9. Check HOTBAR display to see what's in each slot.

consume(item, ticks)
  Eat food by name. Finds item in hotbar, equips, right-clicks. ticks: 32≈1.6s for food.

attack(ticks)
  Attack nearest entity. ticks: 30≈1.5s. Use when hostile mobs appear.

discard(item, num)
  Drop items from inventory. item: "dirt","cobblestone". num: how many.

smeltItem(item, num)
  Smelt in nearest furnace. Finds furnace, opens it, smelts items. item: "raw_iron","raw_copper". num: 1-8. Each takes ~10s.

## Tool Reference — Navigation

searchForBlock(type)
  Find nearest block and walk to it (no mining). Use to position before manual action.

move_to(x, y, z)
  Walk to world coords. Mod navigation: re-aims per tick, strafes walls, jumps. 2-10s depending on distance. Get coords from NEARBY BLOCKS.

moveAway(distance)
  Walk backward N blocks (max 20). Use to back up or flee.

digDown(distance)
  Dig straight down 1-10 blocks. Looks down, mines, jumps into hole.

## Tool Reference — Precise Control

look_at(x, y, z)
  Snap crosshair to coords. Integer coords auto-center (+0.5). Force-refreshes raycast — returns what was actually hit. PREFER THIS over look().

look(dx, dy)
  Relative rotation. dx>0=right (300≈90°). dy>0=DOWN (ground), dy<0=UP (sky). Sensitivity 0.3°/unit. To look at ground: look(0,65).

press(keys, ticks)
  Hold key(s). w/a/s/d=walk, space=jump, shift=sneak, e=inventory, 1-9=hotbar. Walk ≈ ticks/15 blocks. Max 200 ticks.

mine(ticks)
  Mine targeted block. Mod auto-stops when block breaks. ticks=safety timeout (140 for wood, 300 stone). Prefer collect().

## Tool Reference — Query

craftable()
  Query craftable items from current inventory. Returns item:max_count. Call BEFORE craft().

perceive()
  Full state snapshot (<100ms). Auto-injected each turn.

visual_perceive(prompt)
  Screenshot+VLM (3-5s). Use ONLY for GUI.

savedPlaces()
  List all locations saved with rememberHere.

## Tool Reference — Memory

rememberHere(name)
  Save current position with a label. name: "base","cave","farm".

goToRememberedPlace(name)
  Walk to a saved location. Uses move_to navigation.

## Crafting Recipes (craft handles automatically)
1 log→4 planks | 2 planks→4 sticks | 4 planks→1 crafting_table
3 planks+2 sticks→wooden_pickaxe/axe/hoe | 2 planks+1 stick→wooden_sword
1 planks+2 sticks→wooden_shovel | 1 stick+1 coal→4 torches
8 cobblestone→1 furnace | 3 cobblestone+2 sticks→stone_pickaxe/axe
8 planks→1 chest | 6 planks→3 door

## Survival Strategy

### Daytime: gather wood, craft tools, find food
1. collect("oak_log", 8) → craft("oak_planks", 32) → craft("crafting_table", 1) → place("crafting_table")
2. craft("stick", 8) → craft("wooden_pickaxe", 1) → equip(slot) → collect("stone", 20)
3. craft("stone_pickaxe", 1) → craft("stone_axe", 1) → craft("stone_sword", 1)
4. Attack animals for food: check NEARBY ENTITIES for cow/pig/sheep/chicken → move_to(coords) → attack(60). Meat drops on ground — walk over it to collect.
5. Eat when hungry: consume("beef", 32) or consume("porkchop", 32) or consume("mutton", 32)

### Evening: build shelter before night
1. collect("dirt", 30) or craft("oak_planks", 32)
2. PREFER build("dirt_shelter", x, y, z, 0) — auto-builds 3x3 shelter at your position. Use blueprints() to check materials first.
3. Or: digDown(3) → look_at(up) → place("dirt") — hide underground with block above
4. If you have wood, craft("torch", 16) → place("torch") for light

### Night: stay safe or fight
1. If shelter built: stay inside, craft items (tools, torches, furnace, chest)
2. If hostile mobs nearby: combat("melee", 200) for zombies/spiders, combat("kite", 200) for skeletons/creeper
3. If health < 8: combat("retreat", 100) to flee, then consume food
4. Light prevents spawns: place torches every 5 blocks in dark areas

### Mining cave exploration
1. Find a cave entrance or digDown(5) to create shaft
2. Place torches as you go down (block_light < 5 = dangerous)
3. collect("coal_ore", 10) for fuel → collect("iron_ore", 20) for iron
4. smeltItem("raw_iron", 20) → craft("iron_pickaxe", 1) → craft("iron_sword", 1)
5. craft armor: iron_helmet, iron_chestplate, iron_leggings, iron_boots (5+8+7+4=24 iron total)
6. If too dark or lost: dig up with digDown(1) while jumping, or goToSurface

### Food & health management
1. Hungry (hunger<15): check inventory for food → consume("food_name", 32)
2. No food: hunt animals (cow→beef, pig→porkchop, sheep→mutton, chicken→chicken)
3. cook raw meat: smeltItem("beef", N) → cooked_beef (better hunger restore)
4. Low health: run away (moveAway or press("w",60)), eat to regen
5. Plant wheat seeds on farmland for renewable bread: craft("wooden_hoe") → use on grass

## Decision Rules
1. Read STATS+HOTBAR: know position, health, hunger, what's in quick-access slots, nearby blocks
2. Gather with collect() — it handles everything automatically
3. Craft with craft() after checking craftable()
4. Place with place() — look at surface first
5. Navigate with searchForBlock() or move_to() — use NEARBY BLOCKS coords
6. Fight with attack() if mobs attack; flee with moveAway() if health<8
7. Eat with consume() when hunger<15
8. Every response MUST end with a tool call. Tool error→retry adjusted. No faking success.
9. Prefer look_at(x,y,z) over look(dx,dy) — absolute coords are unambiguous.
10. dy>0 = look DOWN (ground). dy<0 = look UP (sky). NEVER use dy<0 to look at ground!

## Response Format
  collect("oak_log", 4) — GOOD
  craft("oak_planks", 8) — GOOD
  "I should collect wood" — BAD (text-only)
  "Need to look around first" — BAD (text-only)
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
    /// Skill library: reusable action sequences learned from experience
    skill_lib: SkillLibrary,
    knowledge_bootstrapped: bool,
    obs_streak: u32,
    /// SelfPrompter: 持续目标注入（参考 Mindcraft SelfPrompter），防止 LLM 偏离长期任务
    self_prompt: Option<String>,
    /// Modes: 反应系统触发计数（参考 Mindcraft modes.js），避免同模式连续注入
    last_mode_trigger: u32,
    /// Session persistence
    pub session: Option<Session>,
    pending_checkpoint: bool,
    session_msg_offset: usize,
    /// Retry abort signal (shared with controller for instant stop)
    pub retry_abort: Arc<AtomicBool>,
    /// 近期工具调用签名（name+args），用于检测死循环
    recent_calls: std::collections::VecDeque<String>,
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
            skill_lib: SkillLibrary::new(20),
            knowledge_bootstrapped: false,
            obs_streak: 0,
            self_prompt: None,
            last_mode_trigger: 0,
            session: None,
            pending_checkpoint: false,
            session_msg_offset: 0,
            retry_abort: Arc::new(AtomicBool::new(false)),
            recent_calls: std::collections::VecDeque::with_capacity(10),
        }
    }

    /// Attach session for persistence
    pub fn with_session(mut self, sess: Session) -> Self {
        // 从 session 恢复状态
        let messages = sess.messages_for_current_path();
        if !messages.is_empty() {
            self.session_msg_offset = messages.len();
            self.messages = messages;
        }

        // 从最近的 checkpoint 恢复 summary/usage/turn/skills
        let path = sess.entries_for_current_path();
        for e in path.iter().rev() {
            if let SessionFileEntry::Checkpoint(cp) = e {
                self.previous_summary = cp.snapshot.previous_summary.clone();
                self.usage = cp.snapshot.usage.clone();
                self.turn = cp.snapshot.turn;
                if let Some(skills_json) = &cp.snapshot.skills_json {
                    if let Ok(skill_lib) =
                        serde_json::from_str::<crate::core::skill::SkillLibrary>(skills_json)
                    {
                        self.skill_lib = skill_lib;
                    }
                }
                break; // 只取最近一个 checkpoint
            }
        }

        // 回放 WorldInfo entry 重建 world_info
        for e in &path {
            if let SessionFileEntry::WorldInfo(wi) = e {
                match wi.action.as_str() {
                    "add" => {
                        if let Some(info) = &wi.info {
                            self.world_info.add(info.clone());
                        }
                    }
                    "remove" => {
                        if let Some(id) = &wi.remove_id {
                            self.world_info.remove_by_id(id);
                        }
                        if let Some(keys) = &wi.remove_keys {
                            self.world_info.remove_by_keys(keys);
                        }
                    }
                    _ => {}
                }
            }
        }

        // 读取 knowledge_bootstrapped 标志
        self.knowledge_bootstrapped = sess.header.knowledge_bootstrapped;

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

    /// 设置 SelfPrompter 持续目标（参考 Mindcraft SelfPrompter）。
    /// 注入后每轮 auto_perceive 后自动追加目标提醒，防止 LLM 偏离长期任务。
    pub fn set_self_prompt(&mut self, goal: impl Into<String>) {
        self.self_prompt = Some(goal.into());
    }

    /// 清除 SelfPrompter 目标
    pub fn clear_self_prompt(&mut self) {
        self.self_prompt = None;
    }

    /// Modes 反应系统（参考 Mindcraft modes.js）。
    /// 检查最新感知的游戏状态，返回需要紧急注入的指令。
    /// 仅在同模式未连续触发时注入（避免重复打扰）。
    fn check_modes(&mut self) -> Option<String> {
        // 从最近 auto_perceive 提取关键状态
        let perception = self.messages.iter().rev().find_map(|m| match m {
            Message::User(u) if u.content.starts_with("【当前游戏状态") => {
                Some(u.content.as_str())
            }
            _ => None,
        })?;

        // Mode: self_preservation — 血量低或饥饿低
        // 注意：必须用 "Health: N/" 精确匹配，否则 "Health: 2" 会匹配 "Health: 20/20"
        let health_low = (0..=5).any(|n| perception.contains(&format!("Health: {n}/")));
        let hunger_low = (0..=5).any(|n| perception.contains(&format!("Hunger: {n}/")));
        if health_low || hunger_low {
            if self.last_mode_trigger != 1 {
                self.last_mode_trigger = 1;
                let action = if health_low {
                    "血量危急！立即 combat(\"retreat\",100) 撤退，然后 consume 食物恢复。"
                } else {
                    "饥饿危急！立即 consume 食物（检查 HOTBAR 找食物）。"
                };
                return Some(format!("[MODE: self_preservation] {action}"));
            }
            return None;
        }

        // Mode: self_defense — 附近有敌对实体
        let has_hostile = perception.contains("zombie")
            || perception.contains("skeleton")
            || perception.contains("creeper")
            || perception.contains("spider")
            || perception.contains("phantom")
            || perception.contains("witch");
        let has_creeper = perception.contains("creeper");
        if has_hostile {
            if self.last_mode_trigger != 2 {
                self.last_mode_trigger = 2;
                let action = if has_creeper {
                    "苦力怕靠近！立即 combat(\"kite\",200) 风筝攻击，保持距离。"
                } else {
                    "敌对生物靠近！立即 combat(\"melee\",200) 近战攻击。"
                };
                return Some(format!("[MODE: self_defense] {action}"));
            }
            return None;
        }

        // Mode: unstuck — 连续纯观察 5+ 步
        if self.obs_streak >= 5 && self.last_mode_trigger != 3 {
            self.last_mode_trigger = 3;
            return Some(format!(
                "[MODE: unstuck] 已连续 {} 步纯观察！选一个完全不同的工具立即行动：collect, craft, build, combat, move_to — 不要再用 perceive/look。",
                self.obs_streak
            ));
        }

        // 状态恢复正常，重置触发计数
        self.last_mode_trigger = 0;
        None
    }

    /// Build system prompt with layered prompt pipeline: identity -> role_desc -> scenario -> jailbreak
    fn build_context(&mut self) -> Context {
        let recent_perception = self
            .messages
            .iter()
            .rev()
            .find_map(|message| match message {
                // 手动 perceive 调用的结果
                Message::ToolResult(result) if result.tool_name == "perceive" => {
                    Some(result.content.as_str())
                }
                // auto_perceive 注入的 User 消息
                Message::User(u) if u.content.starts_with("【当前游戏状态") => {
                    Some(u.content.as_str())
                }
                _ => None,
            })
            .unwrap_or("");
        let dynamic_hints = self.world_info.scan_text(recent_perception, 4_000);

        // Skill library: inject matched skills as examples
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        let skill_examples =
            self.skill_lib
                .to_examples(recent_perception, &self.config.prompt, 3, now_ms);

        let mut jailbreak = "自主行动。工具失败时调整参数重试——不准假装成功。".to_string();
        if !self.knowledge_bootstrapped {
            jailbreak.push_str(" 直接开始执行任务，不需要重新输入游戏知识。");
        }
        if self.obs_streak >= 5 {
            if self.obs_streak >= 10 {
                jailbreak.push_str(
                    " [关键警告: 你已经循环了 10+ 步！ STOP repeating the same action. Pick a COMPLETELY DIFFERENT tool RIGHT NOW — collect, craft, press, mine, move_to — anything but what you've been doing.]",
                );
            } else {
                jailbreak.push_str(&format!(
                    " [Hint: {} steps observing without action. Pick any tool and act now.]",
                    self.obs_streak
                ));
            }
        }

        let mut builder = PromptBuilder::new()
            .identity(&self.config.prompt)
            .role_desc(MC_KNOWLEDGE);
        for example in &skill_examples {
            builder = builder.add_example(example);
        }
        builder = builder.jailbreak(jailbreak);
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
        // 不再用 max(estimated, measured)：压缩后 messages 已减少，但 measured 还是旧的大值，
        // 会导致误判仍需压缩。改为 estimated 为主，measured 仅在 estimated 明显偏低时作下限。
        let measured = u32::try_from(self.usage.total_tokens).unwrap_or(0);
        // 若 estimated 远低于 measured（>2x），可能是中文 token 密度高，用 measured 兜底
        if estimated * 2 < measured {
            measured
        } else {
            estimated
        }
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
        eprintln!(
            "[DBG] continue_run: max_iterations={}",
            self.config.max_iterations
        );

        for _ in 0..self.config.max_iterations {
            eprintln!("[DBG] continue_run: starting iteration");
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
                log.push(format!("[t{turn}] 压缩失败: {e}"));
            }
            self.events.push(AgentEvent::AutoCompactionEnd);
        }

        // Auto-perceive: inject latest game state as user message (Mindcraft style, replaced each turn)
        if self.config.auto_perceive {
            if let Some(tool) = self.tools.get("perceive") {
                match tool.execute("auto_perceive", serde_json::json!({}), None) {
                    Ok(result) => {
                        let state_msg = format!("【当前游戏状态（自动注入）】\n{}", result.message);
                        self.messages.retain(|m| {
                            !matches!(m, Message::User(u) if u.content.starts_with("【当前游戏状态"))
                        });
                        // 保留截图，让多模态 LLM 能看到画面
                        self.messages
                            .push(Message::user_with_images(state_msg, result.images));
                    }
                    Err(e) => {
                        eprintln!("[DBG] auto_perceive FAIL: {e}");
                        log.push(format!("[t{turn}] 自动感知失败: {e}"));
                    }
                }
            }
        }

        // Modes 反应系统：检查游戏状态，注入紧急指令（参考 Mindcraft modes.js）
        if let Some(mode_msg) = self.check_modes() {
            self.messages.push(Message::user(mode_msg.clone()));
            log.push(format!("[t{turn}] {mode_msg}"));
        }

        // SelfPrompter：持续目标注入（参考 Mindcraft SelfPrompter），每轮提醒当前任务
        if let Some(prompt) = &self.self_prompt {
            self.messages
                .push(Message::user(format!("[当前目标] {prompt}")));
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
        eprintln!(
            "[DBG] calling LLM ({msg_count} msgs, {tool_count} tools)...",
            msg_count = ctx.messages.len(),
            tool_count = ctx.tools.len()
        );
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
                        log.push(format!("[t{turn}] LLM 错误 (第{attempt}次): {last_error}"));
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
                    // abort 后跳出重试循环，不再发下一次请求
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
        eprintln!(
            "[DBG] LLM response: {} chars, {} tools",
            response.content.as_ref().map_or(0, |s| s.len()),
            response.tool_calls.len()
        );

        // Track obs streak
        let calls = response.tool_calls.clone();
        if calls.is_empty() {
            // LLM returned text-only — nudge to call a tool
            self.obs_streak += 1;
            self.events.push(AgentEvent::Assistant {
                content: response.content.clone(),
                reasoning: response.reasoning.clone(),
                calls: vec![],
            });
            self.messages.push(Message::assistant_response(&response));

            // 总是注入 nudge 并返回 true，由外层循环（viewer/continue_run）控制总步数
            // 这避免了 max_iterations=1 时 nudge 失效的问题
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
        // Update knowledge bootstrap flag after first successful tool call
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

        // ── 死循环检测：如果最近 6 次调用中有 4 次相同签名，注入打断指令 ──
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
                "【死循环警告】你已连续 {repeat_count} 次执行相同操作 ({})。这表示当前方法不生效。请：\n\
                 1. 检查 perceive 返回的状态，确认当前实际情况\n\
                 2. 换一种完全不同的方法\n\
                 3. 如果在建造，改用 build 蓝图工具而不是手动 place\n\
                 4. 如果在采集，先 move_to 到新位置再 collect\n\
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
                let call_id = tc.id.clone();
                // Handle meta-tool: manage_knowledge
                if tc.name == MANAGE_KNOWLEDGE {
                    let args = tc.arguments.clone();
                    let (msg, _is_err) = self.manage_knowledge(&args);
                    self.messages
                        .push(Message::tool_result(&call_id, &tc.name, &msg));
                    log.push(format!("[t{turn}] manage_knowledge -> {:.100}", msg));
                    continue;
                }

                self.events.push(AgentEvent::ToolExecutionStart {
                    name: tc.name.clone(),
                });
                let args = tc.arguments.clone();
                let result = match self.tools.get(&tc.name) {
                    Some(tool) => tool.execute(&call_id, args, None),
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
                // 写入真实 is_error，让 LLM 能看到工具失败
                let tool_msg = if is_err {
                    Message::tool_error(&call_id, &tc.name, &msg)
                } else {
                    Message::tool_result(&call_id, &tc.name, &msg)
                };
                self.messages.push(tool_msg);
                self.session_entries.push(SessionEntry {
                    id: call_id.clone(),
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
        // 提取技能：如果本轮工具调用非纯观察且无错误，记录为技能
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
        return Ok((log, true));
    }

    fn persist_turn(&mut self) -> Result<()> {
        let Some(sess) = &mut self.session else {
            return Ok(());
        };
        if self.pending_checkpoint {
            let skills_json = serde_json::to_string(&self.skill_lib).ok();
            let snapshot = AgentSnapshot {
                messages: self.messages.clone(),
                previous_summary: self.previous_summary.clone(),
                usage: self.usage.clone(),
                turn: self.turn,
                skills_json,
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
                    keys: keys.clone(),
                    template: template.to_string(),
                    priority: 0,
                    id: id.clone(),
                };
                self.world_info.add(wi.clone());
                // 持久化到 session
                if let Some(ref mut sess) = self.session {
                    sess.append_world_info("add", Some(wi), None, None);
                }
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
                let remove_id = args["id"].as_str().map(|s| s.to_string());
                let remove_keys: Option<Vec<String>> = args["keys"].as_array().map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_lowercase()))
                        .collect()
                });
                if let Some(id) = &remove_id {
                    self.world_info.remove_by_id(id);
                } else if let Some(keys) = &remove_keys {
                    self.world_info.remove_by_keys(keys);
                } else {
                    return ("remove needs 'id' or 'keys' parameter".into(), true);
                }
                let removed = before - self.world_info.len();
                // 持久化到 session
                if let Some(ref mut sess) = self.session {
                    sess.append_world_info("remove", None, remove_id.clone(), remove_keys.clone());
                }
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

        // 对齐切点到 turn 边界：如果切点处是 ToolResult（无对应 Assistant tool_call），
        // 向前移到该 Assistant 之前
        while cut < self.messages.len() {
            let prev_is_assistant = self
                .messages
                .get(cut.wrapping_sub(1))
                .map(|m| matches!(m, Message::Assistant(a) if !a.tool_calls.is_empty()))
                .unwrap_or(false);
            let cur_is_tool_result = self
                .messages
                .get(cut)
                .map(|m| matches!(m, Message::ToolResult(_)))
                .unwrap_or(false);
            if cur_is_tool_result && !prev_is_assistant {
                cut -= 1;
            } else {
                break;
            }
        }
        if cut == 0 {
            return Ok(()); // 无法找到安全切点
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
            let mut result: Option<String> = None;
            let mut last_err = None;
            for attempt in 1..=3 {
                match self.provider.complete(&cm, &[]) {
                    Ok(resp) => {
                        if let Some(t) = resp.content.as_ref().filter(|t| !t.trim().is_empty()) {
                            result = Some(t.clone());
                        } else if let Some(t) =
                            resp.reasoning.as_ref().filter(|t| !t.trim().is_empty())
                        {
                            result = Some(t.clone());
                        } else {
                            // 空响应，重试
                            last_err = Some("empty response".into());
                        }
                        if result.is_some() {
                            break;
                        }
                    }
                    Err(e) => {
                        last_err = Some(format!("{e}"));
                        if attempt < 3 {
                            std::thread::sleep(std::time::Duration::from_millis(
                                500 * attempt as u64,
                            ));
                        }
                    }
                }
            }
            match result {
                Some(s) => s,
                None => {
                    // 压缩失败：不丢历史，直接返回错误让调用方决定
                    return Err(anyhow::anyhow!(
                        "compaction failed after 3 attempts: {}",
                        last_err.unwrap_or_else(|| "unknown".into())
                    ));
                }
            }
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
        // 压缩后清零 usage，避免旧 measured 值在下一轮 estimate 中导致误判振荡
        self.usage = Usage::default();
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

/// 判断工具是否为纯观察类（不产生世界状态变化的工具）。
fn is_obs_tool(name: &str) -> bool {
    matches!(name, "perceive" | "visual_perceive" | "look" | "look_at")
}

// ── Compaction prompts ──

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
        assert!(MC_KNOWLEDGE.contains("Survival Strategy"));
    }

    #[test]
    fn build_context_includes_role_desc() {
        let mut agent = Agent::new(
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
        let mut agent = Agent::new(
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
            ctx.system_prompt.contains("自主行动"),
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
            before.system_prompt.contains("不需要重新输入"),
            "fresh agent should bootstrap"
        );

        agent.knowledge_bootstrapped = true;
        let after = agent.build_context();
        assert!(
            !after.system_prompt.contains("不需要重新输入"),
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
            !fresh.system_prompt.contains("不行动"),
            "obs_streak=0, no hint"
        );

        agent.obs_streak = 5;
        let guarded = agent.build_context();
        assert!(
            guarded.system_prompt.contains("observing"),
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
            .any(|m| matches!(m, Message::User(u) if u.content.contains("继续")));
        assert!(
            has_nudge,
            "text-only responses should trigger nudge message"
        );
    }
}
