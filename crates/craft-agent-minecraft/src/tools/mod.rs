//! Minecraft 工具定义 — 四个 struct 实现 GameTool trait
//!
//! 参考 pi_agent_rust: 每个工具一个 struct, impl Tool trait。
//! 差异: pi 工具操作文件系统 (独立), 游戏工具需要 Adapter 上下文,
//! 因此 execute() 由 Agent::run() 接管, 工具 struct 只提供元数据。

use craft_agent::core::tool::{GameTool, ToolEffects, ToolResult};
use serde_json::Value;

// ── Perceive (感知) ──

pub struct PerceiveTool;

impl GameTool for PerceiveTool {
    fn name(&self) -> &str { "perceive" }
    fn description(&self) -> &str {
        "拍照识别3D世界物体(树/石头/水/动物/矿石), 返回物体列表。"
    }
    fn parameters(&self) -> Value { serde_json::json!({}) }
    fn effects(&self) -> ToolEffects { ToolEffects::read_only() }
    fn execute(&self, _args: Value) -> anyhow::Result<ToolResult> {
        // 实际执行在 Agent::run() 中 (需要 Adapter 上下文)
        unreachable!("perceive executed by Agent")
    }
}

// ── AimAndMine (瞄准挖掘) ──

pub struct AimAndMineTool;

impl GameTool for AimAndMineTool {
    fn name(&self) -> &str { "aim_and_mine" }
    fn description(&self) -> &str {
        "转动视角对准指定目标并挖掘2秒。"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "target": {
                    "type": "string",
                    "description": "目标名称(tree/stone/water/dirt/ore等)"
                }
            },
            "required": ["target"]
        })
    }
    fn execute(&self, _args: Value) -> anyhow::Result<ToolResult> {
        unreachable!("aim_and_mine executed by Agent")
    }
}

// ── MoveForward (前进) ──

pub struct MoveForwardTool;

impl GameTool for MoveForwardTool {
    fn name(&self) -> &str { "move_forward" }
    fn description(&self) -> &str {
        "按住W键向前移动探索新区域。"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "ticks": {
                    "type": "integer",
                    "description": "移动时长, 80≈4秒",
                    "default": 80
                }
            }
        })
    }
    fn execute(&self, _args: Value) -> anyhow::Result<ToolResult> {
        unreachable!("move_forward executed by Agent")
    }
}

// ── Look (观察) ──

pub struct LookTool;

impl GameTool for LookTool {
    fn name(&self) -> &str { "look" }
    fn description(&self) -> &str {
        "转动视角观察四周。dx>0右转, dy>0下看。"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "dx": { "type": "integer", "description": "水平转动量, 300≈90度" },
                "dy": { "type": "integer", "description": "垂直转动量" }
            },
            "required": ["dx", "dy"]
        })
    }
    fn execute(&self, _args: Value) -> anyhow::Result<ToolResult> {
        unreachable!("look executed by Agent")
    }
}

// ── 工具工厂 ──

/// 创建 Minecraft 默认工具集 (pi 风格: for tool in enabled { match name { ... } })
pub fn create_mc_tools() -> Vec<Box<dyn GameTool>> {
    vec![
        Box::new(PerceiveTool),
        Box::new(AimAndMineTool),
        Box::new(MoveForwardTool),
        Box::new(LookTool),
    ]
}
