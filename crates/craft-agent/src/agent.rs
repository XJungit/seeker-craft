//! Agent 主循环骨架（与 game-agent-design.md §3 闭环对齐）
//!
//! 完整版含 记忆 / 规划 / 决策 / Critic / 反思，此处先串起
//! `感知 → 决策(占位) → 执行` 的最小闭环，便于离线验证类型与主循环。

use crate::core::adapter::GameAdapter;
use crate::core::types::{Action, ExecResult, WorldState};
use anyhow::Result;

/// 通用 Agent：持有某个 GameAdapter，驱动单步闭环
pub struct Agent<A: GameAdapter> {
    pub adapter: A,
}

impl<A: GameAdapter> Agent<A> {
    pub fn new(adapter: A) -> Self {
        Self { adapter }
    }

    /// 单步：感知 → 决策（占位）→ 执行
    pub fn step(&mut self) -> Result<ExecResult> {
        let state: WorldState = self.adapter.perceive()?;
        // TODO(Phase 1): 记忆检索 + 层次规划 + LLM 决策 + Critic 自验证
        let action: Action = self.decide(&state);
        self.adapter.execute(action)
    }

    /// 占位决策：先返回"空转视角"——真实实现由 LLM/规划层产出 Action
    fn decide(&self, _state: &WorldState) -> Action {
        Action::Look { dx: 0, dy: 0 }
    }
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
