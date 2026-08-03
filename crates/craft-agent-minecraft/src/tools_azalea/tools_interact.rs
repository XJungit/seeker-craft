//! 交互工具：interact_block / interact_entity / attack / defend / sleep（P3.2 按域拆分，tools_azalea 域模块）。
use super::*;

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
        let r =
            self.ctx
                .adapter
                .execute_shared(Action::Minecraft(MinecraftAction::InteractBlock {
                    x,
                    y,
                    z,
                }))?;
        Ok(ToolResult {
            message: r.detail,
            is_error: !r.ok,
            images: vec![],
        })
    }
}

/// 睡觉跳夜（P85）：夜晚找附近床入睡，跳过夜晚。白天/无床/附近有怪物会失败。
pub struct SleepTool {
    ctx: Arc<AzaleaToolCtx>,
}
impl SleepTool {
    pub fn new(ctx: Arc<AzaleaToolCtx>) -> Self {
        Self { ctx }
    }
}
impl GameTool for SleepTool {
    fn name(&self) -> &str {
        "sleep"
    }
    fn description(&self) -> &str {
        "在附近床上睡觉跳过夜晚（夜晚怪物多/等天亮时用）。自动找 32m 内最近的床、走过去并上床。若白天或无床会返回错误。"
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
            .execute_shared(Action::Minecraft(MinecraftAction::Sleep))?;
        Ok(ToolResult {
            message: r.detail,
            is_error: !r.ok,
            images: vec![],
        })
    }
}

/// 攻击最近的指定种类生物（自卫/狩猎）。
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
    fn is_slow(&self) -> bool {
        true
    }
    fn description(&self) -> &str {
        "攻击近战距离内最近的指定生物。target 必填，如 cow、zombie、creeper。若目标不在近战距离，先 goto 接近或撤退。"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "target": { "type": "string", "description": "实体种类 id，如 cow、zombie、creeper" }
            },
            "required": ["target"]
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
        let r = self
            .ctx
            .adapter
            .execute_shared(Action::Minecraft(MinecraftAction::InteractEntity { kind }))?;
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
    fn is_slow(&self) -> bool {
        true
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
