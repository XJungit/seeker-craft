//! 移动 / 导航 / 视角工具。

use crate::tool_args::schema;
use crate::tools_mod::MinecraftModAdapter;
use crate::tools_mod::SafeLockAdapter;
use crate::tools_mod::find_nearest;
use crate::tools_mod::format_move_result;
use crate::tools_mod::midpoint_fractions;
use crate::tools_mod::needs_midpoint_retry;
use craft_agent::core::tool::{GameTool, ToolEffects, ToolResult, ToolUpdateFn};
use serde_json::Value;
use std::sync::Arc;
use std::sync::Mutex;

pub struct ModMoveToTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModMoveToTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModMoveToTool {
    fn name(&self) -> &str {
        "move_to"
    }
    fn description(&self) -> &str {
        "Navigate to world coordinates. Server-side setDeltaMovement + per-tick re-aim toward target, auto-jumps obstacles. Stops within 1.5m of target. Time: 2-10s depending on distance. Use coordinates from NEARBY BLOCKS section."
    }
    fn parameters(&self) -> Value {
        schema::object()
            .num_req("x", "Target X world coordinate")
            .num_req("y", "Target Y (use block.y + 0.5 for block center)")
            .num_req("z", "Target Z world coordinate")
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
        let x = args["x"].as_f64().unwrap_or(0.0);
        let y = args["y"].as_f64().unwrap_or(0.0);
        let z = args["z"].as_f64().unwrap_or(0.0);
        let adapter = self.adapter.lock_adapter()?;
        // 快速检查：已在目标位置则直接返回成功，省掉 A* 规划 + TCP 往返
        let cur = adapter.reload()?;
        let dx = cur.position[0] - x;
        let dy = cur.position[1] - y;
        let dz = cur.position[2] - z;
        if dx * dx + dy * dy + dz * dz < 0.25 {
            return Ok(ToolResult {
                message: format!("move_to ({:.1},{:.1},{:.1}) already at target", x, y, z),
                is_error: false,
                images: vec![],
            });
        }

        // 首次尝试直达目标
        let ack = adapter.move_to(x, y, z)?;
        let reached = ack.reached.unwrap_or(false);
        let dist = ack.final_dist.unwrap_or(0.0);
        let stuck = ack.stuck.unwrap_or(false);

        // 卡住/过远 → 自动用中间点重规划（Numen 风格 recover，无需 LLM 介入）。
        // 最多三级中间点：先试 1/2 中点，再试 1/4 与 3/4 递进。
        if needs_midpoint_retry(reached, stuck, dist) {
            let (cx, cy, cz) = (cur.position[0], cur.position[1], cur.position[2]);
            for step in midpoint_fractions() {
                let mx = cx + (x - cx) * step;
                let my = cy + (y - cy) * step;
                let mz = cz + (z - cz) * step;
                let mack = self.adapter.lock_adapter()?.move_to(mx, my, mz)?;
                if mack.reached.unwrap_or(false) {
                    // 中间点到达，再冲目标
                    let fack = self.adapter.lock_adapter()?.move_to(x, y, z)?;
                    return Ok(format_move_result(&fack, x, y, z, "via midpoint"));
                }
            }
            // 中间点也失败：返回原 stuck 信息（让 LLM 决定挖/垫脚）
            return Ok(format_move_result(&ack, x, y, z, ""));
        }

        Ok(format_move_result(&ack, x, y, z, ""))
    }
}

/// 原地垫方块升高（脱困用）。在脚下放方块 → 跳起 → 重复。
pub struct ModPillarUpTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModPillarUpTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModPillarUpTool {
    fn name(&self) -> &str {
        "pillar_up"
    }
    fn description(&self) -> &str {
        "Place a block beneath you and jump, repeating N times to pillar out of holes or escape one-deep pits. count: how many layers to pillar (default 3). item: block type to use (default 'dirt'). Uses mod bridge pillar_up command."
    }
    fn parameters(&self) -> Value {
        schema::object()
            .num_opt("count", "Number of pillar layers (default 3)", 3.0)
            .str_opt("item", "Block item to use (default dirt)", "dirt")
            .finish()
    }
    fn effects(&self) -> ToolEffects {
        ToolEffects::append()
    }
    fn execute(
        &self,
        _id: &str,
        args: Value,
        _on_update: Option<ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        let adapter = self.adapter.lock_adapter()?;
        let cnt = args.get("count").and_then(|v| v.as_u64()).map(|c| c as u32);
        let item = args.get("item").and_then(|v| v.as_str());
        let ack = adapter.pillar_up(cnt, item)?;
        let pcount = ack.pillar_count.unwrap_or(0);
        Ok(ToolResult {
            message: format!("{} (placed {})", ack.detail, pcount),
            is_error: ack.status != "ok",
            images: vec![],
        })
    }
}

pub struct ModLookAtTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModLookAtTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModLookAtTool {
    fn name(&self) -> &str {
        "look_at"
    }
    fn description(&self) -> &str {
        "Snap crosshair to a world coordinate. Integer coords auto-offset to block center (+0.5). Returns what block was actually hit (or 'nothing'). Force-refreshes raycast so targeted_block is accurate immediately."
    }
    fn parameters(&self) -> Value {
        schema::object()
            .num_req("x", "Target X coordinate")
            .num_req("y", "Target Y (auto-centers if integer)")
            .num_req("z", "Target Z coordinate")
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
        let x = args["x"].as_f64().unwrap_or(0.0);
        let y = args["y"].as_f64().unwrap_or(0.0);
        let z = args["z"].as_f64().unwrap_or(0.0);
        self.adapter.lock_adapter()?.look_at(x, y, z)?;
        Ok(ToolResult {
            message: format!("looking at ({:.1},{:.1},{:.1})", x, y, z),
            is_error: false,
            images: vec![],
        })
    }
}

// ═══════════════════════════════════════════════════════════════
// searchForBlock / moveAway / digDown — Mindcraft 对齐导航
// ═══════════════════════════════════════════════════════════════

pub struct ModSearchBlockTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModSearchBlockTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModSearchBlockTool {
    fn name(&self) -> &str {
        "searchForBlock"
    }
    fn description(&self) -> &str {
        "Find nearest block of type and walk to it (no mining). Uses move_to for navigation. Returns block type, coordinates, and distance walked. Use to position yourself before manual mining or building."
    }
    fn parameters(&self) -> Value {
        schema::object()
            .str_req(
                "type",
                "Block type: oak_log, stone, crafting_table, chest, etc.",
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
        let target = args["type"].as_str().unwrap_or("oak_log");
        let block = {
            let adapter = self.adapter.lock_adapter()?;
            let Some((block, _)) = find_nearest(&adapter, target) else {
                return Ok(ToolResult {
                    message: format!("searchForBlock: no {target} nearby"),
                    is_error: true,
                    images: vec![],
                });
            };
            block
        };
        let _ = self
            .adapter
            .lock_adapter()?
            .move_to(block.x, block.y + 0.5, block.z)?;
        Ok(ToolResult {
            message: format!(
                "walked to {} at ({:.0},{:.0},{:.0}) dist={:.1}m",
                target, block.x, block.y, block.z, block.dist
            ),
            is_error: false,
            images: vec![],
        })
    }
}

pub struct ModMoveAwayTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModMoveAwayTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModMoveAwayTool {
    fn name(&self) -> &str {
        "moveAway"
    }
    fn description(&self) -> &str {
        "Walk backwards N blocks (away from current facing direction). distance: approximate meters to retreat (max 20). Uses move_to with a computed backward target point."
    }
    fn parameters(&self) -> Value {
        schema::object()
            .int_opt("distance", "Blocks to retreat (1-20)", 3, 1, 20)
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
        let dist = args["distance"].as_u64().unwrap_or(3).min(20) as f64;
        let adapter = self.adapter.lock_adapter()?;
        let st = adapter.reload()?;
        // 根据当前朝向计算反向目标点（yaw: 0=南+z, 90=西-x, 180=北-z, 270=东+x）
        let yaw_rad = st.yaw.to_radians();
        // 前进方向: (-sin(yaw), cos(yaw))，后退方向取反
        let back_x = st.position[0] + yaw_rad.sin() * dist;
        let back_z = st.position[2] - yaw_rad.cos() * dist;
        let _ = adapter.move_to(back_x, st.position[1], back_z)?;
        Ok(ToolResult {
            message: format!(
                "moved back ~{dist:.0} blocks to ({:.1},{:.1},{:.1})",
                back_x, st.position[1], back_z
            ),
            is_error: false,
            images: vec![],
        })
    }
}

pub struct ModDigDownTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModDigDownTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModDigDownTool {
    fn name(&self) -> &str {
        "digDown"
    }
    fn description(&self) -> &str {
        "Dig straight down N blocks by destroying block under feet via dig_at (coordinate-based, no camera aiming). Player falls into hole after each block. Auto-stops if lava/water detected or fall would be ≥4 blocks. Verifies actual Y descent — reports true depth dug. distance: 1-10 blocks."
    }
    fn parameters(&self) -> Value {
        schema::object()
            .int_opt("distance", "Blocks to dig down (1-10)", 1, 1, 10)
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
        let dist = args["distance"].as_u64().unwrap_or(1).min(10) as u32;
        let a = self.adapter.lock_adapter()?;
        let st = a.reload()?;
        let start_y = st.position[1];
        let mut dug = 0u32;
        for _ in 0..dist {
            let current_st = a.reload()?;
            let current_y = current_st.position[1];
            if current_y < start_y - 4.0 {
                return Ok(ToolResult {
                    message: format!(
                        "dug down {dug} blocks (stopped: fall would exceed 4 blocks, now y={:.1})",
                        current_y
                    ),
                    is_error: false,
                    images: vec![],
                });
            }
            // 脚下方块坐标（玩家脚所在格的下方）。
            // 脚底 y_f 对应的脚下方块 = floor(y_f) - 1（方块 B 占 [B,B+1)，脚踩顶面 B+1）。
            let px = current_st.position[0].floor() as i32;
            let py = current_st.position[1].floor() as i32 - 1; // 脚下方块
            let pz = current_st.position[2].floor() as i32;

            // 先按坐标查询脚下方块（dig_at 会返回 block_id），安全检测 lava/water。
            let probe = a.dig_at(px, py, pz)?;
            let block_id = probe.block_id.clone().unwrap_or_default().to_lowercase();
            if block_id.contains("lava") {
                return Ok(ToolResult {
                    message: format!("dug down {dug} blocks (stopped: lava detected below)"),
                    is_error: true,
                    images: vec![],
                });
            }
            if block_id.contains("water") {
                return Ok(ToolResult {
                    message: format!("dug down {dug} blocks (stopped: water detected below)"),
                    is_error: true,
                    images: vec![],
                });
            }

            // dig_at 已执行破坏（destroyBlock）。用服务端真实 broken 结果判断，
            // 不再依赖 nearby_blocks 快照匹配（避免坐标错位导致漏判）。
            match probe.broken {
                Some(true) => {
                    dug += 1;
                }
                Some(false) => {
                    // 没破坏：air / 太远 / 不可破坏
                    return Ok(ToolResult {
                        message: format!(
                            "dug down {dug} blocks (stopped: block at y={py} not broken, block_id={}, player still at y={:.1})",
                            block_id, current_y
                        ),
                        is_error: dug == 0,
                        images: vec![],
                    });
                }
                None => {
                    return Ok(ToolResult {
                        message: format!(
                            "dug down {dug} blocks (stopped: no ack for block at y={py})"
                        ),
                        is_error: dug == 0,
                        images: vec![],
                    });
                }
            }

            // 等待玩家掉落（服务端 destroyBlock 瞬时完成，留余量确认落位）
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
        let final_st = a.reload()?;
        Ok(ToolResult {
            message: format!(
                "dug down {dug} blocks (y: {:.1}→{:.1})",
                start_y, final_st.position[1]
            ),
            is_error: dug == 0,
            images: vec![],
        })
    }
}
