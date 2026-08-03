//! 社交工具：trade / give（P3.2 按域拆分，tools_azalea 域模块）。
use super::*;

/// 村民交易（与最近的村民交易）。
pub struct TradeTool {
    ctx: Arc<AzaleaToolCtx>,
}
impl TradeTool {
    pub fn new(ctx: Arc<AzaleaToolCtx>) -> Self {
        Self { ctx }
    }
}
impl GameTool for TradeTool {
    fn name(&self) -> &str {
        "trade"
    }
    fn description(&self) -> &str {
        "村民交易：与最近的村民交易，选第 offer 个报价（0 起，需用 perceive/interact 先靠近村民）。\n\
         如 trade(0) 买第 1 个报价，trade(1) 买第 2 个。bot 自动打开村民并执行交易。"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "offer": { "type": "integer", "description": "报价索引（从 0 开始）" }
            },
            "required": ["offer"]
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
        let offer = args
            .get("offer")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| anyhow::anyhow!("缺少 offer"))? as u32;
        let r = self
            .ctx
            .adapter
            .execute_shared(Action::Minecraft(MinecraftAction::Trade { offer }))?;
        Ok(ToolResult {
            message: r.detail,
            is_error: !r.ok,
            images: vec![],
        })
    }
}

/// P68：把物品丢在玩家脚边（给予）。基于 discard，但丢在玩家坐标。
/// 玩家可走过去拾取。target 为玩家名（可选，留空=最近玩家）。
pub struct GiveTool {
    ctx: Arc<AzaleaToolCtx>,
}
impl GiveTool {
    pub fn new(ctx: Arc<AzaleaToolCtx>) -> Self {
        Self { ctx }
    }
}
impl GameTool for GiveTool {
    fn name(&self) -> &str {
        "give"
    }
    fn description(&self) -> &str {
        "把物品丢在玩家脚边（给予玩家，让其拾取）。\n\
         item 为物品 id（如 cooked_beef / dirt / oak_log），count 为数量（0=全部）。\n\
         target 为玩家名（可选，留空=最近的玩家）。\n\
         也可以在游戏聊天框直接打 \"give <物品> [数量] [玩家名]\" 触发。\n\
         场景：玩家说\"给我点吃的\"→ 调用 give(cooked_beef, 3)。"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "item": { "type": "string", "description": "物品 id，如 cooked_beef / dirt / oak_log" },
                "count": { "type": "integer", "description": "数量（0=全部，默认 0）", "default": 0 },
                "target": { "type": "string", "description": "玩家名（可选，留空=最近的玩家）" }
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
        let count = args.get("count").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        let target = args
            .get("target")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        let r = self
            .ctx
            .adapter
            .execute_shared(Action::Minecraft(MinecraftAction::Give {
                item,
                count,
                target,
            }))?;
        Ok(ToolResult {
            message: r.detail,
            is_error: !r.ok,
            images: vec![],
        })
    }
}
