//! Minecraft mod 工具集 —— pi 规范版本（每工具含精确限制/范围/行为说明）。
//!   - 描述含限制：超时、范围、截断
//!   - 参数 schema 每字段有 type + description
//!   - effects 精确声明：read=纯读, write=修改
//!   - 结果精确反馈实际执行效果

use crate::adapter_mod::MinecraftModAdapter;
use crate::survival::{FailureTracker, FailureType, SurvivalJournal, append_survival_notes};
use crate::survival_decisions::{
    FoodDecision, ThreatResponse, decide_threat_response, food_priority,
};
use craft_agent::core::tool::{GameTool, ToolEffects, ToolResult, ToolUpdateFn};

use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::{Mutex, MutexGuard, OnceLock};

// ── 工具类别子模块 ──
pub mod agent_meta;
pub use agent_meta::*;
pub mod inventory;
pub use inventory::*;
pub mod movement;
pub use movement::*;
pub mod perceive;
pub use perceive::*;
pub mod building;
pub use building::*;
pub mod combat;
pub use combat::*;
pub mod inventory_cont;
pub use inventory_cont::*;
pub mod memory_places;
pub use memory_places::*;
pub mod survival_misc;
pub use survival_misc::*;
pub mod player;
pub use player::*;
pub mod vehicle;
pub use vehicle::*;
pub mod goal;
pub use goal::*;

// ═══════════════════════════════════════════════════════════════
// 生存层接线（批次 A）：把已写好但未接线的 survival.rs 子系统，
// 通过装饰器统一包住所有工具，在工具执行后记录结构化失败 + 注入 survival notes。
// 零侵入各工具实现，一次覆盖 60+ 工具。
// ═══════════════════════════════════════════════════════════════

/// 全局生存上下文（FailureTracker + Journal + 使能门）。
struct SurvivalContext {
    enabled: bool,
    tracker: FailureTracker,
    journal: SurvivalJournal,
}

static SURVIVAL_CTX: OnceLock<Mutex<SurvivalContext>> = OnceLock::new();

fn survival_ctx() -> &'static Mutex<SurvivalContext> {
    SURVIVAL_CTX.get_or_init(|| {
        Mutex::new(SurvivalContext {
            // 默认开启：bot 自主生存行为增强（吃/逃/战斗提示）。
            // 关闭可设环境变量 CRAFT_AGENT_SURVIVAL=0。
            enabled: std::env::var("CRAFT_AGENT_SURVIVAL")
                .map(|v| v != "0")
                .unwrap_or(true),
            tracker: FailureTracker::new(),
            journal: SurvivalJournal::new(6),
        })
    })
}

/// 从工具错误消息文本推断结构化失败类型（对齐 Numen FailureType）。
pub(crate) fn classify_failure(msg: &str) -> FailureType {
    let m = msg.to_lowercase();
    if m.contains("no path") || m.contains("stuck") || m.contains("unreachable") {
        FailureType::NoPath
    } else if m.contains("too far") || m.contains("out of reach") || m.contains("> 20") {
        FailureType::OutOfReach
    } else if m.contains("no material")
        || m.contains("missing")
        || m.contains("need") && m.contains("pickaxe")
    {
        FailureType::NoMaterial
    } else if m.contains("wrong tool") || m.contains("pickaxe") || m.contains("tool") {
        FailureType::WrongTool
    } else if m.contains("not found") || m.contains("no ") && m.contains("nearby") {
        FailureType::MinedOut
    } else if m.contains("lost") || m.contains("gone") {
        FailureType::TargetLost
    } else if m.contains("hazard") || m.contains("lava") || m.contains("void") {
        FailureType::Hazard
    } else if m.contains("no support") || m.contains("can't stand") {
        FailureType::NoSupport
    } else {
        FailureType::Unknown
    }
}

/// 工具执行后统一记录结果（成功清 tracker，失败记 tracker + journal）。
/// 返回拼好 survival notes 的最终消息。
pub(crate) fn record_tool_outcome(tool: &str, msg: &str, is_error: bool) -> String {
    let ctx = survival_ctx();
    let mut guard = match ctx.lock() {
        Ok(g) => g,
        Err(_) => return msg.to_string(),
    };
    if !guard.enabled {
        return msg.to_string();
    }
    if is_error {
        let ft = classify_failure(msg);
        let kick = guard.tracker.record(tool, ft, msg.to_string());
        // journal 记录自主行为通知（结构化失败）
        guard
            .journal
            .record(crate::survival::SurvivalEvent::ModeSwitch {
                from: "action".into(),
                to: "retry".into(),
                reason: ft.to_llm_text(msg),
            });
        let mut final_msg = if kick {
            format!(
                "{msg} [survival: repeated {} failures — need LLM replan]",
                tool
            )
        } else {
            msg.to_string()
        };
        append_survival_notes(&mut final_msg, &mut guard.journal);
        final_msg
    } else {
        guard.tracker.clear(tool);
        let mut final_msg = msg.to_string();
        append_survival_notes(&mut final_msg, &mut guard.journal);
        final_msg
    }
}

/// 装饰器：透传所有 GameTool 方法，execute 后接生存层记录。
struct SurvivalWrappedTool {
    inner: Box<dyn GameTool>,
}

impl SurvivalWrappedTool {
    fn new(inner: Box<dyn GameTool>) -> Self {
        Self { inner }
    }
}

impl GameTool for SurvivalWrappedTool {
    fn name(&self) -> &str {
        self.inner.name()
    }
    fn label(&self) -> &str {
        self.inner.label()
    }
    fn description(&self) -> &str {
        self.inner.description()
    }
    fn parameters(&self) -> Value {
        self.inner.parameters()
    }
    fn effects(&self) -> ToolEffects {
        self.inner.effects()
    }
    fn execute(
        &self,
        call_id: &str,
        args: Value,
        on_update: Option<ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        let res = self.inner.execute(call_id, args, on_update);
        match res {
            Ok(mut r) => {
                r.message = record_tool_outcome(self.inner.name(), &r.message, r.is_error);
                Ok(r)
            }
            Err(e) => {
                let msg = format!("{} error: {e}", self.inner.name());
                let final_msg = record_tool_outcome(self.inner.name(), &msg, true);
                // 把错误转成 is_error=true 的 ToolResult，避免 anyhow 层 panic 中断循环
                Ok(ToolResult {
                    message: final_msg,
                    is_error: true,
                    images: vec![],
                })
            }
        }
    }
    fn to_openai_def(&self) -> Value {
        self.inner.to_openai_def()
    }
}

/// 安全锁定 adapter 的 trait 扩展，避免 `.lock().unwrap()` 在 mutex 中毒时 panic。
trait SafeLockAdapter {
    fn lock_adapter(&self) -> anyhow::Result<MutexGuard<'_, MinecraftModAdapter>>;
}
impl SafeLockAdapter for Arc<Mutex<MinecraftModAdapter>> {
    fn lock_adapter(&self) -> anyhow::Result<MutexGuard<'_, MinecraftModAdapter>> {
        self.lock()
            .map_err(|e| anyhow::anyhow!("adapter mutex poisoned: {e}"))
    }
}

/// 生存前置检查（借鉴 Numen 生存链优先级，在耗时工具前检查）。
///
/// 返回 Some(warning) 表示有紧急生存状态，应优先处理。
/// 返回 None 表示状态正常，可继续执行工具。
pub(crate) fn survival_precheck(
    health: f32,
    hunger: f32,
    has_food: bool,
    has_weapon: bool,
    threat_present: bool,
) -> Option<String> {
    // 威胁检查（Numen MOB_DEFENSE 优先级 5）
    match decide_threat_response(threat_present, health, has_weapon) {
        ThreatResponse::Flee => {
            return Some(format!(
                "URGENT: health={:.0} threat nearby, FLEE recommended (use move_to to retreat)",
                health
            ));
        }
        ThreatResponse::Fight => {
            // 有威胁但能打——不阻断，但给警告
            // 不返回 Some，让工具继续执行
        }
        ThreatResponse::None => {}
    }

    // 进食检查（Numen FOOD_REGEN=4 / FOOD_HUNGER=3）
    let (food_dec, _) = food_priority(hunger as u32, health, has_food);
    match food_dec {
        FoodDecision::Regen => {
            return Some(format!(
                "URGENT: health={:.0} hunger={:.0}, eat food NOW to regenerate health",
                health, hunger
            ));
        }
        FoodDecision::Hunger => {
            return Some(format!(
                "WARNING: hunger={:.0} very low, eat food soon",
                hunger
            ));
        }
        FoodDecision::Dormant => {}
    }

    None
}

/// 检查背包是否有食物。
pub(crate) fn has_food_in_inventory(inventory: &[crate::bridge::InvSlot]) -> bool {
    inventory.iter().any(|i| {
        let id = i.id.to_lowercase();
        id.contains("bread")
            || id.contains("apple")
            || id.contains("cooked")
            || id.contains("steak")
            || id.contains("porkchop")
            || id.contains("mutton")
            || id.contains("chicken")
            || id.contains("carrot")
            || id.contains("potato")
            || id.contains("beetroot")
            || id.contains("melon")
            || id.contains("berry")
            || id.contains("mushroom_stew")
            || id.contains("rabbit_stew")
    })
}

/// 检查背包是否有武器。
pub(crate) fn has_weapon_in_inventory(inventory: &[crate::bridge::InvSlot]) -> bool {
    inventory.iter().any(|i| {
        let id = i.id.to_lowercase();
        id.contains("sword") || id.contains("axe")
    })
}

/// 检查附近是否有敌对实体。
pub(crate) fn has_hostile_nearby(entities: &[crate::bridge::NearbyEntity]) -> bool {
    entities.iter().any(|e| {
        let t = e.r#type.to_lowercase();
        t.contains("zombie")
            || t.contains("skeleton")
            || t.contains("creeper")
            || t.contains("spider")
            || t.contains("enderman")
            || t.contains("witch")
            || t.contains("blaze")
            || t.contains("ghast")
            || t.contains("pillager")
            || t.contains("vindicator")
            || t.contains("ravager")
            || t.contains("phantom")
            || t.contains("drowned")
            || t.contains("husk")
            || t.contains("stray")
    })
}

// ═══════════════════════════════════════════════════════════════
// Perceive — 游戏状态快照（<100ms，每轮自动注入）
// ═══════════════════════════════════════════════════════════════

// ═══════════════════════════════════════════════════════════════
// place — 放置方块（自动切快捷栏+右键）
// ═══════════════════════════════════════════════════════════════

// ═══════════════════════════════════════════════════════════════
// equip / use_item / attack — 物品/战斗
// ═══════════════════════════════════════════════════════════════

// ═══════════════════════════════════════════════════════════════
// MoveSlot — 精确槽位移动物品（支持拆分）— 已迁至 inventory_cont.rs
// ═══════════════════════════════════════════════════════════════

/// 中间点重规划用的递进比例（沿起→终向量采样）。顺序经过调优：
/// 先试半程，再试近端 / 远端，覆盖"卡在路中间某障碍"的常见情形。
pub(crate) fn midpoint_fractions() -> &'static [f64] {
    &[0.5_f64, 0.25_f64, 0.75_f64]
}

/// 是否需要触发中间点重规划：未到达且（卡住 或 仍过远）。
/// 抽成纯函数以便离线单测，避免依赖真实 MC 连接。
pub(crate) fn needs_midpoint_retry(reached: bool, stuck: bool, dist: f64) -> bool {
    !reached && (stuck || dist > 20.0)
}

/// 把 move_to 的 Ack 渲染成统一 ToolResult（批次 B：卡住自动重规划后统一格式化）。
pub(crate) fn format_move_result(
    ack: &crate::bridge::ModAck,
    x: f64,
    y: f64,
    z: f64,
    suffix: &str,
) -> ToolResult {
    let reached = ack.reached.unwrap_or(false);
    let dist = ack.final_dist.unwrap_or(0.0);
    let stuck = ack.stuck.unwrap_or(false);
    let detail = &ack.detail;
    let detail_suffix = if detail.is_empty() {
        String::new()
    } else {
        format!(" | {detail}")
    };
    let via = if suffix.is_empty() {
        String::new()
    } else {
        format!(" ({suffix})")
    };
    let msg = if reached {
        format!(
            "move_to ({:.1},{:.1},{:.1}) reached, final_dist={:.1}m{}{}",
            x, y, z, dist, detail_suffix, via
        )
    } else if stuck {
        format!(
            "move_to ({:.1},{:.1},{:.1}) STUCK at {:.1}m{} — try: 1) jump 2) dig 3) pillar_up 4) wait then retry",
            x, y, z, dist, detail_suffix
        )
    } else if dist > 20.0 {
        format!(
            "move_to ({:.1},{:.1},{:.1}) TIMEOUT at {:.1}m (still very far){}{} — try intermediate point",
            x, y, z, dist, detail_suffix, via
        )
    } else {
        format!(
            "move_to ({:.1},{:.1},{:.1}) timeout at {:.1}m (close, retry){}{}",
            x, y, z, dist, detail_suffix, via
        )
    };
    ToolResult {
        message: msg,
        is_error: !reached,
        images: vec![],
    }
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::*;
    use crate::bridge::ModAck;

    fn ack(reached: bool, stuck: bool, dist: f64) -> ModAck {
        ModAck {
            reached: Some(reached),
            stuck: Some(stuck),
            final_dist: Some(dist),
            ..Default::default()
        }
    }

    #[test]
    fn midpoint_retry_triggers_on_stuck() {
        assert!(needs_midpoint_retry(false, true, 5.0));
    }

    #[test]
    fn midpoint_retry_triggers_when_far() {
        assert!(needs_midpoint_retry(false, false, 25.0));
    }

    #[test]
    fn midpoint_retry_skipped_when_reached() {
        assert!(!needs_midpoint_retry(true, true, 0.5));
        assert!(!needs_midpoint_retry(true, false, 50.0));
    }

    #[test]
    fn midpoint_retry_skipped_when_close_and_not_stuck() {
        assert!(!needs_midpoint_retry(false, false, 10.0));
    }

    #[test]
    fn midpoint_fractions_three_steps() {
        let f = midpoint_fractions();
        assert_eq!(f.len(), 3);
        assert_eq!(*f, [0.5, 0.25, 0.75]);
    }

    #[test]
    fn format_move_reached_not_error() {
        let r = format_move_result(&ack(true, false, 0.3), 10.0, 64.0, 20.0, "");
        assert!(!r.is_error);
        assert!(r.message.contains("reached"));
    }

    #[test]
    fn format_move_stuck_message() {
        let r = format_move_result(&ack(false, true, 4.0), 10.0, 64.0, 20.0, "");
        assert!(r.is_error);
        assert!(r.message.contains("STUCK"));
    }

    #[test]
    fn format_move_far_timeout() {
        let r = format_move_result(&ack(false, false, 40.0), 10.0, 64.0, 20.0, "");
        assert!(r.is_error);
        assert!(r.message.contains("TIMEOUT"));
        assert!(r.message.contains("intermediate point"));
    }

    #[test]
    fn format_move_via_midpoint_suffix() {
        let r = format_move_result(&ack(true, false, 0.2), 10.0, 64.0, 20.0, "via midpoint");
        assert!(r.message.contains("via midpoint"));
    }
}

// ═══════════════════════════════════════════════════════════════
// 辅助函数
// ═══════════════════════════════════════════════════════════════

/// 检查合成材料是否足够，返回缺失的材料列表（如 ["stick x2", "planks x3"]）
pub(crate) fn check_missing_materials(
    st: &crate::bridge::ModState,
    item: &str,
    count: u32,
) -> Vec<String> {
    let t = item.to_lowercase();
    // 配方表：(材料, 需求量) — 与 craftable 工具一致
    let recipe: &[(&str, u32)] = if t.contains("planks") && !t.contains("stick") {
        &[("log", 1)]
    } else if t.contains("stick") {
        &[("planks", 2)]
    } else if t.contains("crafting_table") {
        &[("planks", 4)]
    } else if t.contains("wooden_pickaxe") || t.contains("wooden_axe") {
        &[("planks", 3), ("stick", 2)]
    } else if t.contains("wooden_sword") {
        &[("planks", 2), ("stick", 1)]
    } else if t.contains("wooden_shovel") {
        &[("planks", 1), ("stick", 2)]
    } else if t.contains("wooden_hoe") {
        &[("planks", 2), ("stick", 2)]
    } else if t.contains("stone_pickaxe") || t.contains("stone_axe") {
        &[("cobblestone", 3), ("stick", 2)]
    } else if t.contains("stone_sword") {
        &[("cobblestone", 2), ("stick", 1)]
    } else if t.contains("stone_shovel") {
        &[("cobblestone", 1), ("stick", 2)]
    } else if t.contains("stone_hoe") {
        &[("cobblestone", 2), ("stick", 2)]
    } else if t.contains("torch") {
        &[("stick", 1), ("coal", 1)]
    } else if t.contains("furnace") {
        &[("cobblestone", 8)]
    } else if t.contains("chest") {
        &[("planks", 8)]
    } else if t.contains("iron_helmet") {
        &[("iron_ingot", 5)]
    } else if t.contains("iron_chestplate") {
        &[("iron_ingot", 8)]
    } else if t.contains("iron_leggings") {
        &[("iron_ingot", 7)]
    } else if t.contains("iron_boots") {
        &[("iron_ingot", 4)]
    } else if t.contains("diamond_helmet") {
        &[("diamond", 5)]
    } else if t.contains("diamond_chestplate") {
        &[("diamond", 8)]
    } else if t.contains("diamond_leggings") {
        &[("diamond", 7)]
    } else if t.contains("diamond_boots") {
        &[("diamond", 4)]
    } else if t.contains("diamond_sword") {
        &[("diamond", 2), ("stick", 1)]
    } else if t.contains("shield") {
        &[("planks", 6), ("iron_ingot", 1)]
    } else {
        return vec![]; // 未知配方，不预检
    };

    let mut missing = vec![];
    for (mat, need_per) in recipe {
        let have: u32 = st
            .inventory
            .iter()
            .filter(|i| i.id.contains(mat))
            .map(|i| i.count)
            .sum();
        let need = need_per * count;
        if have < need {
            missing.push(format!("{mat} x{need} (have {have})"));
        }
    }
    missing
}

/// 简要总结物品栏（用于错误信息）
pub(crate) fn summarize_inventory(st: &crate::bridge::ModState) -> String {
    let items: Vec<String> = st
        .inventory
        .iter()
        .filter(|i| i.count > 0)
        .map(|i| format!("{}x{}", i.id.replace("minecraft:", ""), i.count))
        .collect();
    if items.is_empty() {
        "(empty)".into()
    } else {
        items.join(", ")
    }
}

pub(crate) fn find_nearest(
    adapter: &MinecraftModAdapter,
    target: &str,
) -> Option<(crate::bridge::NearbyBlock, f64)> {
    let st = adapter.reload().ok()?;
    let block = st
        .nearby_blocks
        .iter()
        .filter(|b| b.id.contains(target))
        .min_by(|a, b| {
            a.dist
                .partial_cmp(&b.dist)
                .unwrap_or(std::cmp::Ordering::Equal)
        })?;
    let dx = block.x - st.position[0];
    let dz = block.z - st.position[2];
    let target_yaw = (-dx).atan2(dz).to_degrees();
    let mut yaw_diff = target_yaw - st.yaw;
    while yaw_diff > 180.0 {
        yaw_diff -= 360.0;
    }
    while yaw_diff < -180.0 {
        yaw_diff += 360.0;
    }
    Some((block.clone(), yaw_diff))
}

/// 找最近的同类方块，跳过黑名单（Numen TargetSet.pick 模式）。
#[allow(dead_code)]
pub(crate) fn find_nearest_skipping(
    adapter: &MinecraftModAdapter,
    target: &str,
    blacklist: &std::collections::HashSet<(i32, i32, i32)>,
) -> Option<(crate::bridge::NearbyBlock, f64)> {
    let st = adapter.reload().ok()?;
    let block = st
        .nearby_blocks
        .iter()
        .filter(|b| {
            b.id.contains(target)
                && !blacklist.contains(&(
                    b.x.round() as i32,
                    b.y.round() as i32,
                    b.z.round() as i32,
                ))
        })
        .min_by(|a, b| {
            a.dist
                .partial_cmp(&b.dist)
                .unwrap_or(std::cmp::Ordering::Equal)
        })?;
    let dx = block.x - st.position[0];
    let dz = block.z - st.position[2];
    let target_yaw = (-dx).atan2(dz).to_degrees();
    let mut yaw_diff = target_yaw - st.yaw;
    while yaw_diff > 180.0 {
        yaw_diff -= 360.0;
    }
    while yaw_diff < -180.0 {
        yaw_diff += 360.0;
    }
    Some((block.clone(), yaw_diff))
}

/// collect 专用：找玩家**竖直可够到**的最近目标方块。
/// `player_y` 为玩家脚底 y；过滤掉竖直差 > max_vert 的方块（dig_at 有 5.5m 距离上限，
/// 树顶 log 在头顶太远会 too far，必须选树干上离玩家近的一段）。
pub(crate) fn find_nearest_reachable(
    adapter: &MinecraftModAdapter,
    target: &str,
    blacklist: &std::collections::HashSet<(i32, i32, i32)>,
    player_y: f64,
    max_vert: f64,
) -> Option<(crate::bridge::NearbyBlock, f64)> {
    let st = adapter.reload().ok()?;
    let block = st
        .nearby_blocks
        .iter()
        .filter(|b| {
            b.id.contains(target)
                && (b.y - player_y).abs() <= max_vert
                && !blacklist.contains(&(
                    b.x.round() as i32,
                    b.y.round() as i32,
                    b.z.round() as i32,
                ))
        })
        .min_by(|a, b| {
            a.dist
                .partial_cmp(&b.dist)
                .unwrap_or(std::cmp::Ordering::Equal)
        })?;
    let dx = block.x - st.position[0];
    let dz = block.z - st.position[2];
    let target_yaw = (-dx).atan2(dz).to_degrees();
    let mut yaw_diff = target_yaw - st.yaw;
    while yaw_diff > 180.0 {
        yaw_diff -= 360.0;
    }
    while yaw_diff < -180.0 {
        yaw_diff += 360.0;
    }
    Some((block.clone(), yaw_diff))
}

// ═══════════════════════════════════════════════════════════════
// rememberHere / goToRememberedPlace / savedPlaces — 位置记忆
// ═══════════════════════════════════════════════════════════════

// ═══════════════════════════════════════════════════════════════
// discard / smeltItem — 物品管理（已迁至 inventory_cont.rs）
// ═══════════════════════════════════════════════════════════════

// ═══════════════════════════════════════════════════════════════
// build — 蓝图建造（参考 Mindcraft buildAction + placeBlock）
// ═══════════════════════════════════════════════════════════════

// ═══════════════════════════════════════════════════════════════
// blueprints — 列出可用蓝图 + 材料需求
// ═══════════════════════════════════════════════════════════════

// ═══════════════════════════════════════════════════════════════
// combat — 战斗 AI（mod 侧自主走位：melee/kite/retreat）
// ═══════════════════════════════════════════════════════════════

// ═══════════════════════════════════════════════════════════════
// searchForEntity — 搜索实体并走过去（Mindcraft 对齐）
// ═══════════════════════════════════════════════════════════════

// ═══════════════════════════════════════════════════════════════
// goToBed — 找最近的床并睡觉（Mindcraft 对齐）（已迁至 survival_misc.rs）
// ═══════════════════════════════════════════════════════════════

// ═══════════════════════════════════════════════════════════════
// getCraftingPlan — 合成计划分析（Mindcraft 对齐）
// ═══════════════════════════════════════════════════════════════

// ═══════════════════════════════════════════════════════════════
// useOn — 对实体/方块使用工具（Mindcraft 对齐：剪羊毛/挤牛奶/点燃等）
// ═══════════════════════════════════════════════════════════════

// ═══════════════════════════════════════════════════════════════
// inspect_gui / close_gui / transfer — 容器/GUI 交互（参考 Numen）
// ═══════════════════════════════════════════════════════════════

// ═══════════════════════════════════════════════════════════════
// 玩家交互 + 控制命令（参考 mindcraft + Numen）（已迁至 player.rs / vehicle.rs）
// ═══════════════════════════════════════════════════════════════

// ═══════════════════════════════════════════════════════════════
// 对齐 Mineflayer 的补充工具：骑乘 / 钓鱼 / 睡觉 / 精确朝向（已迁至 vehicle.rs）
// ═══════════════════════════════════════════════════════════════

// ═══════════════════════════════════════════════════════════════
// Agent-local tools（借鉴 Numen AgentTools 模式 —— 无需 mod 通信，本地执行）
// ═══════════════════════════════════════════════════════════════

// ═══════════════════════════════════════════════════════════════
// 工厂
// ═══════════════════════════════════════════════════════════════

pub fn create_mc_mod_tools(
    adapter: Arc<Mutex<MinecraftModAdapter>>,
    image_max_side: Option<u32>,
    shots_dir: Option<PathBuf>,
    enable_visual_perceive: bool,
    pending_goal: Option<Arc<Mutex<Option<String>>>>,
) -> Vec<Box<dyn GameTool>> {
    // mod-bridge 模式：只暴露精确坐标工具，不暴露 enigo 时代的 look/press/mine（依赖准星，低效）
    let mut tools: Vec<Box<dyn GameTool>> = vec![Box::new(ModPerceiveTool::new(
        adapter.clone(),
        image_max_side,
        shots_dir,
    ))];
    if enable_visual_perceive {
        tools.push(Box::new(ModVisualPerceiveTool::new(adapter.clone())));
    }
    tools.push(Box::new(ModCollectTool::new(adapter.clone())));
    tools.push(Box::new(ModCraftTool::new(adapter.clone())));
    tools.push(Box::new(ModPlaceTool::new(adapter.clone())));
    tools.push(Box::new(ModEquipTool::new(adapter.clone())));
    tools.push(Box::new(ModMoveSlotTool::new(adapter.clone())));
    tools.push(Box::new(ModUseItemTool::new(adapter.clone())));
    tools.push(Box::new(ModCombatTool::new(adapter.clone())));
    tools.push(Box::new(ModLookAtTool::new(adapter.clone())));
    tools.push(Box::new(ModSearchBlockTool::new(adapter.clone())));
    tools.push(Box::new(ModMoveAwayTool::new(adapter.clone())));
    tools.push(Box::new(ModDigDownTool::new(adapter.clone())));
    tools.push(Box::new(ModPillarUpTool::new(adapter.clone())));
    tools.push(Box::new(ModConsumeTool::new(adapter.clone())));
    tools.push(Box::new(ModBuildTool::new(adapter.clone())));
    tools.push(Box::new(ModBlueprintsTool::new(adapter.clone())));
    tools.push(Box::new(ModRememberTool::new(adapter.clone())));
    tools.push(Box::new(ModGoPlaceTool::new(adapter.clone())));
    tools.push(Box::new(ModListPlacesTool::new(adapter.clone())));
    tools.push(Box::new(ModDiscardTool::new(adapter.clone())));
    tools.push(Box::new(ModSmeltTool::new(adapter.clone())));
    // Mindcraft 对齐工具
    tools.push(Box::new(ModSearchEntityTool::new(adapter.clone())));
    tools.push(Box::new(ModGoToBedTool::new(adapter.clone())));
    tools.push(Box::new(ModStayTool::new(adapter.clone())));
    tools.push(Box::new(ModGoToSurfaceTool::new(adapter.clone())));
    tools.push(Box::new(ModSetModeTool::new(adapter.clone())));
    tools.push(Box::new(ModCraftingPlanTool::new(adapter.clone())));
    tools.push(Box::new(ModBlueprintLevelTool::new(adapter.clone())));
    tools.push(Box::new(ModUseOnTool::new(adapter.clone())));
    tools.push(Box::new(ModChestTool::new(adapter.clone())));
    tools.push(Box::new(ModClearFurnaceTool::new(adapter.clone())));
    // Numen 参考：容器/GUI 交互 + 物品管理
    tools.push(Box::new(ModInspectGuiTool::new(adapter.clone())));
    tools.push(Box::new(ModTransferTool::new(adapter.clone())));
    tools.push(Box::new(ModCloseGuiTool::new(adapter.clone())));
    tools.push(Box::new(ModEquipItemTool::new(adapter.clone())));
    tools.push(Box::new(ModEatItemTool::new(adapter.clone())));
    tools.push(Box::new(ModDropItemsTool::new(adapter.clone())));
    tools.push(Box::new(ModWaitTool::new(adapter.clone())));
    // mindcraft + Numen：玩家交互 + 控制命令
    tools.push(Box::new(ModListPlayersTool::new(adapter.clone())));
    tools.push(Box::new(ModGoToPlayerTool::new(adapter.clone())));
    tools.push(Box::new(ModAttackPlayerTool::new(adapter.clone())));
    tools.push(Box::new(ModGivePlayerTool::new(adapter.clone())));
    tools.push(Box::new(ModCollectItemsTool::new(adapter.clone())));
    tools.push(Box::new(ModStopTool::new(adapter.clone())));
    {
        let mut tg = ModSetGoalTool::new(adapter.clone());
        if let Some(ref pg) = pending_goal {
            tg = tg.with_pending_goal(pg.clone());
        }
        tools.push(Box::new(tg));
    }
    // 第三批工具（参考 mindcraft 41 actions + 14 queries）
    tools.push(Box::new(ModFollowPlayerTool::new(adapter.clone())));
    tools.push(Box::new(ModSearchWikiTool::new(adapter.clone())));
    tools.push(Box::new(ModVillagerTradesTool::new(adapter.clone())));
    tools.push(Box::new(ModTradeWithVillagerTool::new(adapter.clone())));
    tools.push(Box::new(ModLookAtPlayerTool::new(adapter.clone())));
    tools.push(Box::new(ModLookAtPositionTool::new(adapter.clone())));
    tools.push(Box::new(ModActivateBlockTool::new(adapter.clone())));
    tools.push(Box::new(ModUseOnEntityTool::new(adapter.clone())));
    tools.push(Box::new(ModClearChatTool::new(adapter.clone())));
    tools.push(Box::new(ModActivateNearestBlockTool::new(adapter.clone())));
    tools.push(Box::new(ModGetCraftingPlanTool::new(adapter.clone())));
    tools.push(Box::new(ModDiscardSmartTool::new(adapter.clone())));
    tools.push(Box::new(ModRideTool::new(adapter.clone())));
    tools.push(Box::new(ModFishTool::new(adapter.clone())));
    tools.push(Box::new(ModSleepTool::new(adapter.clone())));
    tools.push(Box::new(ModLookAbsTool::new(adapter.clone())));
    // 附魔 + 维度传送（末影龙通关路线）
    tools.push(Box::new(ModEnchantTool::new(adapter.clone())));
    tools.push(Box::new(ModBuildPortalTool::new(adapter.clone())));
    tools.push(Box::new(ModTeleportToDimensionTool::new(adapter.clone())));
    // 新寻路系统（Numen 风格：异步 A* + 8种 movement）
    tools.push(Box::new(ModNavToTool::new(adapter.clone())));
    tools.push(Box::new(ModNavStatusTool::new(adapter.clone())));
    tools.push(Box::new(ModNavStopTool::new(adapter.clone())));
    // 目标引擎（GoalEngine）：复合目标自动执行
    tools.push(Box::new(ModGoalExecuteTool::new(adapter.clone())));
    tools.push(Box::new(ModGoalStatusTool::new(adapter)));
    // Agent-local tools（Numen 风格：无需 mod 通信）
    tools.push(Box::new(NumenTodoWriteTool));
    tools.push(Box::new(NumenStatusTool));
    // 批次 A：用生存层装饰器统一包住所有工具（记录结构化失败 + 注入 survival notes）
    tools
        .into_iter()
        .map(|t| Box::new(SurvivalWrappedTool::new(t)) as Box<dyn GameTool>)
        .collect()
}

// ═══════════════════════════════════════════════════════════════
// VisualPerceive — 截屏+VLM分析（仅 GUI 场景）
// ═══════════════════════════════════════════════════════════════
