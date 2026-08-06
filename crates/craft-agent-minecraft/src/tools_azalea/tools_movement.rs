//! 移动工具：goto / mine_below / mine_above / pickup / follow / stop_follow（P3.2 按域拆分，tools_azalea 域模块）。
use super::*;

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
    fn is_slow(&self) -> bool {
        true
    }
    fn description(&self) -> &str {
        "走到世界坐标 (x, y, z)，或按已记忆的命名锚点导航（anchor=\"home\"）。\
         二选一：给 anchor 则忽略 x/y/z；给 x/y/z 则按坐标走。\
         锚点由 memory action=anchor 设置。bot 使用内置 A* pathfinder 自动导航。"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "x": { "type": "integer", "description": "目标 X 坐标（anchor 给定时可省略）" },
                "y": { "type": "integer", "description": "目标 Y 坐标（anchor 给定时可省略）" },
                "z": { "type": "integer", "description": "目标 Z 坐标（anchor 给定时可省略）" },
                "anchor": { "type": "string", "description": "已记忆的锚点名（如 home），优先于 x/y/z" }
            }
        })
    }
    fn execute(
        &self,
        _call_id: &str,
        args: Value,
        _on_update: Option<craft_agent::core::tool::ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        // P110：anchor 优先——查 WorldMemory 锚点拿坐标，找不到报错提示。
        if let Some(anchor) = args.get("anchor").and_then(|v| v.as_str()) {
            let anchor = anchor.trim().to_string();
            let found = self.ctx.memory.find_anchor(&anchor);
            return match found.and_then(|a| a.pos) {
                Some(p) => {
                    let r = self.ctx.adapter.execute_shared(Action::Minecraft(
                        MinecraftAction::Goto {
                            x: p.x,
                            y: p.y,
                            z: p.z,
                        },
                    ))?;
                    let msg = format!("锚点 {anchor} @({},{},{})：{}", p.x, p.y, p.z, r.detail);
                    Ok(ToolResult {
                        message: msg,
                        is_error: !r.ok,
                        images: vec![],
                    })
                }
                None => Ok(ToolResult {
                    message: format!(
                        "锚点 {anchor} 不存在。先用 memory action=anchor name={anchor} 设置锚点，\
                         或 memory action=query 查看现有锚点。"
                    ),
                    is_error: true,
                    images: vec![],
                }),
            };
        }
        let x = args
            .get("x")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| anyhow::anyhow!("缺少 x（或给 anchor 参数导航到锚点）"))?
            as i32;
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
    fn is_slow(&self) -> bool {
        true
    }
    fn description(&self) -> &str {
        "挖掉 bot 脚下的方块（向下挖矿井）。无参数。bot 自动挖掘并可能拾取掉落物。\
         \n注意：垂直下挖前先规划脱困——记录入口坐标，挖完用 mine_above 回地表；\
         不要盲目连续垂直下挖（下去容易上来难）。"
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
        if r.ok
            && let Some(p) = self.ctx.memory.find_anchor("__self__").and_then(|a| a.pos)
        {
            self.ctx
                .memory
                .forget_pos(MemoryPos::new(p.x, p.y - 1, p.z));
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
    fn is_slow(&self) -> bool {
        true
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
        // 行动回写：只有实际向上移动后才从世界记忆移除头前方块。
        // P5 修复：原代码无条件 forget_pos，挖掘失败时记忆也被清空。
        if r.ok
            && r.detail.contains("MineAbove progressed")
            && let Some(p) = self.ctx.memory.find_anchor("__self__").and_then(|a| a.pos)
        {
            self.ctx
                .memory
                .forget_pos(MemoryPos::new(p.x, p.y + 1, p.z));
        }
        Ok(ToolResult {
            message: r.detail,
            is_error: !r.ok,
            images: vec![],
        })
    }
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
    fn is_slow(&self) -> bool {
        true
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

/// P68：跟随玩家。让 bot 每 tick 自动走到目标玩家身边（"跟着我"）。
/// target 为玩家名，为空表示跟随最近的其他玩家。配合 stop_follow 解除。
pub struct FollowTool {
    ctx: Arc<AzaleaToolCtx>,
}
impl FollowTool {
    pub fn new(ctx: Arc<AzaleaToolCtx>) -> Self {
        Self { ctx }
    }
}
impl GameTool for FollowTool {
    fn name(&self) -> &str {
        "follow"
    }
    fn is_slow(&self) -> bool {
        true
    }
    fn description(&self) -> &str {
        "跟随玩家（实现\"跟着我\"）。\n\
         bot 会每 tick 自动走到目标玩家身边，直到你调用 stop_follow 解除。\n\
         target 为玩家名（可选，留空则跟随最近的其他玩家）。\n\
         也可以在游戏聊天框直接打 \"follow [玩家名]\" 或 \"follow\" 触发。"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "target": { "type": "string", "description": "玩家名（可选，留空=最近的玩家）" }
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
        let target = args
            .get("target")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        let r = self
            .ctx
            .adapter
            .execute_shared(Action::Minecraft(MinecraftAction::Follow { target }))?;
        Ok(ToolResult {
            message: r.detail,
            is_error: !r.ok,
            images: vec![],
        })
    }
}

/// P111：按玩家名单次导航（对齐 Mindcraft goToPlayer）。解析玩家当前坐标后按
/// goto 导航一次，不持续跟随（持续跟随用 follow；无参时导航到最近的其他玩家）。
pub struct GotoPlayerTool {
    ctx: Arc<AzaleaToolCtx>,
}
impl GotoPlayerTool {
    pub fn new(ctx: Arc<AzaleaToolCtx>) -> Self {
        Self { ctx }
    }
}
impl GameTool for GotoPlayerTool {
    fn name(&self) -> &str {
        "goto_player"
    }
    fn is_slow(&self) -> bool {
        true
    }
    fn description(&self) -> &str {
        "按玩家名单次导航到该玩家所在位置（实现\"去玩家那边\"）。\n\
         bot 会解析玩家当前坐标并走过去（同 goto 的导航逻辑，到达后即停）。\n\
         需要持续跟着玩家走用 follow，本工具只走一次。\n\
         target 为玩家名（可选，留空则导航到最近的其他玩家）。\n\
         也可以在游戏聊天框直接打 \"gotoplayer [玩家名]\" 触发。"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "target": { "type": "string", "description": "玩家名（可选，留空=最近的其他玩家）" }
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
        let target = args
            .get("target")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        let r = self
            .ctx
            .adapter
            .execute_shared(Action::Minecraft(MinecraftAction::GotoPlayer { target }))?;
        Ok(ToolResult {
            message: r.detail,
            is_error: !r.ok,
            images: vec![],
        })
    }
}

/// P113：向远离指定实体的方向移动（对齐 Mindcraft moveAway）。
/// 定位最近的目标实体（无参=最近非玩家实体），朝反向移动 distance 格。
/// 用于预判危险主动拉开距离（creeper 靠近等），cowardice 是被动触发。
pub struct MoveAwayTool {
    ctx: Arc<AzaleaToolCtx>,
}
impl MoveAwayTool {
    pub fn new(ctx: Arc<AzaleaToolCtx>) -> Self {
        Self { ctx }
    }
}
impl GameTool for MoveAwayTool {
    fn name(&self) -> &str {
        "move_away"
    }
    fn is_slow(&self) -> bool {
        true
    }
    fn description(&self) -> &str {
        "向远离指定实体的方向移动（实现\"躲开\"）。\n\
         定位最近的目标实体（target 为实体名，可选；留空=最近的非玩家实体），\n\
         沿反向水平移动 distance 格（默认 8，最大 64）后停下。\n\
         用于主动拉开距离（如 creeper 靠近准备自爆、怪群包围）。\n\
         也可以在游戏聊天框直接打 \"moveaway [实体名] [距离]\" 触发。"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "target": { "type": "string", "description": "实体名（可选，留空=最近的非玩家实体）" },
                "distance": { "type": "integer", "description": "反向移动距离，默认 8，最大 64" }
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
        let target = args
            .get("target")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        let distance = args.get("distance").and_then(|v| v.as_u64()).unwrap_or(8) as u32;
        let r = self
            .ctx
            .adapter
            .execute_shared(Action::Minecraft(MinecraftAction::MoveAway {
                target,
                distance,
            }))?;
        Ok(ToolResult {
            message: r.detail,
            is_error: !r.ok,
            images: vec![],
        })
    }
}

/// P68：停止跟随。
pub struct StopFollowTool {
    ctx: Arc<AzaleaToolCtx>,
}
impl StopFollowTool {
    pub fn new(ctx: Arc<AzaleaToolCtx>) -> Self {
        Self { ctx }
    }
}
impl GameTool for StopFollowTool {
    fn name(&self) -> &str {
        "stop_follow"
    }
    fn description(&self) -> &str {
        "停止跟随（解除 follow 模式）。\n\
         也可以在游戏聊天框直接打 \"stopfollow\" 或 \"stop\" 触发。"
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
            .execute_shared(Action::Minecraft(MinecraftAction::StopFollow))?;
        Ok(ToolResult {
            message: r.detail,
            is_error: !r.ok,
            images: vec![],
        })
    }
}
