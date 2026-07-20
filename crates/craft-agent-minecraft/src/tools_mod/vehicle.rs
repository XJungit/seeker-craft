// ═══════════════════════════════════════════════════════════════
// 载具/补充工具（ride / fish / sleep / look_abs）— 对齐 Mineflayer
// 从 tools_mod.rs 拆分到本子模块（重构 ②）
// ═══════════════════════════════════════════════════════════════

use crate::tool_args::schema;
use crate::tools_mod::MinecraftModAdapter;
use crate::tools_mod::SafeLockAdapter;
use craft_agent::core::tool::{GameTool, ToolEffects, ToolResult, ToolUpdateFn};
use serde_json::Value;
use std::sync::Arc;
use std::sync::Mutex;

/// 骑乘控制（对齐 Mineflayer mount/dismount/moveVehicle）。
pub struct ModRideTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModRideTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModRideTool {
    fn name(&self) -> &str {
        "ride"
    }
    fn description(&self) -> &str {
        "Ride/mount a nearby rideable entity (horse/pig/boat/minecart) or dismount. action: 'mount' (nearest within radius), 'dismount', or 'steer' (drive with left/forward in -1..1). Mount first, then steer to move. Usage: ride(action=\"mount\") ride(action=\"steer\", forward=1.0) ride(action=\"dismount\")"
    }
    fn parameters(&self) -> Value {
        schema::object()
            .str_req("action", "mount | dismount | steer")
            .num_opt("radius", "Search radius for mount", 8.0)
            .num_opt("left", "Steering left (-1..1), only for steer", 0.0)
            .num_opt("forward", "Steering forward (-1..1), only for steer", 1.0)
            .finish()
    }
    fn effects(&self) -> ToolEffects {
        ToolEffects::write()
    }
    fn execute(
        &self,
        _id: &str,
        args: Value,
        _on_update: Option<ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        let action = args["action"].as_str().unwrap_or("mount");
        let radius = args["radius"].as_f64();
        let left = args["left"].as_f64();
        let forward = args["forward"].as_f64();
        let ack = self
            .adapter
            .lock_adapter()?
            .ride(action, radius, left, forward)?;
        let detail = ack.detail.clone();
        let mounted = ack.mounted.clone().unwrap_or_default();
        let msg = match action {
            "mount" => {
                if ack.status.as_str() == "fail" {
                    format!("ride mount FAILED: {}", detail)
                } else {
                    format!("ride mount {mounted} (nearest rideable)")
                }
            }
            "steer" => format!("ride steer {}", detail),
            "dismount" => "ride dismount".to_string(),
            _ => format!("ride unknown action '{action}'"),
        };
        Ok(ToolResult {
            message: msg,
            is_error: action == "mount" && ack.status.as_str() == "fail",
            images: vec![],
        })
    }
}

/// 钓鱼（对齐 Mineflayer bot.fish）。
pub struct ModFishTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModFishTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModFishTool {
    fn name(&self) -> &str {
        "fish"
    }
    fn description(&self) -> &str {
        "Cast and reel a fishing rod. Requires a fishing_rod in inventory. ticks: how long to hold the rod extended (longer = more chance a fish bites; reel happens automatically at end). Usage: fish(ticks=100)"
    }
    fn parameters(&self) -> Value {
        schema::object()
            .int_opt(
                "ticks",
                "Hold duration in ticks (20≈1s, 100≈5s)",
                100,
                20,
                600,
            )
            .finish()
    }
    fn effects(&self) -> ToolEffects {
        ToolEffects::write()
    }
    fn execute(
        &self,
        _id: &str,
        args: Value,
        _on_update: Option<ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        let ticks = args["ticks"].as_u64().unwrap_or(100) as u32;
        let ack = self.adapter.lock_adapter()?.fish(ticks)?;
        let msg = if ack.status.as_str() == "fail" {
            format!("fish FAILED: {}", ack.detail)
        } else {
            format!("fish {} ticks ({})", ticks, ack.detail)
        };
        Ok(ToolResult {
            message: msg,
            is_error: ack.status.as_str() == "fail",
            images: vec![],
        })
    }
}

/// 睡觉跳夜（对齐 Mineflayer bot.sleep）。
pub struct ModSleepTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModSleepTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModSleepTool {
    fn name(&self) -> &str {
        "sleep"
    }
    fn description(&self) -> &str {
        "Sleep in the nearest bed to skip the night (or thunderstorm). Requires a bed placed nearby. Auto-finds the bed foot within radius and sleeps. Usage: sleep(radius=8)"
    }
    fn parameters(&self) -> Value {
        schema::object()
            .num_opt("radius", "Search radius for a bed", 8.0)
            .finish()
    }
    fn effects(&self) -> ToolEffects {
        ToolEffects::write()
    }
    fn execute(
        &self,
        _id: &str,
        args: Value,
        _on_update: Option<ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        let radius = args["radius"].as_f64().unwrap_or(8.0);
        let ack = self.adapter.lock_adapter()?.sleep(radius)?;
        let msg = if ack.status.as_str() == "fail" {
            format!("sleep FAILED: {}", ack.detail)
        } else {
            format!("sleep: {}", ack.detail)
        };
        Ok(ToolResult {
            message: msg,
            is_error: ack.status.as_str() == "fail",
            images: vec![],
        })
    }
}

/// 精确朝向（对齐 Mineflayer bot.look(yaw,pitch)）。
pub struct ModLookAbsTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModLookAbsTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModLookAbsTool {
    fn name(&self) -> &str {
        "look_abs"
    }
    fn description(&self) -> &str {
        "Set absolute facing direction. yaw: 0=south, 90=west, 180=north, 270=east (degrees). pitch: -90=up, 0=horizontal, 90=down. Use for precise aiming without computing a target point. Usage: look_abs(yaw=90, pitch=-30)"
    }
    fn parameters(&self) -> Value {
        schema::object()
            .num_req("yaw", "Yaw in degrees (0=south,90=west,180=north,270=east)")
            .num_req("pitch", "Pitch in degrees (-90=up,0=horizontal,90=down)")
            .finish()
    }
    fn effects(&self) -> ToolEffects {
        ToolEffects::write()
    }
    fn execute(
        &self,
        _id: &str,
        args: Value,
        _on_update: Option<ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        let yaw = args["yaw"].as_f64().unwrap_or(0.0) as f32;
        let pitch = args["pitch"].as_f64().unwrap_or(0.0) as f32;
        self.adapter.lock_adapter()?.look_abs(yaw, pitch)?;
        Ok(ToolResult {
            message: format!("look_abs yaw={yaw} pitch={pitch}"),
            is_error: false,
            images: vec![],
        })
    }
}
