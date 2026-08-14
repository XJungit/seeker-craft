//! 容器工具：open / chest_view / chest_withdraw / chest_deposit（P3.2 按域拆分，tools_azalea 域模块）。
use super::*;

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
                .execute_shared(Action::Minecraft(MinecraftAction::OpenContainer {
                    x,
                    y,
                    z,
                }))?;
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
            .execute_shared(Action::Minecraft(MinecraftAction::ChestView { x, y, z }))?;
        // P157：chest_view 成功后自动写世界记忆——记录容器位置 + 内容概要，
        // 形成"打开箱子 → 记忆位置/内容 → 后续可回访/更新"的闭环。
        // 否则 LLM 打开过箱子却忘记位置，杂物无处可存、物资无法取用。
        if r.ok {
            let summary: String = r
                .detail
                .lines()
                .find(|l| l.contains(':'))
                .map(|l| l.trim().to_string())
                .unwrap_or_else(|| r.detail.trim().to_string());
            self.ctx.memory.record(
                MemoryPos::new(x, y, z),
                MemoryKind::Container,
                Some("chest"),
                &format!("容器@({x},{y},{z}): {summary}"),
                None,
            );
        }
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
        let item = args
            .get("item")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("缺少 item"))?
            .to_string();
        let count = args.get("count").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        let r =
            self.ctx
                .adapter
                .execute_shared(Action::Minecraft(MinecraftAction::ChestWithdraw {
                    x,
                    y,
                    z,
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
        let item = args
            .get("item")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("缺少 item"))?
            .to_string();
        let count = args.get("count").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        let r =
            self.ctx
                .adapter
                .execute_shared(Action::Minecraft(MinecraftAction::ChestDeposit {
                    x,
                    y,
                    z,
                    item,
                    count,
                }))?;
        // P157：deposit 成功后更新容器记忆（内容已变化），保持记忆与实际一致。
        if r.ok {
            let summary: String = r
                .detail
                .lines()
                .find(|l| l.contains(':'))
                .map(|l| l.trim().to_string())
                .unwrap_or_else(|| r.detail.trim().to_string());
            self.ctx.memory.record(
                MemoryPos::new(x, y, z),
                MemoryKind::Container,
                Some("chest"),
                &format!("容器@({x},{y},{z}): {summary}"),
                None,
            );
        }
        Ok(ToolResult {
            message: r.detail,
            is_error: !r.ok,
            images: vec![],
        })
    }
}
