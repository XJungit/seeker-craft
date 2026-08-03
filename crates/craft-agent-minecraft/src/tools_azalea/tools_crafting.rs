//! 合成工具：craft / craft_3x3 / smelt / auto_craft / enchant（P3.2 按域拆分，tools_azalea 域模块）。
use super::*;

/// 合成物品（需要附近有工作台）。
pub struct CraftTool {
    ctx: Arc<AzaleaToolCtx>,
}
impl CraftTool {
    pub fn new(ctx: Arc<AzaleaToolCtx>) -> Self {
        Self { ctx }
    }
}
impl GameTool for CraftTool {
    fn name(&self) -> &str {
        "craft"
    }
    fn description(&self) -> &str {
        "用玩家自带 2×2 背包网格合成物品（无需工作台）。\n\
         item 为目标物品 id（可省略 minecraft: 前缀，如 \"oak_planks\" / \"stick\" / \"crafting_table\" / \"torch\" / \"chest\" / \"wooden_pickaxe\" / \"furnace\"，也支持任意原木如 \"spruce_log\" 合成对应木板）。\n\
         count 为期望数量（默认 1）。合成在后台异步执行，结果通过聊天事件回传。"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "item": { "type": "string", "description": "配方 id，如 minecraft:stick" },
                "count": { "type": "integer", "description": "合成数量（默认 1）" }
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
            .execute_shared(Action::Minecraft(MinecraftAction::Craft { item, count }))?;
        Ok(ToolResult {
            message: r.detail,
            is_error: !r.ok,
            images: vec![],
        })
    }
}

/// 3×3 工作台合成（需已打开工作台）。
pub struct Craft3x3Tool {
    ctx: Arc<AzaleaToolCtx>,
}
impl Craft3x3Tool {
    pub fn new(ctx: Arc<AzaleaToolCtx>) -> Self {
        Self { ctx }
    }
}
impl GameTool for Craft3x3Tool {
    fn name(&self) -> &str {
        "craft_3x3"
    }
    fn description(&self) -> &str {
        "用 3×3 工作台合成物品（如 furnace / chest / wooden_pickaxe / iron_ingot 工具等）。\n\
         **P1-4 自动放收桌**：bot 自动放置工作台→打开→合成→关闭，LLM 一次调用即可。\n\
         - 若附近已有工作台，传入 table_x/table_y/table_z 复用（避免重复放置）。\n\
         - 若不传 table 坐标，bot 在头顶放置一个新工作台并自动收桌。\n\
         - 若背包无 crafting_table，会返回错误，请先 craft(crafting_table) 造一个。\n\
         item 为目标物品 id（如 \"furnace\"/\"chest\"/\"wooden_pickaxe\"），count 为期望数量（默认 1）。"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "item": { "type": "string", "description": "目标物品 id，如 furnace / chest / wooden_pickaxe" },
                "count": { "type": "integer", "description": "合成数量（默认 1）" },
                "table_x": { "type": "integer", "description": "可选：现有工作台 X 坐标，复用而不重新放置" },
                "table_y": { "type": "integer", "description": "可选：现有工作台 Y 坐标" },
                "table_z": { "type": "integer", "description": "可选：现有工作台 Z 坐标" }
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
        // 三个 table 坐标同时提供才算指定桌位
        let table_pos = match (
            args.get("table_x").and_then(|v| v.as_i64()),
            args.get("table_y").and_then(|v| v.as_i64()),
            args.get("table_z").and_then(|v| v.as_i64()),
        ) {
            (Some(x), Some(y), Some(z)) => Some((x as i32, y as i32, z as i32)),
            _ => None,
        };
        let r = self
            .ctx
            .adapter
            .execute_shared(Action::Minecraft(MinecraftAction::Craft3x3 {
                item,
                count,
                table_pos,
            }))?;
        Ok(ToolResult {
            message: r.detail,
            is_error: !r.ok,
            images: vec![],
        })
    }
}

/// 熔炼（需已打开熔炉/高炉/烟熏炉）。
pub struct SmeltTool {
    ctx: Arc<AzaleaToolCtx>,
}
impl SmeltTool {
    pub fn new(ctx: Arc<AzaleaToolCtx>) -> Self {
        Self { ctx }
    }
}
impl GameTool for SmeltTool {
    fn name(&self) -> &str {
        "smelt"
    }
    fn description(&self) -> &str {
        "在熔炉中熔炼物品（如 iron_ingot / glass / charcoal / stone）。\n\
         **P1-4 自动放收炉**：bot 自动放置熔炉→打开→熔炼→关闭，LLM 一次调用即可。\n\
         - 若附近已有熔炉，传入 table_x/table_y/table_z 复用（避免重复放置）。\n\
         - 若不传 table 坐标，bot 在头顶放置一个新熔炉并自动收炉。\n\
         - 若背包无 furnace，会返回错误，请先 craft(furnace) 造一个。\n\
         output 为产物 id（如 \"iron_ingot\"），fuel 为燃料 id（默认 \"coal\"，也可 \"charcoal\"/\"oak_log\"），\n\
         count 为期望数量（默认 1）。"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "output": { "type": "string", "description": "产物 id，如 iron_ingot / glass / charcoal" },
                "fuel": { "type": "string", "description": "燃料 id（默认 coal）" },
                "count": { "type": "integer", "description": "熔炼数量（默认 1）" },
                "table_x": { "type": "integer", "description": "可选：现有熔炉 X 坐标，复用而不重新放置" },
                "table_y": { "type": "integer", "description": "可选：现有熔炉 Y 坐标" },
                "table_z": { "type": "integer", "description": "可选：现有熔炉 Z 坐标" }
            },
            "required": ["output"]
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
        let output = args
            .get("output")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("缺少 output"))?
            .to_string();
        let fuel = args
            .get("fuel")
            .and_then(|v| v.as_str())
            .unwrap_or("coal")
            .to_string();
        let count = args.get("count").and_then(|v| v.as_u64()).unwrap_or(1) as u32;
        let table_pos = match (
            args.get("table_x").and_then(|v| v.as_i64()),
            args.get("table_y").and_then(|v| v.as_i64()),
            args.get("table_z").and_then(|v| v.as_i64()),
        ) {
            (Some(x), Some(y), Some(z)) => Some((x as i32, y as i32, z as i32)),
            _ => None,
        };
        let r = self
            .ctx
            .adapter
            .execute_shared(Action::Minecraft(MinecraftAction::Smelt {
                output,
                fuel,
                count,
                table_pos,
            }))?;
        Ok(ToolResult {
            message: r.detail,
            is_error: !r.ok,
            images: vec![],
        })
    }
}

/// 高层自动合成（木链一键造木制品）。
pub struct AutoCraftTool {
    ctx: Arc<AzaleaToolCtx>,
}
impl AutoCraftTool {
    pub fn new(ctx: Arc<AzaleaToolCtx>) -> Self {
        Self { ctx }
    }
}
impl GameTool for AutoCraftTool {
    fn name(&self) -> &str {
        "auto_craft"
    }
    fn description(&self) -> &str {
        "高层自动合成：一句话造任意已登记物品，bot 沿配方图递归满足全部原料（采集/合成/熔炼），\n\
         自动造并打开放置的工作台/熔炉。支持目标含 oak_planks/stick/crafting_table/chest/furnace/\n\
         ladder/door/trapdoor/fence/iron_ingot/copper_ingot/gold_ingot/glass/stone/charcoal 等。\n\
         如 auto_craft(\"chest\",1) / auto_craft(\"iron_ingot\",3)。其他物品请用分步工具。"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "item": { "type": "string", "description": "木制品 id：oak_planks/stick/crafting_table/chest" },
                "count": { "type": "integer", "description": "数量（默认 1）" }
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
            .execute_shared(Action::Minecraft(MinecraftAction::AutoCraft {
                item,
                count,
            }))?;
        Ok(ToolResult {
            message: r.detail,
            is_error: !r.ok,
            images: vec![],
        })
    }
}

/// 附魔（需先打开附魔台）。
pub struct EnchantTool {
    ctx: Arc<AzaleaToolCtx>,
}
impl EnchantTool {
    pub fn new(ctx: Arc<AzaleaToolCtx>) -> Self {
        Self { ctx }
    }
}
impl GameTool for EnchantTool {
    fn name(&self) -> &str {
        "enchant"
    }
    fn description(&self) -> &str {
        "附魔：在已打开的附魔台中，给背包内的物品 item 附魔（需背包有 item 与青金石 lapis_lazuli）。\n\
         level 取 1/2/3，对应附魔台三个选项槽。使用前请先用 open 打开附魔台坐标，并确认背包有 item 与青金石。"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "item": { "type": "string", "description": "待附魔物品 id，如 iron_sword" },
                "level": { "type": "integer", "description": "附魔等级 1/2/3（默认 1）" }
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
        let level = args.get("level").and_then(|v| v.as_u64()).unwrap_or(1) as u32;
        let r = self
            .ctx
            .adapter
            .execute_shared(Action::Minecraft(MinecraftAction::Enchant { item, level }))?;
        Ok(ToolResult {
            message: r.detail,
            is_error: !r.ok,
            images: vec![],
        })
    }
}
