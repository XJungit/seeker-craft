//! Agent 主循环骨架（与 game-agent-design.md §3 闭环对齐）
//!
//! 完整版含 记忆 / 规划 / 决策 / Critic / 反思，此处先串起
//! `感知 → 决策(占位) → 执行` 的最小闭环，便于离线验证类型与主循环。
//!
//! P2 新增：`step_with_tools` — LLM 自主决定何时调用 VLM 感知、何时执行动作。

use crate::core::adapter::GameAdapter;
use crate::core::types::{Action, ExecResult, WorldState};
use anyhow::Result;

/// LLM 可用的工具枚举
#[derive(Debug, Clone)]
pub enum AgentTool {
    /// 执行拍照 + VLM 感知，返回 WorldState
    Perceive,
    /// 直接执行动作
    Act(Action),
    /// 等待/思考（空转）
    Think(String),
}

/// 通用 Agent：持有某个 GameAdapter，驱动单步闭环
pub struct Agent<A: GameAdapter> {
    pub adapter: A,
    /// 最近一次 perceive 的世界状态（LLM 工具调用后可复用）
    pub last_state: Option<WorldState>,
}

impl<A: GameAdapter> Agent<A> {
    pub fn new(adapter: A) -> Self {
        Self {
            adapter,
            last_state: None,
        }
    }

    /// 单步：感知 → 决策（占位）→ 执行
    pub fn step(&mut self) -> Result<ExecResult> {
        let state: WorldState = self.adapter.perceive()?;
        // TODO(Phase 1): 记忆检索 + 层次规划 + LLM 决策 + Critic 自验证
        let action: Action = self.decide(&state);
        self.adapter.execute(action)
    }

    /// 单步（外部决策函数）：感知 → 外部决策 → 执行
    pub fn step_with<F>(&mut self, decide: F) -> Result<ExecResult>
    where
        F: FnOnce(&WorldState) -> Result<Action>,
    {
        let state = self.adapter.perceive()?;
        let action = decide(&state)?;
        self.adapter.execute(action)
    }

    /// **LLM 动态工具调用**：LLM 返回 AgentTool 而非 Action
    /// - `Perceive` → 调用 VLM，更新 last_state，不执行动作
    /// - `Act(action)` → 执行动作
    /// - `Think(reason)` → 仅记录思考，不行动
    pub fn step_tools<F>(&mut self, decide: F) -> Result<ToolStepResult>
    where
        F: FnOnce(Option<&WorldState>) -> Result<AgentTool>,
    {
        let tool = decide(self.last_state.as_ref())?;
        match tool {
            AgentTool::Perceive => {
                let state = self.adapter.perceive()?;
                let detail = format!(
                    "perceive: {} targets, {} elements, scene={:.60}...",
                    state.detected_targets.len(),
                    state.marked_elements.len(),
                    truncate_str(&state.scene_desc, 60)
                );
                self.last_state = Some(state);
                Ok(ToolStepResult {
                    action_taken: false,
                    detail,
                })
            }
            AgentTool::Act(action) => {
                let result = self.adapter.execute(action)?;
                Ok(ToolStepResult {
                    action_taken: true,
                    detail: result.detail,
                })
            }
            AgentTool::Think(reason) => Ok(ToolStepResult {
                action_taken: false,
                detail: format!("think: {reason}"),
            }),
        }
    }

    /// 占位决策：先返回"空转视角"——真实实现由 LLM/规划层产出 Action
    fn decide(&self, _state: &WorldState) -> Action {
        Action::Look { dx: 0, dy: 0 }
    }
}

/// 工具调用步骤的结果
pub struct ToolStepResult {
    /// 是否执行了真实动作（反之只是感知或思考）
    pub action_taken: bool,
    pub detail: String,
}

/// 安全截断 UTF-8 字符串，不会在多字节字符中间切开
fn truncate_str(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes { return s; }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::fake::FakeGameAdapter;

    #[test]
    fn fake_agent_runs_one_step() {
        let mut agent = Agent::new(FakeGameAdapter);
        let res = agent.step().expect("step 不应失败");
        assert!(res.ok, "fake 执行应成功: {:?}", res.detail);
    }
}
