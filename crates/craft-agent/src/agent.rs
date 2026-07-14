//! Agent 主循环 — 基于 pi_agent_rust 架构
//!
//! 参考: pi_agent_rust (agent.rs / model.rs / tools.rs / session.rs / compaction.rs)
//! 参考: SillyTavern (PromptManager.js / world-info.js)
//!
//! 已落地模式:
//! - Message enum (pi model.rs)
//! - Tool trait + ToolRegistry (pi tools.rs)
//! - PromptBuilder 五层 (酒馆 PromptManager)
//! - Compaction 上下文压缩 (pi compaction.rs)
//! - SessionEntry id/parentId 树 (pi session.rs)
//! - WorldInfo 动态注入 (酒馆 world-info.js)
//! - ToolEffects 副作用声明 (pi ToolEffects)

use crate::core::adapter::GameAdapter;
use crate::core::message::{Message, system_chatml};
use crate::core::prompt::PromptBuilder;
use crate::core::tool::ToolRegistry;
use crate::core::types::{Action, WorldState};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::{SystemTime, UNIX_EPOCH};

// ── Session 树 (pi session.rs SessionEntry) ──

/// 会话条目: id/parentId 树结构 (pi 风格)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionEntry {
    pub id: String,
    pub parent_id: Option<String>,
    pub turn: u32,
    pub tool: String,
    pub detail: String,
    pub timestamp: i64,
}

// ── Agent 配置 ──

pub type DecideFn = dyn Fn(&[Value], &[Value]) -> Result<(Option<String>, Vec<(String, String)>)>;

#[derive(Debug, Clone)]
pub struct AgentConfig {
    pub prompt: PromptBuilder,
    pub max_turns: u32,
    /// 触发压缩的消息数阈值 (pi: MAX_AGENT_MESSAGES)
    pub max_messages: usize,
}

impl AgentConfig {
    pub fn new(prompt: PromptBuilder, max_turns: u32) -> Self {
        Self { prompt, max_turns, max_messages: 50 }
    }
}

// ── Agent 结构 ──

pub struct Agent<A: GameAdapter> {
    pub adapter: A,
    pub config: AgentConfig,
    pub tools: ToolRegistry,
    pub messages: Vec<Message>,
    /// 会话树 (pi session.rs: Vec<SessionEntry>)
    pub session: Vec<SessionEntry>,
    pub last_state: Option<WorldState>,
    turn: u32,
}

impl<A: GameAdapter> Agent<A> {
    pub fn new(adapter: A, config: AgentConfig, tools: ToolRegistry) -> Self {
        Self {
            adapter, config, tools,
            messages: Vec::new(),
            session: Vec::new(),
            last_state: None,
            turn: 0,
        }
    }

    pub fn reset_messages(&mut self) {
        self.messages.clear();
        self.session.clear();
        self.turn = 0;
    }

    /// 🏃 Agent 主循环 —— 基于 pi agent loop
    pub fn run(&mut self, decide: &DecideFn) -> Result<Vec<String>> {
        let mut log: Vec<String> = Vec::new();
        let parent_id = None; // 顶级分支

        for _ in 1..=self.config.max_turns {
            self.turn += 1;
            let turn = self.turn;

            // 1. 压缩检测 (pi compaction.rs)
            if self.messages.len() > self.config.max_messages {
                self.compact()?;
            }

            // 2. 组装上下文
            let system_prompt = self.config.prompt.build();
            let system = system_chatml(&system_prompt);
            let mut chatml: Vec<Value> = vec![system];
            chatml.extend(self.messages.iter().map(Message::to_chatml));

            let tool_defs = self.tools.to_openai_defs();
            let (reasoning, calls) = match decide(&chatml, &tool_defs) {
                Ok(r) => r,
                Err(e) => { log.push(format!("[turn{turn}] LLM: {e}")); break; }
            };
            if calls.is_empty() { break; }

            let (name, args_json) = &calls[0];
            let call_id = format!("call_{turn}");

            // 3. 记录 assistant (含思维链, pi model.rs ThinkingContent)
            let tool = self.tools.get(name);
            let args: Value = serde_json::from_str(args_json).unwrap_or_default();
            if let Some(reason) = reasoning {
                self.messages.push(Message::assistant_with_reasoning(
                    format!("→ {name}"), reason,
                ));
            } else {
                self.messages.push(Message::assistant_tool_call(&call_id, name, args.clone()));
            }

            // 5. 执行工具
            let result = if tool.is_none() {
                format!("未知工具: {name}")
            } else {
                match name.as_str() {
                    "perceive" => {
                        let prompt = args["prompt"].as_str()
                            .unwrap_or("Describe the Minecraft scene. List visible blocks and entities.");
                        let reply = self.adapter.perceive_with_prompt(prompt)?;
                        // WorldInfo 注入感知结果到场景层 (酒馆 world-info.js)
                        self.config.prompt.set_scenario(format!("最近观察: {:.300}", &reply));
                        reply
                    }
                    "press" => {
                        let keys = args["keys"].as_str().unwrap_or("w").to_string();
                        let ticks = args["ticks"].as_u64().unwrap_or(40) as u32;
                        self.adapter.execute(Action::Press { keys: keys.clone(), ticks })?;
                        format!("按键 {} {}ms", keys, ticks as u64 * 50)
                    }
                    "look" => {
                        let dx = args["dx"].as_i64().unwrap_or(0) as i32;
                        let dy = args["dy"].as_i64().unwrap_or(0) as i32;
                        self.adapter.execute(Action::Look { dx, dy })?;
                        format!("转动视角 dx={dx} dy={dy}")
                    }
                    "mine" => {
                        let ticks = args["ticks"].as_u64().unwrap_or(60) as u32;
                        self.adapter.execute(Action::Mine { ticks })?;
                        format!("挖掘 {}ms", ticks as u64 * 50)
                    }
                    _ => format!("未实现: {name}")
                }
            };

            // 6. 工具结果注入
            self.messages.push(Message::tool_result(&call_id, name, &result));

            // 7. 会话树记录 (pi session.rs SessionEntry)
            let entry = SessionEntry {
                id: call_id.clone(),
                parent_id: parent_id.clone(),
                turn,
                tool: name.clone(),
                detail: format!("{:.100}", &result),
                timestamp: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as i64,
            };
            self.session.push(entry);

            log.push(format!("[turn{turn}] {}({})", name, args_json));
        }

        Ok(log)
    }

    /// 上下文压缩 (pi compaction.rs 模式)
    /// 保留最近 10 条消息，把旧消息替换为摘要
    fn compact(&mut self) -> Result<()> {
        let keep = 10; // 保留最近 N 条
        if self.messages.len() <= keep { return Ok(()); }

        let old_count = self.messages.len() - keep;
        let summary = format!(
            "[上下文压缩] 前 {} 轮已完成。继续当前任务。",
            old_count
        );

        // 替换旧消息为压缩摘要
        let compacted = Message::user(summary);
        let recent: Vec<_> = self.messages.drain(self.messages.len() - keep..).collect();
        self.messages = vec![compacted];
        self.messages.extend(recent);

        Ok(())
    }

    /// Fork 会话分支 (pi session.rs tree)
    pub fn fork(&self, from_id: &str) -> Agent<A>
    where A: Clone
    {
        let mut forked = Agent {
            adapter: self.adapter.clone(),
            config: self.config.clone(),
            tools: ToolRegistry::new(), // 简化: 重新注册
            messages: Vec::new(),
            session: Vec::new(),
            last_state: None,
            turn: 0,
        };

        // 复制到分叉点的消息
        let mut found = false;
        for entry in &self.session {
            forked.session.push(entry.clone());
            if entry.id == from_id { found = true; break; }
        }
        if found {
            // 找到分叉点对应的消息
            let idx = self.session.iter().position(|e| e.id == from_id).unwrap_or(0);
            if idx < self.messages.len() {
                forked.messages = self.messages[..=idx].to_vec();
            }
        }

        forked
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::fake::FakeGameAdapter;

    #[test]
    fn agent_compacts_when_full() {
        let config = AgentConfig::new(PromptBuilder::new().identity("test"), 1);
        let mut agent = Agent::new(FakeGameAdapter, config, ToolRegistry::new());
        // 填充超过 50 条消息触发压缩
        for i in 0..55 {
            agent.messages.push(Message::user(format!("msg {i}")));
        }
        assert_eq!(agent.messages.len(), 55);
        agent.compact().unwrap();
        assert!(agent.messages.len() <= 11); // 1 compacted + 10 recent
    }
}
