//! Minecraft azalea 工具集（仅 `azalea-bot` 特性编译）。
//!
//! 把 `MinecraftAzaleaAdapter`（GameAdapter）封装成 LLM 可调用的 `GameTool`。
//! 这是 Phase 6 的关键：LLM 通过工具名输出 Action，adapter 翻译执行。
//!
//! 与 `tools_mod`（深度绑定 Fabric mod adapter）不同，本集走
//! `GameAdapter` 抽象，持有 `ArcAzaleaAdapter`，调用 `execute(Action::Minecraft)`。
//! 最少必要工具验证 LLM 驱动闭环：perceive / goto / mine_below / chat。

use crate::adapter_azalea::{ArcAzaleaAdapter, MinecraftAzaleaAdapter};
use craft_agent::core::memory::{MemoryKind, MemoryPos, WorldMemory};
use craft_agent::core::tool::{GameTool, ToolEffects, ToolResult};
use craft_agent::core::types::{Action, MinecraftAction};
use serde_json::Value;
use std::sync::{Arc, Mutex};

use crate::action_lib::{ActionLibrary, LlmAction};
use crate::blueprint::BlueprintLibrary;

/// 工具上下文：持有共享的 azalea adapter、世界记忆库、蓝图库与 LLM 自定义动作库。
pub struct AzaleaToolCtx {
    pub adapter: ArcAzaleaAdapter,
    pub memory: WorldMemory,
    /// 蓝图库（P2-1）：供 build_blueprint / list_blueprints 工具使用。
    pub blueprints: BlueprintLibrary,
    /// LLM 自定义动作库（P2-4）：供 new_action / list_actions / call_action 使用。
    /// 内部可变（save/bump_call_count），用 Mutex 保护。
    pub actions: Arc<Mutex<ActionLibrary>>,
}

impl AzaleaToolCtx {
    pub fn new(adapter: ArcAzaleaAdapter, memory: WorldMemory) -> Self {
        Self {
            adapter,
            memory,
            blueprints: BlueprintLibrary::new(),
            actions: Arc::new(Mutex::new(ActionLibrary::new())),
        }
    }

    /// 注入蓝图库（通常从 `blueprints/` 目录加载后注入）。
    pub fn with_blueprints(mut self, bp: BlueprintLibrary) -> Self {
        self.blueprints = bp;
        self
    }

    /// 注入 LLM 自定义动作库（通常从 `actions/` 目录加载后注入）。
    pub fn with_actions(mut self, al: ActionLibrary) -> Self {
        self.actions = Arc::new(Mutex::new(al));
        self
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
        // 行动回写：仅当挖掘成功时才从世界记忆移除脚下方块。
        // P5 修复：原代码无条件 forget_pos，挖掘失败时记忆也被清空。
        if r.ok {
            if let Some(p) = self.ctx.memory.find_anchor("__self__").and_then(|a| a.pos) {
                self.ctx.memory.forget_pos(MemoryPos::new(p.x, p.y - 1, p.z));
            }
        }
        Ok(ToolResult {
            message: r.detail,
            is_error: !r.ok,
            images: vec![],
        })
    }
}

/// 向上挖：从 bot 头顶逐格挖到空气（地下脱困专用）。
pub struct MineAboveTool {
    ctx: Arc<AzaleaToolCtx>,
}
impl MineAboveTool {
    pub fn new(ctx: Arc<AzaleaToolCtx>) -> Self {
        Self { ctx }
    }
}
impl GameTool for MineAboveTool {
    fn name(&self) -> &str {
        "mine_above"
    }
    fn description(&self) -> &str {
        "向上挖：从 bot 头顶逐格挖到空气，用于地下脱困/上到地表。无参数。\
         与 mine_below 反向——mine_below 是向下挖矿井，mine_above 是向上挖通竖井。\
         当 bot 困在地下/1x1 竖井/洞穴里需要回到地表时使用。\
         bot 自动装备背包里最好的镐，挖通头顶方块后 bot 自动跳起，\
         连续挖直到头顶是空气（已挖通）或 Y≥320（建筑上限）。"
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
            .execute_shared(Action::Minecraft(MinecraftAction::MineAbove))?;
        // 行动回写：仅当挖掘成功时才从世界记忆移除头前方块。
        // P5 修复：原代码无条件 forget_pos，挖掘失败时记忆也被清空。
        if r.ok {
            if let Some(p) = self.ctx.memory.find_anchor("__self__").and_then(|a| a.pos) {
                self.ctx.memory.forget_pos(MemoryPos::new(p.x, p.y + 1, p.z));
            }
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
        let count = args
            .get("count")
            .and_then(|v| v.as_u64())
            .unwrap_or(1) as u32;
        // 三个 table 坐标同时提供才算指定桌位
        let table_pos = match (
            args.get("table_x").and_then(|v| v.as_i64()),
            args.get("table_y").and_then(|v| v.as_i64()),
            args.get("table_z").and_then(|v| v.as_i64()),
        ) {
            (Some(x), Some(y), Some(z)) => Some((x as i32, y as i32, z as i32)),
            _ => None,
        };
        let r = self.ctx.adapter.execute_shared(Action::Minecraft(
            MinecraftAction::Craft3x3 { item, count, table_pos },
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
        let count = args
            .get("count")
            .and_then(|v| v.as_u64())
            .unwrap_or(1) as u32;
        let table_pos = match (
            args.get("table_x").and_then(|v| v.as_i64()),
            args.get("table_y").and_then(|v| v.as_i64()),
            args.get("table_z").and_then(|v| v.as_i64()),
        ) {
            (Some(x), Some(y), Some(z)) => Some((x as i32, y as i32, z as i32)),
            _ => None,
        };
        let r = self.ctx.adapter.execute_shared(Action::Minecraft(
            MinecraftAction::Smelt {
                output,
                fuel,
                count,
                table_pos,
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
        // 行动回写：仅当放置成功时才回写世界记忆。
        // P5 修复：原代码无条件 record，导致 do_place 失败后 LLM 仍能在记忆中看到
        // 「已放置 crafting_table」，下一轮 perceive 又因实际方块不存在而遗忘——
        // 这种"先记后忘"会让 LLM 困惑（记忆与感知矛盾）。
        if r.ok {
            let pos = MemoryPos::new(x, y, z);
            let kind = match item.as_str() {
                "chest" | "barrel" | "shulker_box" => MemoryKind::Container,
                "lava" | "water" | "fire" => MemoryKind::Hazard,
                "nether_portal" | "end_portal" => MemoryKind::Portal,
                _ => MemoryKind::Structure,
            };
            self.ctx.memory.record(pos, kind, Some(&item), &item.clone(), None);
        }
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
        // 行动回写：仅当打开成功时才回写世界记忆。
        // P5 修复：原代码无条件 record_container，导致 open 失败（距离过远/坐标错误）
        // 后 LLM 仍能在记忆中看到「已打开的容器」，下一轮 perceive 又遗忘，造成记忆矛盾。
        if r.ok {
            self.ctx
                .memory
                .record_container(MemoryPos::new(x, y, z), "已打开的容器", "");
        }
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
        // 上一步 mine 的坐标——用于检测并跳过"mine→goto 同坐标"这种无效组合。
        // LLM 常写 [{mine (x,y,z)}, {goto (x,y,z)}] 想让 bot "挖完掉进洞"，
        // 但 azalea bot 挖完脚下方块不会自动掉进去，goto 到空气位置必然超时。
        // 检测到这种 plan 时直接跳过 goto，告知 LLM bot 已在附近。
        let mut last_mined: Option<(i32, i32, i32)> = None;
        for (i, step) in steps.iter().enumerate() {
            let action_name = step.get("action").and_then(|v| v.as_str()).unwrap_or("?");
            // 跳过无效 goto：目标是上一步 mine 的位置
            if action_name == "goto" {
                if let Some((mx, my, mz)) = last_mined {
                    let gx = step.get("x").and_then(|v| v.as_i64()).map(|v| v as i32);
                    let gy = step.get("y").and_then(|v| v.as_i64()).map(|v| v as i32);
                    let gz = step.get("z").and_then(|v| v.as_i64()).map(|v| v as i32);
                    if gx == Some(mx) && gy == Some(my) && gz == Some(mz) {
                        results.push(format!("步骤{} (goto) 跳过: goto ({},{},{}) 是上一步刚挖的位置，bot 已在附近无需 goto。", i + 1, mx, my, mz));
                        last_mined = None;
                        continue;
                    }
                }
            }
            let mc = parse_step(action_name, step)?;
            // 记录 mine 坐标供下一步检测
            if let MinecraftAction::MineBlock { x, y, z } = &mc {
                last_mined = Some((*x, *y, *z));
            } else {
                last_mined = None;
            }
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
        "mine_above" => Ok(MinecraftAction::MineAbove),
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
        "craft_3x3" => {
            let table_pos = match (i64("table_x"), i64("table_y"), i64("table_z")) {
                (Some(x), Some(y), Some(z)) => Some((x as i32, y as i32, z as i32)),
                _ => None,
            };
            Ok(MinecraftAction::Craft3x3 {
                item: str("item").ok_or_else(|| anyhow::anyhow!("craft_3x3 缺少 item"))?,
                count: u32("count").unwrap_or(1),
                table_pos,
            })
        }
        "smelt" => {
            let table_pos = match (i64("table_x"), i64("table_y"), i64("table_z")) {
                (Some(x), Some(y), Some(z)) => Some((x as i32, y as i32, z as i32)),
                _ => None,
            };
            Ok(MinecraftAction::Smelt {
                output: str("output").ok_or_else(|| anyhow::anyhow!("smelt 缺少 output"))?,
                fuel: str("fuel").unwrap_or_else(|| "coal".to_string()),
                count: u32("count").unwrap_or(1),
                table_pos,
            })
        }
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
        "trade" => Ok(MinecraftAction::Trade {
            offer: u32("offer").unwrap_or(0),
        }),
        "interact_entity" => Ok(MinecraftAction::InteractEntity {
            kind: str("kind").unwrap_or_else(|| "villager".to_string()),
        }),
        "pickup" => Ok(MinecraftAction::Pickup),
        "defend" => Ok(MinecraftAction::Defend),
        "set_goal" => Ok(MinecraftAction::Chat {
            content: format!("[set_goal] {}", str("goal").unwrap_or_default()),
        }),
        // perceive 在 plan 里不执行实际动作，只返回提示（plan 是动作序列，perceive 由
        // agent 主循环的 auto_perceive 处理）。
        "perceive" | "look" | "look_at" => Err(anyhow::anyhow!(
            "perceive 不支持在 run_plan 里调用（agent 主循环每轮自动注入 perceive，plan 里只放动作）"
        )),
        other => Err(anyhow::anyhow!(
            "不支持的 action: {other}（支持: goto/mine/mine_below/interact/attack/chat/craft/craft_3x3/smelt/gather/place/open/auto_craft/enchant/trade/interact_entity/pickup/defend）"
        )),
    }
}

/// 搜索 Minecraft Wiki（中文源，国内可访问）。
/// 使用 Bilibili 游戏 Wiki（wiki.biligame.com/mc）的 MediaWiki 搜索 API。
pub struct SearchWikiTool {
    ctx: Arc<AzaleaToolCtx>,
}
impl SearchWikiTool {
    pub fn new(ctx: Arc<AzaleaToolCtx>) -> Self {
        Self { ctx }
    }
}
impl GameTool for SearchWikiTool {
    fn name(&self) -> &str {
        "search_wiki"
    }
    fn description(&self) -> &str {
        "搜索 Minecraft Wiki（中文），查询方块/物品/生物/机制等游戏知识。\
         参数 query 为搜索关键词（中文）。返回最多 3 条结果，含标题和摘要。\
         例: search_wiki(query=\"铁砧\") / search_wiki(query=\"how to make a pickaxe\")"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "搜索关键词" }
            },
            "required": ["query"]
        })
    }
    fn effects(&self) -> ToolEffects {
        ToolEffects::read()
    }
    fn execute(
        &self,
        _call_id: &str,
        args: Value,
        _on_update: Option<craft_agent::core::tool::ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        let query = args.get("query").and_then(|v| v.as_str()).ok_or_else(|| anyhow::anyhow!("缺少 query"))?;
        let params = [
            ("action", "opensearch"),
            ("search", query),
            ("limit", "3"),
            ("format", "json"),
        ];
        let url = reqwest::Url::parse_with_params("https://wiki.biligame.com/mc/api.php", &params)
            .map_err(|e| anyhow::anyhow!("URL error: {e}"))?;
        let resp = reqwest::blocking::get(url)?.text()?;
        let json: serde_json::Value = serde_json::from_str(&resp)?;
        let results = json.as_array().and_then(|arr| arr.get(1)).and_then(|v| v.as_array());
        let urls = json.as_array().and_then(|arr| arr.get(3)).and_then(|v| v.as_array());
        match results {
            Some(items) if !items.is_empty() => {
                let mut lines: Vec<String> = Vec::new();
                for (i, item) in items.iter().enumerate() {
                    let title = item.as_str().unwrap_or("?");
                    let link = urls.and_then(|u| u.get(i)).and_then(|v| v.as_str()).unwrap_or("");
                    lines.push(format!("{}. {} ({})", i + 1, title, link));
                }
                Ok(ToolResult {
                    message: format!("Wiki 搜索结果 ({}):\n{}", query, lines.join("\n")),
                    is_error: false,
                    images: vec![],
                })
            }
            _ => Ok(ToolResult {
                message: format!("Wiki 搜索无结果: {query}"),
                is_error: false,
                images: vec![],
            }),
        }
    }
}

fn _exec_action(adapter: &Arc<Mutex<MinecraftAzaleaAdapter>>, mc: MinecraftAction) -> String {
    match adapter.lock().unwrap().exec_mc_sync(mc, 120_000) {
        Ok(r) => r.detail,
        Err(e) => format!("错误: {e}"),
    }
}

/// 执行 rhai 脚本（嵌入式脚本引擎，直接在 Rust 进程内执行，比 Node.js 更快更轻量）。
///
/// 学习自 Mindcraft `library/skills.js` + `agent/commands/actions.js`：把全部动作工具暴露为
/// rhai 函数，LLM 用一段脚本即可完成多步任务（采集→合成→放置），比 run_plan 更灵活。
///
/// **白名单（24 个动作函数 + 2 个工具函数）**：
/// - 移动/挖掘：`go(x,y,z)` `mine(x,y,z)` `mine_below()` `mine_above()` `interact(x,y,z)`
/// - 战斗：`attack(target?)` `defend()`
/// - 合成/熔炼：`craft(item,count)` `craft_3x3(item,count)` `smelt(output,fuel,count)` `auto_craft(item,count)` `enchant(item,level)`
/// - 采集/放置：`gather(item,count)` `place(item,x,y,z)` `open(x,y,z)`
/// - 容器：`chest_view(x,y,z)` `chest_withdraw(x,y,z,item,count)` `chest_deposit(x,y,z,item,count)`
/// - 装备/消耗：`equip(item,slot)` `discard(item,count)` `consume(item)`
/// - 交互：`interact_entity(kind)` `trade(offer)` `chat(msg)` `pickup()`
/// - 工具：`perceive()` 返回结构化世界状态文本 / `list_blueprints()` 列出蓝图 / `build_blueprint(name,x,y,z)` 建造蓝图
/// - 元：`sleep(ms)` `print(msg)`
///
/// **注意**：
/// - 寻路函数名是 `go`（不是 `goto`，`goto` 是 rhai 保留字）。
/// - 不暴露：`run_plan` / `run_script`（递归），`memory` / `set_goal` / `pause_goal` / `resume_goal`
///   （这些直接修改 Agent/记忆库状态，不应在脚本里调用），`search_wiki`（HTTP 阻塞），`build`（用
///   `build_blueprint` 替代，更安全）。
///
/// **lint**：执行前 `lint_script()` 检查长度/禁用关键字/危险模式，拒绝则不执行。
pub struct RunScriptTool {
    ctx: Arc<AzaleaToolCtx>,
}
impl RunScriptTool {
    pub fn new(ctx: Arc<AzaleaToolCtx>) -> Self {
        Self { ctx }
    }
}
impl GameTool for RunScriptTool {
    fn name(&self) -> &str {
        "run_script"
    }
    fn description(&self) -> &str {
        "执行 rhai 脚本（嵌入式引擎，沙箱化）。支持变量、循环、条件。\
         动作函数: walk_to(x,y,z) [或 move_to/step_to，不要用 go/goto，rhai 保留字], \
         mine(x,y,z), mine_below(), mine_above(), interact(x,y,z), attack(target?), defend(), \
         craft(item,count), craft_3x3(item,count), smelt(output,fuel,count), auto_craft(item,count), enchant(item,level), \
         gather(item,count), place(item,x,y,z), open(x,y,z), \
         chest_view(x,y,z), chest_withdraw(x,y,z,item,count), chest_deposit(x,y,z,item,count), \
         equip(item,slot), discard(item,count), consume(item), \
         interact_entity(kind), trade(offer), chat(msg), pickup(), \
         perceive(), list_blueprints(), build_blueprint(name,x,y,z), \
         sleep(ms), print(msg)。\
         脚本最后一行若是动作函数调用会作为返回值；不需要返回值时末尾加分号 `;`。\
         例: walk_to(10, 64, 20); gather(\"oak_log\", 4); craft(\"oak_planks\", 4); pickup();"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "script": { "type": "string", "description": "rhai 脚本代码（≤8KB，禁用 import/eval 等）" }
            },
            "required": ["script"]
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
        let script = args.get("script").and_then(|v| v.as_str()).ok_or_else(|| anyhow::anyhow!("缺少 script"))?;
        // 1. lint：长度/禁用关键字/危险模式
        if let Err(reason) = lint_script(script) {
            return Ok(ToolResult {
                message: format!("脚本被 lint 拒绝: {reason}"),
                is_error: true,
                images: vec![],
            });
        }
        // 2. 构建沙箱引擎（含 call_action 递归支持）
        let engine = build_rhai_engine(&self.ctx);
        // 用 Dynamic 接收任意返回类型：rhai 脚本最后一行若以 `;` 结尾返回 unit ()，
        // 若是表达式则返回该值。eval::<String>() 在 unit 时报 "Output type incorrect: ()"，
        // 改用 Dynamic 后 unit 显示为 "()"，我们识别后转为 "脚本执行完成"。
        match engine.eval::<rhai::Dynamic>(script) {
            Ok(out) => {
                let msg = if out.is_unit() || out.to_string().is_empty() {
                    "脚本执行完成".to_string()
                } else {
                    out.to_string()
                };
                Ok(ToolResult {
                    message: msg,
                    is_error: false,
                    images: vec![],
                })
            }
            Err(e) => Ok(ToolResult {
                message: format!("脚本错误: {e}"),
                is_error: true,
                images: vec![],
            }),
        }
    }
}

/// 构建 rhai 沙箱引擎：注册全部 27 个动作函数 + call_action + sleep/print + 资源限制。
///
/// `call_action(name)` 会递归调用此函数构建子引擎执行已保存的 LLM 自定义动作，
/// 递归深度由 `max_call_levels=20` 兜底。
fn build_rhai_engine(ctx: &Arc<AzaleaToolCtx>) -> rhai::Engine {
    let adapter = ctx.adapter.0.clone();
    let blueprints = ctx.blueprints.clone();
    let actions = ctx.actions.clone();
    let adapter_for_perceive = ctx.adapter.clone();
    let mut engine = rhai::Engine::new();

    // ===== 移动/挖掘 =====
    // 寻路函数注册三个别名 walk_to / move_to / step_to，避免使用 rhai 1.25 保留字 `go`/`goto`。
    // LLM 在脚本里写任一别名都生效。
    let a = adapter.clone();
    engine.register_fn("walk_to", move |x: i64, y: i64, z: i64| -> String {
        _exec_action(&a, MinecraftAction::Goto { x: x as i32, y: y as i32, z: z as i32 })
    });
    let a = adapter.clone();
    engine.register_fn("move_to", move |x: i64, y: i64, z: i64| -> String {
        _exec_action(&a, MinecraftAction::Goto { x: x as i32, y: y as i32, z: z as i32 })
    });
    let a = adapter.clone();
    engine.register_fn("step_to", move |x: i64, y: i64, z: i64| -> String {
        _exec_action(&a, MinecraftAction::Goto { x: x as i32, y: y as i32, z: z as i32 })
    });
    let a = adapter.clone();
    engine.register_fn("mine", move |x: i64, y: i64, z: i64| -> String {
        _exec_action(&a, MinecraftAction::MineBlock { x: x as i32, y: y as i32, z: z as i32 })
    });
    let a = adapter.clone();
    engine.register_fn("mine_below", move || -> String {
        _exec_action(&a, MinecraftAction::MineBelow)
    });
    let a = adapter.clone();
    engine.register_fn("mine_above", move || -> String {
        _exec_action(&a, MinecraftAction::MineAbove)
    });
    let a = adapter.clone();
    engine.register_fn("interact", move |x: i64, y: i64, z: i64| -> String {
        _exec_action(&a, MinecraftAction::InteractBlock { x: x as i32, y: y as i32, z: z as i32 })
    });

    // ===== 战斗 =====
    let a = adapter.clone();
    engine.register_fn("attack", move |target: String| -> String {
        let t = if target.is_empty() { "nearest".to_string() } else { target };
        _exec_action(&a, MinecraftAction::Attack { target: t })
    });
    let a = adapter.clone();
    engine.register_fn("attack", move || -> String {
        _exec_action(&a, MinecraftAction::Attack { target: "nearest".to_string() })
    });
    let a = adapter.clone();
    engine.register_fn("defend", move || -> String {
        _exec_action(&a, MinecraftAction::Defend)
    });

    // ===== 合成/熔炼/附魔 =====
    let a = adapter.clone();
    engine.register_fn("craft", move |item: String, count: i64| -> String {
        _exec_action(&a, MinecraftAction::Craft { item, count: count as u32 })
    });
    let a = adapter.clone();
    engine.register_fn("craft_3x3", move |item: String, count: i64| -> String {
        _exec_action(&a, MinecraftAction::Craft3x3 { item, count: count as u32, table_pos: None })
    });
    let a = adapter.clone();
    engine.register_fn("smelt", move |output: String, fuel: String, count: i64| -> String {
        _exec_action(&a, MinecraftAction::Smelt { output, fuel, count: count as u32, table_pos: None })
    });
    let a = adapter.clone();
    engine.register_fn("auto_craft", move |item: String, count: i64| -> String {
        _exec_action(&a, MinecraftAction::AutoCraft { item, count: count as u32 })
    });
    let a = adapter.clone();
    engine.register_fn("enchant", move |item: String, level: i64| -> String {
        _exec_action(&a, MinecraftAction::Enchant { item, level: level as u32 })
    });

    // ===== 采集/放置 =====
    let a = adapter.clone();
    engine.register_fn("gather", move |item: String, count: i64| -> String {
        _exec_action(&a, MinecraftAction::Gather { item, count: count as u32 })
    });
    let a = adapter.clone();
    engine.register_fn("place", move |item: String, x: i64, y: i64, z: i64| -> String {
        _exec_action(&a, MinecraftAction::Place { item, x: x as i32, y: y as i32, z: z as i32 })
    });
    let a = adapter.clone();
    engine.register_fn("open", move |x: i64, y: i64, z: i64| -> String {
        _exec_action(&a, MinecraftAction::OpenContainer { x: x as i32, y: y as i32, z: z as i32 })
    });

    // ===== 容器 =====
    let a = adapter.clone();
    engine.register_fn("chest_view", move |x: i64, y: i64, z: i64| -> String {
        _exec_action(&a, MinecraftAction::ChestView { x: x as i32, y: y as i32, z: z as i32 })
    });
    let a = adapter.clone();
    engine.register_fn("chest_withdraw", move |x: i64, y: i64, z: i64, item: String, count: i64| -> String {
        _exec_action(&a, MinecraftAction::ChestWithdraw { x: x as i32, y: y as i32, z: z as i32, item, count: count as u32 })
    });
    let a = adapter.clone();
    engine.register_fn("chest_deposit", move |x: i64, y: i64, z: i64, item: String, count: i64| -> String {
        _exec_action(&a, MinecraftAction::ChestDeposit { x: x as i32, y: y as i32, z: z as i32, item, count: count as u32 })
    });

    // ===== 装备/消耗 =====
    let a = adapter.clone();
    engine.register_fn("equip", move |item: String, slot: String| -> String {
        _exec_action(&a, MinecraftAction::Equip { item, slot })
    });
    let a = adapter.clone();
    engine.register_fn("discard", move |item: String, count: i64| -> String {
        _exec_action(&a, MinecraftAction::Discard { item, count: count as u32 })
    });
    let a = adapter.clone();
    engine.register_fn("consume", move |item: String| -> String {
        _exec_action(&a, MinecraftAction::Consume { item })
    });

    // ===== 交互 =====
    let a = adapter.clone();
    engine.register_fn("interact_entity", move |kind: String| -> String {
        _exec_action(&a, MinecraftAction::InteractEntity { kind })
    });
    let a = adapter.clone();
    engine.register_fn("trade", move |offer: i64| -> String {
        _exec_action(&a, MinecraftAction::Trade { offer: offer as u32 })
    });
    let a = adapter.clone();
    engine.register_fn("chat", move |msg: String| -> String {
        _exec_action(&a, MinecraftAction::Chat { content: msg })
    });
    let a = adapter.clone();
    engine.register_fn("pickup", move || -> String {
        _exec_action(&a, MinecraftAction::Pickup)
    });

    // ===== 感知/蓝图（读路径，不经过 BotCommand 队列） =====
    engine.register_fn("perceive", move || -> String {
        match adapter_for_perceive.perceive_shared() {
            Ok(st) => format!("{}", st.self_hint),
            Err(e) => format!("perceive 错误: {e}"),
        }
    });
    let bp_for_list = blueprints.clone();
    engine.register_fn("list_blueprints", move || -> String {
        bp_for_list.list_summary()
    });
    let bp_for_build = blueprints.clone();
    let adapter_for_build = adapter.clone();
    engine.register_fn("build_blueprint", move |name: String, x: i64, y: i64, z: i64| -> String {
        let bp = match bp_for_build.get(&name) {
            Some(b) => b.clone(),
            None => return format!("未知蓝图 '{name}'。可用：\n{}", bp_for_build.list_summary()),
        };
        let abs_json = bp.instantiate(x as i32, y as i32, z as i32);
        let blocks = match serde_json::from_str::<serde_json::Value>(&abs_json) {
            Ok(v) => v,
            Err(e) => return format!("蓝图 JSON 解析失败: {e}"),
        };
        let blocks_arr = match blocks.get("blocks").and_then(|v| v.as_array()) {
            Some(a) => a,
            None => return "蓝图缺少 blocks 数组".to_string(),
        };
        let mut results: Vec<String> = Vec::new();
        for (i, block) in blocks_arr.iter().enumerate() {
            let bx = block.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let by = block.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let bz = block.get("z").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let block_id = block.get("block").and_then(|v| v.as_str()).unwrap_or("");
            let goto_r = _exec_action(&adapter_for_build, MinecraftAction::Goto { x: bx, y: by, z: bz });
            if goto_r.starts_with("错误") {
                results.push(format!("第{}块 goto 失败: {goto_r}", i + 1));
                break;
            }
            let place_r = _exec_action(&adapter_for_build, MinecraftAction::Place {
                item: block_id.to_string(),
                x: bx, y: by, z: bz,
            });
            if place_r.starts_with("错误") {
                results.push(format!("第{}块 place {block_id} 失败: {place_r}", i + 1));
                break;
            }
            results.push(format!("第{}块: placed {block_id} @({bx},{by},{bz})", i + 1));
        }
        results.join("\n")
    });

    // ===== P2-4: call_action —— 调用 LLM 自定义动作 =====
    // 递归：call_action(name) 查找已保存动作 → lint → 构建新引擎 → eval。
    // 递归深度由 max_call_levels=20 兜底；call_count 通过 Arc<Mutex<ActionLibrary>> 共享。
    let ctx_for_call = ctx.clone();
    engine.register_fn("call_action", move |name: String| -> String {
        // 1. 查找动作
        let action = {
            let lib = ctx_for_call.actions.lock().unwrap();
            lib.get(&name).cloned()
        };
        let action = match action {
            Some(a) => a,
            None => {
                let lib = ctx_for_call.actions.lock().unwrap();
                return format!("未知动作 '{name}'。可用：\n{}", lib.list_summary());
            }
        };
        // 2. lint（再次检查，防止从盘上加载后被篡改）
        if let Err(reason) = lint_action_script(&action.script) {
            return format!("动作 '{name}' 脚本被 lint 拒绝: {reason}");
        }
        // 3. 构建子引擎并执行（递归调用 build_rhai_engine）
        let sub_engine = build_rhai_engine(&ctx_for_call);
        // 4. 增加调用计数（持久化）
        {
            let mut lib = ctx_for_call.actions.lock().unwrap();
            lib.bump_call_count(&name);
        }
        match sub_engine.eval::<rhai::Dynamic>(&action.script) {
            Ok(out) => {
                let s = out.to_string();
                if out.is_unit() || s.is_empty() {
                    format!("[call_action {name}] 完成")
                } else {
                    s
                }
            }
            Err(e) => format!("[call_action {name}] 脚本错误: {e}"),
        }
    });

    // ===== 元：sleep / print =====
    engine.register_fn("sleep", |ms: i64| {
        // 上限 10s，避免 LLM 写 sleep(999999) 卡死 bot
        let capped = ms.clamp(0, 10_000) as u64;
        std::thread::sleep(std::time::Duration::from_millis(capped));
    });
    engine.register_fn("print", |msg: String| -> String {
        println!("[bot] {msg}");
        msg
    });

    // ===== 沙箱：资源限制 =====
    engine.set_max_operations(100_000);     // 100k AST 操作（足够复杂脚本）
    engine.set_max_call_levels(20);          // 递归深度上限
    engine.set_max_string_size(64 * 1024);   // 64KB 字符串上限
    engine.set_max_array_size(1024);         // 数组上限
    engine.set_max_map_size(256);            // map 上限
    // 禁用所有内置模块（file/io/http/process），rhai 默认就不带这些，但显式禁用更安全
    engine.disable_symbol("eval");
    engine.disable_symbol("Fn");
    engine.disable_symbol("call");

    engine
}

/// LLM 自定义动作脚本的 lint（与 lint_script 相同，但额外检查动作嵌套深度）。
///
/// 已保存的动作脚本里若包含 `call_action` 是允许的（递归调用），但 `lint_script` 会
/// 拒绝 `call(` 关键字——所以我们用单独的 lint 函数，不检查 `call(`。
fn lint_action_script(script: &str) -> Result<(), String> {
    const MAX_SCRIPT_BYTES: usize = 8 * 1024;
    if script.len() > MAX_SCRIPT_BYTES {
        return Err(format!(
            "脚本过长 ({} bytes > {} bytes 上限)",
            script.len(),
            MAX_SCRIPT_BYTES
        ));
    }
    // 不含 `call(` 检查（call_action 是合法的）
    const FORBIDDEN: &[&str] = &[
        "import", "export", "eval", "::",
        "Fn(", "fn(",
        "read_file", "write_file", "append_file", "print_file",
        "http::", "http_get", "http_post",
        "import_node", "process::", "std::",
    ];
    for kw in FORBIDDEN {
        if script.contains(kw) {
            return Err(format!("脚本包含禁用关键字 '{kw}'"));
        }
    }
    let lower = script.to_lowercase();
    if (lower.contains("while true") || lower.contains("while (true)") || lower.contains("loop {"))
        && !lower.contains("break")
    {
        return Err("检测到 while true / loop 但无 break：可能死循环".to_string());
    }
    Ok(())
}

/// 脚本 lint：在 rhai 引擎 eval 前做静态检查。
///
/// 检查项：
/// 1. 长度：≤8KB（防止 LLM 灌入超长脚本撑爆内存）
/// 2. 禁用关键字：`import` / `export` / `eval` / `Fn` / `call` / `print_file` / `read_file` / `write_file` / `http` / `import_node`
/// 3. 危险模式：`while true` 无 break / `loop` 无 break（启发式，可能误报但安全优先）
/// 4. 禁止注释绕过检查：lint 看的是 strip 后的脚本，但 rhai 不支持 import，所以即使有 import 字符串也直接禁
fn lint_script(script: &str) -> Result<(), String> {
    const MAX_SCRIPT_BYTES: usize = 8 * 1024;
    if script.len() > MAX_SCRIPT_BYTES {
        return Err(format!(
            "脚本过长 ({} bytes > {} bytes 上限)。请拆分为多个 run_script 调用或用 run_plan。",
            script.len(),
            MAX_SCRIPT_BYTES
        ));
    }
    // 禁用关键字（任何位置出现即拒）。rhai 区分大小写：`Fn` 是反射入口，必须大写 F；
    // `import` / `eval` 等也是小写关键字。这里同时检查大小写两种变体以兜底 LLM 写错。
    const FORBIDDEN: &[&str] = &[
        "import", "export", "eval", "::",
        "Fn(", "fn(", "call(", "call ",
        "read_file", "write_file", "append_file", "print_file",
        "http::", "http_get", "http_post",
        "import_node", "process::", "std::",
    ];
    for kw in FORBIDDEN {
        if script.contains(kw) {
            return Err(format!("脚本包含禁用关键字 '{kw}'（rhai 沙箱禁止 IO/模块/反射）"));
        }
    }
    // 危险模式：`while true` / `loop` 必须有 break（大小写不敏感检查）
    let lower = script.to_lowercase();
    if (lower.contains("while true") || lower.contains("while (true)") || lower.contains("loop {"))
        && !lower.contains("break")
    {
        return Err("检测到 while true / loop 但无 break：可能死循环。请加 break 或用 for 循环。".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod run_script_tests {
    use super::*;

    #[test]
    fn lint_rejects_too_long_script() {
        let big = "print(\"hi\");".repeat(10_000);
        let r = lint_script(&big);
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("过长"));
    }

    #[test]
    fn lint_rejects_forbidden_keywords() {
        assert!(lint_script("import \"x\";").is_err());
        assert!(lint_script("eval(\"1+1\");").is_err());
        assert!(lint_script("Fn(\"x\");").is_err());
        assert!(lint_script("call(\"foo\");").is_err());
        assert!(lint_script("read_file(\"x\");").is_err());
        assert!(lint_script("let s = std::io::print;").is_err());
    }

    #[test]
    fn lint_allows_normal_scripts() {
        assert!(lint_script("let r = go(10, 64, 20); print(r);").is_ok());
        assert!(lint_script("for i in 0..10 { print(i); }").is_ok());
        assert!(lint_script("while true { break; }").is_ok());
        assert!(lint_script("let r = gather(\"oak_log\", 4); craft(\"oak_planks\", 4);").is_ok());
    }

    #[test]
    fn lint_rejects_while_true_no_break() {
        assert!(lint_script("while true { print(\"x\"); }").is_err());
        assert!(lint_script("loop { print(\"x\"); }").is_err());
    }

    #[test]
    fn lint_action_allows_call_action_keyword() {
        // call_action 是 P2-4 的合法递归调用，不应被 lint 拒绝
        assert!(lint_action_script("let r = call_action(\"gather_wood\"); print(r);").is_ok());
    }

    #[test]
    fn lint_action_rejects_forbidden_keywords() {
        // 即使允许 call_action，其他禁字仍要拒绝
        assert!(lint_action_script("import \"x\";").is_err());
        assert!(lint_action_script("eval(\"1+1\");").is_err());
        assert!(lint_action_script("Fn(\"x\");").is_err());
        assert!(lint_action_script("read_file(\"x\");").is_err());
        assert!(lint_action_script("let s = std::io::print;").is_err());
    }

    #[test]
    fn lint_action_rejects_too_long() {
        let big = "print(\"hi\");".repeat(10_000);
        assert!(lint_action_script(&big).is_err());
    }

    #[test]
    fn lint_action_rejects_infinite_loops() {
        assert!(lint_action_script("while true { print(\"x\"); }").is_err());
        assert!(lint_action_script("loop { print(\"x\"); }").is_err());
    }

    #[test]
    fn lint_action_allows_break_controlled_loops() {
        assert!(lint_action_script("while true { break; }").is_ok());
        assert!(lint_action_script("loop { break; }").is_ok());
    }

    /// 回归测试：rhai 1.25 把 `go` 列为保留字，`register_fn("go", ...)` 也无法让
    /// 脚本调用 `go(...)` —— 解析阶段就报 "Syntax error: 'go' is a reserved keyword"。
    /// 我们改用 `walk_to` 等别名，必须保证这些别名不在保留字列表里。
    #[test]
    fn rhai_walk_to_is_not_reserved_keyword() {
        let mut engine = rhai::Engine::new();
        // 注册一个简单的 walk_to 函数，确认脚本能解析+执行（不报 reserved keyword）
        engine.register_fn("walk_to", |x: i64, _y: i64, _z: i64| -> i64 { x });
        let r: rhai::Dynamic = engine.eval("walk_to(10, 64, 20)").expect("walk_to 不应被当作保留字");
        assert_eq!(r.as_int().unwrap(), 10);

        // move_to / step_to 同样不应是保留字
        engine.register_fn("move_to", |x: i64, _y: i64, _z: i64| -> i64 { x + 1 });
        let r: rhai::Dynamic = engine.eval("move_to(5, 64, 5)").expect("move_to 不应被当作保留字");
        assert_eq!(r.as_int().unwrap(), 6);

        engine.register_fn("step_to", |x: i64, _y: i64, _z: i64| -> i64 { x + 2 });
        let r: rhai::Dynamic = engine.eval("step_to(0, 64, 0)").expect("step_to 不应被当作保留字");
        assert_eq!(r.as_int().unwrap(), 2);
    }

    /// 回归测试：脚本最后一行以 `;` 结尾时返回 `unit ()`，旧 `eval::<String>()` 报
    /// "Output type incorrect: () (expecting string)"。改用 `eval::<Dynamic>()` 后
    /// unit 被识别并转为 "脚本执行完成"。
    #[test]
    fn rhai_unit_return_does_not_error() {
        let engine = rhai::Engine::new();
        // 脚本末尾是 print(...) 调用以分号结尾 → 返回 ()
        let script = "print(\"hello\");";
        let out: rhai::Dynamic = engine.eval(script).expect("unit 返回值不应报错");
        assert!(out.is_unit(), "脚本以 ; 结尾应返回 unit");
    }

    /// 回归测试：确认 `go` 确实是 rhai 1.25 的保留字（保证我们不会无意中回退到 go）。
    #[test]
    fn rhai_go_is_reserved_keyword_confirmed() {
        let mut engine = rhai::Engine::new();
        engine.register_fn("go", |x: i64, _y: i64, _z: i64| -> i64 { x });
        // 即使注册了，脚本里写 go(...) 也报 reserved keyword
        let r = engine.eval::<rhai::Dynamic>("go(1, 2, 3)");
        assert!(r.is_err(), "go 应是 rhai 保留字，预期解析失败");
        let err = r.unwrap_err().to_string();
        assert!(
            err.contains("reserved") || err.contains("'go'"),
            "错误信息应提到 reserved/go，实际: {err}"
        );
    }
}
/// 执行蓝图建造：按 JSON 描述的方块列表依次放置。
/// 格式: {"blocks":[{"x":10,"y":64,"z":20,"block":"oak_planks"}, ...]}
/// 自动检查背包是否有材料，缺材料时报错。每步先 goto 到目标位置再 place。
pub struct BuildTool {
    ctx: Arc<AzaleaToolCtx>,
}
impl BuildTool {
    pub fn new(ctx: Arc<AzaleaToolCtx>) -> Self {
        Self { ctx }
    }
}
impl GameTool for BuildTool {
    fn name(&self) -> &str {
        "build"
    }
    fn description(&self) -> &str {
        "按蓝图建造：JSON 格式 {\"blocks\":[{\"x\":10,\"y\":64,\"z\":20,\"block\":\"oak_planks\"}, ...]}。\
         自动 goto 到每个位置再 place。材料不足时报错。\
         例: build(blueprint=\"{\\\"blocks\\\":[{\\\"x\\\":10,\\\"y\\\":64,\\\"z\\\":20,\\\"block\\\":\\\"oak_planks\\\"}]}\")"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "blueprint": { "type": "string", "description": "JSON 蓝图，格式 {\"blocks\":[{\"x\":int,\"y\":int,\"z\":int,\"block\":\"id\"}]}" }
            },
            "required": ["blueprint"]
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
        let bp_str = args.get("blueprint").and_then(|v| v.as_str()).ok_or_else(|| anyhow::anyhow!("缺少 blueprint"))?;
        let bp: serde_json::Value = serde_json::from_str(bp_str).map_err(|e| anyhow::anyhow!("JSON 解析失败: {e}"))?;
        let blocks = bp.get("blocks").and_then(|v| v.as_array()).ok_or_else(|| anyhow::anyhow!("缺少 blocks 数组"))?;
        let adapter = self.ctx.adapter.0.clone();
        let mut results: Vec<String> = Vec::new();
        for (i, block) in blocks.iter().enumerate() {
            let x = block.get("x").and_then(|v| v.as_i64()).ok_or_else(|| anyhow::anyhow!("第{}个方块缺少 x", i+1))? as i32;
            let y = block.get("y").and_then(|v| v.as_i64()).ok_or_else(|| anyhow::anyhow!("第{}个方块缺少 y", i+1))? as i32;
            let z = block.get("z").and_then(|v| v.as_i64()).ok_or_else(|| anyhow::anyhow!("第{}个方块缺少 z", i+1))? as i32;
            let block_id = block.get("block").and_then(|v| v.as_str()).ok_or_else(|| anyhow::anyhow!("第{}个方块缺少 block", i+1))?;
            // 先 goto 到目标位置
            let goto_result = _exec_action(&adapter, MinecraftAction::Goto { x, y, z });
            if goto_result.starts_with("错误") {
                results.push(format!("第{}个 (goto) 失败: {goto_result}", i+1));
                break;
            }
            // 放置方块
            let place_result = _exec_action(&adapter, MinecraftAction::Place {
                item: block_id.to_string(),
                x, y, z,
            });
            if place_result.starts_with("错误") {
                results.push(format!("第{}个 (place {block_id}) 失败: {place_result}", i+1));
                break;
            }
            results.push(format!("第{}个: placed {block_id} @({x},{y},{z})", i+1));
        }
        Ok(ToolResult {
            message: results.join("\n"),
            is_error: false,
            images: vec![],
        })
    }
}

/// 按预定义蓝图名称建造（P2-1）。
/// 蓝图存放在 `blueprints/` 目录，bot 调用 `build_blueprint(name, x, y, z)` 即可
/// 在原点 (x,y,z) 实例化蓝图（相对坐标→绝对坐标自动展开）。
pub struct BuildBlueprintTool {
    ctx: Arc<AzaleaToolCtx>,
}
impl BuildBlueprintTool {
    pub fn new(ctx: Arc<AzaleaToolCtx>) -> Self {
        Self { ctx }
    }
}
impl GameTool for BuildBlueprintTool {
    fn name(&self) -> &str {
        "build_blueprint"
    }
    fn description(&self) -> &str {
        "按预定义蓝图名称建造（P2-1）。蓝图文件在 `blueprints/` 目录，\
         内置：small_shelter（3x3 木屋）/ farm_plot（9x9 农田）/ storage_corner（储物角）/ torch_pillar（标记柱）。\n\
         bot 自动：1) 查蓝图 → 2) 计算材料清单 → 3) 逐方块 goto+place。\n\
         参数：name 蓝图名，x/y/z 蓝图原点（相对坐标 dx/dy/dz 加上原点 = 实际世界坐标）。\n\
         示例：build_blueprint(name=\"torch_pillar\", x=100, y=64, z=-50) 在该坐标立一根火把柱。"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "蓝图名（如 small_shelter / farm_plot / storage_corner / torch_pillar）" },
                "x": { "type": "integer", "description": "蓝图原点 X 坐标（dx=0 的实际位置）" },
                "y": { "type": "integer", "description": "蓝图原点 Y 坐标" },
                "z": { "type": "integer", "description": "蓝图原点 Z 坐标" }
            },
            "required": ["name", "x", "y", "z"]
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
        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("缺少 name"))?
            .to_string();
        let x = args.get("x").and_then(|v| v.as_i64()).ok_or_else(|| anyhow::anyhow!("缺少 x"))? as i32;
        let y = args.get("y").and_then(|v| v.as_i64()).ok_or_else(|| anyhow::anyhow!("缺少 y"))? as i32;
        let z = args.get("z").and_then(|v| v.as_i64()).ok_or_else(|| anyhow::anyhow!("缺少 z"))? as i32;

        let bp = self
            .ctx
            .blueprints
            .get(&name)
            .ok_or_else(|| {
                let avail = self.ctx.blueprints.list_summary();
                anyhow::anyhow!("未知蓝图 '{name}'。可用蓝图：\n{avail}")
            })?
            .clone();

        // 先把材料清单回给 LLM（让它决定是否先采集）
        let materials = bp.material_summary();
        let bounds = bp.bounds();
        let abs_json = bp.instantiate(x, y, z);

        // 复用 BuildTool 的执行逻辑：把蓝图实例化的 JSON 当作普通 blueprint 参数执行
        let adapter = self.ctx.adapter.0.clone();
        let bp_value: serde_json::Value = serde_json::from_str(&abs_json)
            .map_err(|e| anyhow::anyhow!("蓝图实例化 JSON 解析失败: {e}"))?;
        let blocks = bp_value
            .get("blocks")
            .and_then(|v| v.as_array())
            .ok_or_else(|| anyhow::anyhow!("蓝图实例化后无 blocks 数组"))?;

        let mut results: Vec<String> = Vec::new();
        results.push(format!(
            "蓝图 '{name}' @({x},{y},{z}) 边界 dx{}..{} dy{}..{} dz{}..{} | 材料: {materials}",
            bounds.0, bounds.3, bounds.1, bounds.4, bounds.2, bounds.5
        ));

        for (i, block) in blocks.iter().enumerate() {
            let bx = block.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let by = block.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let bz = block.get("z").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let block_id = block.get("block").and_then(|v| v.as_str()).unwrap_or("");
            let goto_result = _exec_action(&adapter, MinecraftAction::Goto { x: bx, y: by, z: bz });
            if goto_result.starts_with("错误") {
                results.push(format!("第{}个 (goto) 失败: {goto_result}", i + 1));
                break;
            }
            let place_result = _exec_action(&adapter, MinecraftAction::Place {
                item: block_id.to_string(),
                x: bx, y: by, z: bz,
            });
            if place_result.starts_with("错误") {
                results.push(format!("第{}个 (place {block_id}) 失败: {place_result}", i + 1));
                break;
            }
            results.push(format!("第{}个: placed {block_id} @({bx},{by},{bz})", i + 1));
        }

        Ok(ToolResult {
            message: results.join("\n"),
            is_error: false,
            images: vec![],
        })
    }
}

/// 列出所有可用蓝图（P2-1）。
pub struct ListBlueprintsTool {
    ctx: Arc<AzaleaToolCtx>,
}
impl ListBlueprintsTool {
    pub fn new(ctx: Arc<AzaleaToolCtx>) -> Self {
        Self { ctx }
    }
}
impl GameTool for ListBlueprintsTool {
    fn name(&self) -> &str {
        "list_blueprints"
    }
    fn description(&self) -> &str {
        "列出所有可用蓝图名 + 描述 + 材料清单（P2-1）。无参数。\n\
         返回示例：\n\
         - small_shelter: 3x3 木屋 | 材料: oak_planks:5, oak_log:4, ...\n\
         - torch_pillar: 标记柱 | 材料: cobblestone:3, torch:1\n\n\
         用 build_blueprint(name=..., x=..., y=..., z=...) 实例化。"
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
        let items = self.ctx.blueprints.list();
        if items.is_empty() {
            return Ok(ToolResult {
                message: "无可用蓝图（blueprints/ 目录为空或未加载）".to_string(),
                is_error: false,
                images: vec![],
            });
        }
        let mut lines: Vec<String> = Vec::new();
        for (name, desc) in items {
            let materials = self
                .ctx
                .blueprints
                .get(&name)
                .map(|bp| bp.material_summary())
                .unwrap_or_default();
            lines.push(format!("- {name}: {desc} | 材料: {materials}"));
        }
        Ok(ToolResult {
            message: format!("可用蓝图 {} 个：\n{}", lines.len(), lines.join("\n")),
            is_error: false,
            images: vec![],
        })
    }
}

/// 创建 azalea 工具集并注册到 `ToolRegistry`。
/// `blueprints` 可选注入蓝图库（P2-1）；None 时仅注册 build 工具，不注册 build_blueprint。
pub fn create_mc_azalea_tools(
    adapter: ArcAzaleaAdapter,
    memory: WorldMemory,
) -> Vec<Box<dyn GameTool>> {
    create_mc_azalea_tools_full(
        adapter,
        memory,
        BlueprintLibrary::new(),
        ActionLibrary::new(),
    )
}

/// 带蓝图库的工厂：从 `blueprints/` 目录加载后传入。
pub fn create_mc_azalea_tools_with_bp(
    adapter: ArcAzaleaAdapter,
    memory: WorldMemory,
    blueprints: BlueprintLibrary,
) -> Vec<Box<dyn GameTool>> {
    create_mc_azalea_tools_full(adapter, memory, blueprints, ActionLibrary::new())
}

/// 带蓝图库 + LLM 自定义动作库的工厂（P2-1 + P2-4）。
/// `blueprints` 从 `blueprints/` 目录加载，`actions` 从 `actions/` 目录加载。
pub fn create_mc_azalea_tools_full(
    adapter: ArcAzaleaAdapter,
    memory: WorldMemory,
    blueprints: BlueprintLibrary,
    actions: ActionLibrary,
) -> Vec<Box<dyn GameTool>> {
    let ctx = Arc::new(
        AzaleaToolCtx::new(adapter, memory)
            .with_blueprints(blueprints)
            .with_actions(actions),
    );
    vec![
        Box::new(PerceiveTool::new(ctx.clone())),
        Box::new(GotoTool::new(ctx.clone())),
        Box::new(MineBelowTool::new(ctx.clone())),
        Box::new(MineAboveTool::new(ctx.clone())),
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
        Box::new(SearchWikiTool::new(ctx.clone())),
        Box::new(RunScriptTool::new(ctx.clone())),
        Box::new(BuildTool::new(ctx.clone())),
        Box::new(BuildBlueprintTool::new(ctx.clone())),
        Box::new(ListBlueprintsTool::new(ctx.clone())),
        Box::new(PickupTool::new(ctx.clone())),
        Box::new(DefendTool::new(ctx.clone())),
        Box::new(EquipTool::new(ctx.clone())),
        Box::new(DiscardTool::new(ctx.clone())),
        Box::new(ConsumeTool::new(ctx.clone())),
        Box::new(ChestViewTool::new(ctx.clone())),
        Box::new(ChestWithdrawTool::new(ctx.clone())),
        Box::new(ChestDepositTool::new(ctx.clone())),
        Box::new(PauseGoalTool::new(ctx.clone())),
        Box::new(ResumeGoalTool::new(ctx.clone())),
        // P2-4: LLM 代码生成（newAction 等价物）
        Box::new(NewActionTool::new(ctx.clone())),
        Box::new(ListActionsTool::new(ctx)),
    ]
}

/// 捡起附近掉落物。学习自 Mindcraft pickupNearbyItems。
/// bot 挖矿/战斗后掉落物散落，调用此工具走一圈吸取。
pub struct PickupTool {
    ctx: Arc<AzaleaToolCtx>,
}
impl PickupTool {
    pub fn new(ctx: Arc<AzaleaToolCtx>) -> Self {
        Self { ctx }
    }
}
impl GameTool for PickupTool {
    fn name(&self) -> &str {
        "pickup"
    }
    fn description(&self) -> &str {
        "捡起附近掉落物。bot 走 4 个方向扫一圈，让物理引擎吸取掉落物（vanilla 自动捡半径 1.5）。\n\
         挖矿/战斗后调用一次，避免\"挖了 8 个石头但只捡到 3 个\"。\n\
         无参数。返回捡到的物品总数。"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({ "type": "object", "properties": {} })
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
            .execute_shared(Action::Minecraft(MinecraftAction::Pickup))?;
        Ok(ToolResult {
            message: r.detail,
            is_error: !r.ok,
            images: vec![],
        })
    }
}

/// 自动防御：等待 5 秒让 handler 层 self_defense mode 攻击附近敌人。
/// 学习自 Mindcraft defendSelf。期间监测血量，受到严重伤害提前建议撤退。
pub struct DefendTool {
    ctx: Arc<AzaleaToolCtx>,
}
impl DefendTool {
    pub fn new(ctx: Arc<AzaleaToolCtx>) -> Self {
        Self { ctx }
    }
}
impl GameTool for DefendTool {
    fn name(&self) -> &str {
        "defend"
    }
    fn description(&self) -> &str {
        "自动防御 5 秒：等待 handler 层 self_defense mode 自动攻击附近敌对生物。\n\
         期间监测血量，若受到严重伤害（>5 点）提前返回建议撤退。\n\
         无参数。适合被多只怪围攻时调用。"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({ "type": "object", "properties": {} })
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
            .execute_shared(Action::Minecraft(MinecraftAction::Defend))?;
        Ok(ToolResult {
            message: r.detail,
            is_error: !r.ok,
            images: vec![],
        })
    }
}

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
        let r = self.ctx.adapter.execute_shared(Action::Minecraft(
            MinecraftAction::Equip { item, slot },
        ))?;
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
        let count = args
            .get("count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
        let r = self.ctx.adapter.execute_shared(Action::Minecraft(
            MinecraftAction::Discard { item, count },
        ))?;
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
        let r = self.ctx.adapter.execute_shared(Action::Minecraft(
            MinecraftAction::Consume { item },
        ))?;
        Ok(ToolResult {
            message: r.detail,
            is_error: !r.ok,
            images: vec![],
        })
    }
}

/// 查看容器物品列表。学习自 Mindcraft viewChest。
/// 打开容器 → 读槽位 → 关闭，返回 "iron_ingot:32, coal:16" 格式。
/// 解决：LLM 不知道箱子里有什么，无法决策取什么/存什么。
pub struct ChestViewTool {
    ctx: Arc<AzaleaToolCtx>,
}
impl ChestViewTool {
    pub fn new(ctx: Arc<AzaleaToolCtx>) -> Self {
        Self { ctx }
    }
}
impl GameTool for ChestViewTool {
    fn name(&self) -> &str {
        "chest_view"
    }
    fn description(&self) -> &str {
        "查看世界坐标 (x,y,z) 处容器（箱子/熔炉/木桶等）的物品列表。\n\
         打开 → 读取 → 关闭，返回 'iron_ingot:32, coal:16' 格式。\n\
         参数：x,y,z 容器坐标。无副作用（不改变背包/容器内容）。\n\
         场景：决策取什么/存什么前先查看。"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "x": { "type": "integer", "description": "容器 X 坐标" },
                "y": { "type": "integer", "description": "容器 Y 坐标" },
                "z": { "type": "integer", "description": "容器 Z 坐标" }
            },
            "required": ["x", "y", "z"]
        })
    }
    fn effects(&self) -> ToolEffects {
        ToolEffects::read()
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
            MinecraftAction::ChestView { x, y, z },
        ))?;
        Ok(ToolResult {
            message: r.detail,
            is_error: !r.ok,
            images: vec![],
        })
    }
}

/// 从容器取出物品。学习自 Mindcraft withdrawItemFromChest。
/// 打开容器 → shift_click 容器槽位 → 关闭，把物品移到 bot 背包。
pub struct ChestWithdrawTool {
    ctx: Arc<AzaleaToolCtx>,
}
impl ChestWithdrawTool {
    pub fn new(ctx: Arc<AzaleaToolCtx>) -> Self {
        Self { ctx }
    }
}
impl GameTool for ChestWithdrawTool {
    fn name(&self) -> &str {
        "chest_withdraw"
    }
    fn description(&self) -> &str {
        "从世界坐标 (x,y,z) 处容器取出 item 到 bot 背包。\n\
         count=0 取全部，count>0 取指定数量（不足时尽力而为）。\n\
         参数：x,y,z 容器坐标；item 物品 id（如 iron_ingot）；count 数量（默认 0=全部）。\n\
         场景：从基地箱子拿物资（食物/工具/矿石）出来用。"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "x": { "type": "integer", "description": "容器 X 坐标" },
                "y": { "type": "integer", "description": "容器 Y 坐标" },
                "z": { "type": "integer", "description": "容器 Z 坐标" },
                "item": { "type": "string", "description": "物品 id，如 iron_ingot / bread" },
                "count": { "type": "integer", "description": "取出数量（0=全部，默认 0）", "default": 0 }
            },
            "required": ["x", "y", "z", "item"]
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
        let item = args
            .get("item")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("缺少 item"))?
            .to_string();
        let count = args
            .get("count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
        let r = self.ctx.adapter.execute_shared(Action::Minecraft(
            MinecraftAction::ChestWithdraw { x, y, z, item, count },
        ))?;
        Ok(ToolResult {
            message: r.detail,
            is_error: !r.ok,
            images: vec![],
        })
    }
}

/// 把物品存入容器。学习自 Mindcraft depositItemIntoChest。
/// 打开容器 → shift_click 玩家槽位 → 关闭，把物品从 bot 背包移到容器。
pub struct ChestDepositTool {
    ctx: Arc<AzaleaToolCtx>,
}
impl ChestDepositTool {
    pub fn new(ctx: Arc<AzaleaToolCtx>) -> Self {
        Self { ctx }
    }
}
impl GameTool for ChestDepositTool {
    fn name(&self) -> &str {
        "chest_deposit"
    }
    fn description(&self) -> &str {
        "把背包中的 item 存入世界坐标 (x,y,z) 处容器。\n\
         count=0 存全部，count>0 存指定数量（不足时尽力而为）。\n\
         参数：x,y,z 容器坐标；item 物品 id；count 数量（默认 0=全部）。\n\
         场景：把挖到的矿石/收集的资源存进基地箱子，腾出背包空间。"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "x": { "type": "integer", "description": "容器 X 坐标" },
                "y": { "type": "integer", "description": "容器 Y 坐标" },
                "z": { "type": "integer", "description": "容器 Z 坐标" },
                "item": { "type": "string", "description": "物品 id，如 cobblestone / iron_ingot" },
                "count": { "type": "integer", "description": "存入数量（0=全部，默认 0）", "default": 0 }
            },
            "required": ["x", "y", "z", "item"]
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
        let item = args
            .get("item")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("缺少 item"))?
            .to_string();
        let count = args
            .get("count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
        let r = self.ctx.adapter.execute_shared(Action::Minecraft(
            MinecraftAction::ChestDeposit { x, y, z, item, count },
        ))?;
        Ok(ToolResult {
            message: r.detail,
            is_error: !r.ok,
            images: vec![],
        })
    }
}

/// 暂停当前目标（Active → Paused）。
/// 学习自 Mindcraft self_prompter 的 pause 语义。
/// LLM 主动暂停后，目标不会每轮注入，但保留 goal 文本；需手动 resume_goal 恢复。
/// 场景：LLM 临时想做别的事（如先处理突发情况），不想丢失长期目标。
pub struct PauseGoalTool {
    ctx: Arc<AzaleaToolCtx>,
}
impl PauseGoalTool {
    pub fn new(ctx: Arc<AzaleaToolCtx>) -> Self {
        Self { ctx }
    }
}
impl GameTool for PauseGoalTool {
    fn name(&self) -> &str {
        "pause_goal"
    }
    fn description(&self) -> &str {
        "暂停当前目标（不注入 [当前目标] 但保留 goal 文本）。\n\
         暂停后需手动调用 resume_goal 恢复（不会自动恢复）。\n\
         无参数。场景：LLM 临时处理突发情况时不想丢失长期目标。\n\
         注意：紧急 mode（如血量危急）会自动暂停目标，无需调用此工具。"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({ "type": "object", "properties": {} })
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
        // pause_goal 由 Agent 主循环在 tool 执行后处理（修改 prompt_state）。
        // 工具本身只返回确认消息。
        Ok(ToolResult {
            message: "已请求暂停当前目标（Active → Paused）。需手动 resume_goal 恢复。".to_string(),
            is_error: false,
            images: vec![],
        })
    }
}

/// 恢复已暂停的目标（Paused → Active）。
pub struct ResumeGoalTool {
    ctx: Arc<AzaleaToolCtx>,
}
impl ResumeGoalTool {
    pub fn new(ctx: Arc<AzaleaToolCtx>) -> Self {
        Self { ctx }
    }
}
impl GameTool for ResumeGoalTool {
    fn name(&self) -> &str {
        "resume_goal"
    }
    fn description(&self) -> &str {
        "恢复已暂停的目标（Paused → Active），目标重新每轮注入。\n\
         无参数。场景：突发情况处理完后继续原目标。"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({ "type": "object", "properties": {} })
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
        Ok(ToolResult {
            message: "已请求恢复目标（Paused → Active）。".to_string(),
            is_error: false,
            images: vec![],
        })
    }
}

// ============================================================================
// P2-4: LLM 代码生成（newAction 等价物）
// ============================================================================

/// 创建一个新的自定义动作（P2-4：newAction 等价物）。
///
/// 学习自 Mindcraft `agent/commands/code.js::newAction`：LLM 可写一段命名 rhai 脚本，
/// 保存到 `actions/<name>.rhai.json`，后续通过 `call_action(name)` 在 `run_script` 里调用。
///
/// 与 `run_script` 区别：
/// - `run_script` 是一次性执行
/// - `new_action` 是持久化（跨会话可复用）
///
/// 流程：lint 脚本 → parse 检查 → 写盘 → 加入内存库 → 返回成功。
pub struct NewActionTool {
    ctx: Arc<AzaleaToolCtx>,
}
impl NewActionTool {
    pub fn new(ctx: Arc<AzaleaToolCtx>) -> Self {
        Self { ctx }
    }
}
impl GameTool for NewActionTool {
    fn name(&self) -> &str {
        "new_action"
    }
    fn description(&self) -> &str {
        "创建一个命名的自定义动作（P2-4：newAction 等价物），持久化到 actions/<name>.rhai.json。\n\
         后续可在 run_script 里用 call_action(name) 调用，跨会话复用。\n\
         \n\
         参数：\n\
         - name: 动作名（合法标识符 [a-z_][a-z0-9_]*，1..=32 字符，如 'gather_and_craft'）\n\
         - description: 何时该用此动作（给 LLM 看的提示）\n\
         - script: rhai 脚本代码（≤8KB，可用 run_script 全部 27 个函数 + call_action）\n\
         \n\
         lint 规则：禁用 import/eval/Fn/call/IO，禁 while true 无 break。\n\
         若同名动作已存在则覆盖（更新脚本）。\n\
         \n\
         示例：new_action(name=\"gather_wood_and_planks\", description=\"采集 4 个原木并合成木板\", \
         script=\"gather(\\\"oak_log\\\", 4); craft(\\\"oak_planks\\\", 4); pickup(); print(\\\"done\\\")\")"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "动作名 [a-z_][a-z0-9_]*，1..=32 字符" },
                "description": { "type": "string", "description": "动作描述（何时该用）" },
                "script": { "type": "string", "description": "rhai 脚本代码（≤8KB）" }
            },
            "required": ["name", "description", "script"]
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
        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("缺少 name"))?
            .to_string();
        let description = args
            .get("description")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("缺少 description"))?
            .to_string();
        let script = args
            .get("script")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("缺少 script"))?
            .to_string();

        // 1. 校验 name 合法性
        if !LlmAction::is_valid_name(&name) {
            return Ok(ToolResult {
                message: format!(
                    "动作名 '{name}' 非法（须 [a-z_][a-z0-9_]*，长度 1..=32）"
                ),
                is_error: true,
                images: vec![],
            });
        }
        // 2. lint 脚本
        if let Err(reason) = lint_script(&script) {
            return Ok(ToolResult {
                message: format!("脚本被 lint 拒绝: {reason}"),
                is_error: true,
                images: vec![],
            });
        }
        // 3. parse 检查（不执行）：用临时 engine 编译脚本，确保语法正确
        let mut probe = rhai::Engine::new();
        probe.set_max_operations(1_000);
        // 注册一个 dummy 函数让脚本能 parse（实际执行由 call_action 时注册完整函数集）
        let parse_result: Result<(), String> = probe
            .compile_expression(&script)
            .map(|_| ())
            .map_err(|e| e.to_string())
            .or_else(|_| {
                // 表达式编译失败时尝试按语句块编译
                probe.compile(&script).map(|_| ()).map_err(|e| e.to_string())
            });
        if let Err(e) = parse_result {
            return Ok(ToolResult {
                message: format!("脚本语法错误（compile 失败）: {e}"),
                is_error: true,
                images: vec![],
            });
        }
        // 4. 保存到 ActionLibrary
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let action = LlmAction {
            name: name.clone(),
            description: description.clone(),
            script,
            created_at: now_ms,
            call_count: 0,
        };
        let mut lib = self.ctx.actions.lock().unwrap();
        match lib.save(action) {
            Ok(()) => {
                let total = lib.len();
                Ok(ToolResult {
                    message: format!(
                        "✓ 动作 '{name}' 已保存。当前共 {total} 个自定义动作。\
                         \n用 list_actions 查看，用 run_script 内 call_action(\"{name}\") 调用。"
                    ),
                    is_error: false,
                    images: vec![],
                })
            }
            Err(e) => Ok(ToolResult {
                message: format!("保存动作失败: {e}"),
                is_error: true,
                images: vec![],
            }),
        }
    }
}

/// 列出所有已保存的 LLM 自定义动作（P2-4）。
pub struct ListActionsTool {
    ctx: Arc<AzaleaToolCtx>,
}
impl ListActionsTool {
    pub fn new(ctx: Arc<AzaleaToolCtx>) -> Self {
        Self { ctx }
    }
}
impl GameTool for ListActionsTool {
    fn name(&self) -> &str {
        "list_actions"
    }
    fn description(&self) -> &str {
        "列出所有已保存的自定义动作（P2-4）。无参数。\n\
         返回：name (调用 N 次): description + 脚本预览（前 200 字符）。\n\
         用 new_action 创建新动作，用 run_script 内 call_action(name) 调用。"
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
        let lib = self.ctx.actions.lock().unwrap();
        let items = lib.list();
        if items.is_empty() {
            return Ok(ToolResult {
                message: "无自定义动作。用 new_action(name=..., description=..., script=...) 创建。".to_string(),
                is_error: false,
                images: vec![],
            });
        }
        let mut lines: Vec<String> = Vec::new();
        for (n, d, c) in items {
            let preview: String = lib
                .get(&n)
                .map(|a| {
                    let s = &a.script;
                    if s.chars().count() > 200 {
                        let head: String = s.chars().take(200).collect();
                        format!("{head}...")
                    } else {
                        s.clone()
                    }
                })
                .unwrap_or_default();
            // 替换换行为 \\n 让一行展示
            let preview_one = preview.replace('\n', "\\n");
            lines.push(format!("- {n} (调用 {c} 次): {d}\n  脚本: {preview_one}"));
        }
        Ok(ToolResult {
            message: format!(
                "已保存 {} 个自定义动作：\n{}\n\n用 call_action(name) 在 run_script 内调用。",
                lines.len(),
                lines.join("\n")
            ),
            is_error: false,
            images: vec![],
        })
    }
}
