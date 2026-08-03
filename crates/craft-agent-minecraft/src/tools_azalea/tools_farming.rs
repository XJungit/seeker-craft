//! 采集工具：gather / till_and_sow / harvest（P3.2 按域拆分，tools_azalea 域模块）。
use super::*;

/// 犁地+播种（P84）：目标方块需为 dirt/grass_block/farmland，自动持锄头犁地、
/// 持种子播种并验证。一次调用完成，无需分步。
pub struct TillAndSowTool {
    ctx: Arc<AzaleaToolCtx>,
}
impl TillAndSowTool {
    pub fn new(ctx: Arc<AzaleaToolCtx>) -> Self {
        Self { ctx }
    }
}
impl GameTool for TillAndSowTool {
    fn name(&self) -> &str {
        "till_and_sow"
    }
    fn is_slow(&self) -> bool {
        true
    }
    fn description(&self) -> &str {
        "在指定世界坐标 (x,y,z) 犁地并播种。目标必须是草方块/泥土/已耕地；需背包有锄头和种子。种子支持 wheat_seeds/beetroot_seeds/carrot/potato/melon_seeds/pumpkin_seeds。若目标过远需先 goto 靠近。"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "x": { "type": "integer", "description": "目标 X 坐标" },
                "y": { "type": "integer", "description": "目标 Y 坐标" },
                "z": { "type": "integer", "description": "目标 Z 坐标" },
                "seed": { "type": "string", "description": "种子物品 id，如 wheat_seeds" }
            },
            "required": ["x", "y", "z", "seed"]
        })
    }
    fn effects(&self) -> ToolEffects {
        ToolEffects::write()
    }
    fn execute(
        &self,
        _call_id: &str,
        args: Value,
        _on_update: Option<craft_agent::core::tool::ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        let x = args
            .get("x")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| anyhow::anyhow!("缺少 x"))? as i32;
        let y = args
            .get("y")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| anyhow::anyhow!("缺少 y"))? as i32;
        let z = args
            .get("z")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| anyhow::anyhow!("缺少 z"))? as i32;
        let seed = args
            .get("seed")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("缺少 seed"))?
            .to_string();
        let r =
            self.ctx
                .adapter
                .execute_shared(Action::Minecraft(MinecraftAction::TillAndSow {
                    x,
                    y,
                    z,
                    seed,
                }))?;
        Ok(ToolResult {
            message: r.detail,
            is_error: !r.ok,
            images: vec![],
        })
    }
}

/// 收割成熟作物：扫描附近成熟小麦/胡萝卜/土豆/甜菜/下界疣并挖取。
pub struct HarvestTool {
    ctx: Arc<AzaleaToolCtx>,
}
impl HarvestTool {
    pub fn new(ctx: Arc<AzaleaToolCtx>) -> Self {
        Self { ctx }
    }
}
impl GameTool for HarvestTool {
    fn name(&self) -> &str {
        "harvest"
    }
    fn is_slow(&self) -> bool {
        true
    }
    fn description(&self) -> &str {
        "收割附近 32m 内所有成熟的农作物（小麦/胡萝卜/土豆/甜菜/下界疣），自动走到并徒手挖取，掉落物自动拾取。未成熟的作物不会掉落，需要等待。"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({ "type": "object", "properties": {}, "required": [] })
    }
    fn effects(&self) -> ToolEffects {
        ToolEffects::write()
    }
    fn execute(
        &self,
        _call_id: &str,
        _args: Value,
        _on_update: Option<craft_agent::core::tool::ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        let r = self
            .ctx
            .adapter
            .execute_shared(Action::Minecraft(MinecraftAction::Harvest))?;
        Ok(ToolResult {
            message: r.detail,
            is_error: !r.ok,
            images: vec![],
        })
    }
}

/// 采集最近方块（砍树/挖石/挖矿）。
pub struct GatherTool {
    ctx: Arc<AzaleaToolCtx>,
}
impl GatherTool {
    pub fn new(ctx: Arc<AzaleaToolCtx>) -> Self {
        Self { ctx }
    }
}
impl GameTool for GatherTool {
    fn name(&self) -> &str {
        "gather"
    }
    fn is_slow(&self) -> bool {
        true
    }
    fn description(&self) -> &str {
        "走到最近的指定方块并挖掘，直到背包有 count 个（早期游戏采集：砍树/挖石/挖矿）。\n\
         item 为方块物品 id（如 \"oak_log\" / \"stone\" / \"coal_ore\"），count 为期望数量（默认 1）。"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "item": { "type": "string", "description": "方块物品 id，如 oak_log / stone / coal_ore" },
                "count": { "type": "integer", "description": "采集数量（默认 1）" }
            },
            "required": ["item"]
        })
    }
    fn effects(&self) -> ToolEffects {
        ToolEffects::write()
    }
    fn execute(
        &self,
        _call_id: &str,
        args: Value,
        _on_update: Option<craft_agent::core::tool::ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        let item = args
            .get("item")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("缺少 item"))?
            .to_string();
        let count = args.get("count").and_then(|v| v.as_u64()).unwrap_or(1) as u32;
        let r = self
            .ctx
            .adapter
            .execute_shared(Action::Minecraft(MinecraftAction::Gather { item, count }))?;
        Ok(ToolResult {
            message: r.detail,
            is_error: !r.ok,
            images: vec![],
        })
    }
}
