//! Agent 主循环 — Pi 风格：Agent 拥有循环、工具定义、消息历史。
//!
//! 参考 Pi Coding Agent: while tool_calls -> execute -> observe -> continue
//! 参考 SillyTavern: 多层 prompt 组装 + 丰富的 tool result 文本
//!
//! 设计原则:
//! - Agent 不感知具体游戏 (Minecraft/StarCraft)
//! - 工具执行通过闭包注入，Agent 只负责编排
//! - 系统 prompt 从 config 加载，不在代码里硬编码
//! - 消息历史完整积累，LLM 知道之前所有行动的后果

use crate::core::adapter::GameAdapter;
use crate::core::message::{Message, system_chatml};
use crate::core::prompt::PromptBuilder;
use crate::core::tool::ToolRegistry;
use crate::core::types::{Action, ExecResult, WorldState};
use anyhow::Result;
use serde_json::Value;

/// LLM 决策回调: 接收 ChatML 消息数组 + 工具定义, 返回 (tool_name, arguments_json)
pub type DecideFn = dyn Fn(&[Value], &[Value]) -> Result<Vec<(String, String)>>;

/// Agent 配置
#[derive(Debug, Clone)]
pub struct AgentConfig {
    /// 五层 prompt 组装器 (酒馆风格)
    pub prompt: PromptBuilder,
    /// 最大总轮次
    pub max_turns: u32,
}

/// 通用 Agent: 拥有循环、消息历史、game adapter
///
/// Pi 风格: agent.run() 是主入口, 不需要外部编排。
pub struct Agent<A: GameAdapter> {
    pub adapter: A,
    pub config: AgentConfig,
    /// 工具注册表 (pi 风格: Vec<Box<dyn GameTool>>)
    pub tools: ToolRegistry,
    /// 会话消息历史
    pub messages: Vec<Message>,
    /// 最近一次 perceive 的世界状态
    pub last_state: Option<WorldState>,
}

impl<A: GameAdapter> Agent<A> {
    pub fn new(adapter: A, config: AgentConfig, tools: ToolRegistry) -> Self {
        Self {
            adapter,
            config,
            tools,
            messages: Vec::new(),
            last_state: None,
        }
    }

    /// 重置消息历史
    pub fn reset_messages(&mut self) {
        self.messages = Vec::new();
    }

    /// 🏃 运行 Agent 主循环 (Pi 风格: 工具在 Agent 内部执行)
    ///
    /// `decide` — 调用 LLM 返回 tool_calls
    ///
    /// 循环: LLM决定 -> Agent内部执行 -> 结果注入 -> LLM再次决定 -> ... -> 结束
    pub fn run(
        &mut self,
        decide: &DecideFn,
    ) -> Result<Vec<String>> {
        let mut log: Vec<String> = Vec::new();

        for turn in 1..=self.config.max_turns {
            // 1. 组装 system prompt (五层) 并调用 LLM
            let system_prompt = self.config.prompt.build();
            let system = system_chatml(&system_prompt);
            let mut chatml: Vec<Value> = vec![system];
            chatml.extend(self.messages.iter().map(Message::to_chatml));

            let tool_defs = self.tools.to_openai_defs();
            let calls = match decide(&chatml, &tool_defs) {
                Ok(c) => c,
                Err(e) => {
                    let msg = format!("[turn{turn}] LLM error: {e}");
                    log.push(msg);
                    break;
                }
            };
            if calls.is_empty() { break; }

            // 取第一个 tool call
            let (name, args_json) = &calls[0];
            let call_id = format!("call_{turn}");

            // 2. 记录 assistant 的 tool_call (类型化)
            let args: Value = serde_json::from_str(args_json).unwrap_or_default();
            self.messages.push(Message::assistant_tool_call(
                &call_id, name, args.clone(),
            ));

            // 3. Agent 内部执行工具 (pi 风格: 先查 registry 验证工具存在)
            let result = if self.tools.get(name).is_none() {
                format!("未知工具: {name}")
            } else {
                let args: Value = serde_json::from_str(args_json).unwrap_or_default();
                match name.as_str() {
                    "perceive" => {
                        let state = self.adapter.perceive()?;
                        let is_empty = state.detected_targets.is_empty();
                        let list: Vec<_> = state.detected_targets.iter().map(|t| t.label.clone()).collect();
                        let raw = state.scene_desc.clone();
                        self.last_state = Some(state);
                        if is_empty {
                            format!("VLM原文:\n{raw}\n\n目标: 无。应look或move_forward。")
                        } else {
                            format!("VLM原文:\n{raw}\n\n解析目标: {}。选一个aim_and_mine。", list.join("、"))
                        }
                    }
                    "aim_and_mine" => {
                        let target = args["target"].as_str().unwrap_or("?").to_string();
                        let r = self.adapter.execute(Action::AimAndMine { target })?;
                        format!("转动视角对准目标并挖掘2秒。{}", r.detail)
                    }
                    "move_forward" => {
                        let ticks = args["ticks"].as_u64().unwrap_or(80) as u32;
                        self.adapter.execute(Action::Move { dir: crate::core::types::Direction::Forward, ticks })?;
                        format!("向前移动{:.1}秒。场景已变化。", ticks as f32 * 0.05)
                    }
                    "look" => {
                        let dx = args["dx"].as_i64().unwrap_or(200) as i32;
                        let dy = args["dy"].as_i64().unwrap_or(0) as i32;
                        self.adapter.execute(Action::Look { dx, dy })?;
                        let dir = if dx > 0 { "右" } else if dx < 0 { "左" } else { "前" };
                        format!("向{dir}转动视角(dx={dx},dy={dy})。")
                    }
                    _ => format!("未实现的工具: {name}")
                }
            };

            // 4. 工具结果注入 (类型化)
            self.messages.push(Message::tool_result(&call_id, name, &result));

            log.push(format!("[turn{turn}] {}({})", name, args_json));
        }

        Ok(log)
    }

    // ── 以下为兼容旧 API 的辅助方法 ──

    /// 执行 perceive：VLM 拍照 → 更新 last_state
    pub fn perceive(&mut self) -> Result<&WorldState> {
        let state = self.adapter.perceive()?;
        self.last_state = Some(state);
        Ok(self.last_state.as_ref().unwrap())
    }

    /// 执行游戏动作
    pub fn execute(&mut self, action: Action) -> Result<ExecResult> {
        self.adapter.execute(action)
    }

    /// 单步 (兼容旧 API)
    pub fn step(&mut self) -> Result<ExecResult> {
        let _state = self.adapter.perceive()?;
        let action = Action::Look { dx: 0, dy: 0 };
        self.adapter.execute(action)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::fake::FakeGameAdapter;

    #[test]
    fn agent_runs_basic_loop() {
        let config = AgentConfig {
            prompt: PromptBuilder::new().identity("test"),
            max_turns: 1,
        };
        let mut agent = Agent::new(FakeGameAdapter, config, ToolRegistry::new());
        let log = agent.run(
            &|_msgs, _tools| Ok(vec![]),
        ).unwrap();
        assert!(log.is_empty());
    }
}
