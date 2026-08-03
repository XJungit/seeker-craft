//! 背包工具：equip / discard / consume（P3.2 按域拆分，tools_azalea 域模块）。
use super::*;

/// 装备背包中的物品到指定槽位。学习自 Mindcraft equip。
/// slot="hand" 切换主手物品（武器/工具/方块），slot="helmet"/"chestplate"/"leggings"/"boots" 穿盔甲。
/// 解决：bot 挖矿时徒手（未装备镐）/战斗时拿错武器/有盔甲不穿的问题。
pub struct EquipTool {
    ctx: Arc<AzaleaToolCtx>,
}
impl EquipTool {
    pub fn new(ctx: Arc<AzaleaToolCtx>) -> Self {
        Self { ctx }
    }
}
impl GameTool for EquipTool {
    fn name(&self) -> &str {
        "equip"
    }
    fn description(&self) -> &str {
        "装备背包中的物品到指定槽位。\n\
         slot=\"hand\"：把物品移到主手（武器/工具/方块都走这条路径）。\n\
         slot=\"helmet\"/\"chestplate\"/\"leggings\"/\"boots\"：穿戴对应盔甲。\n\
         参数：item（物品 id 如 wooden_pickaxe/iron_sword/iron_helmet），slot（槽位名）。\n\
         场景：挖矿前装备镐、战斗前装备剑、有盔甲时穿上。"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "item": { "type": "string", "description": "物品 id，如 wooden_pickaxe / iron_sword / iron_helmet" },
                "slot": {
                    "type": "string",
                    "enum": ["hand", "helmet", "chestplate", "leggings", "boots"],
                    "description": "目标槽位"
                }
            },
            "required": ["item", "slot"]
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
        let slot = args
            .get("slot")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("缺少 slot"))?
            .to_string();
        let r = self
            .ctx
            .adapter
            .execute_shared(Action::Minecraft(MinecraftAction::Equip { item, slot }))?;
        Ok(ToolResult {
            message: r.detail,
            is_error: !r.ok,
            images: vec![],
        })
    }
}

/// 丢弃背包中的指定物品。学习自 Mindcraft discard。
/// count=0 丢全部，count>0 丢指定数量。解决：背包满时丢垃圾腾空间。
pub struct DiscardTool {
    ctx: Arc<AzaleaToolCtx>,
}
impl DiscardTool {
    pub fn new(ctx: Arc<AzaleaToolCtx>) -> Self {
        Self { ctx }
    }
}
impl GameTool for DiscardTool {
    fn name(&self) -> &str {
        "discard"
    }
    fn description(&self) -> &str {
        "丢弃背包中的指定物品以腾出空间。\n\
         count=0 丢弃全部，count>0 丢弃指定数量（按堆丢，最后不足一堆用单个丢）。\n\
         丢弃后物品以掉落物形式存在于 bot 脚边，可重新捡起。\n\
         场景：背包满（如挖了一堆泥土/沙砾）时丢弃无用物品。"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "item": { "type": "string", "description": "物品 id，如 dirt / gravel / cobblestone" },
                "count": { "type": "integer", "description": "丢弃数量（0=全部，默认 0）", "default": 0 }
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
        let r = self
            .ctx
            .adapter
            .execute_shared(Action::Minecraft(MinecraftAction::Discard { item, count }))?;
        Ok(ToolResult {
            message: r.detail,
            is_error: !r.ok,
            images: vec![],
        })
    }
}

/// 消耗（吃/喝）背包中的指定物品。学习自 Mindcraft consumeItem。
/// 把物品移到主手并按住右键使用。解决：血量低时不会吃食物回血。
pub struct ConsumeTool {
    ctx: Arc<AzaleaToolCtx>,
}
impl ConsumeTool {
    pub fn new(ctx: Arc<AzaleaToolCtx>) -> Self {
        Self { ctx }
    }
}
impl GameTool for ConsumeTool {
    fn name(&self) -> &str {
        "consume"
    }
    fn description(&self) -> &str {
        "消耗（吃/喝）背包中的指定物品。\n\
         把物品移到主手并按住右键使用。食物 1.6s 吃完一个，药水 1.6s 喝完。\n\
         参数：item（物品 id，如 cooked_beef / bread / apple / potion）。\n\
         场景：血量/饱食度低时吃食物回血。"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "item": { "type": "string", "description": "物品 id，如 cooked_beef / bread / apple / carrot / potion" }
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
        let r = self
            .ctx
            .adapter
            .execute_shared(Action::Minecraft(MinecraftAction::Consume { item }))?;
        Ok(ToolResult {
            message: r.detail,
            is_error: !r.ok,
            images: vec![],
        })
    }
}
