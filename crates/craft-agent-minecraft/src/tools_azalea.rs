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
use std::sync::atomic::AtomicBool;

/// 工具上下文：持有共享的 azalea adapter、世界记忆库、蓝图库与 LLM 自定义动作库。
pub struct AzaleaToolCtx {
    pub adapter: ArcAzaleaAdapter,
    pub memory: WorldMemory,
    /// 蓝图库（P2-1）：供 build_blueprint / list_blueprints 工具使用。
    pub blueprints: BlueprintLibrary,
    /// LLM 自定义动作库（P2-4）：供 new_action / list_actions / call_action 使用。
    /// 内部可变（save/bump_call_count），用 Mutex 保护。
    pub actions: Arc<Mutex<ActionLibrary>>,
    /// 任务完成停止标志：TaskCompleteTool 验证通过后置 true。
    pub should_stop: Arc<AtomicBool>,
}

impl AzaleaToolCtx {
    pub fn new(adapter: ArcAzaleaAdapter, memory: WorldMemory) -> Self {
        Self {
            adapter,
            memory,
            blueprints: BlueprintLibrary::new(),
            actions: Arc::new(Mutex::new(ActionLibrary::new())),
            should_stop: Arc::new(AtomicBool::new(false)),
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

// ============================================================================
// P3.2: tools_azalea 域模块拆分（tools_azalea.rs → tools_azalea/ 目录）。
// 新增工具必须同时同步：本文件 pub use 列表 + 对应域模块 + ALL_TOOL_NAMES。
// ============================================================================
mod tools_container;
mod tools_crafting;
mod tools_farming;
mod tools_interact;
mod tools_inventory;
mod tools_meta;
mod tools_mining;
mod tools_movement;
mod tools_perceive;
mod tools_placement;
mod tools_social;

pub use tools_container::{ChestDepositTool, ChestViewTool, ChestWithdrawTool, OpenContainerTool};
pub use tools_crafting::{AutoCraftTool, Craft3x3Tool, CraftTool, EnchantTool, SmeltTool};
pub use tools_farming::{GatherTool, HarvestTool, TillAndSowTool};
pub use tools_interact::{
    AttackTool, DefendTool, InteractBlockTool, InteractEntityTool, SleepTool,
};
pub use tools_inventory::{ConsumeTool, DiscardTool, EquipTool};
pub use tools_meta::{
    ChatTool, ListActionsTool, NewActionTool, PauseGoalTool, ResumeGoalTool, RunPlanTool,
    RunScriptTool, SetGoalTool, TaskCompleteTool, TaskRetryTool,
};
#[cfg(test)]
pub(crate) use tools_meta::{lint_action_script, lint_script};
pub use tools_mining::{MakeObsidianTool, MineTool};
pub use tools_movement::{
    FollowTool, GotoPlayerTool, GotoTool, MineAboveTool, MineBelowTool, MoveAwayTool, PickupTool,
    SetModeTool, StopFollowTool,
};
pub use tools_perceive::{MemoryTool, PerceiveTool, SearchBlockTool, SearchWikiTool};
pub use tools_placement::{BuildBlueprintTool, BuildTool, ListBlueprintsTool, PlaceTool};
pub use tools_social::{GiveTool, TradeTool};

/// ── P1.2 工具↔动作映射表（集中登记）──────────────────────────────────────
/// 每个 LLM 工具名 → 它构造/产生的主要 MinecraftAction 变体名。
/// 元工具（感知/记忆/规划/脚本/蓝图/任务声明/目标管理）不产生 MinecraftAction → None。
/// **新增工具必须同时**：1) 加入 `create_mc_azalea_tools_full` 的 vec；
/// 2) 登记到 `ALL_TOOL_NAMES`；3) 在 `action_for` 或 `META_TOOL_NAMES` 登记。
/// 防线：`regression_every_registered_tool_maps_to_action`（漏登记即红）。
pub fn action_for(name: &str) -> Option<&'static str> {
    match name {
        "goto" => Some("Goto"),
        "mine" => Some("MineBlock"),
        "mine_below" => Some("MineBelow"),
        "mine_above" => Some("MineAbove"),
        "interact_block" => Some("InteractBlock"),
        "till_and_sow" => Some("TillAndSow"),
        "sleep" => Some("Sleep"),
        "harvest" => Some("Harvest"),
        "chat" => Some("Chat"),
        "attack" => Some("Attack"),
        "craft" => Some("Craft"),
        "craft_3x3" => Some("Craft3x3"),
        "smelt" => Some("Smelt"),
        "gather" => Some("Gather"),
        "make_obsidian" => Some("MakeObsidian"),
        "place" => Some("Place"),
        "open" => Some("OpenContainer"),
        "auto_craft" => Some("AutoCraft"),
        "enchant" => Some("Enchant"),
        "trade" => Some("Trade"),
        "interact_entity" => Some("InteractEntity"),
        "pickup" => Some("Pickup"),
        "defend" => Some("Defend"),
        "equip" => Some("Equip"),
        "discard" => Some("Discard"),
        "consume" => Some("Consume"),
        "chest_view" => Some("ChestView"),
        "chest_withdraw" => Some("ChestWithdraw"),
        "chest_deposit" => Some("ChestDeposit"),
        "follow" => Some("Follow"),
        "goto_player" => Some("GotoPlayer"),
        "stop_follow" => Some("StopFollow"),
        "give" => Some("Give"),
        "search_for_block" => Some("SearchBlock"),
        "move_away" => Some("MoveAway"),
        "set_mode" => Some("SetMode"),
        _ => None,
    }
}

/// 不产生 MinecraftAction 的元工具（感知/记忆/规划/脚本/蓝图/任务声明/目标管理）。
pub const META_TOOL_NAMES: &[&str] = &[
    "perceive",
    "memory",
    "set_goal",
    "run_plan",
    "search_wiki",
    "run_script",
    "build",
    "build_blueprint",
    "list_blueprints",
    "task_complete",
    "task_retry",
    "pause_goal",
    "resume_goal",
    "new_action",
    "list_actions",
];

/// 全部已注册 LLM 工具名（与 `create_mc_azalea_tools_full` 的 vec 一一对应，顺序一致）。
pub const ALL_TOOL_NAMES: &[&str] = &[
    "perceive",
    "goto",
    "mine_below",
    "mine_above",
    "mine",
    "interact_block",
    "till_and_sow",
    "sleep",
    "harvest",
    "attack",
    "craft",
    "craft_3x3",
    "smelt",
    "gather",
    "make_obsidian",
    "place",
    "open",
    "auto_craft",
    "enchant",
    "trade",
    "interact_entity",
    "chat",
    "memory",
    "set_goal",
    "run_plan",
    "search_wiki",
    "run_script",
    "build",
    "build_blueprint",
    "list_blueprints",
    "pickup",
    "defend",
    "equip",
    "discard",
    "follow",
    "stop_follow",
    "give",
    "consume",
    "chest_view",
    "chest_withdraw",
    "chest_deposit",
    "pause_goal",
    "resume_goal",
    "new_action",
    "list_actions",
    "task_complete",
    "task_retry",
];

/// MinecraftAction 全部变体名（core/types.rs，新增变体时同步更新；映射表合法性校验用）。
pub const MINECRAFT_ACTION_VARIANTS: &[&str] = &[
    "Goto",
    "MineBlock",
    "MineBelow",
    "MineAbove",
    "InteractBlock",
    "TillAndSow",
    "Sleep",
    "Harvest",
    "Chat",
    "Attack",
    "Craft",
    "Craft3x3",
    "Smelt",
    "Gather",
    "MakeObsidian",
    "Place",
    "OpenContainer",
    "AutoCraft",
    "Enchant",
    "Trade",
    "InteractEntity",
    "Pickup",
    "Defend",
    "Equip",
    "Discard",
    "Consume",
    "ChestView",
    "ChestWithdraw",
    "ChestDeposit",
    "Follow",
    "GotoPlayer",
    "StopFollow",
    "Give",
    "SearchBlock",
    "MoveAway",
    "SetMode",
];

/// 将 plan 步骤中的 action 名和参数解析为 MinecraftAction。
fn parse_step(action: &str, step: &serde_json::Value) -> anyhow::Result<MinecraftAction> {
    let i64 = |key: &str| step.get(key).and_then(|v| v.as_i64()).map(|v| v as i32);
    let str = |key: &str| {
        step.get(key)
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    };
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
        "till_and_sow" | "tillandsow" => Ok(MinecraftAction::TillAndSow {
            x: i64("x").ok_or_else(|| anyhow::anyhow!("till_and_sow 缺少 x"))?,
            y: i64("y").ok_or_else(|| anyhow::anyhow!("till_and_sow 缺少 y"))?,
            z: i64("z").ok_or_else(|| anyhow::anyhow!("till_and_sow 缺少 z"))?,
            seed: str("seed").ok_or_else(|| anyhow::anyhow!("till_and_sow 缺少 seed"))?,
        }),
        "sleep" => Ok(MinecraftAction::Sleep),
        "harvest" => Ok(MinecraftAction::Harvest),
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
                (Some(x), Some(y), Some(z)) => Some((x, y, z)),
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
                (Some(x), Some(y), Some(z)) => Some((x, y, z)),
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
        "make_obsidian" => Ok(MinecraftAction::MakeObsidian {
            count: u32("count").unwrap_or(1),
        }),
        "equip" => Ok(MinecraftAction::Equip {
            item: str("item").ok_or_else(|| anyhow::anyhow!("equip 缺少 item"))?,
            slot: str("slot").unwrap_or_else(|| "hand".to_string()),
        }),
        "discard" => Ok(MinecraftAction::Discard {
            item: str("item").ok_or_else(|| anyhow::anyhow!("discard 缺少 item"))?,
            count: u32("count").unwrap_or(0),
        }),
        "consume" => Ok(MinecraftAction::Consume {
            item: str("item").ok_or_else(|| anyhow::anyhow!("consume 缺少 item"))?,
        }),
        "chest_view" => Ok(MinecraftAction::ChestView {
            x: i64("x").ok_or_else(|| anyhow::anyhow!("chest_view 缺少 x"))?,
            y: i64("y").ok_or_else(|| anyhow::anyhow!("chest_view 缺少 y"))?,
            z: i64("z").ok_or_else(|| anyhow::anyhow!("chest_view 缺少 z"))?,
        }),
        "chest_withdraw" => Ok(MinecraftAction::ChestWithdraw {
            x: i64("x").ok_or_else(|| anyhow::anyhow!("chest_withdraw 缺少 x"))?,
            y: i64("y").ok_or_else(|| anyhow::anyhow!("chest_withdraw 缺少 y"))?,
            z: i64("z").ok_or_else(|| anyhow::anyhow!("chest_withdraw 缺少 z"))?,
            item: str("item").ok_or_else(|| anyhow::anyhow!("chest_withdraw 缺少 item"))?,
            count: u32("count").unwrap_or(0),
        }),
        "chest_deposit" => Ok(MinecraftAction::ChestDeposit {
            x: i64("x").ok_or_else(|| anyhow::anyhow!("chest_deposit 缺少 x"))?,
            y: i64("y").ok_or_else(|| anyhow::anyhow!("chest_deposit 缺少 y"))?,
            z: i64("z").ok_or_else(|| anyhow::anyhow!("chest_deposit 缺少 z"))?,
            item: str("item").ok_or_else(|| anyhow::anyhow!("chest_deposit 缺少 item"))?,
            count: u32("count").unwrap_or(0),
        }),
        "follow" => Ok(MinecraftAction::Follow {
            target: str("target"),
        }),
        "goto_player" => Ok(MinecraftAction::GotoPlayer {
            target: str("target"),
        }),
        "stop_follow" => Ok(MinecraftAction::StopFollow),
        "give" => Ok(MinecraftAction::Give {
            item: str("item").ok_or_else(|| anyhow::anyhow!("give 缺少 item"))?,
            count: u32("count").unwrap_or(0),
            target: str("target"),
        }),
        "search_block" => Ok(MinecraftAction::SearchBlock {
            item: str("item").ok_or_else(|| anyhow::anyhow!("search_block 缺少 item"))?,
            radius: u32("radius").unwrap_or(32),
        }),
        "move_away" => Ok(MinecraftAction::MoveAway {
            target: str("target"),
            distance: u32("distance").unwrap_or(8),
        }),
        "set_mode" => Ok(MinecraftAction::SetMode {
            mode: str("mode").ok_or_else(|| anyhow::anyhow!("set_mode 缺少 mode"))?,
            enabled: step
                .get("enabled")
                .and_then(|v| v.as_bool())
                .unwrap_or(true),
        }),
        "set_goal" => Ok(MinecraftAction::Chat {
            content: format!("[set_goal] {}", str("goal").unwrap_or_default()),
        }),
        // perceive 在 plan 里不执行实际动作，只返回提示（plan 是动作序列，perceive 由
        // agent 主循环的 auto_perceive 处理）。
        "perceive" | "look" | "look_at" => Err(anyhow::anyhow!(
            "perceive 不支持在 run_plan 里调用（agent 主循环每轮自动注入 perceive，plan 里只放动作）"
        )),
        other => Err(anyhow::anyhow!(
            "不支持的 action: {other}（支持: goto/mine/mine_below/mine_above/interact/interact_entity/attack/chat/craft/craft_3x3/smelt/gather/place/open/auto_craft/enchant/trade/pickup/defend/make_obsidian/equip/discard/consume/chest_view/chest_withdraw/chest_deposit/follow/goto_player/stop_follow/give/search_block/move_away/set_mode）"
        )),
    }
}

fn _exec_action(adapter: &Arc<Mutex<MinecraftAzaleaAdapter>>, mc: MinecraftAction) -> String {
    match adapter.lock().unwrap().exec_mc_sync(mc, 120_000) {
        Ok(r) => r.detail,
        Err(e) => format!("错误: {e}"),
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
        Box::new(TillAndSowTool::new(ctx.clone())),
        Box::new(SleepTool::new(ctx.clone())),
        Box::new(HarvestTool::new(ctx.clone())),
        Box::new(AttackTool::new(ctx.clone())),
        Box::new(CraftTool::new(ctx.clone())),
        Box::new(Craft3x3Tool::new(ctx.clone())),
        Box::new(SmeltTool::new(ctx.clone())),
        Box::new(GatherTool::new(ctx.clone())),
        Box::new(MakeObsidianTool::new(ctx.clone())),
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
        Box::new(FollowTool::new(ctx.clone())),
        Box::new(GotoPlayerTool::new(ctx.clone())),
        Box::new(StopFollowTool::new(ctx.clone())),
        Box::new(GiveTool::new(ctx.clone())),
        Box::new(SearchBlockTool::new(ctx.clone())),
        Box::new(MoveAwayTool::new(ctx.clone())),
        Box::new(SetModeTool::new(ctx.clone())),
        Box::new(ConsumeTool::new(ctx.clone())),
        Box::new(ChestViewTool::new(ctx.clone())),
        Box::new(ChestWithdrawTool::new(ctx.clone())),
        Box::new(ChestDepositTool::new(ctx.clone())),
        Box::new(PauseGoalTool::new(ctx.clone())),
        Box::new(ResumeGoalTool::new(ctx.clone())),
        // P2-4: LLM 代码生成（newAction 等价物）
        Box::new(NewActionTool::new(ctx.clone())),
        Box::new(ListActionsTool::new(ctx.clone())),
        // 阶段完成工具：记录里程碑，但不终止长期生存目标。
        Box::new(TaskCompleteTool::new(ctx)),
        Box::new(TaskRetryTool),
    ]
}

#[cfg(test)]
mod tool_mapping_tests {
    use super::*;

    /// 防线：每个注册工具必须有动作映射（action_for）或显式登记为元工具（META）。
    /// 新增工具漏登记 → 红。反向校验：映射表无僵尸条目。
    #[test]
    fn regression_every_registered_tool_maps_to_action() {
        // 1) 每个注册工具名：有动作映射 或 在元工具清单
        for name in ALL_TOOL_NAMES {
            let mapped = action_for(name).is_some();
            let meta = META_TOOL_NAMES.contains(name);
            assert!(
                mapped || meta,
                "工具 `{name}` 未登记：必须加入 action_for 或 META_TOOL_NAMES"
            );
            assert!(
                !(mapped && meta),
                "工具 `{name}` 同时出现在 action_for 与 META_TOOL_NAMES（应二选一）"
            );
        }
        // 2) 反向：META_TOOL_NAMES 里没有未注册的名字（防僵尸条目）
        for name in META_TOOL_NAMES {
            assert!(
                ALL_TOOL_NAMES.contains(name),
                "META_TOOL_NAMES 含未注册工具 `{name}`"
            );
        }
        // 3) 动作映射值必须是 MinecraftAction 真实变体名
        for name in ALL_TOOL_NAMES {
            if let Some(variant) = action_for(name) {
                assert!(
                    MINECRAFT_ACTION_VARIANTS.contains(&variant),
                    "工具 `{name}` 映射到未知变体 `{variant}`（MINECRAFT_ACTION_VARIANTS 未登记或变体不存在）"
                );
            }
        }
        // 4) 无重复注册名
        let mut sorted: Vec<&str> = ALL_TOOL_NAMES.to_vec();
        sorted.sort();
        sorted.dedup();
        assert_eq!(
            sorted.len(),
            ALL_TOOL_NAMES.len(),
            "ALL_TOOL_NAMES 存在重复"
        );
    }

    /// run_plan 支持的 action 名与 action_for 动作工具集一致（parse_step 漏分支 → 红）。
    /// 这里验证 parse_step 的每个可解析 action 都有对应工具名登记。
    #[test]
    fn regression_run_plan_actions_covered_by_tool_names() {
        let step = serde_json::json!({
            "x": 0, "y": 0, "z": 0,
            "item": "stick", "count": 1, "output": "iron_ingot", "fuel": "coal",
            "seed": "wheat_seeds", "content": "hi", "target": "nearest",
            "goal": "g", "offer": 0, "kind": "villager", "slot": "hand",
            "level": 1, "table_x": 0, "table_y": 0, "table_z": 0,
        });
        for action in [
            "goto",
            "mine",
            "mine_block",
            "mine_below",
            "mine_above",
            "interact",
            "interact_block",
            "till_and_sow",
            "tillandsow",
            "sleep",
            "harvest",
            "chat",
            "attack",
            "craft",
            "craft_2x2",
            "craft_3x3",
            "smelt",
            "gather",
            "collect",
            "place",
            "open",
            "open_container",
            "auto_craft",
            "enchant",
            "trade",
            "interact_entity",
            "pickup",
            "defend",
            "make_obsidian",
            "equip",
            "discard",
            "consume",
            "chest_view",
            "chest_withdraw",
            "chest_deposit",
            "follow",
            "stop_follow",
            "give",
        ] {
            assert!(
                parse_step(action, &step).is_ok(),
                "run_plan action `{action}` 解析失败（parse_step 分支与工具登记不一致？）"
            );
        }
    }
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
        let r: rhai::Dynamic = engine
            .eval("walk_to(10, 64, 20)")
            .expect("walk_to 不应被当作保留字");
        assert_eq!(r.as_int().unwrap(), 10);

        // move_to / step_to 同样不应是保留字
        engine.register_fn("move_to", |x: i64, _y: i64, _z: i64| -> i64 { x + 1 });
        let r: rhai::Dynamic = engine
            .eval("move_to(5, 64, 5)")
            .expect("move_to 不应被当作保留字");
        assert_eq!(r.as_int().unwrap(), 6);

        engine.register_fn("step_to", |x: i64, _y: i64, _z: i64| -> i64 { x + 2 });
        let r: rhai::Dynamic = engine
            .eval("step_to(0, 64, 0)")
            .expect("step_to 不应被当作保留字");
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

    /// P104 回归测试：pos_x/pos_y/pos_z 作为无参返回 f64 的函数注册后，
    /// 脚本可正常调用（LLM 曾写 pos_x() 报 Function not found）。
    /// 这里复现 build_rhai_engine 的注册签名，防止签名/命名回归。
    #[test]
    fn rhai_pos_functions_registrable() {
        let mut engine = rhai::Engine::new();
        engine.register_fn("pos_x", || -> f64 { -490.5 });
        engine.register_fn("pos_y", || -> f64 { 103.0 });
        engine.register_fn("pos_z", || -> f64 { -155.7 });
        let out: String = engine
            .eval("let x = pos_x(); let y = pos_y(); let z = pos_z(); `(${x},${y},${z})`")
            .expect("pos_x/pos_y/pos_z 应可调用");
        assert_eq!(out, "(-490.5,103.0,-155.7)");
    }
}
