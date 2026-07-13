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
use crate::core::types::{Action, ExecResult, WorldState};
use anyhow::Result;
use serde_json::Value;

/// Agent 可用工具的 JSON Schema 定义 (OpenAI function calling 格式)
pub type ToolDef = Value;

/// LLM 决策回调: 接收 ChatML 消息数组 + 工具定义, 返回 (tool_name, arguments_json)
pub type DecideFn = dyn Fn(&[Value], &[ToolDef]) -> Result<Vec<(String, String)>>;

/// Agent 配置 (从 agent.toml 的 [agent] 段加载)
#[derive(Debug, Clone, serde::Deserialize)]
pub struct AgentConfig {
    /// 系统提示词 (必填, 短)
    pub system_prompt: String,
    /// 工具定义 JSON (OpenAI function calling 格式数组, 必填)
    pub tools: Vec<ToolDef>,
    /// 最大总轮次
    #[serde(default = "default_max_turns")]
    pub max_turns: u32,
}

fn default_max_turns() -> u32 { 20 }

/// 通用 Agent: 拥有循环、消息历史、game adapter
///
/// Pi 风格: agent.run() 是主入口, 不需要外部编排。
pub struct Agent<A: GameAdapter> {
    pub adapter: A,
    pub config: AgentConfig,
    /// 会话消息历史 (system + assistant tool_calls + tool results)
    pub messages: Vec<Message>,
    /// 最近一次 perceive 的世界状态
    pub last_state: Option<WorldState>,
}

impl<A: GameAdapter> Agent<A> {
    pub fn new(adapter: A, config: AgentConfig) -> Self {
        let mut agent = Self {
            adapter,
            config,
            messages: Vec::new(),
            last_state: None,
        };
        agent.reset_messages();
        agent
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
            // 1. 组装上下文 (系统 prompt + 消息历史) 并调用 LLM
            let system = system_chatml(&self.config.system_prompt);
            let mut chatml: Vec<Value> = vec![system];
            chatml.extend(self.messages.iter().map(Message::to_chatml));

            let calls = match decide(&chatml, &self.config.tools) {
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

            // 3. Agent 内部执行工具
            let result = match name.as_str() {
                "perceive" => {
                    let state = self.adapter.perceive()?;
                    let is_empty = state.detected_targets.is_empty();
                    let list: Vec<_> = state.detected_targets.iter().map(|t| t.label.clone()).collect();
                    self.last_state = Some(state);
                    if is_empty {
                        "观察了周围。VLM未检测到3D物体。应look或move_forward。".into()
                    } else {
                        format!("观察了周围。检测到: {}。选一个aim_and_mine挖掘。", list.join("、"))
                    }
                }
                "aim_and_mine" => {
                    let args: serde_json::Value = serde_json::from_str(args_json).unwrap_or_default();
                    let target = args["target"].as_str().unwrap_or("?").to_string();
                    let r = self.adapter.execute(Action::AimAndMine { target })?;
                    format!("转动视角对准目标并挖掘2秒。{}", r.detail)
                }
                "move_forward" => {
                    let args: serde_json::Value = serde_json::from_str(args_json).unwrap_or_default();
                    let ticks = args["ticks"].as_u64().unwrap_or(80) as u32;
                    self.adapter.execute(Action::Move { dir: crate::core::types::Direction::Forward, ticks })?;
                    format!("向前移动{:.1}秒。场景已变化。", ticks as f32 * 0.05)
                }
                "look" => {
                    let args: serde_json::Value = serde_json::from_str(args_json).unwrap_or_default();
                    let dx = args["dx"].as_i64().unwrap_or(200) as i32;
                    let dy = args["dy"].as_i64().unwrap_or(0) as i32;
                    self.adapter.execute(Action::Look { dx, dy })?;
                    let dir = if dx > 0 { "右" } else if dx < 0 { "左" } else { "前" };
                    format!("向{dir}转动视角(dx={dx},dy={dy})。")
                }
                _ => format!("未知工具: {name}")
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
            system_prompt: "test".into(),
            tools: vec![],
            max_turns: 1,
        };
        let mut agent = Agent::new(FakeGameAdapter, config);
        let log = agent.run(
            &|_msgs, _tools| Ok(vec![]),
        ).unwrap();
        assert!(log.is_empty()); // 空 calls → 立即退出
    }
}
