//! Minecraft azalea 工具集（仅 `azalea-bot` 特性编译）。
//!
//! 把 `MinecraftAzaleaAdapter`（GameAdapter）封装成 LLM 可调用的 `GameTool`。
//! 这是 Phase 6 的关键：LLM 通过工具名输出 Action，adapter 翻译执行。
//!
//! 与 `tools_mod`（深度绑定 Fabric mod adapter）不同，本集走
//! `GameAdapter` 抽象，持有 `ArcAzaleaAdapter`，调用 `execute(Action::Minecraft)`。
//! 最少必要工具验证 LLM 驱动闭环：perceive / goto / mine_below / chat。

use crate::adapter_azalea::ArcAzaleaAdapter;
use craft_agent::core::tool::{GameTool, ToolEffects, ToolResult};
use craft_agent::core::types::{Action, MinecraftAction};
use serde_json::Value;
use std::sync::Arc;

/// 工具上下文：持有共享的 azalea adapter。
pub struct AzaleaToolCtx {
    pub adapter: ArcAzaleaAdapter,
}

impl AzaleaToolCtx {
    pub fn new(adapter: ArcAzaleaAdapter) -> Self {
        Self { adapter }
    }
}

/// 感知：读取结构化世界状态（坐标/背包/附近玩家），无需 VLM。
pub struct PerceiveTool {
    ctx: Arc<AzaleaToolCtx>,
}
impl PerceiveTool {
    pub fn new(ctx: Arc<AzaleaToolCtx>) -> Self {
        Self { ctx }
    }
}
impl GameTool for PerceiveTool {
    fn name(&self) -> &str {
        "perceive"
    }
    fn description(&self) -> &str {
        "读取当前世界结构化状态：玩家坐标、背包前5格、附近玩家数。返回文本描述，供决策使用。无参数。"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({})
    }
    fn effects(&self) -> ToolEffects {
        ToolEffects::read()
    }
    fn execute(
        &self,
        _call_id: &str,
        _args: Value,
        _on_update: Option<craft_agent::core::tool::ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        let st = self.ctx.adapter.perceive_shared()?;
        Ok(ToolResult {
            message: format!("{}", st.self_hint),
            is_error: false,
            images: vec![],
        })
    }
}

/// 走到世界坐标 (x, y, z)。
pub struct GotoTool {
    ctx: Arc<AzaleaToolCtx>,
}
impl GotoTool {
    pub fn new(ctx: Arc<AzaleaToolCtx>) -> Self {
        Self { ctx }
    }
}
impl GameTool for GotoTool {
    fn name(&self) -> &str {
        "goto"
    }
    fn description(&self) -> &str {
        "走到世界坐标 (x, y, z)。bot 使用内置 A* pathfinder 自动导航。参数：x,y,z 整数。"
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
        let x = args.get("x").and_then(|v| v.as_i64()).ok_or_else(|| anyhow::anyhow!("缺少 x"))? as i32;
        let y = args.get("y").and_then(|v| v.as_i64()).ok_or_else(|| anyhow::anyhow!("缺少 y"))? as i32;
        let z = args.get("z").and_then(|v| v.as_i64()).ok_or_else(|| anyhow::anyhow!("缺少 z"))? as i32;
        let r = self
            .ctx
            .adapter
            .execute_shared(Action::Minecraft(MinecraftAction::Goto { x, y, z }))?;
        Ok(ToolResult {
            message: r.detail,
            is_error: !r.ok,
            images: vec![],
        })
    }
}

/// 挖掉 bot 脚下方块（向下挖矿井）。
pub struct MineBelowTool {
    ctx: Arc<AzaleaToolCtx>,
}
impl MineBelowTool {
    pub fn new(ctx: Arc<AzaleaToolCtx>) -> Self {
        Self { ctx }
    }
}
impl GameTool for MineBelowTool {
    fn name(&self) -> &str {
        "mine_below"
    }
    fn description(&self) -> &str {
        "挖掉 bot 脚下的方块（向下挖矿井）。无参数。bot 自动挖掘并可能拾取掉落物。"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({})
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
            .execute_shared(Action::Minecraft(MinecraftAction::MineBelow))?;
        Ok(ToolResult {
            message: r.detail,
            is_error: !r.ok,
            images: vec![],
        })
    }
}

/// 发送聊天消息（也用作 LLM 指令回显 / 与玩家沟通）。
pub struct ChatTool {
    ctx: Arc<AzaleaToolCtx>,
}
impl ChatTool {
    pub fn new(ctx: Arc<AzaleaToolCtx>) -> Self {
        Self { ctx }
    }
}
impl GameTool for ChatTool {
    fn name(&self) -> &str {
        "chat"
    }
    fn description(&self) -> &str {
        "发送聊天消息到游戏。参数 content 为消息文本。"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "content": { "type": "string", "description": "聊天内容" }
            },
            "required": ["content"]
        })
    }
    fn execute(
        &self,
        _call_id: &str,
        args: Value,
        _on_update: Option<craft_agent::core::tool::ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        let content = args
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("缺少 content"))?
            .to_string();
        let r = self
            .ctx
            .adapter
            .execute_shared(Action::Minecraft(MinecraftAction::Chat { content }))?;
        Ok(ToolResult {
            message: r.detail,
            is_error: !r.ok,
            images: vec![],
        })
    }
}

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
        let x = args.get("x").and_then(|v| v.as_i64()).ok_or_else(|| anyhow::anyhow!("缺少 x"))? as i32;
        let y = args.get("y").and_then(|v| v.as_i64()).ok_or_else(|| anyhow::anyhow!("缺少 y"))? as i32;
        let z = args.get("z").and_then(|v| v.as_i64()).ok_or_else(|| anyhow::anyhow!("缺少 z"))? as i32;
        let r = self
            .ctx
            .adapter
            .execute_shared(Action::Minecraft(MinecraftAction::MineBlock { x, y, z }))?;
        Ok(ToolResult {
            message: r.detail,
            is_error: !r.ok,
            images: vec![],
        })
    }
}

/// 对着世界坐标 (x, y, z) 的方块交互（放置/右键）。
pub struct InteractBlockTool {
    ctx: Arc<AzaleaToolCtx>,
}
impl InteractBlockTool {
    pub fn new(ctx: Arc<AzaleaToolCtx>) -> Self {
        Self { ctx }
    }
}
impl GameTool for InteractBlockTool {
    fn name(&self) -> &str {
        "interact_block"
    }
    fn description(&self) -> &str {
        "对着指定世界坐标 (x,y,z) 的方块交互（放置方块/右键激活）。参数：x,y,z 整数。"
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
        let x = args.get("x").and_then(|v| v.as_i64()).ok_or_else(|| anyhow::anyhow!("缺少 x"))? as i32;
        let y = args.get("y").and_then(|v| v.as_i64()).ok_or_else(|| anyhow::anyhow!("缺少 y"))? as i32;
        let z = args.get("z").and_then(|v| v.as_i64()).ok_or_else(|| anyhow::anyhow!("缺少 z"))? as i32;
        let r = self
            .ctx
            .adapter
            .execute_shared(Action::Minecraft(MinecraftAction::InteractBlock { x, y, z }))?;
        Ok(ToolResult {
            message: r.detail,
            is_error: !r.ok,
            images: vec![],
        })
    }
}

/// 攻击最近的生物（自卫/狩猎）。target 预留为实体种类关键词（当前实现攻击最近非玩家实体）。
pub struct AttackTool {
    ctx: Arc<AzaleaToolCtx>,
}
impl AttackTool {
    pub fn new(ctx: Arc<AzaleaToolCtx>) -> Self {
        Self { ctx }
    }
}
impl GameTool for AttackTool {
    fn name(&self) -> &str {
        "attack"
    }
    fn description(&self) -> &str {
        "攻击最近的生物（自卫/狩猎）。无参数（当前总是攻击最近的「非玩家」实体）。bot 会持续攻击直到目标消失。"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "target": { "type": "string", "description": "预留：实体种类关键词（如 zombie）。当前忽略，总是攻击最近非玩家实体" }
            }
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
        let target = args
            .get("target")
            .and_then(|v| v.as_str())
            .unwrap_or("nearest")
            .to_string();
        let r = self
            .ctx
            .adapter
            .execute_shared(Action::Minecraft(MinecraftAction::Attack { target }))?;
        Ok(ToolResult {
            message: r.detail,
            is_error: !r.ok,
            images: vec![],
        })
    }
}

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
        let count = args
            .get("count")
            .and_then(|v| v.as_u64())
            .unwrap_or(1) as u32;
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
        "用已打开的 3×3 工作台合成物品（如 furnace / chest）。\n\
         item 为目标物品 id（如 \"furnace\"），count 为期望数量（默认 1）。\n\
         需先右键打开工作台，再调用本工具。"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "item": { "type": "string", "description": "目标物品 id，如 furnace / chest" },
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
        let count = args
            .get("count")
            .and_then(|v| v.as_u64())
            .unwrap_or(1) as u32;
        let r = self.ctx.adapter.execute_shared(Action::Minecraft(
            MinecraftAction::Craft3x3 { item, count },
        ))?;
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
        "在已打开的熔炉/高炉/烟熏炉中熔炼物品（如 iron_ingot / glass / charcoal）。\n\
         output 为产物 id（如 \"iron_ingot\"），fuel 为燃料 id（默认 \"coal\"，也可 \"charcoal\"/\"oak_log\"），\n\
         count 为期望数量（默认 1）。需先右键打开熔炉，且背包有输入物与燃料。"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "output": { "type": "string", "description": "产物 id，如 iron_ingot / glass / charcoal" },
                "fuel": { "type": "string", "description": "燃料 id（默认 coal）" },
                "count": { "type": "integer", "description": "熔炼数量（默认 1）" }
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
        let count = args
            .get("count")
            .and_then(|v| v.as_u64())
            .unwrap_or(1) as u32;
        let r = self.ctx.adapter.execute_shared(Action::Minecraft(
            MinecraftAction::Smelt {
                output,
                fuel,
                count,
            },
        ))?;
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
        let count = args
            .get("count")
            .and_then(|v| v.as_u64())
            .unwrap_or(1) as u32;
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

/// 放置方块（把手持物品放到坐标旁）。
pub struct PlaceTool {
    ctx: Arc<AzaleaToolCtx>,
}
impl PlaceTool {
    pub fn new(ctx: Arc<AzaleaToolCtx>) -> Self {
        Self { ctx }
    }
}
impl GameTool for PlaceTool {
    fn name(&self) -> &str {
        "place"
    }
    fn description(&self) -> &str {
        "把手持物品 item 放置到世界坐标 (x,y,z) 旁（右键放置）。\n\
         需背包持有该物品；常用于放置工作台/熔炉以便后续 craft_3x3 / smelt。\n\
         item 为目标物品 id（如 \"crafting_table\"），坐标用整数。"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "item": { "type": "string", "description": "物品 id，如 crafting_table / furnace" },
                "x": { "type": "integer" },
                "y": { "type": "integer" },
                "z": { "type": "integer" }
            },
            "required": ["item", "x", "y", "z"]
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
        let x = args.get("x").and_then(|v| v.as_i64()).ok_or_else(|| anyhow::anyhow!("缺少 x"))? as i32;
        let y = args.get("y").and_then(|v| v.as_i64()).ok_or_else(|| anyhow::anyhow!("缺少 y"))? as i32;
        let z = args.get("z").and_then(|v| v.as_i64()).ok_or_else(|| anyhow::anyhow!("缺少 z"))? as i32;
        let r = self.ctx.adapter.execute_shared(Action::Minecraft(
            MinecraftAction::Place { item, x, y, z },
        ))?;
        Ok(ToolResult {
            message: r.detail,
            is_error: !r.ok,
            images: vec![],
        })
    }
}

/// 打开容器（工作台/熔炉/箱子）。
pub struct OpenContainerTool {
    ctx: Arc<AzaleaToolCtx>,
}
impl OpenContainerTool {
    pub fn new(ctx: Arc<AzaleaToolCtx>) -> Self {
        Self { ctx }
    }
}
impl GameTool for OpenContainerTool {
    fn name(&self) -> &str {
        "open"
    }
    fn description(&self) -> &str {
        "打开世界坐标 (x,y,z) 处的容器（工作台/熔炉/箱子等）。\n\
         打开后配合 craft_3x3 / smelt 使用。坐标用整数。"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "x": { "type": "integer" },
                "y": { "type": "integer" },
                "z": { "type": "integer" }
            },
            "required": ["x", "y", "z"]
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
        let x = args.get("x").and_then(|v| v.as_i64()).ok_or_else(|| anyhow::anyhow!("缺少 x"))? as i32;
        let y = args.get("y").and_then(|v| v.as_i64()).ok_or_else(|| anyhow::anyhow!("缺少 y"))? as i32;
        let z = args.get("z").and_then(|v| v.as_i64()).ok_or_else(|| anyhow::anyhow!("缺少 z"))? as i32;
        let r = self.ctx.adapter.execute_shared(Action::Minecraft(
            MinecraftAction::OpenContainer { x, y, z },
        ))?;
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
        let count = args
            .get("count")
            .and_then(|v| v.as_u64())
            .unwrap_or(1) as u32;
        let r = self.ctx.adapter.execute_shared(Action::Minecraft(
            MinecraftAction::AutoCraft { item, count },
        ))?;
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
        let level = args
            .get("level")
            .and_then(|v| v.as_u64())
            .unwrap_or(1) as u32;
        let r = self.ctx.adapter.execute_shared(Action::Minecraft(
            MinecraftAction::Enchant { item, level },
        ))?;
        Ok(ToolResult {
            message: r.detail,
            is_error: !r.ok,
            images: vec![],
        })
    }
}

/// 创建 azalea 工具集并注册到 `ToolRegistry`。
pub fn create_mc_azalea_tools(adapter: ArcAzaleaAdapter) -> Vec<Box<dyn GameTool>> {
    let ctx = Arc::new(AzaleaToolCtx::new(adapter));
    vec![
        Box::new(PerceiveTool::new(ctx.clone())),
        Box::new(GotoTool::new(ctx.clone())),
        Box::new(MineBelowTool::new(ctx.clone())),
        Box::new(MineTool::new(ctx.clone())),
        Box::new(InteractBlockTool::new(ctx.clone())),
        Box::new(AttackTool::new(ctx.clone())),
        Box::new(CraftTool::new(ctx.clone())),
        Box::new(Craft3x3Tool::new(ctx.clone())),
        Box::new(SmeltTool::new(ctx.clone())),
        Box::new(GatherTool::new(ctx.clone())),
        Box::new(PlaceTool::new(ctx.clone())),
        Box::new(OpenContainerTool::new(ctx.clone())),
        Box::new(AutoCraftTool::new(ctx.clone())),
        Box::new(EnchantTool::new(ctx.clone())),
        Box::new(ChatTool::new(ctx.clone())),
    ]
}
