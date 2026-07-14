//! Minecraft 工具 — LLM 完全控制版
//!
//! 设计: LLM 决定一切, 没有中间处理层。
//! - perceive(prompt): LLM 自定义 VLM 提示词, 返回 VLM 原文
//! - press(keys, ticks): LLM 控制任意按键
//! - look(dx, dy): 视角旋转
//! - mine(ticks): 原地挖矿

use craft_agent::core::tool::{GameTool, ToolEffects, ToolResult};
use serde_json::Value;

pub struct PerceiveTool;
impl GameTool for PerceiveTool {
    fn name(&self) -> &str { "perceive" }
    fn description(&self) -> &str {
        "拍照并用自定义prompt让VLM描述场景。prompt应问清楚: 周围有什么? 树在哪? 石头在哪?"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "prompt": {
                    "type": "string",
                    "description": "给VLM的提示词, 用英文问清楚, 如: 'What blocks are near the crosshair? List trees, stones, animals, water.'"
                }
            }
        })
    }
    fn effects(&self) -> ToolEffects { ToolEffects::read_only() }
    fn execute(&self, _args: Value) -> anyhow::Result<ToolResult> {
        unreachable!("perceive executed by Agent")
    }
}

pub struct PressTool;
impl GameTool for PressTool {
    fn name(&self) -> &str { "press" }
    fn description(&self) -> &str {
        "按下按键。可用: w(前),a(左),s(后),d(右),space(跳),shift(潜行),e(背包),1~9(快捷栏)"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "keys": {
                    "type": "string",
                    "description": "按键字母, 如 'w', 'space', 'a+d'(组合)"
                },
                "ticks": {
                    "type": "integer",
                    "description": "持续时间, 40≈2秒, 80≈4秒",
                    "default": 40
                }
            },
            "required": ["keys"]
        })
    }
    fn execute(&self, _args: Value) -> anyhow::Result<ToolResult> {
        unreachable!("press executed by Agent")
    }
}

pub struct LookTool;
impl GameTool for LookTool {
    fn name(&self) -> &str { "look" }
    fn description(&self) -> &str {
        "转动视角。dx>0右转, dx<0左转。dy>0低头, dy<0抬头。约300≈90度。"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "dx": { "type": "integer", "description": "水平转动" },
                "dy": { "type": "integer", "description": "垂直转动" }
            },
            "required": ["dx", "dy"]
        })
    }
    fn execute(&self, _args: Value) -> anyhow::Result<ToolResult> {
        unreachable!("look executed by Agent")
    }
}

pub struct MineTool;
impl GameTool for MineTool {
    fn name(&self) -> &str { "mine" }
    fn description(&self) -> &str {
        "按住左键挖掘准星对准的方块。perceive确认目标对准后再用。"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "ticks": {
                    "type": "integer",
                    "description": "挖掘时长, 60≈3秒, 120≈6秒(矿石)",
                    "default": 60
                }
            }
        })
    }
    fn effects(&self) -> ToolEffects { ToolEffects::destructive() }
    fn execute(&self, _args: Value) -> anyhow::Result<ToolResult> {
        unreachable!("mine executed by Agent")
    }
}

pub fn create_mc_tools() -> Vec<Box<dyn GameTool>> {
    vec![
        Box::new(PerceiveTool),
        Box::new(PressTool),
        Box::new(LookTool),
        Box::new(MineTool),
    ]
}
