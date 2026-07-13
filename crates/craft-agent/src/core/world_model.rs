//! 世界模型接口（预留，与 game-agent-design.md §4.7 对齐）
//!
//! Phase 3 接入 DreamerV3 / V-JEPA2 / Genie 等后端。
//! 后端多为 Python/PyTorch：可用 ONNX 导出后 `ort` 推理，或起 Python 推理
//! sidecar 供 Rust 通过 HTTP/gRPC 调用。

use crate::core::types::{Action, WorldState};

/// 潜空间状态
#[derive(Debug, Clone)]
pub struct Latent(pub Vec<f32>);

/// 动作序列计划
#[derive(Debug, Clone)]
pub struct Plan(pub Vec<Action>);

/// 状态轨迹（rollout 结果）
#[derive(Debug, Clone)]
pub struct Trajectory(pub Vec<WorldState>);

/// 世界模型接口：让 agent 在"想象"中试错再执行
pub trait WorldModel {
    /// 把世界状态编码进潜空间
    fn encode(&self, state: &WorldState) -> Latent;

    /// 在潜空间中预测某动作后的下一状态
    fn predict(&self, latent: &Latent, action: &Action) -> Latent;

    /// 从给定状态按计划 rollout 出轨迹
    fn rollout(&self, state: &WorldState, plan: &Plan) -> Trajectory;
}
