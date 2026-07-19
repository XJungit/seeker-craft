// ═══════════════════════════════════════════════════════════════
// 物品/容器续集工具（collect/craft/equip/move_slot/useItem/consume/
// discard/smelt/chest/clearFurnace/wait/discardSmart/enchant）
// 从 tools_mod.rs 拆分到本子模块（重构 ②）
// ═══════════════════════════════════════════════════════════════

use crate::tool_args::schema;
use crate::tools_mod::MinecraftModAdapter;
use crate::tools_mod::SafeLockAdapter;
use crate::tools_mod::find_nearest_reachable;
use crate::tools_mod::has_food_in_inventory;
use crate::tools_mod::has_hostile_nearby;
use crate::tools_mod::has_weapon_in_inventory;
use crate::tools_mod::survival_precheck;
use craft_agent::core::tool::{GameTool, ToolEffects, ToolResult, ToolUpdateFn};
use serde_json::Value;
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::Mutex;

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
        let mut blacklisted: HashSet<(i32, i32, i32)> = HashSet::new();
        let mut consecutive_failures = 0;
        for attempt in 1..=max_attempts {
            if got >= before + want {
                break;
            }
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
            let player_y = adapter.reload()?.position[1];
            let ack = adapter.move_to(block.x, player_y, block.z)?;
            let reached = ack.reached.unwrap_or(false);
            let stuck = ack.stuck.unwrap_or(false);
            let final_dist = ack.final_dist.unwrap_or(999.0);
            let bx = block.x.round() as i32;
            let by = block.y.round() as i32;
            let bz = block.z.round() as i32;
            if !reached && (stuck || final_dist > 5.0) {
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
                blacklisted.insert((bx, by, bz));
                consecutive_failures + 1
            };
            if consecutive_failures >= 3 {
                break;
            }
        }
        let actual = got.saturating_sub(before);
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
        let missing = crate::tools_mod::check_missing_materials(&st, item, count);
        if !missing.is_empty() {
            return Ok(ToolResult {
                message: format!(
                    "craft {item} FAILED — missing materials: {}. Have: {}. Tip: craft sticks first (2 planks → 4 sticks), then craft tools.",
                    missing.join(", "),
                    crate::tools_mod::summarize_inventory(&st)
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
                crate::tools_mod::summarize_inventory(&st)
            )
        };
        Ok(ToolResult {
            message: msg,
            is_error: got == 0,
            images: vec![],
        })
    }
}

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
        let ack = self.adapter.lock_adapter()?.select_slot(slot - 1)?;
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
        let _ = adapter.move_to(chest.x, chest.y + 0.5, chest.z)?;
        std::thread::sleep(std::time::Duration::from_millis(300));
        adapter.look_at(chest.x, chest.y + 0.5, chest.z)?;
        std::thread::sleep(std::time::Duration::from_millis(150));
        let _ = adapter.use_item(5)?;
        std::thread::sleep(std::time::Duration::from_millis(500));
        match action {
            "view" => Ok(ToolResult {
                message: format!(
                    "opened chest at ({:.0},{:.0},{:.0}). Use inventory to check your items after put/take.",
                    chest.x, chest.y, chest.z
                ),
                is_error: false,
                images: vec![],
            }),
            "put" if !item.is_empty() => {
                let slot = st
                    .inventory
                    .iter()
                    .find(|i| i.id.contains(item) && i.slot < 9)
                    .map(|i| i.slot);
                match slot {
                    Some(s) => {
                        let _ = adapter.select_slot(s)?;
                        std::thread::sleep(std::time::Duration::from_millis(100));
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
            "take" if !item.is_empty() => Ok(ToolResult {
                message: format!(
                    "chest opened at ({:.0},{:.0},{:.0}). Manual GUI interaction required for take (mod limitation).",
                    chest.x, chest.y, chest.z
                ),
                is_error: false,
                images: vec![],
            }),
            _ => Ok(ToolResult {
                message: "chest: invalid action or missing item param".to_string(),
                is_error: true,
                images: vec![],
            }),
        }
    }
}

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
        schema::no_args()
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
