//! 挖掘工具：mine / make_obsidian（P3.2 按域拆分，tools_azalea 域模块）。
use super::*;

/// 挖掉世界坐标 (x, y, z) 处的方块（指定坐标挖掘）。
pub struct MineTool {
    ctx: Arc<AzaleaToolCtx>,
}
impl MineTool {
    pub fn new(ctx: Arc<AzaleaToolCtx>) -> Self {
        Self { ctx }
    }
}
impl GameTool for MineTool {
    fn name(&self) -> &str {
        "mine"
    }
    fn is_slow(&self) -> bool {
        true
    }
    fn description(&self) -> &str {
        "挖掉指定世界坐标 (x,y,z) 的方块。bot 会对该方块发起挖掘（需在其可达范围内）。参数：x,y,z 整数。"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "x": { "type": "integer", "description": "目标 X 坐标" },
                "y": { "type": "integer", "description": "目标 Y 坐标" },
                "z": { "type": "integer", "description": "目标 Z 坐标" }
            },
            "required": ["x", "y", "z"]
        })
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
        let r = self
            .ctx
            .adapter
            .execute_shared(Action::Minecraft(MinecraftAction::MineBlock { x, y, z }))?;
        // 行动回写：仅当挖掘成功时才从世界记忆移除。
        // P5 修复：原代码无条件 forget_pos，导致挖掘失败时记忆也被清空——
        // LLM 下一轮 perceive 时虽然会重新扫到该方块，但短期内会丢失「这是已知资源点」的标签。
        if r.ok {
            self.ctx.memory.forget_pos(MemoryPos::new(x, y, z));
        }
        Ok(ToolResult {
            message: r.detail,
            is_error: !r.ok,
            images: vec![],
        })
    }
}

/// 自动造黑曜石（P67）：bot 需手持 water_bucket，且附近（半径12）有岩浆源。
/// 工具自动在岩浆旁放水→生成黑曜石→用钻石镐挖下，重复 count 次。用于下界传送门框架。
pub struct MakeObsidianTool {
    ctx: Arc<AzaleaToolCtx>,
}
impl MakeObsidianTool {
    pub fn new(ctx: Arc<AzaleaToolCtx>) -> Self {
        Self { ctx }
    }
}
impl GameTool for MakeObsidianTool {
    fn name(&self) -> &str {
        "make_obsidian"
    }
    fn is_slow(&self) -> bool {
        true
    }
    fn description(&self) -> &str {
        "自动制造黑曜石（用于下界传送门框架）。\n\
         **前置条件**：bot 必须手持 water_bucket（先到水源 interact 装满水），且附近（半径12）有岩浆源。\n\
         - 工具会自动：在岩浆旁放一格水→生成黑曜石→用钻石镐挖下→重复 count 次。\n\
         - 若没水/没岩浆/没钻石镐会返回错误。\n\
         count 为期望黑曜石数量（默认 10，传送门框架需 10 块）。"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "count": { "type": "integer", "description": "黑曜石数量（默认 10）" }
            },
            "required": []
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
        let count = args.get("count").and_then(|v| v.as_u64()).unwrap_or(10) as u32;
        let r = self
            .ctx
            .adapter
            .execute_shared(Action::Minecraft(MinecraftAction::MakeObsidian { count }))?;
        Ok(ToolResult {
            message: r.detail,
            is_error: !r.ok,
            images: vec![],
        })
    }
}
