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
        "合成物品（当前 azalea 版本暂不支持程序化合成，调用会返回错误提示）。\n\
         item 为配方 id（如 \"minecraft:stick\"），count 为数量。\n\
         备选：需要合成时请改用 mine + interact_block 手动搭工作台，或待 azalea 升级。"
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
        Box::new(ChatTool::new(ctx.clone())),
    ]
}
