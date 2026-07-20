// ═══════════════════════════════════════════════════════════════
// 生存杂项工具（goToBed / stay / goToSurface / setMode）
// 从 tools_mod.rs 拆分到本子模块（重构 ②）
// ═══════════════════════════════════════════════════════════════

use crate::tool_args::schema;
use crate::tools_mod::MinecraftModAdapter;
use crate::tools_mod::SafeLockAdapter;
use craft_agent::core::tool::{GameTool, ToolEffects, ToolResult, ToolUpdateFn};
use serde_json::Value;
use std::sync::Arc;
use std::sync::Mutex;

pub struct ModGoToBedTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModGoToBedTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModGoToBedTool {
    fn name(&self) -> &str {
        "goToBed"
    }
    fn description(&self) -> &str {
        "Find nearest bed block and sleep in it to skip night. Searches nearby blocks for any bed type (red_bed, blue_bed, etc). Walks to bed and right-clicks to sleep. Only works at night or during thunderstorms."
    }
    fn parameters(&self) -> Value {
        schema::no_args()
    }
    fn effects(&self) -> ToolEffects {
        ToolEffects::write()
    }
    fn execute(
        &self,
        _id: &str,
        _args: Value,
        _on_update: Option<ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        let adapter = self.adapter.lock_adapter()?;
        let st = adapter.reload()?;
        let t = st.time % 24000;
        let is_night = !(230..=13000).contains(&t);
        let is_thunder = st.thundering;
        if !is_night && !is_thunder {
            return Ok(ToolResult {
                message: "goToBed: not night or thundering, no need to sleep".into(),
                is_error: false,
                images: vec![],
            });
        }
        let ack = adapter.sleep(8.0)?;
        let msg = if ack.status.as_str() == "fail" {
            format!("goToBed FAILED: {}", ack.detail)
        } else {
            format!("goToBed: {}", ack.detail)
        };
        Ok(ToolResult {
            message: msg,
            is_error: ack.status.as_str() == "fail",
            images: vec![],
        })
    }
}

pub struct ModStayTool;
impl ModStayTool {
    pub fn new(_a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self
    }
}
impl GameTool for ModStayTool {
    fn name(&self) -> &str {
        "stay"
    }
    fn description(&self) -> &str {
        "Stay in current position for N seconds. Pauses all movement. type: seconds to wait (-1 = forever, but capped at 30 for safety). Use to wait for daytime, crop growth, or to avoid danger."
    }
    fn parameters(&self) -> Value {
        schema::object()
            .int_opt(
                "type",
                "Seconds to stay (1-30, -1=forever but capped at 30)",
                5,
                -1,
                30,
            )
            .finish()
    }
    fn effects(&self) -> ToolEffects {
        ToolEffects::read()
    }
    fn execute(
        &self,
        _id: &str,
        args: Value,
        _on_update: Option<ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        let secs = args["type"].as_i64().unwrap_or(5).clamp(-1, 30) as i32;
        let wait = if secs < 0 { 30 } else { secs } as u64;
        std::thread::sleep(std::time::Duration::from_secs(wait));
        Ok(ToolResult {
            message: format!("stayed for {wait} seconds"),
            is_error: false,
            images: vec![],
        })
    }
}

pub struct ModGoToSurfaceTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModGoToSurfaceTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModGoToSurfaceTool {
    fn name(&self) -> &str {
        "goToSurface"
    }
    fn description(&self) -> &str {
        "Move to the surface (highest non-air block above current position). Useful when underground or in a cave. Finds the highest solid block in nearby_blocks and walks to it."
    }
    fn parameters(&self) -> Value {
        schema::no_args()
    }
    fn effects(&self) -> ToolEffects {
        ToolEffects::write()
    }
    fn execute(
        &self,
        _id: &str,
        _args: Value,
        _on_update: Option<ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        let adapter = self.adapter.lock_adapter()?;
        let st = adapter.reload()?;
        let cur_x = st.position[0].round() as i32;
        let cur_z = st.position[2].round() as i32;
        let surface = st
            .nearby_blocks
            .iter()
            .filter(|b| {
                let bx = b.x.round() as i32;
                let bz = b.z.round() as i32;
                (bx - cur_x).abs() <= 2 && (bz - cur_z).abs() <= 2 && !b.id.contains("air")
            })
            .max_by(|a, b| a.y.partial_cmp(&b.y).unwrap_or(std::cmp::Ordering::Equal));
        match surface {
            Some(b) => {
                let (sx, sy, sz) = (b.x, b.y + 1.0, b.z);
                drop(adapter);
                let _ = self.adapter.lock_adapter()?.move_to(sx, sy, sz)?;
                Ok(ToolResult {
                    message: format!("went to surface at ({:.0},{:.0},{:.0})", sx, sy, sz),
                    is_error: false,
                    images: vec![],
                })
            }
            None => Ok(ToolResult {
                message: "goToSurface: already at surface or no surface block found nearby".into(),
                is_error: false,
                images: vec![],
            }),
        }
    }
}

pub struct ModSetModeTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModSetModeTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModSetModeTool {
    fn name(&self) -> &str {
        "setMode"
    }
    fn description(&self) -> &str {
        "Toggle a behavior mode on/off. Modes are automatic behaviors checked every turn: 'self_preservation' (auto-flee when health<6), 'self_defense' (auto-attack nearby hostiles), 'unstuck' (auto-recover when stuck), 'cowardice' (always flee from hostiles), 'hunting' (auto-hunt nearby animals for food), 'torch_placing' (auto-place torches when dark), 'idle_staring' (look at nearby entities when idle). Returns current mode states."
    }
    fn parameters(&self) -> Value {
        schema::object()
            .str_req("mode_name", "Mode name: self_preservation, self_defense, unstuck, cowardice, hunting, torch_placing, idle_staring")
            .bool_opt("on", "true=enable, false=disable", true)
            .finish()
    }
    fn effects(&self) -> ToolEffects {
        ToolEffects::read()
    }
    fn execute(
        &self,
        _id: &str,
        args: Value,
        _on_update: Option<ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        let mode = args["mode_name"].as_str().unwrap_or("self_defense");
        let on = args["on"].as_bool().unwrap_or(true);
        let adapter = self.adapter.lock_adapter()?;
        adapter.set_mode(mode, on);
        let modes_list = adapter.list_modes();
        Ok(ToolResult {
            message: format!("mode '{mode}' set to {on}\nCurrent modes:\n{modes_list}"),
            is_error: false,
            images: vec![],
        })
    }
}
