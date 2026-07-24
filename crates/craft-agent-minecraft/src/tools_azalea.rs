//! Minecraft azalea 工具集（仅 `azalea-bot` 特性编译）。
//!
//! 把 `MinecraftAzaleaAdapter`（GameAdapter）封装成 LLM 可调用的 `GameTool`。
//! 这是 Phase 6 的关键：LLM 通过工具名输出 Action，adapter 翻译执行。
//!
//! 与 `tools_mod`（深度绑定 Fabric mod adapter）不同，本集走
//! `GameAdapter` 抽象，持有 `ArcAzaleaAdapter`，调用 `execute(Action::Minecraft)`。
//! 最少必要工具验证 LLM 驱动闭环：perceive / goto / mine_below / chat。

use crate::adapter_azalea::ArcAzaleaAdapter;
use craft_agent::core::memory::{MemoryKind, MemoryPos, WorldMemory};
use craft_agent::core::tool::{GameTool, ToolEffects, ToolResult};
use craft_agent::core::types::{Action, MinecraftAction};
use serde_json::Value;
use std::sync::Arc;

/// 工具上下文：持有共享的 azalea adapter 与世界记忆库。
pub struct AzaleaToolCtx {
    pub adapter: ArcAzaleaAdapter,
    pub memory: WorldMemory,
}

impl AzaleaToolCtx {
    pub fn new(adapter: ArcAzaleaAdapter, memory: WorldMemory) -> Self {
        Self { adapter, memory }
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
        // 行动回写：挖掉的脚下方块从世界记忆移除（用 __self__ 锚点推算脚下方块）
        if let Some(p) = self.ctx.memory.find_anchor("__self__").and_then(|a| a.pos) {
            self.ctx.memory.forget_pos(MemoryPos::new(p.x, p.y - 1, p.z));
        }
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
        // 行动回写：挖掉的方块从世界记忆中移除（避免被反复推荐为资源点）
        self.ctx.memory.forget_pos(MemoryPos::new(x, y, z));
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
            MinecraftAction::Place { item: item.clone(), x, y, z },
        ))?;
        // 行动回写：放置的方块计入世界记忆（便于后续直接 reuse，如工作台/熔炉/箱子）
        let pos = MemoryPos::new(x, y, z);
        let kind = match item.as_str() {
            "chest" | "barrel" | "shulker_box" => MemoryKind::Container,
            "lava" | "water" | "fire" => MemoryKind::Hazard,
            "nether_portal" | "end_portal" => MemoryKind::Portal,
            _ => MemoryKind::Structure,
        };
        self.ctx.memory.record(pos, kind, Some(&item), &item.clone(), None);
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
        // 行动回写：打开的容器计入世界记忆（箱子/熔炉/工作台等）
        self.ctx
            .memory
            .record_container(MemoryPos::new(x, y, z), "已打开的容器", "");
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
        let r = self.ctx.adapter.execute_shared(Action::Minecraft(
            MinecraftAction::Trade { offer },
        ))?;
        Ok(ToolResult {
            message: r.detail,
            is_error: !r.ok,
            images: vec![],
        })
    }
}

/// 实体右键交互（打开村民/动物/展示框等）。
pub struct InteractEntityTool {
    ctx: Arc<AzaleaToolCtx>,
}
impl InteractEntityTool {
    pub fn new(ctx: Arc<AzaleaToolCtx>) -> Self {
        Self { ctx }
    }
}
impl GameTool for InteractEntityTool {
    fn name(&self) -> &str {
        "interact_entity"
    }
    fn description(&self) -> &str {
        "实体右键交互：与最近的指定种类实体交互（打开村民界面/动物/物品展示框等）。\n\
         kind 为实体种类关键词，如 villager。需先走到该实体附近。"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "kind": { "type": "string", "description": "实体种类，如 villager" }
            },
            "required": ["kind"]
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
        let kind = args
            .get("kind")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("缺少 kind"))?
            .to_string();
        let r = self.ctx.adapter.execute_shared(Action::Minecraft(
            MinecraftAction::InteractEntity { kind },
        ))?;
        Ok(ToolResult {
            message: r.detail,
            is_error: !r.ok,
            images: vec![],
        })
    }
}

/// 世界记忆工具：让 LLM 显式记录/查询/遗忘空间记忆（资源点/结构/容器/锚点等）。
///
/// 动作：
/// - save: 在 (x,y,z) 记录一条记忆（kind 取 resource/structure/container/entity/hazard/portal/note；item 可选）
/// - anchor: 设置命名锚点（name, x, y, z, label）
/// - query: 查询（around 半径内邻近；或 by_item 按物品过滤；或 by_anchor 查锚点）
/// - forget: 按坐标遗忘；或 forget_anchor 按名称遗忘
pub struct MemoryTool {
    ctx: Arc<AzaleaToolCtx>,
}
impl MemoryTool {
    pub fn new(ctx: Arc<AzaleaToolCtx>) -> Self {
        Self { ctx }
    }
}
impl GameTool for MemoryTool {
    fn name(&self) -> &str {
        "memory"
    }
    fn description(&self) -> &str {
        "世界长期记忆：记录/查询/遗忘空间事实（资源点、结构、容器、村民、传送门、锚点）。\
         action=save 记录坐标记忆；action=anchor 设命名锚点；action=query 查询；action=forget 删除。"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["save", "anchor", "query", "forget"] },
                "x": { "type": "integer" },
                "y": { "type": "integer" },
                "z": { "type": "integer" },
                "kind": { "type": "string", "enum": ["resource","structure","container","entity","hazard","portal","note"] },
                "item": { "type": "string", "description": "方块/物品 id，如 oak_log" },
                "label": { "type": "string" },
                "name": { "type": "string", "description": "锚点名称" },
                "radius": { "type": "integer", "description": "query 邻近半径，默认 64" },
                "by_item": { "type": "string", "description": "query 按物品过滤" },
                "by_anchor": { "type": "string", "description": "query 按锚点名查询" }
            },
            "required": ["action"]
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
        let mem = &self.ctx.memory;
        let action = args["action"].as_str().unwrap_or("");
        let res = match action {
            "save" => {
                let pos = MemoryPos::new(
                    args["x"].as_i64().unwrap_or(0) as i32,
                    args["y"].as_i64().unwrap_or(0) as i32,
                    args["z"].as_i64().unwrap_or(0) as i32,
                );
                let kind = match args["kind"].as_str() {
                    Some("structure") => MemoryKind::Structure,
                    Some("container") => MemoryKind::Container,
                    Some("entity") => MemoryKind::Entity,
                    Some("hazard") => MemoryKind::Hazard,
                    Some("portal") => MemoryKind::Portal,
                    Some("note") => MemoryKind::Note,
                    _ => MemoryKind::Resource,
                };
                let label = args["label"].as_str().unwrap_or("记忆点");
                let item = args["item"].as_str();
                mem.record(pos, kind, item, label, None);
                format!("已记录记忆 @({},{},{}) kind={:?} label={}", pos.x, pos.y, pos.z, kind, label)
            }
            "anchor" => {
                let pos = MemoryPos::new(
                    args["x"].as_i64().unwrap_or(0) as i32,
                    args["y"].as_i64().unwrap_or(0) as i32,
                    args["z"].as_i64().unwrap_or(0) as i32,
                );
                let name = args["name"].as_str().unwrap_or("anchor");
                let label = args["label"].as_str().unwrap_or(name);
                mem.set_anchor(name, Some(pos), label);
                format!("已设锚点 {name} @({},{},{})", pos.x, pos.y, pos.z)
            }
            "query" => {
                if let Some(an) = args["by_anchor"].as_str() {
                    return Ok(ToolResult {
                        message: match mem.find_anchor(an) {
                            Some(a) => format!("锚点 {an}: {} {:?}", a.label, a.pos),
                            None => format!("未找到锚点 {an}"),
                        },
                        is_error: false,
                        images: vec![],
                    });
                }
                if let Some(item) = args["by_item"].as_str() {
                    let v = mem.query(None, Some(item));
                    let s = if v.is_empty() {
                        format!("无 {item} 相关记忆")
                    } else {
                        v.iter()
                            .map(|c| format!("{} @({},{},{})", c.label, c.pos.x, c.pos.y, c.pos.z))
                            .collect::<Vec<_>>()
                            .join("\n")
                    };
                    return Ok(ToolResult { message: s, is_error: false, images: vec![] });
                }
                let around = mem
                    .find_anchor("__self__")
                    .and_then(|a| a.pos)
                    .unwrap_or(MemoryPos::new(0, 64, 0));
                let r = args["radius"].as_i64().unwrap_or(64) as i32;
                mem.render_nearby(around, r)
            }
            "forget" => {
                if let Some(an) = args["name"].as_str() {
                    mem.forget_anchor(an);
                    format!("已遗忘锚点 {an}")
                } else {
                    let pos = MemoryPos::new(
                        args["x"].as_i64().unwrap_or(0) as i32,
                        args["y"].as_i64().unwrap_or(0) as i32,
                        args["z"].as_i64().unwrap_or(0) as i32,
                    );
                    mem.forget_pos(pos);
                    format!("已遗忘坐标 ({},{},{})", pos.x, pos.y, pos.z)
                }
            }
            other => format!("memory 未知 action: {other}"),
        };
        Ok(ToolResult {
            message: res,
            is_error: false,
            images: vec![],
        })
    }
}

/// 设置/更新当前目标（self-prompt）。bot 会持续朝此目标行动直到调用 set_goal("") 清空。
pub struct SetGoalTool {
    ctx: Arc<AzaleaToolCtx>,
}
impl SetGoalTool {
    pub fn new(ctx: Arc<AzaleaToolCtx>) -> Self {
        Self { ctx }
    }
}
impl GameTool for SetGoalTool {
    fn name(&self) -> &str {
        "set_goal"
    }
    fn description(&self) -> &str {
        "设置或更新当前目标。bot 会持续朝此目标行动直到调用 set_goal(goal=\"\") 清空。\
         goal 为英文目标描述，如 \"Get 3 iron ingots\" / \"Build a house\"。\
         调用后系统每轮自动注入此目标，bot 持续行动直到目标达成。"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "goal": { "type": "string", "description": "目标描述（英文）。传空字符串清空目标。" }
            },
            "required": ["goal"]
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
        let goal = args.get("goal").and_then(|v| v.as_str()).unwrap_or("");
        if goal.is_empty() {
            Ok(ToolResult {
                message: "目标已清空".to_string(),
                is_error: false,
                images: vec![],
            })
        } else {
            Ok(ToolResult {
                message: format!("目标已设置: {goal}"),
                is_error: false,
                images: vec![],
            })
        }
    }
}

/// 执行多步计划：按顺序执行一系列工具调用（支持 goto/mine/craft/gather/place 等）。
/// 每一步等待前一步完成再执行下一步，返回所有步骤的汇总结果。
/// 比 Mindcraft 的代码执行更安全——只使用已注册工具，不执行任意代码。
pub struct RunPlanTool {
    ctx: Arc<AzaleaToolCtx>,
}
impl RunPlanTool {
    pub fn new(ctx: Arc<AzaleaToolCtx>) -> Self {
        Self { ctx }
    }
}
impl GameTool for RunPlanTool {
    fn name(&self) -> &str {
        "run_plan"
    }
    fn description(&self) -> &str {
        "执行多步计划：按顺序执行一系列工具调用。steps 为 JSON 数组，每步格式为 {\"action\":\"工具名\", \"参数名\":值}。\
         支持动作: goto, mine, craft, gather, place, open, interact, attack, chat, mine_below。\
         例: [{\"action\":\"goto\",\"x\":10,\"y\":64,\"z\":8}, {\"action\":\"mine\",\"x\":10,\"y\":63,\"z\":8}]。\
         会等待每一步完成再执行下一步，返回所有步骤的汇总结果。"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "steps": {
                    "type": "array",
                    "description": "动作序列，每步 {\"action\":\"工具名\", 参数...}",
                    "items": {
                        "type": "object",
                        "properties": {
                            "action": { "type": "string", "description": "工具名: goto/mine/craft/gather/place/open/interact/attack/chat/mine_below" }
                        },
                        "required": ["action"]
                    }
                }
            },
            "required": ["steps"]
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
        let steps = args.get("steps").and_then(|v| v.as_array()).ok_or_else(|| anyhow::anyhow!("缺少 steps 数组"))?;
        let mut results: Vec<String> = Vec::new();
        for (i, step) in steps.iter().enumerate() {
            let action_name = step.get("action").and_then(|v| v.as_str()).unwrap_or("?");
            let mc = parse_step(action_name, step)?;
            match self.ctx.adapter.execute_shared(Action::Minecraft(mc)) {
                Ok(r) => {
                    results.push(format!("步骤{} ({}) 完成: {}", i + 1, action_name, r.detail));
                }
                Err(e) => {
                    results.push(format!("步骤{} ({}) 失败: {}", i + 1, action_name, e));
                    break;
                }
            }
        }
        Ok(ToolResult {
            message: results.join("\n"),
            is_error: false,
            images: vec![],
        })
    }
}

/// 将 plan 步骤中的 action 名和参数解析为 MinecraftAction。
fn parse_step(action: &str, step: &serde_json::Value) -> anyhow::Result<MinecraftAction> {
    let i64 = |key: &str| step.get(key).and_then(|v| v.as_i64()).map(|v| v as i32);
    let str = |key: &str| step.get(key).and_then(|v| v.as_str()).map(|s| s.to_string());
    let u32 = |key: &str| step.get(key).and_then(|v| v.as_u64()).map(|v| v as u32);
    match action {
        "goto" => Ok(MinecraftAction::Goto {
            x: i64("x").ok_or_else(|| anyhow::anyhow!("goto 缺少 x"))?,
            y: i64("y").ok_or_else(|| anyhow::anyhow!("goto 缺少 y"))?,
            z: i64("z").ok_or_else(|| anyhow::anyhow!("goto 缺少 z"))?,
        }),
        "mine" | "mine_block" => Ok(MinecraftAction::MineBlock {
            x: i64("x").ok_or_else(|| anyhow::anyhow!("mine 缺少 x"))?,
            y: i64("y").ok_or_else(|| anyhow::anyhow!("mine 缺少 y"))?,
            z: i64("z").ok_or_else(|| anyhow::anyhow!("mine 缺少 z"))?,
        }),
        "mine_below" => Ok(MinecraftAction::MineBelow),
        "interact" | "interact_block" => Ok(MinecraftAction::InteractBlock {
            x: i64("x").ok_or_else(|| anyhow::anyhow!("interact 缺少 x"))?,
            y: i64("y").ok_or_else(|| anyhow::anyhow!("interact 缺少 y"))?,
            z: i64("z").ok_or_else(|| anyhow::anyhow!("interact 缺少 z"))?,
        }),
        "chat" => Ok(MinecraftAction::Chat {
            content: str("content").ok_or_else(|| anyhow::anyhow!("chat 缺少 content"))?,
        }),
        "attack" => Ok(MinecraftAction::Attack {
            target: str("target").unwrap_or_else(|| "nearest".to_string()),
        }),
        "craft" | "craft_2x2" => Ok(MinecraftAction::Craft {
            item: str("item").ok_or_else(|| anyhow::anyhow!("craft 缺少 item"))?,
            count: u32("count").unwrap_or(1),
        }),
        "craft_3x3" => Ok(MinecraftAction::Craft3x3 {
            item: str("item").ok_or_else(|| anyhow::anyhow!("craft_3x3 缺少 item"))?,
            count: u32("count").unwrap_or(1),
        }),
        "smelt" => Ok(MinecraftAction::Smelt {
            output: str("output").ok_or_else(|| anyhow::anyhow!("smelt 缺少 output"))?,
            fuel: str("fuel").unwrap_or_else(|| "coal".to_string()),
            count: u32("count").unwrap_or(1),
        }),
        "gather" | "collect" => Ok(MinecraftAction::Gather {
            item: str("item").ok_or_else(|| anyhow::anyhow!("gather 缺少 item"))?,
            count: u32("count").unwrap_or(1),
        }),
        "place" => Ok(MinecraftAction::Place {
            item: str("item").ok_or_else(|| anyhow::anyhow!("place 缺少 item"))?,
            x: i64("x").ok_or_else(|| anyhow::anyhow!("place 缺少 x"))?,
            y: i64("y").ok_or_else(|| anyhow::anyhow!("place 缺少 y"))?,
            z: i64("z").ok_or_else(|| anyhow::anyhow!("place 缺少 z"))?,
        }),
        "open" | "open_container" => Ok(MinecraftAction::OpenContainer {
            x: i64("x").ok_or_else(|| anyhow::anyhow!("open 缺少 x"))?,
            y: i64("y").ok_or_else(|| anyhow::anyhow!("open 缺少 y"))?,
            z: i64("z").ok_or_else(|| anyhow::anyhow!("open 缺少 z"))?,
        }),
        "auto_craft" => Ok(MinecraftAction::AutoCraft {
            item: str("item").ok_or_else(|| anyhow::anyhow!("auto_craft 缺少 item"))?,
            count: u32("count").unwrap_or(1),
        }),
        "enchant" => Ok(MinecraftAction::Enchant {
            item: str("item").ok_or_else(|| anyhow::anyhow!("enchant 缺少 item"))?,
            level: u32("level").unwrap_or(1),
        }),
        other => Err(anyhow::anyhow!("不支持的 action: {other}（支持: goto/mine/craft/gather/place/open/interact/attack/chat/mine_below）")),
    }
}

/// 创建 azalea 工具集并注册到 `ToolRegistry`。
pub fn create_mc_azalea_tools(
    adapter: ArcAzaleaAdapter,
    memory: WorldMemory,
) -> Vec<Box<dyn GameTool>> {
    let ctx = Arc::new(AzaleaToolCtx::new(adapter, memory));
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
        Box::new(TradeTool::new(ctx.clone())),
        Box::new(InteractEntityTool::new(ctx.clone())),
        Box::new(ChatTool::new(ctx.clone())),
        Box::new(MemoryTool::new(ctx.clone())),
        Box::new(SetGoalTool::new(ctx.clone())),
        Box::new(RunPlanTool::new(ctx.clone())),
    ]
}
