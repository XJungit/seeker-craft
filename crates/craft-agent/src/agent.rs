//! Agent 核心 — 严格基于 pi_agent_rust (agent.rs / tools.rs / model.rs / compaction.rs / provider.rs)
//!
//! 与 pi 同构:
//!   Agent { provider, tools, messages, config }
//!   run() → build_context() → provider.complete() → plan_tool_effect_batches → tool.execute() → loop
//!   compact(): pi generate_summary (LLM 6 段结构化摘要) + previous_summary 增量更新
//!
//! 相对旧版的实质修复 (不是改名):
//!   1. 一轮内**所有** tool_calls 都执行 (旧版只取 calls[0], 其余丢弃)
//!   2. ToolEffects 位掩码 + plan_tool_effect_batches 分组 (旧版 bool 未用于调度)
//!   3. 压缩用 pi 完整 6 段 prompt + 增量 previous_summary (旧版 4 段 + 每次从零)
//!   4. 真实 Usage 优先 (pi estimate_context_tokens 优先用 provider total_tokens)
//!   5. AgentEvent 生命周期 + steering/follow_up 队列 (旧版只有 Vec<String> log)

use crate::core::message::{Message, Usage, system_chatml};
use crate::core::tool::{ToolEffects, ToolRegistry, ToolResult, plan_tool_effect_batches};
use anyhow::Result;
use serde::Serialize;
use serde_json::Value;
use std::collections::VecDeque;
use std::time::{SystemTime, UNIX_EPOCH};

// ── Provider (pi provider.rs: 极简 trait, 仅 complete + 身份) ──

pub trait LlmProvider: Send + Sync {
    /// 返回 (思维链, tool_calls[(name, args_json)], Usage)
    fn complete(
        &self,
        messages: &[Value],
        tools: &[Value],
    ) -> Result<(Option<String>, Vec<(String, String)>, Usage)>;
}

// ── AgentEvent (pi agent.rs AgentEvent L935, 全周期事件) ──

#[derive(Debug, Clone, Serialize)]
pub enum AgentEvent {
    AgentStart,
    TurnStart { turn: u32 },
    Assistant { reasoning: Option<String>, calls: Vec<String> },
    ToolExecutionStart { name: String },
    ToolExecutionEnd { name: String, is_error: bool },
    TurnEnd { turn: u32 },
    AgentEnd,
    AutoCompactionStart,
    AutoCompactionEnd,
}

// ── Session (pi session.rs SessionEntry 树, 简化) ──

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

// ── Compaction (pi ResolvedCompactionSettings) ──

#[derive(Debug, Clone)]
pub struct CompactionConfig {
    pub context_window: u32, // 模型上下文窗口 token 数
    pub reserve: u32,        // 预留 token (给后续对话)
    pub keep_recent: u32,    // 压缩时保留最近 N token
}
impl Default for CompactionConfig {
    fn default() -> Self {
        // LongCat 1M 上下文; 触发阈值 = window - reserve
        Self {
            context_window: 1_000_000,
            reserve: 200_000,
            keep_recent: 200_000,
        }
    }
}

// ── Config ──

pub struct AgentConfig {
    pub prompt: String,
    pub max_iterations: u32, // pi: max_tool_iterations
    pub compaction: CompactionConfig,
}
impl AgentConfig {
    pub fn new(prompt: String, max_iterations: u32) -> Self {
        Self {
            prompt,
            max_iterations,
            compaction: CompactionConfig::default(),
        }
    }
    /// 自定义压缩参数 (测试/小窗口模型用)
    pub fn with_compaction(mut self, c: CompactionConfig) -> Self {
        self.compaction = c;
        self
    }
}

// ── Minecraft 完整知识 (酒馆 World Info 风格, 注入 system prompt) ──

pub const MC_KNOWLEDGE: &str = r#"
## Minecraft 控制知识
### 移动
- press w: 前进
- press a: 左移
- press s: 后退
- press d: 右移
- press space: 跳跃 (1格高)
- press shift: 潜行 (不会从边缘掉落)
- press ctrl: 疾跑
### 交互
- press e: 打开/关闭背包
- press 1~9: 切换到快捷栏对应物品
- press q: 丢弃手中物品
- press f: 切换到副手
- 鼠标左键=挖掘/攻击, 右键=放置/使用
### 视角
- look dx=N: 水平转视角 (300≈90度, 150≈45度, 负=左)
- look dy=N: 垂直转视角 (正=低头, 负=抬头)
### 挖掘
- mine ticks=60: 按住左键挖3秒 (木头/泥土)
- mine ticks=120: 6秒 (石头)
- mine ticks=200: 10秒 (矿石, 无合适工具时)
### 生存知识
- 优先收集木头(tree) → 做木斧/木镐
- 石头(stone) → 圆石 → 石镐 → 挖铁
- 晚上要躲怪物: 挖三格深的洞, 上面盖住
- 快捷栏物品: press 1~9 切换
- 背包满了: press e 打开背包整理
- 怪物(僵尸/骷髅/苦力怕)靠近时: 优先 mine 或 press ctrl 逃跑
"#;

// ── Context (pi provider.rs Context, 组装 system+messages+tools) ──

pub struct Context {
    pub system_prompt: String,
    pub messages: Vec<Value>,
    pub tools: Vec<Value>,
}

// ── Agent (pi 同构) ──

pub struct Agent {
    pub provider: Box<dyn LlmProvider>,
    pub tools: ToolRegistry,
    pub messages: Vec<Message>,
    pub session: Vec<SessionEntry>,
    pub config: AgentConfig,
    /// 结构化事件 (pi AgentEvent)
    pub events: Vec<AgentEvent>,
    /// 真实 token 用量 (pi Usage, estimate_context_tokens 优先用它)
    usage: Usage,
    /// 上一次压缩摘要 (pi previous_summary, 增量更新)
    previous_summary: Option<String>,
    /// 异步注入队列 (pi MessageQueue: steering / follow_up)
    steering: VecDeque<String>,
    follow_up: VecDeque<String>,
    turn: u32,
}

impl Agent {
    pub fn new(provider: Box<dyn LlmProvider>, tools: ToolRegistry, config: AgentConfig) -> Self {
        Self {
            provider,
            tools,
            config,
            messages: vec![],
            session: vec![],
            events: vec![],
            usage: Usage::default(),
            previous_summary: None,
            steering: VecDeque::new(),
            follow_up: VecDeque::new(),
            turn: 0,
        }
    }

    // ── 队列 (pi queue_steering / queue_follow_up / drain_*) ──
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
            self.messages.push(Message::user(format!("[follow_up] {m}")));
        }
    }

    /// pi build_context(): system prompt + messages + tools → Context
    fn build_context(&self) -> Context {
        let full_prompt = format!("{}\n\n{}", self.config.prompt, MC_KNOWLEDGE);
        let system = system_chatml(&full_prompt);
        let mut chatml = vec![system];
        chatml.extend(self.messages.iter().map(Message::to_chatml));
        let tool_defs = self.tools.to_openai_defs();
        Context {
            system_prompt: full_prompt,
            messages: chatml,
            tools: tool_defs,
        }
    }

    /// pi estimate_tokens(): chars / 3 (CHARS_PER_TOKEN_ESTIMATE=3, 保守高估)
    fn estimate_tokens(&self) -> u32 {
        // pi estimate_context_tokens: 优先用 provider 返回的 total_tokens
        if self.usage.total_tokens > 0 {
            return self.usage.total_tokens as u32;
        }
        let chars = self.config.prompt.len()
            + MC_KNOWLEDGE.len()
            + self.messages.iter().map(Self::msg_chars).sum::<usize>();
        (chars / 3) as u32
    }

    fn msg_chars(m: &Message) -> usize {
        match m {
            Message::User(u) => u.content.len(),
            Message::Assistant(a) => {
                a.reasoning.as_deref().map_or(0, |s| s.len())
                    + a.content.as_deref().map_or(0, |s| s.len())
                    + a.tool_calls
                        .iter()
                        .map(|tc| tc.name.len() + tc.arguments.to_string().len())
                        .sum::<usize>()
            }
            Message::ToolResult(r) => r.content.len(),
        }
    }

    /// 🏃 pi run_loop() 同构 — 一轮 = 一次 LLM 调用, 处理该轮**所有** tool_calls
    pub fn run(&mut self) -> Result<Vec<String>> {
        let mut log: Vec<String> = Vec::new();
        self.events.push(AgentEvent::AgentStart);

        for _ in 0..self.config.max_iterations {
            self.turn += 1;
            let turn = self.turn;
            self.events.push(AgentEvent::TurnStart { turn });

            // 1. drain steering/follow_up (pi drain_steering_messages)
            self.drain_queues();

            // 2. compaction (pi maybe_compact at run entry)
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

            // 3. build_context → provider.complete()
            let ctx = self.build_context();
            let (reasoning, calls, usage) = match self.provider.complete(&ctx.messages, &ctx.tools) {
                Ok(r) => r,
                Err(e) => {
                    log.push(format!("[t{turn}] LLM 错误: {e}"));
                    break;
                }
            };
            self.usage = usage; // 用最新一次真实用量

            // 4. 无 tool_calls → 纯文本/结束 (pi: 无 tool_calls 且队列空 → break)
            if calls.is_empty() {
                if let Some(text) = &reasoning {
                    log.push(format!("[t{turn}] 文本回复: {:.200}", text));
                }
                self.events.push(AgentEvent::TurnEnd { turn });
                break;
            }

            self.events.push(AgentEvent::Assistant {
                reasoning: reasoning.clone(),
                calls: calls.iter().map(|(n, _)| n.clone()).collect(),
            });

            // 5. 一条 assistant 消息携带所有 tool_calls (pi: 一轮可能多个 tool_call)
            let call_id = format!("call_{turn}");
            self.messages
                .push(Message::assistant_tool_calls(&call_id, &calls, reasoning.clone()));

            // 6. 按 ToolEffects 分组 (pi plan_tool_effect_batches)
            let effects: Vec<ToolEffects> = calls
                .iter()
                .map(|(n, _)| {
                    self.tools
                        .get(n)
                        .map(|t| t.effects())
                        .unwrap_or(ToolEffects::write())
                })
                .collect();
            let batches = plan_tool_effect_batches(&effects);

            // 7. 逐批执行 (MC 输入设备单一, 批内串行; pi 会在兼容批内 buffer_unordered 并行)
            for batch in &batches {
                for &idx in batch {
                    let (name, args_json) = &calls[idx];
                    let args: Value =
                        serde_json::from_str(args_json).unwrap_or(Value::Null);
                    let sub_id = format!("{call_id}_{idx}");

                    self.events.push(AgentEvent::ToolExecutionStart {
                        name: name.clone(),
                    });
                    let result = match self.tools.get(name) {
                        Some(tool) => tool.execute(&sub_id, args, None),
                        None => Ok(ToolResult {
                            message: format!("未知工具: {name}"),
                            is_error: true,
                        }),
                    };
                    let (msg, is_err) = match result {
                        Ok(r) => (r.message, r.is_error),
                        Err(e) => (format!("执行错误: {e}"), true),
                    };
                    self.events.push(AgentEvent::ToolExecutionEnd {
                        name: name.clone(),
                        is_error: is_err,
                    });

                    self.messages
                        .push(Message::tool_result(&sub_id, name, &msg));
                    self.session.push(SessionEntry {
                        id: sub_id.clone(),
                        parent_id: Some(call_id.clone()),
                        turn,
                        tool: name.clone(),
                        reasoning: reasoning.clone(),
                        detail: format!("{:.120}", msg),
                        timestamp: SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap()
                            .as_millis() as i64,
                    });
                    log.push(format!("[t{turn}] {}({}) → {:.100}", name, args_json, msg));
                }
            }
            self.events.push(AgentEvent::TurnEnd { turn });
        }

        self.events.push(AgentEvent::AgentEnd);
        Ok(log)
    }

    /// pi compact(): LLM 生成历史摘要 (pi generate_summary + SUMMARIZATION_PROMPT)
    fn compact(&mut self) -> Result<()> {
        let keep_tokens = self.config.compaction.keep_recent;
        // find_cut_point: 从尾部累加, 直到 >= keep_recent (pi L998)
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
            return Ok(()); // 没到阈值, 不压缩
        }

        let old: Vec<String> = self.messages[..cut].iter().map(Self::serialize_msg).collect();
        // pi generate_summary: <conversation> 包裹 + 可选 <previous-summary> 增量
        let mut prompt = format!("<conversation>\n{}\n</conversation>\n\n", old.join("\n\n"));
        let system = if let Some(prev) = &self.previous_summary {
            prompt.push_str(&format!("<previous-summary>\n{prev}\n</previous-summary>\n\n"));
            prompt.push_str(UPDATE_SUMMARIZATION_PROMPT);
            COMPACTION_SYSTEM
        } else {
            prompt.push_str(SUMMARIZATION_PROMPT);
            COMPACTION_SYSTEM
        };

        let cm = vec![system_chatml(system), Message::user(prompt).to_chatml()];
        let summary = match self.provider.complete(&cm, &[]) {
            Ok((text, _, _)) => {
                text.unwrap_or_else(|| format!("{} 条消息已压缩", cut))
            }
            Err(_) => format!("{} 条消息已压缩", cut),
        };

        let recent: Vec<_> = self.messages.drain(cut..).collect();
        // pi: 摘要作为 User 消息, 包 <summary> 标签, 置于保留消息之前
        let summary_msg = Message::user(format!(
            "The conversation history before this point was compacted into the following summary:\n\n<summary>\n{summary}\n</summary>"
        ));
        self.messages = vec![summary_msg];
        self.messages.extend(recent);
        self.previous_summary = Some(summary);
        Ok(())
    }

    /// 把一条消息序列化为可读文本给 LLM 做摘要 (pi serialize_conversation)
    fn serialize_msg(m: &Message) -> String {
        match m {
            Message::User(u) => format!("user: {}", u.content),
            Message::Assistant(a) => {
                let mut s = String::new();
                if let Some(r) = &a.reasoning {
                    s.push_str(&format!("[思考] {r}\n"));
                }
                if let Some(c) = &a.content
                    && !c.is_empty()
                {
                    s.push_str(&format!("{c}\n"));
                }
                for tc in &a.tool_calls {
                    s.push_str(&format!("→ {}({})\n", tc.name, tc.arguments));
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

// ── 压缩提示词 (pi compaction.rs SUMMARIZATION_PROMPT, 6 段结构化 + 游戏化) ──

const COMPACTION_SYSTEM: &str = "You are a context summarization assistant. Your task is to read a Minecraft gameplay conversation, then produce a structured summary following the exact format specified.\n\nDo NOT continue the conversation. Do NOT respond to any questions. ONLY output the structured summary.";

const SUMMARIZATION_PROMPT: &str = "The messages above are a conversation to summarize. Create a structured context checkpoint summary that another LLM will use to continue the Minecraft gameplay.\n\nUse this EXACT format:\n\n## Goal\n[What is the agent trying to accomplish? e.g. collect wood, build shelter, find iron]\n\n## Constraints & Preferences\n- [e.g. prefer trees over stone; keep tool durability; avoid creepers at night]\n\n## Progress\n### Done\n- [x] [blocks mined, areas explored, mobs avoided, items crafted]\n### In Progress\n- [ ] [current action]\n### Blocked\n- [ ] [if any]\n\n## Key Decisions\n- **[Decision]**: [why, e.g. mined oak first to craft a crafting table]\n\n## Next Steps\n1. [recommended next action]\n\n## Critical Context\n- [biomes, exact block/creature/item names, relative directions, inventory state]\n\nKeep each section concise. Preserve exact block, creature, and item names.";

const UPDATE_SUMMARIZATION_PROMPT: &str = "The messages above are NEW conversation messages to incorporate into the existing summary in <previous-summary>.\n\nUpdate the existing structured summary with new information. RULES:\n- PRESERVE all existing information from the previous summary\n- ADD new progress, decisions, and context from the new messages\n- UPDATE the Progress section: move items from \"In Progress\" to \"Done\" when completed\n- UPDATE \"Next Steps\" to reflect what should happen now\n- PRESERVE exact block, creature, and item names\n- If something is no longer relevant, you may remove it\n\nUse this EXACT format:\n\n## Goal / ## Constraints & Preferences / ## Progress(Done/In Progress/Blocked) / ## Key Decisions / ## Next Steps / ## Critical Context";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::message::Usage;
    use crate::core::tool::{GameTool, ToolEffects, ToolResult, ToolUpdateFn};
    use serde_json::Value;

    struct DummyTool { name: &'static str, effect: ToolEffects }
    impl GameTool for DummyTool {
        fn name(&self) -> &str { self.name }
        fn description(&self) -> &str { "dummy" }
        fn parameters(&self) -> Value { serde_json::json!({}) }
        fn effects(&self) -> ToolEffects { self.effect }
        fn execute(&self, _id: &str, _args: Value, _on: Option<ToolUpdateFn>) -> anyhow::Result<ToolResult> {
            Ok(ToolResult { message: format!("ran {}", self.name), is_error: false })
        }
    }

    // 假 provider: 第 0 轮返回两个 tool_call, 之后返回空 (结束)
    struct MultiCallProvider { calls: std::sync::atomic::AtomicU32 }
    impl LlmProvider for MultiCallProvider {
        fn complete(&self, _m: &[Value], _t: &[Value]) -> anyhow::Result<(Option<String>, Vec<(String, String)>, Usage)> {
            use std::sync::atomic::Ordering;
            let n = self.calls.fetch_add(1, Ordering::SeqCst);
            if n == 0 {
                Ok((Some("我先看再挖".into()), vec![
                    ("perceive".into(), "{\"prompt\":\"x\"}".into()),
                    ("mine".into(), "{\"ticks\":60}".into()),
                ], Usage::default()))
            } else {
                Ok((None, vec![], Usage::default()))
            }
        }
    }

    #[test]
    fn executes_all_tool_calls_in_one_turn() {
        let mut reg = ToolRegistry::new();
        reg.register(Box::new(DummyTool { name: "perceive", effect: ToolEffects::read() }));
        reg.register(Box::new(DummyTool { name: "mine", effect: ToolEffects::write() }));
        let cfg = AgentConfig::new("sys".into(), 5);
        let mut agent = Agent::new(Box::new(MultiCallProvider { calls: std::sync::atomic::AtomicU32::new(0) }), reg, cfg);
        let log = agent.run().unwrap();
        assert!(log.iter().any(|l| l.contains("perceive")), "perceive 应被执行: {log:?}");
        assert!(log.iter().any(|l| l.contains("mine")), "mine 应被执行: {log:?}");
        // 一轮只产生 1 条 assistant (含 2 tool_calls) + 2 tool_result
        let assistants = agent.messages.iter().filter(|m| matches!(m, Message::Assistant(_))).count();
        assert_eq!(assistants, 1, "一轮只应产生 1 条 assistant 消息");
        let tool_results = agent.messages.iter().filter(|m| matches!(m, Message::ToolResult(_))).count();
        assert_eq!(tool_results, 2, "应产生 2 条 tool_result");
    }

    // 压缩测试: 小窗口, 每轮塞大文本撑爆预算
    struct ManyCallsProvider { n: std::sync::atomic::AtomicU32 }
    impl LlmProvider for ManyCallsProvider {
        fn complete(&self, _m: &[Value], _t: &[Value]) -> anyhow::Result<(Option<String>, Vec<(String, String)>, Usage)> {
            use std::sync::atomic::Ordering;
            let k = self.n.fetch_add(1, Ordering::SeqCst);
            if k < 10 {
                Ok((Some("x".repeat(5000)), vec![("perceive".into(), "{\"prompt\":\"x\"}".into())], Usage::default()))
            } else {
                Ok((None, vec![], Usage::default()))
            }
        }
    }

    #[test]
    fn compaction_triggers_and_summarizes() {
        let mut reg = ToolRegistry::new();
        reg.register(Box::new(DummyTool { name: "perceive", effect: ToolEffects::read() }));
        let cfg = AgentConfig::new("sys".into(), 20)
            .with_compaction(CompactionConfig { context_window: 1000, reserve: 200, keep_recent: 300 });
        let mut agent = Agent::new(Box::new(ManyCallsProvider { n: std::sync::atomic::AtomicU32::new(0) }), reg, cfg);
        let _ = agent.run();
        assert!(agent.previous_summary.is_some(), "压缩后应有 previous_summary");
        assert!(matches!(agent.messages.first(), Some(Message::User(u)) if u.content.contains("<summary>")), "首条消息应为 summary");
        assert!(agent.events.iter().any(|e| matches!(e, AgentEvent::AutoCompactionStart)), "应触发过压缩");
    }
}
