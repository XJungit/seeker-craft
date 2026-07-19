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
use crate::tool_args;
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
// collect — 自动采集（找→走→挖，核心工具）
// ═══════════════════════════════════════════════════════════════

pub struct ModCollectTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModCollectTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModCollectTool {
    fn name(&self) -> &str {
        "collect"
    }
    fn description(&self) -> &str {
        "AUTO find nearest target block → walk to it → dig by coordinate (no camera aiming needed). Each block 1-3s. Trees handled column-by-column (top-down). If nearest block is unreachable, skips to next. Stops when count reached or no more blocks within 30m. Usage: collect(target=\"oak_log\", count=10) or collect(target=\"coal_ore\")"
    }
    fn parameters(&self) -> Value {
        use tool_args::schema;
        schema::object()
            .str_req(
                "target",
                "Block ID substring, e.g. oak_log, stone, coal_ore, iron_ore",
            )
            .int_opt("count", "Number to collect (1-64, default 1)", 1, 1, 64)
            .finish()
    }
    fn effects(&self) -> ToolEffects {
        ToolEffects::write()
    }
    fn execute(
        &self,
        _id: &str,
        args: Value,
        _on_update: Option<ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        let target = args["target"].as_str().unwrap_or("oak_log");
        let want = args["count"].as_u64().unwrap_or(1).min(64) as u32;
        let adapter = self.adapter.lock_adapter()?;
        let st = adapter.reload()?;
        // 生存前置检查（借鉴 Numen 生存链）：紧急状态优先处理，不执行采集
        if let Some(warning) = survival_precheck(
            st.health,
            st.hunger,
            has_food_in_inventory(&st.inventory),
            has_weapon_in_inventory(&st.inventory),
            has_hostile_nearby(&st.entities),
        ) {
            return Ok(ToolResult {
                message: format!("collect ABORTED: {warning}"),
                is_error: true,
                images: vec![],
            });
        }
        let before: u32 = st
            .inventory
            .iter()
            .filter(|i| i.id.contains(target))
            .map(|i| i.count)
            .sum();
        let max_attempts = want.clamp(3, 10);
        let mut got = before;
        // Numen TargetSet 黑名单：不可达方块坐标拉黑，换下一个同类方块（CollectItems skip_target 模式）
        let mut blacklisted: std::collections::HashSet<(i32, i32, i32)> =
            std::collections::HashSet::new();
        let mut consecutive_failures = 0;
        for attempt in 1..=max_attempts {
            if got >= before + want {
                break;
            }
            // 找最近方块时跳过黑名单；只选竖直可够到的一段树干（dig_at 有 5.5m 距离上限）
            let py = adapter.reload()?.position[1];
            let Some((block, _)) = find_nearest_reachable(&adapter, target, &blacklisted, py, 4.0)
            else {
                let msg = if got > before {
                    format!(
                        "collected {target}: {before}→{got} (no more nearby, tried {attempt}, blacklisted {})",
                        blacklisted.len()
                    )
                } else {
                    format!(
                        "collected {target}: {before}→{got} (no {target} found nearby, blacklisted {})",
                        blacklisted.len()
                    )
                };
                return Ok(ToolResult {
                    message: msg,
                    is_error: got == before,
                    images: vec![],
                });
            };
            if block.dist > 30.0 {
                return Ok(ToolResult {
                    message: format!(
                        "collected {target}: {before}→{got} (nearest {target} too far: {:.1}m)",
                        block.dist
                    ),
                    is_error: got == before,
                    images: vec![],
                });
            }
            // 走到方块附近：目标用玩家当前 y 坐标（方块 y 可能在头顶/脚下，A* 到不了）
            let player_y = adapter.reload()?.position[1];
            let ack = adapter.move_to(block.x, player_y, block.z)?;
            let reached = ack.reached.unwrap_or(false);
            let stuck = ack.stuck.unwrap_or(false);
            let final_dist = ack.final_dist.unwrap_or(999.0);
            let bx = block.x.round() as i32;
            let by = block.y.round() as i32;
            let bz = block.z.round() as i32;
            if !reached && (stuck || final_dist > 5.0) {
                // 没到达且卡住/太远 → 拉黑这个方块，换下一个
                blacklisted.insert((bx, by, bz));
                consecutive_failures += 1;
                if consecutive_failures >= 3 {
                    break;
                }
                continue;
            }
            if !reached {
                // 超时但在 5m 内，尝试原地挖
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
            // 直接按坐标破坏方块（mod 侧 digAt 内部 lookAt+destroyBlock，不依赖准星 raycast）
            let _ = adapter.dig_at(bx, by, bz)?;
            std::thread::sleep(std::time::Duration::from_millis(600));
            got = adapter
                .reload()?
                .inventory
                .iter()
                .filter(|i| i.id.contains(target))
                .map(|i| i.count)
                .sum();
            consecutive_failures = if got > before + (attempt - 1) {
                0
            } else {
                // dig_at 没收获 → 也拉黑这个方块
                blacklisted.insert((bx, by, bz));
                consecutive_failures + 1
            };
            if consecutive_failures >= 3 {
                break;
            }
        }
        let actual = got.saturating_sub(before);
        // Numen 尽力返回：收集到部分也算成功
        let msg = if actual > 0 {
            if actual >= want {
                format!("collected {target}: {before}→{got} (+{actual}, wanted +{want})")
            } else {
                format!(
                    "collected {target}: {before}→{got} (+{actual}, wanted +{want}, partial, blacklisted {})",
                    blacklisted.len()
                )
            }
        } else {
            format!(
                "collected {target}: {before}→{got} (failed to collect any, blacklisted {} blocks)",
                blacklisted.len()
            )
        };
        Ok(ToolResult {
            message: msg,
            is_error: actual == 0,
            images: vec![],
        })
    }
}

// ═══════════════════════════════════════════════════════════════
// craft — 合成（mod Inventory 直接操作）
// ═══════════════════════════════════════════════════════════════

pub struct ModCraftTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModCraftTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModCraftTool {
    fn name(&self) -> &str {
        "craft"
    }
    fn description(&self) -> &str {
        "Craft items via direct inventory manipulation (2×2 grid). Covers: planks, sticks, crafting_table, wooden/stone/iron/diamond tools, torch, furnace, chest, iron/diamond armor (helmet, chestplate, leggings, boots), shield. For 3×3 recipes: craft crafting_table → place it → activate_nearest_block(\"crafting_table\") then craft again. Usage: craft(item=\"diamond_pickaxe\", count=1)  craft(item=\"iron_chestplate\")  craft(item=\"shield\")"
    }
    fn parameters(&self) -> Value {
        use tool_args::schema;
        schema::object()
            .str_req("item", "Item to craft: oak_planks, stick, crafting_table, furnace, torch, wooden_pickaxe, stone_sword, etc.")
            .int_opt("count", "How many to craft (1-64)", 1, 1, 64)
            .finish()
    }
    fn effects(&self) -> ToolEffects {
        ToolEffects::write()
    }
    fn execute(
        &self,
        _id: &str,
        args: Value,
        _on_update: Option<ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        let item = args["item"].as_str().unwrap_or("oak_planks");
        let count = args["count"].as_u64().unwrap_or(1) as u32;
        let adapter = self.adapter.lock_adapter()?;
        let st = adapter.reload()?;
        let before: u32 = st
            .inventory
            .iter()
            .filter(|i| i.id.contains(item))
            .map(|i| i.count)
            .sum();

        // 预检材料：如果合成失败，明确告诉 LLM 缺什么
        let missing = check_missing_materials(&st, item, count);
        if !missing.is_empty() {
            return Ok(ToolResult {
                message: format!(
                    "craft {item} FAILED — missing materials: {}. Have: {}. Tip: craft sticks first (2 planks → 4 sticks), then craft tools.",
                    missing.join(", "),
                    summarize_inventory(&st)
                ),
                is_error: true,
                images: vec![],
            });
        }

        adapter.craft(item, count)?;
        std::thread::sleep(std::time::Duration::from_millis(200));
        let after: u32 = adapter
            .reload()?
            .inventory
            .iter()
            .filter(|i| i.id.contains(item))
            .map(|i| i.count)
            .sum();
        let got = after.saturating_sub(before);
        let msg = if got > 0 {
            format!("crafted {item}: {before}→{after} (+{got})")
        } else {
            format!(
                "craft {item} returned 0 (mod handler may not cover this recipe). Have: {}",
                summarize_inventory(&st)
            )
        };
        Ok(ToolResult {
            message: msg,
            is_error: got == 0,
            images: vec![],
        })
    }
}

// ═══════════════════════════════════════════════════════════════
// place — 放置方块（自动切快捷栏+右键）
// ═══════════════════════════════════════════════════════════════

// ═══════════════════════════════════════════════════════════════
// equip / use_item / attack — 物品/战斗
// ═══════════════════════════════════════════════════════════════

pub struct ModEquipTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModEquipTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModEquipTool {
    fn name(&self) -> &str {
        "equip"
    }
    fn description(&self) -> &str {
        "Switch active hotbar slot. slot: 1-9 (corresponds to HOTBAR display). Use to hold a specific item before mining or placing."
    }
    fn parameters(&self) -> Value {
        use tool_args::schema;
        schema::object()
            .int_req("slot", "Hotbar slot number 1-9", 1, 9)
            .finish()
    }
    fn effects(&self) -> ToolEffects {
        ToolEffects::write()
    }
    fn execute(
        &self,
        _id: &str,
        args: Value,
        _on_update: Option<ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        let slot = args["slot"].as_u64().unwrap_or(1).clamp(1, 9) as u32;
        // mod 侧反射设置 Inventory.selected + 发送 SetCarriedItemPacket（立即生效）
        // 之前用 Action::Press 模拟数字键不生效（MC 数字键是边沿触发，setDown 不触发 click）
        let ack = self.adapter.lock_adapter()?.select_slot(slot - 1)?; // 0-indexed
        let actual = ack
            .slot
            .as_ref()
            .and_then(|v| v.as_u64())
            .unwrap_or(slot as u64 - 1) as u32
            + 1;
        let held = ack.held_item.clone().unwrap_or_default();
        let msg = if actual == slot {
            if held.is_empty() {
                format!("equipped slot {slot} (empty hand)")
            } else {
                format!("equipped slot {slot} → {held}")
            }
        } else {
            format!("equip FAILED: wanted slot {slot}, actual slot {actual}")
        };
        Ok(ToolResult {
            message: msg,
            is_error: actual != slot,
            images: vec![],
        })
    }
}

// ═══════════════════════════════════════════════════════════════
// MoveSlot — 精确槽位移动物品（支持拆分）
// ═══════════════════════════════════════════════════════════════

pub struct ModMoveSlotTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModMoveSlotTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModMoveSlotTool {
    fn name(&self) -> &str {
        "move_slot"
    }
    fn description(&self) -> &str {
        "Move items between two inventory slots precisely. Supports splitting a stack. Slot index: 0-8 = hotbar, 9-35 = main inventory (matches INVENTORY display 'slot N'). count: omit/null = move whole stack; integer = move that many (split). Rules: empty target → place; same item → merge up to max stack; different items → swap (only when moving whole stack, cannot split into occupied different-item slot). Use for fine-grained inventory management that move_to_hotbar/equip cannot do."
    }
    fn parameters(&self) -> Value {
        use tool_args::schema;
        schema::object()
            .int_req(
                "from_slot",
                "Source slot: 0-8=hotbar, 9-35=main inventory",
                0,
                35,
            )
            .int_req(
                "to_slot",
                "Destination slot: 0-8=hotbar, 9-35=main inventory",
                0,
                35,
            )
            .int_opt("count", "Items to move (omit=whole stack)", 1, 1, 64)
            .finish()
    }
    fn effects(&self) -> ToolEffects {
        ToolEffects::write()
    }
    fn execute(
        &self,
        _id: &str,
        args: Value,
        _on_update: Option<ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        let from_slot = args["from_slot"].as_u64().unwrap_or(0) as u32;
        let to_slot = args["to_slot"].as_u64().unwrap_or(0) as u32;
        let count = args["count"].as_u64().map(|n| n as u32);
        if from_slot > 35 || to_slot > 35 {
            return Ok(ToolResult {
                message: format!(
                    "move_slot FAILED: slot index out of range (from={from_slot}, to={to_slot}, valid 0-35)"
                ),
                is_error: true,
                images: vec![],
            });
        }
        if from_slot == to_slot {
            return Ok(ToolResult {
                message: format!("move_slot FAILED: from_slot == to_slot ({from_slot})"),
                is_error: true,
                images: vec![],
            });
        }
        let ack = self
            .adapter
            .lock_adapter()?
            .move_slot(from_slot, to_slot, count)?;
        let moved = ack.moved.unwrap_or(false);
        let msg = if moved {
            format!("move_slot OK: {}", ack.detail)
        } else {
            format!("move_slot FAILED: {}", ack.detail)
        };
        Ok(ToolResult {
            message: msg,
            is_error: !moved,
            images: vec![],
        })
    }
}

pub struct ModUseItemTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModUseItemTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModUseItemTool {
    fn name(&self) -> &str {
        "use_item"
    }
    fn description(&self) -> &str {
        "Use item in hand (server-side useItem). For eating food (32 ticks≈1.6s), opening doors (5 ticks), placing blocks (5 ticks). ticks: hold duration. Prefer consume() for eating specific foods."
    }
    fn parameters(&self) -> Value {
        use tool_args::schema;
        schema::object()
            .int_opt(
                "ticks",
                "Hold duration in ticks (20≈1s, 32=eat)",
                20,
                1,
                100,
            )
            .finish()
    }
    fn effects(&self) -> ToolEffects {
        ToolEffects::write()
    }
    fn execute(
        &self,
        _id: &str,
        args: Value,
        _on_update: Option<ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        let ticks = args["ticks"].as_u64().unwrap_or(20) as u32;
        let ack = self.adapter.lock_adapter()?.use_item(ticks)?;
        let consumed = ack.consumed.unwrap_or(false);
        let msg = if consumed {
            format!("use_item {ticks} ticks (consumed)")
        } else {
            format!("use_item {ticks} ticks (not consumed)")
        };
        Ok(ToolResult {
            message: msg,
            is_error: false,
            images: vec![],
        })
    }
}

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

/// 原地垫方块升高（脱困用）。在脚下放方块 → 跳起 → 重复。
pub struct ModConsumeTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModConsumeTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModConsumeTool {
    fn name(&self) -> &str {
        "consume"
    }
    fn description(&self) -> &str {
        "Eat food to restore hunger/health. Auto-finds food in hotbar (must already be in slots 1-9). Each food heals: bread=2.5, cooked_beef=4, golden_apple=4+regen. Usage: consume(item=\"cooked_beef\")  consume(item=\"bread\")"
    }
    fn parameters(&self) -> Value {
        use tool_args::schema;
        schema::object()
            .str_req(
                "item",
                "Food: cooked_beef, bread, apple, steak, porkchop, chicken, carrot, etc.",
            )
            .finish()
    }
    fn effects(&self) -> ToolEffects {
        ToolEffects::write()
    }
    fn execute(
        &self,
        _id: &str,
        args: Value,
        _on_update: Option<ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        let item = args["item"].as_str().unwrap_or("bread");
        let ticks = args["ticks"].as_u64().unwrap_or(32) as u32;
        let a = self.adapter.lock_adapter()?;
        let slot = a
            .reload()?
            .inventory
            .iter()
            .find(|i| i.id.contains(item) && i.slot < 9 && i.count > 0)
            .map(|i| i.slot + 1)
            .unwrap_or(1);
        let _ = a.select_slot(slot - 1)?;
        std::thread::sleep(std::time::Duration::from_millis(100));
        let _ = a.use_item(ticks)?;
        Ok(ToolResult {
            message: format!("ate {item} (from slot {slot})"),
            is_error: false,
            images: vec![],
        })
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

pub struct ModRememberTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModRememberTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModRememberTool {
    fn name(&self) -> &str {
        "rememberHere"
    }
    fn description(&self) -> &str {
        "Save current position with a name for later recall. name: label like 'base', 'cave_entrance', 'tree_farm'."
    }
    fn parameters(&self) -> Value {
        use tool_args::schema;
        schema::object()
            .str_req("name", "Label for this location")
            .finish()
    }
    fn effects(&self) -> ToolEffects {
        ToolEffects::read()
    }
    fn execute(
        &self,
        _id: &str,
        args: Value,
        _on_update: Option<ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        let name = args["name"].as_str().unwrap_or("here");
        Ok(ToolResult {
            message: self.adapter.lock_adapter()?.remember_here(name),
            is_error: false,
            images: vec![],
        })
    }
}

pub struct ModGoPlaceTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModGoPlaceTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModGoPlaceTool {
    fn name(&self) -> &str {
        "goToRememberedPlace"
    }
    fn description(&self) -> &str {
        "Walk to a previously saved location. name: label from rememberHere. Uses move_to for navigation."
    }
    fn parameters(&self) -> Value {
        use tool_args::schema;
        schema::object()
            .str_req("name", "Location label from rememberHere")
            .finish()
    }
    fn effects(&self) -> ToolEffects {
        ToolEffects::write()
    }
    fn execute(
        &self,
        _id: &str,
        args: Value,
        _on_update: Option<ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        let name = args["name"].as_str().unwrap_or("here");
        Ok(ToolResult {
            message: self.adapter.lock_adapter()?.go_to_place(name)?,
            is_error: false,
            images: vec![],
        })
    }
}

pub struct ModListPlacesTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModListPlacesTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModListPlacesTool {
    fn name(&self) -> &str {
        "savedPlaces"
    }
    fn description(&self) -> &str {
        "List all saved location names and coordinates from rememberHere."
    }
    fn parameters(&self) -> Value {
        tool_args::schema::no_args()
    }
    fn effects(&self) -> ToolEffects {
        ToolEffects::read()
    }
    fn execute(
        &self,
        _id: &str,
        _args: Value,
        _on_update: Option<ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        Ok(ToolResult {
            message: self.adapter.lock_adapter()?.list_places(),
            is_error: false,
            images: vec![],
        })
    }
}

// ═══════════════════════════════════════════════════════════════
// discard / smeltItem — 物品管理
// ═══════════════════════════════════════════════════════════════

pub struct ModDiscardTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModDiscardTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModDiscardTool {
    fn name(&self) -> &str {
        "discard"
    }
    fn description(&self) -> &str {
        "Throw away items from inventory. item: item name like dirt, cobblestone. num: how many to discard."
    }
    fn parameters(&self) -> Value {
        use tool_args::schema;
        schema::object()
            .str_req("item", "Item to discard")
            .int_opt("num", "Count to discard", 1, 1, 64)
            .finish()
    }
    fn effects(&self) -> ToolEffects {
        ToolEffects::write()
    }
    fn execute(
        &self,
        _id: &str,
        args: Value,
        _on_update: Option<ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        let item = args["item"].as_str().unwrap_or("dirt");
        let num = args["num"].as_u64().unwrap_or(1) as u32;
        self.adapter.lock_adapter()?.discard_item(item, num)?;
        Ok(ToolResult {
            message: format!("discarded {num}x {item}"),
            is_error: false,
            images: vec![],
        })
    }
}

pub struct ModSmeltTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModSmeltTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModSmeltTool {
    fn name(&self) -> &str {
        "smeltItem"
    }
    fn description(&self) -> &str {
        "Smelt items in nearest furnace. Finds furnace, opens it, places items+fuel, waits for smelting. item: raw material like raw_iron, oak_log(for charcoal). num: how many to smelt (each takes ~10s)."
    }
    fn parameters(&self) -> Value {
        use tool_args::schema;
        schema::object()
            .str_req(
                "item",
                "Raw material: raw_iron, raw_gold, raw_copper, sand, cobblestone, clay, oak_log",
            )
            .int_opt("num", "Count to smelt (1-64)", 1, 1, 64)
            .finish()
    }
    fn effects(&self) -> ToolEffects {
        ToolEffects::write()
    }
    fn execute(
        &self,
        _id: &str,
        args: Value,
        _on_update: Option<ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        let item = args["item"].as_str().unwrap_or("raw_iron");
        let num = args["num"].as_u64().unwrap_or(1) as u32;
        self.adapter.lock_adapter()?.smelt_item(item, num)?;
        Ok(ToolResult {
            message: format!("smelting {num}x {item} in furnace"),
            is_error: false,
            images: vec![],
        })
    }
}

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
// goToBed — 找最近的床并睡觉（Mindcraft 对齐）
// ═══════════════════════════════════════════════════════════════

pub struct ModGoToBedTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModGoToBedTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModGoToBedTool {
    fn name(&self) -> &str {
        "goToBed"
    }
    fn description(&self) -> &str {
        "Find nearest bed block and sleep in it to skip night. Searches nearby blocks for any bed type (red_bed, blue_bed, etc). Walks to bed and right-clicks to sleep. Only works at night or during thunderstorms."
    }
    fn parameters(&self) -> Value {
        tool_args::schema::no_args()
    }
    fn effects(&self) -> ToolEffects {
        ToolEffects::write()
    }
    fn execute(
        &self,
        _id: &str,
        _args: Value,
        _on_update: Option<ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        let adapter = self.adapter.lock_adapter()?;
        let st = adapter.reload()?;
        // 检查时间（MC 时间 >13000 = 夜晚）
        let t = st.time % 24000;
        let is_night = !(230..=13000).contains(&t);
        let is_thunder = st.thundering;
        if !is_night && !is_thunder {
            return Ok(ToolResult {
                message: "goToBed: not night or thundering, no need to sleep".into(),
                is_error: false,
                images: vec![],
            });
        }
        // 用 sleep 命令真睡觉（26.2 use_item 点床不触发睡眠，需 startSleeping）
        let ack = adapter.sleep(8.0)?;
        let msg = if ack.status.as_str() == "fail" {
            format!("goToBed FAILED: {}", ack.detail)
        } else {
            format!("goToBed: {}", ack.detail)
        };
        Ok(ToolResult {
            message: msg,
            is_error: ack.status.as_str() == "fail",
            images: vec![],
        })
    }
}

// ═══════════════════════════════════════════════════════════════
// stay — 原地等待（Mindcraft 对齐）
// ═══════════════════════════════════════════════════════════════

pub struct ModStayTool;
impl ModStayTool {
    pub fn new(_a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self
    }
}
impl GameTool for ModStayTool {
    fn name(&self) -> &str {
        "stay"
    }
    fn description(&self) -> &str {
        "Stay in current position for N seconds. Pauses all movement. type: seconds to wait (-1 = forever, but capped at 30 for safety). Use to wait for daytime, crop growth, or to avoid danger."
    }
    fn parameters(&self) -> Value {
        use tool_args::schema;
        schema::object()
            .int_opt(
                "type",
                "Seconds to stay (1-30, -1=forever but capped at 30)",
                5,
                -1,
                30,
            )
            .finish()
    }
    fn effects(&self) -> ToolEffects {
        ToolEffects::read()
    }
    fn execute(
        &self,
        _id: &str,
        args: Value,
        _on_update: Option<ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        let secs = args["type"].as_i64().unwrap_or(5).clamp(-1, 30) as i32;
        let wait = if secs < 0 { 30 } else { secs } as u64;
        std::thread::sleep(std::time::Duration::from_secs(wait));
        Ok(ToolResult {
            message: format!("stayed for {wait} seconds"),
            is_error: false,
            images: vec![],
        })
    }
}

// ═══════════════════════════════════════════════════════════════
// goToSurface — 回到地表（Mindcraft 对齐）
// ═══════════════════════════════════════════════════════════════

pub struct ModGoToSurfaceTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModGoToSurfaceTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModGoToSurfaceTool {
    fn name(&self) -> &str {
        "goToSurface"
    }
    fn description(&self) -> &str {
        "Move to the surface (highest non-air block above current position). Useful when underground or in a cave. Finds the highest solid block in nearby_blocks and walks to it."
    }
    fn parameters(&self) -> Value {
        tool_args::schema::no_args()
    }
    fn effects(&self) -> ToolEffects {
        ToolEffects::write()
    }
    fn execute(
        &self,
        _id: &str,
        _args: Value,
        _on_update: Option<ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        let adapter = self.adapter.lock_adapter()?;
        let st = adapter.reload()?;
        let cur_x = st.position[0].round() as i32;
        let cur_z = st.position[2].round() as i32;
        // 找 nearby_blocks 中 y 最高的非空气方块
        let surface = st
            .nearby_blocks
            .iter()
            .filter(|b| {
                let bx = b.x.round() as i32;
                let bz = b.z.round() as i32;
                (bx - cur_x).abs() <= 2 && (bz - cur_z).abs() <= 2 && !b.id.contains("air")
            })
            .max_by(|a, b| a.y.partial_cmp(&b.y).unwrap_or(std::cmp::Ordering::Equal));
        match surface {
            Some(b) => {
                let (sx, sy, sz) = (b.x, b.y + 1.0, b.z);
                drop(adapter);
                let _ = self.adapter.lock_adapter()?.move_to(sx, sy, sz)?;
                Ok(ToolResult {
                    message: format!("went to surface at ({:.0},{:.0},{:.0})", sx, sy, sz),
                    is_error: false,
                    images: vec![],
                })
            }
            None => {
                // nearby_blocks 有限，可能已经在地表
                Ok(ToolResult {
                    message: "goToSurface: already at surface or no surface block found nearby"
                        .into(),
                    is_error: false,
                    images: vec![],
                })
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════
// setMode — 模式管理（Mindcraft 对齐）
// ═══════════════════════════════════════════════════════════════

pub struct ModSetModeTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModSetModeTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModSetModeTool {
    fn name(&self) -> &str {
        "setMode"
    }
    fn description(&self) -> &str {
        "Toggle a behavior mode on/off. Modes are automatic behaviors checked every turn: 'self_preservation' (auto-flee when health<6), 'self_defense' (auto-attack nearby hostiles), 'unstuck' (auto-recover when stuck), 'cowardice' (always flee from hostiles), 'hunting' (auto-hunt nearby animals for food), 'torch_placing' (auto-place torches when dark), 'idle_staring' (look at nearby entities when idle). Returns current mode states."
    }
    fn parameters(&self) -> Value {
        use tool_args::schema;
        schema::object()
            .str_req("mode_name", "Mode name: self_preservation, self_defense, unstuck, cowardice, hunting, torch_placing, idle_staring")
            .bool_opt("on", "true=enable, false=disable", true)
            .finish()
    }
    fn effects(&self) -> ToolEffects {
        ToolEffects::read()
    }
    fn execute(
        &self,
        _id: &str,
        args: Value,
        _on_update: Option<ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        let mode = args["mode_name"].as_str().unwrap_or("self_defense");
        let on = args["on"].as_bool().unwrap_or(true);
        let adapter = self.adapter.lock_adapter()?;
        adapter.set_mode(mode, on);
        let modes_list = adapter.list_modes();
        Ok(ToolResult {
            message: format!("mode '{mode}' set to {on}\nCurrent modes:\n{modes_list}"),
            is_error: false,
            images: vec![],
        })
    }
}

// ═══════════════════════════════════════════════════════════════
// getCraftingPlan — 合成计划分析（Mindcraft 对齐）
// ═══════════════════════════════════════════════════════════════

// ═══════════════════════════════════════════════════════════════
// useOn — 对实体/方块使用工具（Mindcraft 对齐：剪羊毛/挤牛奶/点燃等）
// ═══════════════════════════════════════════════════════════════

// ═══════════════════════════════════════════════════════════════
// putInChest / takeFromChest / viewChest — 箱子操作（Mindcraft 对齐）
// ═══════════════════════════════════════════════════════════════

pub struct ModChestTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModChestTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModChestTool {
    fn name(&self) -> &str {
        "chest"
    }
    fn description(&self) -> &str {
        "Interact with nearest chest. action: 'view' (list contents), 'put' (deposit items), 'take' (withdraw items). item: item name for put/take. num: count. Walks to chest, opens it, performs action. Returns chest contents or transfer result."
    }
    fn parameters(&self) -> Value {
        use tool_args::schema;
        schema::object()
            .str_req("action", "view=list contents, put=deposit, take=withdraw")
            .str_opt("item", "Item to put/take (ignored for view)", "")
            .int_opt("num", "Count to put/take", 1, 1, 64)
            .finish()
    }
    fn effects(&self) -> ToolEffects {
        ToolEffects::write()
    }
    fn execute(
        &self,
        _id: &str,
        args: Value,
        _on_update: Option<ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        let action = args["action"].as_str().unwrap_or("view");
        let item = args["item"].as_str().unwrap_or("");
        let _num = args["num"].as_u64().unwrap_or(1) as u32;
        let adapter = self.adapter.lock_adapter()?;
        let st = adapter.reload()?;

        // 找箱子
        let chest = st
            .nearby_blocks
            .iter()
            .filter(|b| b.id.contains("chest") && !b.id.contains("minecart"))
            .min_by(|a, b| {
                a.dist
                    .partial_cmp(&b.dist)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
        let Some(chest) = chest else {
            return Ok(ToolResult {
                message: "chest: no chest found nearby".into(),
                is_error: true,
                images: vec![],
            });
        };

        // 走到箱子并打开（ServerPlayer 架构：useItemOn 打开容器）
        let _ = adapter.move_to(chest.x, chest.y + 0.5, chest.z)?;
        std::thread::sleep(std::time::Duration::from_millis(300));
        adapter.look_at(chest.x, chest.y + 0.5, chest.z)?;
        std::thread::sleep(std::time::Duration::from_millis(150));
        let _ = adapter.use_item(5)?;
        std::thread::sleep(std::time::Duration::from_millis(500));

        match action {
            "view" => {
                // ServerPlayer 架构：打开 GUI 后，容器内容需要 mod 侧新命令读取
                Ok(ToolResult {
                    message: format!(
                        "opened chest at ({:.0},{:.0},{:.0}). Use inventory to check your items after put/take.",
                        chest.x, chest.y, chest.z
                    ),
                    is_error: false,
                    images: vec![],
                })
            }
            "put" if !item.is_empty() => {
                // 装备要放入的物品（select_slot 用 0-indexed）
                let slot = st
                    .inventory
                    .iter()
                    .find(|i| i.id.contains(item) && i.slot < 9)
                    .map(|i| i.slot);
                match slot {
                    Some(s) => {
                        let _ = adapter.select_slot(s)?;
                        std::thread::sleep(std::time::Duration::from_millis(100));
                        // ServerPlayer 架构下容器转移需要 mod 侧容器 API（尚未实现）
                        // 暂时提示手动操作
                        Ok(ToolResult {
                            message: format!(
                                "chest opened at ({:.0},{:.0},{:.0}) with {item} equipped. Manual GUI transfer required (mod limitation).",
                                chest.x, chest.y, chest.z
                            ),
                            is_error: false,
                            images: vec![],
                        })
                    }
                    None => Ok(ToolResult {
                        message: format!("put: {item} not in hotbar"),
                        is_error: true,
                        images: vec![],
                    }),
                }
            }
            "take" if !item.is_empty() => {
                // ServerPlayer 架构下从容器取出需要 mod 侧容器 API（尚未实现）
                Ok(ToolResult {
                    message: format!(
                        "chest opened at ({:.0},{:.0},{:.0}). Manual GUI interaction required for take (mod limitation).",
                        chest.x, chest.y, chest.z
                    ),
                    is_error: false,
                    images: vec![],
                })
            }
            _ => Ok(ToolResult {
                message: "chest: invalid action or missing item param".to_string(),
                is_error: true,
                images: vec![],
            }),
        }
    }
}

// ═══════════════════════════════════════════════════════════════
// clearFurnace — 清空熔炉（Mindcraft 对齐）
// ═══════════════════════════════════════════════════════════════

pub struct ModClearFurnaceTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModClearFurnaceTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModClearFurnaceTool {
    fn name(&self) -> &str {
        "clearFurnace"
    }
    fn description(&self) -> &str {
        "Open nearest furnace and take out all items (result + unsmelted input + fuel). Walks to furnace, opens it, extracts items. Returns what was taken."
    }
    fn parameters(&self) -> Value {
        tool_args::schema::no_args()
    }
    fn effects(&self) -> ToolEffects {
        ToolEffects::write()
    }
    fn execute(
        &self,
        _id: &str,
        _args: Value,
        _on_update: Option<ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        let adapter = self.adapter.lock_adapter()?;
        let st = adapter.reload()?;
        let furnace = st
            .nearby_blocks
            .iter()
            .filter(|b| {
                b.id.contains("furnace")
                    || b.id.contains("blast_furnace")
                    || b.id.contains("smoker")
            })
            .min_by(|a, b| {
                a.dist
                    .partial_cmp(&b.dist)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
        let Some(furnace) = furnace else {
            return Ok(ToolResult {
                message: "clearFurnace: no furnace found nearby".into(),
                is_error: true,
                images: vec![],
            });
        };
        let _ = adapter.move_to(furnace.x, furnace.y + 0.5, furnace.z)?;
        std::thread::sleep(std::time::Duration::from_millis(300));
        adapter.look_at(furnace.x, furnace.y + 0.5, furnace.z)?;
        std::thread::sleep(std::time::Duration::from_millis(150));
        let _ = adapter.use_item(5)?;
        std::thread::sleep(std::time::Duration::from_millis(500));
        // ServerPlayer 架构下容器操作需要 mod 侧容器 API（尚未实现）
        // 暂时提示手动操作
        Ok(ToolResult {
            message: format!(
                "opened furnace at ({:.0},{:.0},{:.0}), GUI available for manual extraction",
                furnace.x, furnace.y, furnace.z
            ),
            is_error: false,
            images: vec![],
        })
    }
}

// ═══════════════════════════════════════════════════════════════
// inspect_gui / close_gui / transfer — 容器/GUI 交互（参考 Numen）
// ═══════════════════════════════════════════════════════════════

pub struct ModWaitTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModWaitTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModWaitTool {
    fn name(&self) -> &str {
        "wait"
    }
    fn description(&self) -> &str {
        "Wait for N seconds. Use after placing items in furnace, waiting for crops, health regen, or combat cooldown. Usage: wait(seconds=10)"
    }
    fn parameters(&self) -> Value {
        use tool_args::schema;
        schema::object()
            .int_opt("seconds", "Seconds to wait (1-30)", 5, 1, 30)
            .finish()
    }
    fn effects(&self) -> ToolEffects {
        ToolEffects::read()
    }
    fn execute(
        &self,
        _id: &str,
        args: Value,
        _on_update: Option<ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        let seconds = args["seconds"].as_u64().unwrap_or(5).min(30) as u32;
        let ack = self.adapter.lock_adapter()?.wait(seconds)?;
        Ok(ToolResult {
            message: ack.detail,
            is_error: false,
            images: vec![],
        })
    }
}

// ═══════════════════════════════════════════════════════════════
// 玩家交互 + 控制命令（参考 mindcraft + Numen）
// ═══════════════════════════════════════════════════════════════

pub struct ModListPlayersTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModListPlayersTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModListPlayersTool {
    fn name(&self) -> &str {
        "list_players"
    }
    fn description(&self) -> &str {
        "List all online players with name, position, and distance. Use before go_to_player or attack_player."
    }
    fn parameters(&self) -> Value {
        tool_args::schema::no_args()
    }
    fn effects(&self) -> ToolEffects {
        ToolEffects::read()
    }
    fn execute(
        &self,
        _id: &str,
        _args: Value,
        _on_update: Option<ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        let ack = self.adapter.lock_adapter()?.list_players()?;
        let count = ack.count.unwrap_or(0);
        let players = ack.players.clone().unwrap_or_default();
        let mut lines = vec![format!("Online players: {count}")];
        if let Some(arr) = players.as_array() {
            for p in arr {
                let name = p["name"].as_str().unwrap_or("?");
                let dist = p["dist"].as_f64().unwrap_or(0.0);
                let x = p["position"][0].as_f64().unwrap_or(0.0);
                let y = p["position"][1].as_f64().unwrap_or(0.0);
                let z = p["position"][2].as_f64().unwrap_or(0.0);
                lines.push(format!(
                    "  {name} at ({x:.0},{y:.0},{z:.0}) dist={dist:.1}m"
                ));
            }
        }
        Ok(ToolResult {
            message: lines.join("\n"),
            is_error: false,
            images: vec![],
        })
    }
}

pub struct ModGoToPlayerTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModGoToPlayerTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModGoToPlayerTool {
    fn name(&self) -> &str {
        "go_to_player"
    }
    fn description(&self) -> &str {
        "Navigate to another player's position. player_name: exact player name from list_players. closeness: how close to get (default 2.0m)."
    }
    fn parameters(&self) -> Value {
        use tool_args::schema;
        schema::object()
            .str_req("player_name", "Exact player name")
            .num_opt("closeness", "How close to get (meters)", 3.0)
            .finish()
    }
    fn effects(&self) -> ToolEffects {
        ToolEffects::write()
    }
    fn execute(
        &self,
        _id: &str,
        args: Value,
        _on_update: Option<ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        let name = args["player_name"].as_str().unwrap_or("");
        let closeness = args["closeness"].as_f64();
        let ack = self.adapter.lock_adapter()?.go_to_player(name, closeness)?;
        let reached = ack.reached.unwrap_or(false);
        let dist = ack.final_dist.unwrap_or(0.0);
        Ok(ToolResult {
            message: format!("go_to_player {name}: reached={reached} dist={dist:.1}m"),
            is_error: !reached,
            images: vec![],
        })
    }
}

pub struct ModGivePlayerTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModGivePlayerTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModGivePlayerTool {
    fn name(&self) -> &str {
        "give_player"
    }
    fn description(&self) -> &str {
        "Give items to another player. Walks to player if far, then drops items as ItemEntity. player_name: exact name. item: item name. num: how many."
    }
    fn parameters(&self) -> Value {
        use tool_args::schema;
        schema::object()
            .str_req("player_name", "Exact player name")
            .str_req("item", "Item name")
            .int_opt("num", "Count", 1, 1, 64)
            .finish()
    }
    fn effects(&self) -> ToolEffects {
        ToolEffects::write()
    }
    fn execute(
        &self,
        _id: &str,
        args: Value,
        _on_update: Option<ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        let name = args["player_name"].as_str().unwrap_or("");
        let item = args["item"].as_str().unwrap_or("");
        let num = args["num"].as_u64().unwrap_or(1) as u32;
        let ack = self.adapter.lock_adapter()?.give_player(name, item, num)?;
        let dropped = ack.dropped.unwrap_or(0);
        Ok(ToolResult {
            message: format!("give_player {item} x{dropped} to {name}"),
            is_error: dropped == 0,
            images: vec![],
        })
    }
}

pub struct ModCollectItemsTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModCollectItemsTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModCollectItemsTool {
    fn name(&self) -> &str {
        "collect_items"
    }
    fn description(&self) -> &str {
        "Automatically pick up nearby dropped items on the ground. Scans for ItemEntity, walks to each, and lets vanilla pickup handle collection. item_ids: filter by item names (empty = all). radius: search radius (default 16). max_count: max items to collect (default 64)."
    }
    fn parameters(&self) -> Value {
        use tool_args::schema;
        schema::object()
            .str_opt(
                "item_ids",
                "Filter items (empty=all), JSON array of strings",
                "[]",
            )
            .num_opt("radius", "Search radius", 16.0)
            .int_opt("max_count", "Max to collect", 64, 1, 256)
            .finish()
    }
    fn effects(&self) -> ToolEffects {
        ToolEffects::write()
    }
    fn execute(
        &self,
        _id: &str,
        args: Value,
        _on_update: Option<ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        let item_ids: Vec<String> = match &args["item_ids"] {
            Value::Array(arr) => arr
                .iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect(),
            Value::String(s) => serde_json::from_str(s).unwrap_or_default(),
            _ => vec![],
        };
        let radius = args["radius"].as_f64().unwrap_or(16.0);
        let max_count = args["max_count"].as_u64().unwrap_or(64) as u32;
        let ack = self
            .adapter
            .lock_adapter()?
            .collect_items(item_ids, radius, max_count)?;
        let collected = ack.collected.unwrap_or(0);
        Ok(ToolResult {
            message: format!("collect_items: collected {collected} items"),
            is_error: collected == 0,
            images: vec![],
        })
    }
}

pub struct ModStopTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModStopTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModStopTool {
    fn name(&self) -> &str {
        "stop"
    }
    fn description(&self) -> &str {
        "Stop all current actions immediately. Use when agent is stuck or doing something wrong. Equivalent to mindcraft !stop."
    }
    fn parameters(&self) -> Value {
        tool_args::schema::no_args()
    }
    fn effects(&self) -> ToolEffects {
        ToolEffects::write()
    }
    fn execute(
        &self,
        _id: &str,
        _args: Value,
        _on_update: Option<ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        let ack = self.adapter.lock_adapter()?.stop()?;
        Ok(ToolResult {
            message: ack.detail,
            is_error: false,
            images: vec![],
        })
    }
}

pub struct ModSetGoalTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
    pending_goal: Option<Arc<std::sync::Mutex<Option<String>>>>,
}
impl ModSetGoalTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self {
            adapter: a,
            pending_goal: None,
        }
    }
    pub fn with_pending_goal(mut self, pg: Arc<std::sync::Mutex<Option<String>>>) -> Self {
        self.pending_goal = Some(pg);
        self
    }
}
impl GameTool for ModSetGoalTool {
    fn name(&self) -> &str {
        "set_goal"
    }
    fn description(&self) -> &str {
        "Set a persistent goal that stays active across turns. Clear with empty goal. Used by SelfPrompter for continuous motivation. goal: description of what to achieve."
    }
    fn parameters(&self) -> Value {
        use tool_args::schema;
        schema::object()
            .str_opt("goal", "Goal description (empty to clear)", "")
            .finish()
    }
    fn effects(&self) -> ToolEffects {
        ToolEffects::write()
    }
    fn execute(
        &self,
        _id: &str,
        args: Value,
        _on_update: Option<ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        let goal = args["goal"].as_str().unwrap_or("");
        let ack = self.adapter.lock_adapter()?.set_goal(goal)?;
        if let Some(ref pg) = self.pending_goal
            && let Ok(mut g) = pg.lock()
        {
            if goal.is_empty() {
                *g = None;
            } else {
                *g = Some(goal.to_string());
            }
        }
        Ok(ToolResult {
            message: ack.detail,
            is_error: false,
            images: vec![],
        })
    }
}

// ═══════════════════════════════════════════════════════════════
// 第三批工具（参考 mindcraft 41 actions + 14 queries）
// ═══════════════════════════════════════════════════════════════

pub struct ModFollowPlayerTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModFollowPlayerTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModFollowPlayerTool {
    fn name(&self) -> &str {
        "follow_player"
    }
    fn description(&self) -> &str {
        "Endlessly follow the given player (mindcraft !followPlayer resume=true). Mod-side tick loop keeps chasing. Use stop() to cancel. player_name: target player. follow_dist: distance to maintain (default 3m)."
    }
    fn parameters(&self) -> Value {
        use tool_args::schema;
        schema::object()
            .str_req("player_name", "Player name to follow")
            .num_opt("follow_dist", "Distance to maintain (default 3m)", 3.0)
            .finish()
    }
    fn effects(&self) -> ToolEffects {
        ToolEffects::write()
    }
    fn execute(
        &self,
        _id: &str,
        args: Value,
        _on_update: Option<ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        let name = args["player_name"].as_str().unwrap_or("");
        let dist = args["follow_dist"].as_f64();
        let ack = self.adapter.lock_adapter()?.follow_player(name, dist)?;
        Ok(ToolResult {
            message: ack.detail,
            is_error: ack.status == "fail",
            images: vec![],
        })
    }
}

pub struct ModSearchWikiTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModSearchWikiTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModSearchWikiTool {
    fn name(&self) -> &str {
        "search_wiki"
    }
    fn description(&self) -> &str {
        "Search minecraft.wiki for crafting/behavior info. Mod-side HTTP request + HTML extraction, 2000 char truncation. query: search term (e.g. 'redstone repeater recipe')."
    }
    fn parameters(&self) -> Value {
        use tool_args::schema;
        schema::object().str_req("query", "Search query").finish()
    }
    fn effects(&self) -> ToolEffects {
        ToolEffects::read()
    }
    fn execute(
        &self,
        _id: &str,
        args: Value,
        _on_update: Option<ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        let q = args["query"].as_str().unwrap_or("");
        let ack = self.adapter.lock_adapter()?.search_wiki(q)?;
        let text = ack.wiki_text.unwrap_or(ack.detail);
        Ok(ToolResult {
            message: text,
            is_error: ack.status == "fail",
            images: vec![],
        })
    }
}

pub struct ModVillagerTradesTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModVillagerTradesTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModVillagerTradesTool {
    fn name(&self) -> &str {
        "villager_trades"
    }
    fn description(&self) -> &str {
        "Show trades of nearest villager (mindcraft !showVillagerTrades). Returns trade list with 1-indexed positions. radius: search radius (default 8m)."
    }
    fn parameters(&self) -> Value {
        use tool_args::schema;
        schema::object()
            .num_opt("radius", "Search radius (default 8m)", 8.0)
            .finish()
    }
    fn effects(&self) -> ToolEffects {
        ToolEffects::read()
    }
    fn execute(
        &self,
        _id: &str,
        args: Value,
        _on_update: Option<ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        let radius = args["radius"].as_f64();
        let ack = self.adapter.lock_adapter()?.villager_trades(radius)?;
        let trades = ack
            .trades
            .map(|t| t.to_string())
            .unwrap_or_else(|| "[]".into());
        Ok(ToolResult {
            message: format!("{} | trades={}", ack.detail, trades),
            is_error: ack.status == "fail",
            images: vec![],
        })
    }
}

pub struct ModTradeWithVillagerTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModTradeWithVillagerTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModTradeWithVillagerTool {
    fn name(&self) -> &str {
        "trade_with_villager"
    }
    fn description(&self) -> &str {
        "Trade with nearest villager (mindcraft !tradeWithVillager). index: 1-indexed trade position from villager_trades. count: how many trades (default 1). radius: search radius (default 8m)."
    }
    fn parameters(&self) -> Value {
        use tool_args::schema;
        schema::object()
            .int_req("index", "1-indexed trade position", 1, 100)
            .int_opt("count", "How many trades (default 1)", 1, 1, 64)
            .num_opt("radius", "Search radius (default 8m)", 8.0)
            .finish()
    }
    fn effects(&self) -> ToolEffects {
        ToolEffects::write()
    }
    fn execute(
        &self,
        _id: &str,
        args: Value,
        _on_update: Option<ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        let index = args["index"].as_u64().unwrap_or(1) as u32;
        let count = args["count"].as_u64().map(|n| n as u32);
        let radius = args["radius"].as_f64();
        let ack = self
            .adapter
            .lock_adapter()?
            .trade_with_villager(index, count, radius)?;
        let traded = ack.traded.unwrap_or(0);
        Ok(ToolResult {
            message: format!("traded {} of index {}", traded, index),
            is_error: ack.status == "fail" || traded == 0,
            images: vec![],
        })
    }
}

pub struct ModLookAtPlayerTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModLookAtPlayerTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModLookAtPlayerTool {
    fn name(&self) -> &str {
        "look_at_player"
    }
    fn description(&self) -> &str {
        "Look at the given player (only orientation, no movement). player_name: target player."
    }
    fn parameters(&self) -> Value {
        use tool_args::schema;
        schema::object()
            .str_req("player_name", "Player name to look at")
            .finish()
    }
    fn effects(&self) -> ToolEffects {
        ToolEffects::write()
    }
    fn execute(
        &self,
        _id: &str,
        args: Value,
        _on_update: Option<ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        let name = args["player_name"].as_str().unwrap_or("");
        let ack = self.adapter.lock_adapter()?.look_at_player(name)?;
        Ok(ToolResult {
            message: ack.detail,
            is_error: ack.status == "fail",
            images: vec![],
        })
    }
}

pub struct ModLookAtPositionTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModLookAtPositionTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModLookAtPositionTool {
    fn name(&self) -> &str {
        "look_at_position"
    }
    fn description(&self) -> &str {
        "Look at a specific x/y/z coordinate (only orientation, no movement)."
    }
    fn parameters(&self) -> Value {
        use tool_args::schema;
        schema::object()
            .num_req("x", "World X")
            .num_req("y", "World Y")
            .num_req("z", "World Z")
            .finish()
    }
    fn effects(&self) -> ToolEffects {
        ToolEffects::write()
    }
    fn execute(
        &self,
        _id: &str,
        args: Value,
        _on_update: Option<ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        let x = args["x"].as_f64().unwrap_or(0.0);
        let y = args["y"].as_f64().unwrap_or(0.0);
        let z = args["z"].as_f64().unwrap_or(0.0);
        let ack = self.adapter.lock_adapter()?.look_at_position(x, y, z)?;
        Ok(ToolResult {
            message: ack.detail,
            is_error: ack.status == "fail",
            images: vec![],
        })
    }
}

pub struct ModClearChatTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModClearChatTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModClearChatTool {
    fn name(&self) -> &str {
        "clear_chat"
    }
    fn description(&self) -> &str {
        "Clear the chat history (mindcraft !clearChat). Starts fresh conversation from scratch. Useful after long sessions to reset context."
    }
    fn parameters(&self) -> Value {
        tool_args::schema::no_args()
    }
    fn effects(&self) -> ToolEffects {
        ToolEffects::write()
    }
    fn execute(
        &self,
        _id: &str,
        _args: Value,
        _on_update: Option<ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        // mod 侧仅 ack，Rust 侧由 agent 清空 history
        let _ = self.adapter.lock_adapter()?.clear_chat()?;
        Ok(ToolResult {
            message:
                "Chat history cleared (mod ack only — agent runtime should clear its own history)."
                    .into(),
            is_error: false,
            images: vec![],
        })
    }
}

pub struct ModDiscardSmartTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModDiscardSmartTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModDiscardSmartTool {
    fn name(&self) -> &str {
        "discard_smart"
    }
    fn description(&self) -> &str {
        "Smart discard (mindcraft !discard pattern): moveAway 5m + drop items + return to origin. Prevents immediate re-pickup. item: item ID substring. num: how many to drop."
    }
    fn parameters(&self) -> Value {
        use tool_args::schema;
        schema::object()
            .str_req("item", "Item ID substring")
            .int_req("num", "How many to drop", 1, 64)
            .finish()
    }
    fn effects(&self) -> ToolEffects {
        ToolEffects::write()
    }
    fn execute(
        &self,
        _id: &str,
        args: Value,
        _on_update: Option<ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        let item = args["item"].as_str().unwrap_or("");
        let num = args["num"].as_u64().unwrap_or(1) as u32;
        let ack = self.adapter.lock_adapter()?.discard_smart(item, num)?;
        let dropped = ack.dropped.unwrap_or(0);
        Ok(ToolResult {
            message: format!(
                "discard_smart {}: {} dropped (moved away + returned)",
                item, dropped
            ),
            is_error: dropped == 0,
            images: vec![],
        })
    }
}

// ═══════════════════════════════════════════════════════════════
// 对齐 Mineflayer 的补充工具：骑乘 / 钓鱼 / 睡觉 / 精确朝向
// ═══════════════════════════════════════════════════════════════

/// 骑乘控制（对齐 Mineflayer mount/dismount/moveVehicle）。
pub struct ModRideTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModRideTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModRideTool {
    fn name(&self) -> &str {
        "ride"
    }
    fn description(&self) -> &str {
        "Ride/mount a nearby rideable entity (horse/pig/boat/minecart) or dismount. action: 'mount' (nearest within radius), 'dismount', or 'steer' (drive with left/forward in -1..1). Mount first, then steer to move. Usage: ride(action=\"mount\") ride(action=\"steer\", forward=1.0) ride(action=\"dismount\")"
    }
    fn parameters(&self) -> Value {
        use tool_args::schema;
        schema::object()
            .str_req("action", "mount | dismount | steer")
            .num_opt("radius", "Search radius for mount", 8.0)
            .num_opt("left", "Steering left (-1..1), only for steer", 0.0)
            .num_opt("forward", "Steering forward (-1..1), only for steer", 1.0)
            .finish()
    }
    fn effects(&self) -> ToolEffects {
        ToolEffects::write()
    }
    fn execute(
        &self,
        _id: &str,
        args: Value,
        _on_update: Option<ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        let action = args["action"].as_str().unwrap_or("mount");
        let radius = args["radius"].as_f64();
        let left = args["left"].as_f64();
        let forward = args["forward"].as_f64();
        let ack = self
            .adapter
            .lock_adapter()?
            .ride(action, radius, left, forward)?;
        let detail = ack.detail.clone();
        let mounted = ack.mounted.clone().unwrap_or_default();
        let msg = match action {
            "mount" => {
                if ack.status.as_str() == "fail" {
                    format!("ride mount FAILED: {}", detail)
                } else {
                    format!("ride mount {mounted} (nearest rideable)")
                }
            }
            "steer" => format!("ride steer {}", detail),
            "dismount" => "ride dismount".to_string(),
            _ => format!("ride unknown action '{action}'"),
        };
        Ok(ToolResult {
            message: msg,
            is_error: action == "mount" && ack.status.as_str() == "fail",
            images: vec![],
        })
    }
}

/// 钓鱼（对齐 Mineflayer bot.fish）。
pub struct ModFishTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModFishTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModFishTool {
    fn name(&self) -> &str {
        "fish"
    }
    fn description(&self) -> &str {
        "Cast and reel a fishing rod. Requires a fishing_rod in inventory. ticks: how long to hold the rod extended (longer = more chance a fish bites; reel happens automatically at end). Usage: fish(ticks=100)"
    }
    fn parameters(&self) -> Value {
        use tool_args::schema;
        schema::object()
            .int_opt(
                "ticks",
                "Hold duration in ticks (20≈1s, 100≈5s)",
                100,
                20,
                600,
            )
            .finish()
    }
    fn effects(&self) -> ToolEffects {
        ToolEffects::write()
    }
    fn execute(
        &self,
        _id: &str,
        args: Value,
        _on_update: Option<ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        let ticks = args["ticks"].as_u64().unwrap_or(100) as u32;
        let ack = self.adapter.lock_adapter()?.fish(ticks)?;
        let msg = if ack.status.as_str() == "fail" {
            format!("fish FAILED: {}", ack.detail)
        } else {
            format!("fish {} ticks ({})", ticks, ack.detail)
        };
        Ok(ToolResult {
            message: msg,
            is_error: ack.status.as_str() == "fail",
            images: vec![],
        })
    }
}

/// 睡觉跳夜（对齐 Mineflayer bot.sleep）。
pub struct ModSleepTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModSleepTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModSleepTool {
    fn name(&self) -> &str {
        "sleep"
    }
    fn description(&self) -> &str {
        "Sleep in the nearest bed to skip the night (or thunderstorm). Requires a bed placed nearby. Auto-finds the bed foot within radius and sleeps. Usage: sleep(radius=8)"
    }
    fn parameters(&self) -> Value {
        use tool_args::schema;
        schema::object()
            .num_opt("radius", "Search radius for a bed", 8.0)
            .finish()
    }
    fn effects(&self) -> ToolEffects {
        ToolEffects::write()
    }
    fn execute(
        &self,
        _id: &str,
        args: Value,
        _on_update: Option<ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        let radius = args["radius"].as_f64().unwrap_or(8.0);
        let ack = self.adapter.lock_adapter()?.sleep(radius)?;
        let msg = if ack.status.as_str() == "fail" {
            format!("sleep FAILED: {}", ack.detail)
        } else {
            format!("sleep: {}", ack.detail)
        };
        Ok(ToolResult {
            message: msg,
            is_error: ack.status.as_str() == "fail",
            images: vec![],
        })
    }
}

/// 精确朝向（对齐 Mineflayer bot.look(yaw,pitch)）。
pub struct ModLookAbsTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModLookAbsTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModLookAbsTool {
    fn name(&self) -> &str {
        "look_abs"
    }
    fn description(&self) -> &str {
        "Set absolute facing direction. yaw: 0=south, 90=west, 180=north, 270=east (degrees). pitch: -90=up, 0=horizontal, 90=down. Use for precise aiming without computing a target point. Usage: look_abs(yaw=90, pitch=-30)"
    }
    fn parameters(&self) -> Value {
        use tool_args::schema;
        schema::object()
            .num_req("yaw", "Yaw in degrees (0=south,90=west,180=north,270=east)")
            .num_req("pitch", "Pitch in degrees (-90=up,0=horizontal,90=down)")
            .finish()
    }
    fn effects(&self) -> ToolEffects {
        ToolEffects::write()
    }
    fn execute(
        &self,
        _id: &str,
        args: Value,
        _on_update: Option<ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        let yaw = args["yaw"].as_f64().unwrap_or(0.0) as f32;
        let pitch = args["pitch"].as_f64().unwrap_or(0.0) as f32;
        self.adapter.lock_adapter()?.look_abs(yaw, pitch)?;
        Ok(ToolResult {
            message: format!("look_abs yaw={yaw} pitch={pitch}"),
            is_error: false,
            images: vec![],
        })
    }
}

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
    tools.push(Box::new(ModMoveToTool::new(adapter.clone())));
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
    tools.push(Box::new(ModTeleportToDimensionTool::new(adapter)));
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

// ═══════════════════════════════════════════════════════════════
// enchant — 附魔物品（消耗 XP）
// ═══════════════════════════════════════════════════════════════

pub struct ModEnchantTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModEnchantTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModEnchantTool {
    fn name(&self) -> &str {
        "enchant"
    }
    fn description(&self) -> &str {
        "Enchant an item using XP levels. Finds item in inventory, spends levels (1-30), applies random enchantments from the enchanting table pool. Higher levels = better enchantments. Usage: enchant(item=\"diamond_pickaxe\", levels=30)"
    }
    fn parameters(&self) -> Value {
        use tool_args::schema;
        schema::object()
            .str_req(
                "item",
                "Item to enchant, e.g. diamond_pickaxe, iron_sword, diamond_chestplate",
            )
            .int_opt("levels", "XP levels to spend (1-30)", 30, 1, 30)
            .finish()
    }
    fn effects(&self) -> ToolEffects {
        ToolEffects::write()
    }
    fn execute(
        &self,
        _id: &str,
        args: Value,
        _on_update: Option<ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        let item = args["item"].as_str().unwrap_or("");
        let levels = args["levels"].as_u64().unwrap_or(30).min(30) as u32;
        if item.is_empty() {
            return Ok(ToolResult {
                message: "enchant: item required".into(),
                is_error: true,
                images: vec![],
            });
        }
        let ack = self.adapter.lock_adapter()?.enchant(item, levels)?;
        Ok(ToolResult {
            message: ack.detail,
            is_error: ack.status != "ok",
            images: vec![],
        })
    }
}
