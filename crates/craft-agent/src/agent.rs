//! Agent 核心 — 基于 pi_agent_rust agent.rs / model.rs / compaction.rs
//!
//! 与 pi 同构:
//!   Agent { provider, tools, messages, config }
//!   run() → build_context() → provider.complete() → tool.execute() → loop
//!   compact() → LLM 摘要 → 替换旧消息

use crate::core::message::{Message, system_chatml};
use crate::core::tool::ToolRegistry;
use anyhow::Result;
use serde::Serialize;
use serde_json::Value;
use std::time::{SystemTime, UNIX_EPOCH};

// ── Compaction 提示词 (pi compaction.rs SUMMARIZATION_PROMPT) ──

const COMPACTION_SYSTEM: &str = "You are a context summarization assistant. Read gameplay history, output structured summary. Do NOT continue, ONLY output the summary.";

const COMPACTION_PROMPT: &str = "Summarize this Minecraft gameplay. Use EXACT format:\n\n\
## Goal\n[What was the agent trying to achieve?]\n\n\
## Progress\n### Done\n- [x] [Completed: blocks mined, areas explored]\n\n\
### In Progress\n- [ ] [Current action]\n\n\
## Observations\n- [Biomes, blocks, creatures seen]\n\n\
## Next\n1. [Recommended next action]\n\n\
Concise. Preserve block and creature names.";

// ── Provider ──

pub trait LlmProvider: Send + Sync {
    fn complete(
        &self, messages: &[Value], tools: &[Value],
    ) -> Result<(Option<String>, Vec<(String, String)>)>;
}

// ── Session ──

#[derive(Debug, Clone, Serialize)]
pub struct SessionEntry {
    pub id: String, pub parent_id: Option<String>, pub turn: u32,
    pub tool: String, pub reasoning: Option<String>,
    pub detail: String, pub timestamp: i64,
}

// ── Compaction (pi ResolvedCompactionSettings) ──

#[derive(Debug, Clone)]
pub struct CompactionConfig {
    pub context_window: u32,  // 模型上下文窗口 token 数
    pub reserve: u32,         // 预留 token (给后续对话)
    pub keep_recent: u32,     // 保留最近 N token
}
impl Default for CompactionConfig {
    fn default() -> Self { Self { context_window: 1_000_000, reserve: 200_000, keep_recent: 200_000 } }
}

// ── Config ──

pub struct AgentConfig {
    pub prompt: String,
    pub max_iterations: u32,  // pi: max_tool_iterations
    pub compaction: CompactionConfig,
}
impl AgentConfig {
    pub fn new(prompt: String, max_iterations: u32) -> Self {
        Self { prompt, max_iterations, compaction: CompactionConfig::default() }
    }
}

// ── Minecraft 完整知识 (酒馆 World Info 风格, 注入到 prompt) ──

pub const MC_KNOWLEDGE: &str = r#"
## Minecraft 控制知识
### 移动
- press w: 前进
- press a: 左移
- press s: 后退  
- press d: 右移
- press space: 跳跃 (1格高)
- press shift: 潜行 (不会从边缘掉落)
### 交互
- press e: 打开/关闭背包
- press 1~9: 切换到快捷栏对应物品
- 鼠标左键=挖掘/攻击, 右键=放置/使用
### 视角
- look dx=N: 水平转视角 (300≈90度, 150≈45度)
- look dy=N: 垂直转视角 (正值低头, 负值抬头)
### 挖掘
- mine ticks=60: 按住左键挖3秒 (木头/泥土)
- mine ticks=120: 6秒 (石头)
- mine ticks=200: 10秒 (矿石, 不用工具时)
### 生存知识
- 优先收集木头(tree) → 做木斧/木镐
- 石头(stone) → 圆石 → 石镐 → 挖铁
- 晚上要躲怪物: 挖三格深的洞, 上面盖住
- 快捷栏物品: press 1~9 切换
- 背包满了: press e 打开背包整理
"#;

// ── Agent (pi 同构) ──

pub struct Agent {
    pub provider: Box<dyn LlmProvider>,
    pub tools: ToolRegistry,
    pub messages: Vec<Message>,
    pub session: Vec<SessionEntry>,
    pub config: AgentConfig,
    turn: u32,
}

impl Agent {
    pub fn new(provider: Box<dyn LlmProvider>, tools: ToolRegistry, config: AgentConfig) -> Self {
        Self { provider, tools, config, messages: vec![], session: vec![], turn: 0 }
    }

    /// pi build_context(): system prompt + messages → Context
    fn build_context(&self) -> (String, Vec<Value>, Vec<Value>) {
        let full_prompt = format!("{}\n\n{}", self.config.prompt, MC_KNOWLEDGE);
        let system = system_chatml(&full_prompt);
        let mut chatml = vec![system];
        chatml.extend(self.messages.iter().map(Message::to_chatml));
        let tool_defs = self.tools.to_openai_defs();
        (full_prompt, chatml, tool_defs)
    }

    /// pi estimate_tokens(): chars / 3
    fn estimate_tokens(&self) -> u32 {
        let chars: usize = self.messages.iter()
            .map(|m| format!("{}", serde_json::to_string(m).unwrap_or_default()).len())
            .sum();
        (chars.saturating_div(3) + self.config.prompt.len().saturating_div(3)) as u32
    }

    /// 🏃 pi run_loop() 同构
    pub fn run(&mut self) -> Result<Vec<String>> {
        let mut log: Vec<String> = Vec::new();

        for _ in 0..self.config.max_iterations {
            self.turn += 1;
            let turn = self.turn;

            // Compaction
            let budget = self.config.compaction.context_window.saturating_sub(self.config.compaction.reserve);
            if self.estimate_tokens() > budget {
                self.compact()?;
            }

            // build_context → provider.complete()
            let (_, chatml, tools) = self.build_context();
            let (reasoning, calls) = match self.provider.complete(&chatml, &tools) {
                Ok(r) => r,
                Err(e) => { log.push(format!("[t{turn}] {e}")); break; }
            };
            if calls.is_empty() { break; }

            let (name, args_json) = &calls[0];
            let call_id = format!("call_{turn}");
            let args: Value = serde_json::from_str(args_json).unwrap_or_default();

            // 记录 assistant (含思维链)
            if let Some(r) = reasoning.as_ref() {
                self.messages.push(Message::assistant_with_reasoning(
                    format!("→ {name}"), r.clone(),
                ));
            } else {
                self.messages.push(Message::assistant_tool_call(&call_id, name, args.clone()));
            }

            // tool.execute() — Agent 不关心工具做什么
            let result = match self.tools.get(name) {
                Some(tool) => {
                    let r = tool.execute(&call_id, args)?;
                    r.message
                }
                None => format!("未知: {name}")
            };

            self.messages.push(Message::tool_result(&call_id, name, &result));

            self.session.push(SessionEntry {
                id: call_id, parent_id: None, turn,
                tool: name.clone(), reasoning,
                detail: format!("{:.100}", &result),
                timestamp: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as i64,
            });

            log.push(format!("[t{turn}] {}({args_json})", name));
        }

        Ok(log)
    }

    /// pi compact(): LLM 生成历史摘要
    fn compact(&mut self) -> Result<()> {
        let keep_tokens = self.config.compaction.keep_recent;
        let mut kept = 0u32;
        let mut cut = self.messages.len();

        for (i, msg) in self.messages.iter().enumerate().rev() {
            let t = serde_json::to_string(msg).map(|s| s.len() as u32 / 3).unwrap_or(0);
            if kept + t > keep_tokens { cut = i + 1; break; }
            kept += t;
        }
        if cut >= self.messages.len() || cut == 0 { return Ok(()); }

        let old: Vec<String> = self.messages[..cut].iter().map(|m| match m {
            Message::Assistant(a) => {
                let tc = a.tool_calls.first().map(|t| t.name.as_str()).unwrap_or("");
                format!("{}{}", tc, a.reasoning.as_deref().map(|r| format!(": {r}")).unwrap_or_default())
            }
            Message::ToolResult(r) => format!("result({}): {:.200}", r.tool_name, r.content),
            Message::User(u) => format!("user: {:.200}", u.content),
        }).collect();

        let cm = vec![
            system_chatml(COMPACTION_SYSTEM),
            Message::user(format!("<conversation>\n{}\n</conversation>\n\n{COMPACTION_PROMPT}", old.join("\n"))).to_chatml(),
        ];
        let summary = self.provider.complete(&cm, &[])
            .map(|(t, _)| t.unwrap_or_else(|| format!("{} actions", cut)))
            .unwrap_or_else(|_| format!("{} actions", cut));

        let recent = self.messages.drain(cut..).collect::<Vec<_>>();
        self.messages = vec![Message::user(format!("[summary] {summary}"))];
        self.messages.extend(recent);
        Ok(())
    }
}
