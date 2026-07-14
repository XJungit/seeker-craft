//! Agent 核心 — 严格基于 pi_agent_rust agent.rs 架构
//!
//! pi 的核心:
//!   Agent { provider, tools, messages }
//!   run() → provider.complete() → tool.execute() → loop
//!
//! 我们不做 callback, 不做 Adapter 耦合。
//! Provider = LLM trait, Tool = GameTool trait, Message = 类型化消息。

use crate::core::message::{Message, system_chatml};
use crate::core::tool::ToolRegistry;
use anyhow::Result;
use serde::Serialize;
use serde_json::Value;
use std::time::{SystemTime, UNIX_EPOCH};

// ── LLM Provider trait (pi 的 Provider) ──

/// LLM Provider: Agent 不关心是 LongCat 还是 MiniCPM, 只调这个 trait
pub trait LlmProvider: Send + Sync {
    fn complete(
        &self,
        messages: &[Value],
        tools: &[Value],
    ) -> Result<(Option<String>, Vec<(String, String)>)>;
}

// ── Session 树 (pi session.rs) ──

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

// ── Compaction 配置 (pi ResolvedCompactionSettings) ──

#[derive(Debug, Clone)]
pub struct CompactionConfig {
    /// 模型上下文窗口 token 数
    pub context_window: u32,
    /// 预留 token (给系统 prompt + 后续对话)
    pub reserve: u32,
    /// 保留最近 N token (不被压缩)
    pub keep_recent: u32,
}

impl Default for CompactionConfig {
    fn default() -> Self {
        Self { context_window: 128_000, reserve: 10_240, keep_recent: 12_800 }
    }
}

// ── Agent 配置 ──

pub struct AgentConfig {
    pub prompt: String,
    pub max_turns: u32,
    /// 压缩设置 (pi 风格: 基于 token 预算)
    pub compaction: CompactionConfig,
}

impl AgentConfig {
    pub fn new(prompt: String, max_turns: u32) -> Self {
        Self { prompt, max_turns, compaction: CompactionConfig::default() }
    }
}

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

    /// 🏃 主循环 — pi 的 run_loop()
    pub fn run(&mut self) -> Result<Vec<String>> {
        let mut log: Vec<String> = Vec::new();

        for _ in 1..=self.config.max_turns {
            self.turn += 1;
            let turn = self.turn;

            // Compaction: token 预算检查 (pi 风格)
            if self.estimate_tokens() > self.config.compaction.context_window.saturating_sub(self.config.compaction.reserve) {
                self.compact()?;
            }

            // 组装上下文: system prompt + 历史消息
            let system = system_chatml(&self.config.prompt);
            let mut chatml = vec![system];
            chatml.extend(self.messages.iter().map(Message::to_chatml));

            // provider.complete() → LLM 决策
            let tool_defs = self.tools.to_openai_defs();
            let (reasoning, calls) = match self.provider.complete(&chatml, &tool_defs) {
                Ok(r) => r,
                Err(e) => { log.push(format!("[turn{turn}] {e}")); break; }
            };
            if calls.is_empty() { break; }

            let (name, args_json) = &calls[0];
            let call_id = format!("call_{turn}");
            let args: Value = serde_json::from_str(args_json).unwrap_or_default();

            // 记录 assistant (含思维链)
            if let Some(r) = reasoning.as_ref() {
                self.messages.push(Message::assistant_with_reasoning(format!("→ {name}"), r.clone()));
            } else {
                self.messages.push(Message::assistant_tool_call(&call_id, name, args.clone()));
            }

            // 工具执行 — pi 风格: agent 只调 execute(), 不知道工具做什么
            let result = match self.tools.get(name) {
                Some(tool) => {
                    let r = tool.execute(args)?;
                    r.message
                }
                None => format!("未知工具: {name}")
            };

            self.messages.push(Message::tool_result(&call_id, name, &result));

            // 会话记录 (pi session.rs EntryBase)
            self.session.push(SessionEntry {
                id: call_id,
                parent_id: None,
                turn,
                tool: name.clone(),
                reasoning,
                detail: format!("{:.100}", &result),
                timestamp: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as i64,
            });

            log.push(format!("[turn{turn}] {}({})", name, args_json));
        }

        Ok(log)
    }

    /// 估算当前消息总 token 数 (pi: chars/3 per token)
    fn estimate_tokens(&self) -> u32 {
        let chars: usize = self.messages.iter()
            .map(|m| format!("{}", serde_json::to_string(&m).unwrap_or_default()).len())
            .sum();
        (chars.saturating_div(3)) as u32
    }

    /// Compaction — pi 风格: LLM 摘要 + token 预算
    fn compact(&mut self) -> Result<()> {
        let keep_tokens = self.config.compaction.keep_recent;
        let mut kept = 0u32;
        let mut cut_idx = self.messages.len();

        // 从后往前找到要保留的最近 N 条 (基于 token 估算)
        for (i, msg) in self.messages.iter().enumerate().rev() {
            let t = serde_json::to_string(msg).map(|s| s.len().saturating_div(3) as u32).unwrap_or(0);
            if kept + t > keep_tokens {
                cut_idx = i + 1;
                break;
            }
            kept += t;
        }
        if cut_idx >= self.messages.len() { return Ok(()); }

        // 旧消息拼成人类可读格式
        let old: Vec<String> = self.messages[..cut_idx]
            .iter()
            .map(|m| match m {
                Message::Assistant(a) => {
                    let r = a.reasoning.as_deref().unwrap_or("");
                    let tc = a.tool_calls.first().map(|t| t.name.as_str()).unwrap_or("?");
                    format!("assistant({tc}): {r}")
                }
                Message::ToolResult(r) => format!("result({}): {:.200}", r.tool_name, r.content),
                Message::User(u) => format!("user: {:.200}", u.content),
            })
            .collect();
        let old_text = old.join("\n");

        // LLM 生成摘要
        let summary_prompt = vec![system_chatml(
            "Summarize this gameplay history in 2-3 sentences. Key: what was mined, where explored."
        )];
        let summary_msg = Message::user(format!("{:.3000}", old_text));
        let mut chatml = summary_prompt;
        chatml.push(summary_msg.to_chatml());

        let summary = match self.provider.complete(&chatml, &[]) {
            Ok((text, _)) => text.unwrap_or_else(|| format!("{} actions completed", cut_idx)),
            Err(_) => format!("{} actions completed", cut_idx),
        };

        // 保留最近
        let recent: Vec<_> = self.messages.drain(cut_idx..).collect();
        self.messages = vec![Message::user(format!("[summary] {summary}"))];
        self.messages.extend(recent);

        Ok(())
    }
}
