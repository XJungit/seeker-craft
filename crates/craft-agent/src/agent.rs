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

// ── Agent 配置 ──

pub struct AgentConfig {
    pub prompt: String, // 直接用字符串, 不用 PromptBuilder
    pub max_turns: u32,
    pub max_messages: usize,
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

            // Compaction (pi compaction.rs: LLM 摘要)
            if self.messages.len() > self.config.max_messages {
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

    /// Compaction — pi 风格: LLM 生成摘要
    fn compact(&mut self) -> Result<()> {
        let keep = 10;
        if self.messages.len() <= keep { return Ok(()); }

        // 把旧消息拼成文本, 让 LLM 生成摘要
        let old: Vec<String> = self.messages[..self.messages.len() - keep]
            .iter()
            .map(|m| format!("{:?}", m))
            .collect();
        let old_text = old.join("\n");

        // 调 LLM 生成摘要 (简化: 直接用 provider)
        let summary_prompt = vec![system_chatml(
            "Summarize this Minecraft gameplay history in 2-3 sentences. Keep key facts: \
             what was mined, where we explored, what we saw.\n\nHISTORY:"
        )];
        let summary_msg = Message::user(format!("{:.2000}", old_text));
        let mut chatml = summary_prompt;
        chatml.push(summary_msg.to_chatml());

        let summary = match self.provider.complete(&chatml, &[]) {
            Ok((text, _)) => text.unwrap_or_else(|| format!("前 {} 轮已完成", self.messages.len() - keep)),
            Err(_) => format!("[压缩] 前 {} 轮已完成", self.messages.len() - keep),
        };

        // 保留最近
        let recent: Vec<_> = self.messages.drain(self.messages.len() - keep..).collect();
        self.messages = vec![Message::user(format!("[历史摘要] {summary}"))];
        self.messages.extend(recent);

        Ok(())
    }
}
