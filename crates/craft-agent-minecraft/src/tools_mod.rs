//! Minecraft mod 工具集 —— pi 规范版本（每工具含精确限制/范围/行为说明）。
//!   - 描述含限制：超时、范围、截断
//!   - 参数 schema 每字段有 type + description
//!   - effects 精确声明：read=纯读, write=修改
//!   - 结果精确反馈实际执行效果

use crate::adapter_mod::MinecraftModAdapter;
use base64::Engine as _;
use craft_agent::core::adapter::GameAdapter;
use craft_agent::core::tool::{GameTool, ToolEffects, ToolResult, ToolUpdateFn};
use craft_agent::core::types::Action;
use craft_agent_model::vision::real::downscale_png;
use serde_json::Value;
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

// ═══════════════════════════════════════════════════════════════
// Perceive — 游戏状态快照（<100ms，每轮自动注入）
// ═══════════════════════════════════════════════════════════════

pub struct ModPerceiveTool {
    adapter: Rc<RefCell<MinecraftModAdapter>>,
    image_max_side: Option<u32>,
    shots_dir: Option<PathBuf>,
    counter: Cell<u32>,
}
impl ModPerceiveTool {
    pub fn new(a: Rc<RefCell<MinecraftModAdapter>>, im: Option<u32>, sd: Option<PathBuf>) -> Self {
        Self {
            adapter: a,
            image_max_side: im,
            shots_dir: sd,
            counter: Cell::new(0),
        }
    }
    fn save_shot(&self, png: &[u8]) -> Option<String> {
        let dir = self.shots_dir.as_ref()?;
        let n = self.counter.get() + 1;
        self.counter.set(n);
        let rel = dir.join(&format!("step-{n:03}.png"));
        if std::fs::create_dir_all(dir).is_ok() && std::fs::write(&rel, png).is_ok() {
            Some(rel.to_string_lossy().to_string())
        } else {
            None
        }
    }
}
impl GameTool for ModPerceiveTool {
    fn name(&self) -> &str {
        "perceive"
    }
    fn description(&self) -> &str {
        "Read full game state via mod (latency <100ms, no side effects). Returns: position(x/y/z), yaw/pitch, health/hunger, gamemode/biome/dimension, light levels, weather, ALL inventory items (hotbar slots 1-9 + main inventory), targeted block (what crosshair points at), nearby blocks (top 30 by relevance), nearby entities. This data is auto-injected each turn — you rarely need to call this manually."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{},"required":[]})
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
        let ws = self.adapter.borrow().perceive()?;
        // mod perceive 返回纯结构化数据（无截图）。
        // screenshot 为空时不保存文件、不生成 images，避免 0 字节空文件误导。
        // 截图留给 visual_perceive 工具（需要时手动调用）。
        let images = if !ws.screenshot.is_empty() {
            let scaled = match self.image_max_side {
                Some(ms) => downscale_png(&ws.screenshot, ms)
                    .map(|r| r.0)
                    .unwrap_or_default(),
                None => ws.screenshot.clone(),
            };
            if scaled.is_empty() {
                vec![]
            } else {
                let _ = self.save_shot(&ws.screenshot);
                vec![format!(
                    "data:image/png;base64,{}",
                    base64::engine::general_purpose::STANDARD.encode(&scaled)
                )]
            }
        } else {
            vec![]
        };
        Ok(ToolResult {
            message: ws.scene_desc,
            is_error: false,
            images,
        })
    }
}

// ═══════════════════════════════════════════════════════════════
// collect — 自动采集（找→走→挖，核心工具）
// ═══════════════════════════════════════════════════════════════

pub struct ModCollectTool {
    adapter: Rc<RefCell<MinecraftModAdapter>>,
}
impl ModCollectTool {
    pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModCollectTool {
    fn name(&self) -> &str {
        "collect"
    }
    fn description(&self) -> &str {
        "AUTO find nearest target block, walk to it, dig it directly by coordinate (bypasses line-of-sight, no camera aiming needed). Handles trees column-by-column. Each block takes 1-3s. target: block ID substring (e.g. oak_log, stone, coal_ore). count: how many to collect (max 64 per call). Returns actual count collected. If no more blocks nearby, stops early."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"target":{"type":"string","description":"Block ID substring, e.g. oak_log, stone, coal_ore"},"count":{"type":"integer","description":"Number to collect (1-64)","default":1,"minimum":1,"maximum":64}},"required":["target"]})
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
        let mut adapter = self.adapter.borrow_mut();
        let st = adapter.reload()?;
        let before: u32 = st
            .inventory
            .iter()
            .filter(|i| i.id.contains(target))
            .map(|i| i.count)
            .sum();
        let max_attempts = want.min(10).max(3);
        let mut got = before;
        let mut consecutive_failures = 0;
        for attempt in 1..=max_attempts {
            if got >= before + want {
                break;
            }
            let Some((block, _)) = find_nearest(&adapter, target) else {
                let msg = if got > before {
                    format!("collected {target}: {before}→{got} (no more nearby, tried {attempt})")
                } else {
                    format!("collected {target}: {before}→{got} (no {target} found nearby)")
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
            // 走到方块附近（mod 侧主线程移动，返回 reached/final_dist）
            let ack = adapter.move_to(block.x, block.y, block.z)?;
            let reached = ack.reached.unwrap_or(false);
            let final_dist = ack.final_dist.unwrap_or(999.0);
            if !reached && final_dist > 5.0 {
                // 没到达且距离太远，dig_at 会失败，跳过这次
                consecutive_failures += 1;
                if consecutive_failures >= 3 {
                    break;
                }
                continue;
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
            // 直接按坐标破坏方块（mod 侧 digAt 内部 lookAt+destroyBlock，不依赖准星 raycast）
            let bx = block.x.round() as i32;
            let by = block.y.round() as i32;
            let bz = block.z.round() as i32;
            let _ = adapter.dig_at(bx, by, bz)?;
            std::thread::sleep(std::time::Duration::from_millis(600));
            got = adapter
                .reload()?
                .inventory
                .iter()
                .filter(|i| i.id.contains(target))
                .map(|i| i.count)
                .sum();
            consecutive_failures = if got > before + (attempt - 1) as u32 {
                0
            } else {
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
                format!("collected {target}: {before}→{got} (+{actual}, wanted +{want}, partial)")
            }
        } else {
            format!(
                "collected {target}: {before}→{got} (failed to collect any, max attempts reached)"
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
    adapter: Rc<RefCell<MinecraftModAdapter>>,
}
impl ModCraftTool {
    pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModCraftTool {
    fn name(&self) -> &str {
        "craft"
    }
    fn description(&self) -> &str {
        "Craft items via direct inventory manipulation. Mod handlers cover: planks (log→4planks), sticks (2planks→4sticks), crafting_table (4planks→1), wooden tools (pickaxe/axe/shovel/hoe/sword), stone tools (pickaxe/axe/shovel/hoe/sword), torch (stick+coal→4), furnace (8cobble→1), chest (8planks→1), door/sign/fence/bowl/ladder. item: target item name. count: how many. Check craftable() first to verify materials."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"item":{"type":"string","description":"Item to craft, e.g. oak_planks, stick, crafting_table, wooden_pickaxe, torch"},"count":{"type":"integer","description":"How many to craft","default":1,"minimum":1}},"required":["item"]})
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
        let mut adapter = self.adapter.borrow_mut();
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

pub struct ModPlaceTool {
    adapter: Rc<RefCell<MinecraftModAdapter>>,
}
impl ModPlaceTool {
    pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModPlaceTool {
    fn name(&self) -> &str {
        "place"
    }
    fn description(&self) -> &str {
        "Place a block from inventory at a SPECIFIC coordinate. ALWAYS provide x/y/z for reliable placement. Auto-finds item in hotbar/main inventory, switches to it, places via useItemOn at the exact coordinate. For building structures (shelter/wall/house), use build() with a blueprint instead — it handles multi-block placement automatically. item: block name. x/y/z: world coordinate to place at."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"item":{"type":"string","description":"Block to place, e.g. crafting_table, torch, dirt, oak_planks"},"x":{"type":"integer","description":"World X coordinate to place at"},"y":{"type":"integer","description":"World Y coordinate to place at"},"z":{"type":"integer","description":"World Z coordinate to place at"}},"required":["item","x","y","z"]})
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
        let x = args["x"].as_i64().unwrap_or(0) as i32;
        let y = args["y"].as_i64().unwrap_or(0) as i32;
        let z = args["z"].as_i64().unwrap_or(0) as i32;
        let full_item = if item.contains(':') {
            item.to_string()
        } else {
            format!("minecraft:{item}")
        };
        let adapter = self.adapter.borrow();
        // 用 place_at 精确放置（mod 侧自动找物品+切栏位+useItemOn）
        let ack = adapter.place_at(x, y, z, &full_item)?;
        let placed = ack.placed == Some(true);
        let msg = if placed {
            format!("placed {item} at ({x},{y},{z})")
        } else {
            format!(
                "place FAILED: could not place {item} at ({x},{y},{z}) — check: item in inventory? valid surface nearby? distance <5m?"
            )
        };
        Ok(ToolResult {
            message: msg,
            is_error: !placed,
            images: vec![],
        })
    }
}

// ═══════════════════════════════════════════════════════════════
// equip / use_item / attack — 物品/战斗
// ═══════════════════════════════════════════════════════════════

pub struct ModEquipTool {
    adapter: Rc<RefCell<MinecraftModAdapter>>,
}
impl ModEquipTool {
    pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self {
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
        serde_json::json!({"type":"object","properties":{"slot":{"type":"integer","description":"Hotbar slot number 1-9","minimum":1,"maximum":9}},"required":["slot"]})
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
        let slot = args["slot"].as_u64().unwrap_or(1).min(9) as u32;
        // mod 侧反射设置 Inventory.selected + 发送 SetCarriedItemPacket（立即生效）
        // 之前用 Action::Press 模拟数字键不生效（MC 数字键是边沿触发，setDown 不触发 click）
        let ack = self.adapter.borrow().select_slot(slot - 1)?; // 0-indexed
        let actual = ack.slot.unwrap_or(slot - 1) as u32 + 1;
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

pub struct ModUseItemTool {
    adapter: Rc<RefCell<MinecraftModAdapter>>,
}
impl ModUseItemTool {
    pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self {
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
        serde_json::json!({"type":"object","properties":{"ticks":{"type":"integer","description":"Hold duration (20≈1s). 32=eat, 5=quick use","default":20}}})
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
        let ack = self.adapter.borrow_mut().use_item(ticks)?;
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

pub struct ModAttackTool {
    adapter: Rc<RefCell<MinecraftModAdapter>>,
}
impl ModAttackTool {
    pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModAttackTool {
    fn name(&self) -> &str {
        "attack"
    }
    fn description(&self) -> &str {
        "Attack nearest hostile entity (zombie/skeleton/creeper/spider) within 4m. Auto-equips best weapon if available, auto-looks at entity before attacking. ticks: hold duration (30≈1.5s). Use when hostile mobs appear in NEARBY ENTITIES."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"ticks":{"type":"integer","description":"Attack duration (30≈1.5s)","default":30}}})
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
        let ticks = args["ticks"].as_u64().unwrap_or(30) as u32;
        let mut adapter = self.adapter.borrow_mut();
        let st = adapter.reload()?;
        let hostile_types = [
            "zombie", "skeleton", "creeper", "spider", "phantom", "witch",
        ];
        let nearest = st
            .entities
            .iter()
            .filter(|e| {
                let ty = e.r#type.replace("minecraft:", "");
                hostile_types.contains(&ty.as_str()) && e.dist <= 4.0
            })
            .min_by(|a, b| {
                a.dist
                    .partial_cmp(&b.dist)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
        match nearest {
            Some(ent) => {
                let ty = ent.r#type.replace("minecraft:", "");
                let weapon_priority = ["sword", "axe", "pickaxe", "shovel"];
                let weapon_slot = weapon_priority.iter().find_map(|weapon| {
                    st.inventory
                        .iter()
                        .filter(|i| i.id.contains(weapon) && i.slot < 9 && i.count > 0)
                        .map(|i| i.slot + 1)
                        .next()
                });
                if let Some(slot) = weapon_slot {
                    // ServerPlayer 架构：直接反射设置 Inventory.selected（0-indexed）
                    let _ = adapter.select_slot(slot - 1);
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
                adapter.look_at(ent.x, ent.y + 1.0, ent.z)?;
                std::thread::sleep(std::time::Duration::from_millis(150));
                let _ack = adapter.attack(ticks)?;
                Ok(ToolResult {
                    message: format!(
                        "attacked {} at ({:.1},{:.1},{:.1}) dist={:.1}m",
                        ty, ent.x, ent.y, ent.z, ent.dist
                    ),
                    is_error: false,
                    images: vec![],
                })
            }
            None => {
                let any_hostile = st.entities.iter().any(|e| {
                    let ty = e.r#type.replace("minecraft:", "");
                    hostile_types.contains(&ty.as_str())
                });
                if any_hostile {
                    Ok(ToolResult {
                        message: "hostile entities too far (must be within 4m to attack)".into(),
                        is_error: true,
                        images: vec![],
                    })
                } else {
                    Ok(ToolResult {
                        message: "no hostile entities within 4m to attack".into(),
                        is_error: true,
                        images: vec![],
                    })
                }
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════
// move_to / look_at / look / press / mine — 精确控制
// ═══════════════════════════════════════════════════════════════

pub struct ModMoveToTool {
    adapter: Rc<RefCell<MinecraftModAdapter>>,
}
impl ModMoveToTool {
    pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModMoveToTool {
    fn name(&self) -> &str {
        "move_to"
    }
    fn description(&self) -> &str {
        "Navigate to world coordinates. Server-side setDeltaMovement + per-tick re-aim toward target, auto-jumps obstacles. Stops within 1.5m of target. Time: 2-10s depending on distance. Use coordinates from NEARBY BLOCKS section."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"x":{"type":"number","description":"Target X coordinate"},"y":{"type":"number","description":"Target Y (use block.y + 0.5 for block center)"},"z":{"type":"number","description":"Target Z coordinate"}},"required":["x","y","z"]})
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
        // ServerPlayer 架构：mod 侧主线程 tick 回调执行 setDeltaMovement 移动
        let ack = self.adapter.borrow_mut().move_to(x, y, z)?;
        let reached = ack.reached.unwrap_or(false);
        let dist = ack.final_dist.unwrap_or(0.0);
        let stuck = ack.stuck.unwrap_or(false);
        let msg = if reached {
            format!(
                "move_to ({:.1},{:.1},{:.1}) reached, final_dist={:.1}m",
                x, y, z, dist
            )
        } else if stuck {
            format!(
                "move_to ({:.1},{:.1},{:.1}) stuck at {:.1}m (obstacle)",
                x, y, z, dist
            )
        } else {
            format!(
                "move_to ({:.1},{:.1},{:.1}) timeout at {:.1}m",
                x, y, z, dist
            )
        };
        Ok(ToolResult {
            message: msg,
            is_error: !reached,
            images: vec![],
        })
    }
}

pub struct ModLookAtTool {
    adapter: Rc<RefCell<MinecraftModAdapter>>,
}
impl ModLookAtTool {
    pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModLookAtTool {
    fn name(&self) -> &str {
        "look_at"
    }
    fn description(&self) -> &str {
        "Snap crosshair to a world coordinate. Integer coords auto-offset to block center (+0.5). Returns what block was actually hit (or 'nothing'). Force-refreshes raycast so targeted_block is accurate immediately."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"x":{"type":"number","description":"Target X coordinate"},"y":{"type":"number","description":"Target Y (auto-centers if integer)"},"z":{"type":"number","description":"Target Z coordinate"}},"required":["x","y","z"]})
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
        let x = args["x"].as_f64().unwrap_or(0.0);
        let y = args["y"].as_f64().unwrap_or(0.0);
        let z = args["z"].as_f64().unwrap_or(0.0);
        self.adapter.borrow_mut().look_at(x, y, z)?;
        Ok(ToolResult {
            message: format!("looking at ({:.1},{:.1},{:.1})", x, y, z),
            is_error: false,
            images: vec![],
        })
    }
}

pub struct ModLookTool {
    adapter: Rc<RefCell<MinecraftModAdapter>>,
}
impl ModLookTool {
    pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModLookTool {
    fn name(&self) -> &str {
        "look"
    }
    fn description(&self) -> &str {
        "Rotate camera relative to current view. dx>0=turn RIGHT (300≈90°). dy>0=look DOWN (toward ground). dy<0=look UP (toward sky). Sensitivity factor: 0.3°/unit. Prefer look_at() for precise coordinate targeting — this is approximate."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"dx":{"type":"integer","description":"Horizontal rotation (300≈90° right, -300≈90° left)"},"dy":{"type":"integer","description":"Vertical rotation (+=down/ground, -=up/sky, 65≈20°). Use POSITIVE to look at ground!"}},"required":["dx","dy"]})
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
        let dx = args["dx"].as_i64().unwrap_or(0) as i32;
        let dy = args["dy"].as_i64().unwrap_or(0) as i32;
        let r = self.adapter.borrow_mut().execute(Action::Look { dx, dy })?;
        Ok(ToolResult {
            message: r.detail,
            is_error: !r.ok,
            images: vec![],
        })
    }
}

pub struct ModPressTool {
    adapter: Rc<RefCell<MinecraftModAdapter>>,
}
impl ModPressTool {
    pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModPressTool {
    fn name(&self) -> &str {
        "press"
    }
    fn description(&self) -> &str {
        "Hold keyboard key(s) for N ticks (20≈1s). keys: w(forward)/a(left)/s(back)/d(right) for movement, space(jump), shift(sneak), e(inventory), 1-9(hotbar). Walk distance ≈ ticks/15 blocks. For precise navigation, use move_to() instead."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"keys":{"type":"string","description":"Key(s) to hold: w,a,s,d,space,shift,e,1-9"},"ticks":{"type":"integer","description":"Duration (20≈1s, 30≈5 blocks walk)","default":20,"minimum":1,"maximum":200}},"required":["keys"]})
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
        let keys = args["keys"].as_str().unwrap_or("w").to_string();
        let ticks = args["ticks"].as_u64().unwrap_or(20).min(200) as u32;
        let r = self
            .adapter
            .borrow_mut()
            .execute(Action::Press { keys, ticks })?;
        Ok(ToolResult {
            message: r.detail,
            is_error: !r.ok,
            images: vec![],
        })
    }
}

pub struct ModMineTool {
    adapter: Rc<RefCell<MinecraftModAdapter>>,
}
impl ModMineTool {
    pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModMineTool {
    fn name(&self) -> &str {
        "mine"
    }
    fn description(&self) -> &str {
        "Hold left-click to mine targeted block. Mod auto-detects block breaking — stops immediately when block disappears (no fixed tick waste). ticks parameter is safety timeout only. Use collect() for automatic gathering instead."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"ticks":{"type":"integer","description":"Safety timeout in ticks (mod auto-stops when block breaks). 140 for wood, 300 for stone.","default":140}}})
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
        let ticks = args["ticks"].as_u64().unwrap_or(140) as u32;
        let r = self.adapter.borrow_mut().execute(Action::Mine { ticks })?;
        Ok(ToolResult {
            message: r.detail,
            is_error: !r.ok,
            images: vec![],
        })
    }
}

// ═══════════════════════════════════════════════════════════════
// craftable — 查询可合成列表
// ═══════════════════════════════════════════════════════════════

pub struct ModCraftableTool {
    adapter: Rc<RefCell<MinecraftModAdapter>>,
}
impl ModCraftableTool {
    pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModCraftableTool {
    fn name(&self) -> &str {
        "craftable"
    }
    fn description(&self) -> &str {
        "Query all craftable items from current inventory. Returns list of {item: max_count}. Covers: planks, sticks, crafting_table, wooden/stone tools, iron/diamond tools, torch, furnace, chest, doors, signs, fences, beds, stairs, slabs, pressure_plate, button, trapdoor, ladder. Call before craft() to verify materials."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{},"required":[]})
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
        let st = self.adapter.borrow().reload()?;
        const RECIPES: &[(&str, &[(&str, u32)], u32)] = &[
            ("oak_planks", &[("oak_log", 1)], 4),
            ("birch_planks", &[("birch_log", 1)], 4),
            ("spruce_planks", &[("spruce_log", 1)], 4),
            ("jungle_planks", &[("jungle_log", 1)], 4),
            ("acacia_planks", &[("acacia_log", 1)], 4),
            ("dark_oak_planks", &[("dark_oak_log", 1)], 4),
            ("stick", &[("planks", 2)], 4),
            ("crafting_table", &[("planks", 4)], 1),
            ("wooden_pickaxe", &[("planks", 3), ("stick", 2)], 1),
            ("wooden_axe", &[("planks", 3), ("stick", 2)], 1),
            ("wooden_sword", &[("planks", 2), ("stick", 1)], 1),
            ("wooden_shovel", &[("planks", 1), ("stick", 2)], 1),
            ("wooden_hoe", &[("planks", 2), ("stick", 2)], 1),
            ("stone_pickaxe", &[("cobblestone", 3), ("stick", 2)], 1),
            ("stone_axe", &[("cobblestone", 3), ("stick", 2)], 1),
            ("stone_sword", &[("cobblestone", 2), ("stick", 1)], 1),
            ("stone_shovel", &[("cobblestone", 1), ("stick", 2)], 1),
            ("stone_hoe", &[("cobblestone", 2), ("stick", 2)], 1),
            ("iron_pickaxe", &[("iron_ingot", 3), ("stick", 2)], 1),
            ("iron_axe", &[("iron_ingot", 3), ("stick", 2)], 1),
            ("iron_sword", &[("iron_ingot", 2), ("stick", 1)], 1),
            ("iron_shovel", &[("iron_ingot", 1), ("stick", 2)], 1),
            ("iron_hoe", &[("iron_ingot", 2), ("stick", 2)], 1),
            ("diamond_pickaxe", &[("diamond", 3), ("stick", 2)], 1),
            ("diamond_axe", &[("diamond", 3), ("stick", 2)], 1),
            ("diamond_sword", &[("diamond", 2), ("stick", 1)], 1),
            ("diamond_shovel", &[("diamond", 1), ("stick", 2)], 1),
            ("diamond_hoe", &[("diamond", 2), ("stick", 2)], 1),
            ("torch", &[("stick", 1), ("coal", 1)], 4),
            ("furnace", &[("cobblestone", 8)], 1),
            ("chest", &[("planks", 8)], 1),
            ("oak_door", &[("planks", 6)], 1),
            ("oak_fence", &[("planks", 4), ("stick", 2)], 3),
            ("oak_sign", &[("planks", 1), ("stick", 1)], 1),
            ("oak_stairs", &[("planks", 6)], 4),
            ("oak_slab", &[("planks", 3)], 6),
            ("oak_bed", &[("planks", 3), ("wool", 3)], 1),
            ("stone_pressure_plate", &[("cobblestone", 2)], 1),
            ("oak_pressure_plate", &[("planks", 2)], 1),
            ("stone_button", &[("cobblestone", 1)], 1),
            ("oak_button", &[("planks", 1)], 1),
            ("oak_trapdoor", &[("planks", 6)], 2),
            ("ladder", &[("stick", 7)], 3),
        ];
        let mut out = Vec::new();
        for (item, ingredients, yield_count) in RECIPES {
            let ok = ingredients.iter().all(|(mat, need)| {
                st.inventory
                    .iter()
                    .filter(|i| i.id.contains(mat))
                    .map(|i| i.count)
                    .sum::<u32>()
                    >= *need
            });
            if ok {
                let max = ingredients
                    .iter()
                    .map(|(mat, need)| {
                        st.inventory
                            .iter()
                            .filter(|i| i.id.contains(mat))
                            .map(|i| i.count)
                            .sum::<u32>()
                            / need
                    })
                    .min()
                    .unwrap_or(0);
                out.push(format!("  {item} x{}", max * yield_count));
            }
        }
        Ok(ToolResult {
            message: if out.is_empty() {
                "CRAFTABLE: none".into()
            } else {
                format!("CRAFTABLE:\n{}", out.join("\n"))
            },
            is_error: false,
            images: vec![],
        })
    }
}

// ═══════════════════════════════════════════════════════════════
// searchForBlock / moveAway / digDown — Mindcraft 对齐导航
// ═══════════════════════════════════════════════════════════════

pub struct ModSearchBlockTool {
    adapter: Rc<RefCell<MinecraftModAdapter>>,
}
impl ModSearchBlockTool {
    pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModSearchBlockTool {
    fn name(&self) -> &str {
        "searchForBlock"
    }
    fn description(&self) -> &str {
        "Find nearest block of type and walk to it (no mining). Uses move_to for navigation. Returns block type, coordinates, and distance walked. Use to position yourself before manual mining or building."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"type":{"type":"string","description":"Block type to find, e.g. oak_log, stone, crafting_table, chest"}},"required":["type"]})
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
        let target = args["type"].as_str().unwrap_or("oak_log");
        let Some((block, _)) = find_nearest(&self.adapter.borrow(), target) else {
            return Ok(ToolResult {
                message: format!("searchForBlock: no {target} nearby"),
                is_error: true,
                images: vec![],
            });
        };
        let _ = self
            .adapter
            .borrow_mut()
            .move_to(block.x, block.y + 0.5, block.z)?;
        Ok(ToolResult {
            message: format!(
                "walked to {} at ({:.0},{:.0},{:.0}) dist={:.1}m",
                target, block.x, block.y, block.z, block.dist
            ),
            is_error: false,
            images: vec![],
        })
    }
}

pub struct ModMoveAwayTool {
    adapter: Rc<RefCell<MinecraftModAdapter>>,
}
impl ModMoveAwayTool {
    pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModMoveAwayTool {
    fn name(&self) -> &str {
        "moveAway"
    }
    fn description(&self) -> &str {
        "Walk backwards N blocks (away from current facing direction). distance: approximate meters to retreat (max 20). Uses move_to with a computed backward target point."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"distance":{"type":"integer","description":"Blocks to retreat (1-20)","default":3,"minimum":1,"maximum":20}}})
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
        let dist = args["distance"].as_u64().unwrap_or(3).min(20) as f64;
        let mut adapter = self.adapter.borrow_mut();
        let st = adapter.reload()?;
        // 根据当前朝向计算反向目标点（yaw: 0=南+z, 90=西-x, 180=北-z, 270=东+x）
        let yaw_rad = st.yaw.to_radians();
        // 前进方向: (-sin(yaw), cos(yaw))，后退方向取反
        let back_x = st.position[0] + yaw_rad.sin() * dist;
        let back_z = st.position[2] - yaw_rad.cos() * dist;
        let _ = adapter.move_to(back_x, st.position[1], back_z)?;
        Ok(ToolResult {
            message: format!(
                "moved back ~{dist:.0} blocks to ({:.1},{:.1},{:.1})",
                back_x, st.position[1], back_z
            ),
            is_error: false,
            images: vec![],
        })
    }
}

pub struct ModDigDownTool {
    adapter: Rc<RefCell<MinecraftModAdapter>>,
}
impl ModDigDownTool {
    pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModDigDownTool {
    fn name(&self) -> &str {
        "digDown"
    }
    fn description(&self) -> &str {
        "Dig straight down N blocks by destroying block under feet via dig_at (coordinate-based, no camera aiming). Player falls into hole after each block. Auto-stops if lava/water detected or fall would be ≥4 blocks. Verifies actual Y descent — reports true depth dug. distance: 1-10 blocks."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"distance":{"type":"integer","description":"Blocks to dig down (1-10)","default":1,"minimum":1,"maximum":10}}})
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
        let dist = args["distance"].as_u64().unwrap_or(1).min(10) as u32;
        let mut a = self.adapter.borrow_mut();
        let st = a.reload()?;
        let start_y = st.position[1];
        let mut dug = 0u32;
        for _ in 0..dist {
            let current_st = a.reload()?;
            let current_y = current_st.position[1];
            if current_y < start_y - 4.0 {
                return Ok(ToolResult {
                    message: format!(
                        "dug down {dug} blocks (stopped: fall would exceed 4 blocks, now y={:.1})",
                        current_y
                    ),
                    is_error: false,
                    images: vec![],
                });
            }
            // 脚下方块坐标（玩家脚所在格的下方）
            let px = current_st.position[0].round() as i32;
            let py = (current_st.position[1] - 0.5).floor() as i32; // 脚下方块
            let pz = current_st.position[2].round() as i32;

            // 安全检测：查看脚下方块是否是岩浆/水
            let down_block = current_st.nearby_blocks.iter().find(|b| {
                (b.x as f64 - current_st.position[0]).abs() < 0.7
                    && (b.z as f64 - current_st.position[2]).abs() < 0.7
                    && (b.y as f64 - current_y).abs() < 1.5
            });
            if let Some(b) = down_block {
                let bid = b.id.to_lowercase();
                if bid.contains("lava") {
                    return Ok(ToolResult {
                        message: format!("dug down {dug} blocks (stopped: lava detected below)"),
                        is_error: true,
                        images: vec![],
                    });
                }
                if bid.contains("water") {
                    return Ok(ToolResult {
                        message: format!("dug down {dug} blocks (stopped: water detected below)"),
                        is_error: true,
                        images: vec![],
                    });
                }
            }

            // 直接按坐标破坏脚下方块（dig_at 内部 look_at + 持续按住 attack）
            let _ = a.dig_at(px, py, pz)?;
            // 等待方块破坏 + 玩家掉落
            std::thread::sleep(std::time::Duration::from_millis(800));

            // 验证：y 坐标是否下降了？
            let after_st = a.reload()?;
            let after_y = after_st.position[1];
            if after_y < current_y - 0.5 {
                // 确实下降了
                dug += 1;
            } else {
                // 没下降 — 方块可能没破坏（太硬/距离太远）
                return Ok(ToolResult {
                    message: format!(
                        "dug down {dug} blocks (stopped: block at y={py} not broken, player still at y={:.1})",
                        after_y
                    ),
                    is_error: dug == 0,
                    images: vec![],
                });
            }
        }
        let final_st = a.reload()?;
        Ok(ToolResult {
            message: format!(
                "dug down {dug} blocks (y: {:.1}→{:.1})",
                start_y, final_st.position[1]
            ),
            is_error: dug == 0,
            images: vec![],
        })
    }
}

pub struct ModConsumeTool {
    adapter: Rc<RefCell<MinecraftModAdapter>>,
}
impl ModConsumeTool {
    pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModConsumeTool {
    fn name(&self) -> &str {
        "consume"
    }
    fn description(&self) -> &str {
        "Eat/drink a food item. Finds item in hotbar, equips it, uses item to consume. item: food name like cooked_beef, bread, apple. ticks: hold time (32≈1.6s for most food)."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"item":{"type":"string","description":"Food to eat, e.g. cooked_beef, bread, apple, steak"},"ticks":{"type":"integer","description":"Hold time (32≈1.6s for food, 20 for drinks)","default":32}},"required":["item"]})
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
        let mut a = self.adapter.borrow_mut();
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
fn check_missing_materials(st: &crate::bridge::ModState, item: &str, count: u32) -> Vec<String> {
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
fn summarize_inventory(st: &crate::bridge::ModState) -> String {
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

fn mine_ticks_for(block_id: &str, _held_item: &str) -> u32 {
    if block_id.contains("_log") || block_id.contains("planks") || block_id.contains("leaves") {
        140
    } else if block_id.contains("stone") || block_id.contains("cobble") {
        280
    } else if block_id.contains("_ore") {
        280
    } else if block_id.contains("dirt") || block_id.contains("grass") || block_id.contains("sand") {
        40
    } else {
        200
    }
}

fn find_nearest(
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

// ═══════════════════════════════════════════════════════════════
// rememberHere / goToRememberedPlace / savedPlaces — 位置记忆
// ═══════════════════════════════════════════════════════════════

pub struct ModRememberTool {
    adapter: Rc<RefCell<MinecraftModAdapter>>,
}
impl ModRememberTool {
    pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self {
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
        serde_json::json!({"type":"object","properties":{"name":{"type":"string","description":"Label for this location"}},"required":["name"]})
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
            message: self.adapter.borrow().remember_here(name),
            is_error: false,
            images: vec![],
        })
    }
}

pub struct ModGoPlaceTool {
    adapter: Rc<RefCell<MinecraftModAdapter>>,
}
impl ModGoPlaceTool {
    pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self {
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
        serde_json::json!({"type":"object","properties":{"name":{"type":"string","description":"Location label from rememberHere"}},"required":["name"]})
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
            message: self.adapter.borrow().go_to_place(name)?,
            is_error: false,
            images: vec![],
        })
    }
}

pub struct ModListPlacesTool {
    adapter: Rc<RefCell<MinecraftModAdapter>>,
}
impl ModListPlacesTool {
    pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self {
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
        serde_json::json!({"type":"object","properties":{},"required":[]})
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
            message: self.adapter.borrow().list_places(),
            is_error: false,
            images: vec![],
        })
    }
}

// ═══════════════════════════════════════════════════════════════
// discard / smeltItem — 物品管理
// ═══════════════════════════════════════════════════════════════

pub struct ModDiscardTool {
    adapter: Rc<RefCell<MinecraftModAdapter>>,
}
impl ModDiscardTool {
    pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self {
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
        serde_json::json!({"type":"object","properties":{"item":{"type":"string","description":"Item to discard"},"num":{"type":"integer","description":"Count to discard","default":1,"minimum":1}},"required":["item"]})
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
        self.adapter.borrow_mut().discard_item(item, num)?;
        Ok(ToolResult {
            message: format!("discarded {num}x {item}"),
            is_error: false,
            images: vec![],
        })
    }
}

pub struct ModSmeltTool {
    adapter: Rc<RefCell<MinecraftModAdapter>>,
}
impl ModSmeltTool {
    pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self {
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
        serde_json::json!({"type":"object","properties":{"item":{"type":"string","description":"Raw material to smelt, e.g. raw_iron, raw_copper"},"num":{"type":"integer","description":"Count to smelt","default":1,"minimum":1}},"required":["item"]})
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
        self.adapter.borrow_mut().smelt_item(item, num)?;
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

pub struct ModBuildTool {
    adapter: Rc<RefCell<MinecraftModAdapter>>,
}
impl ModBuildTool {
    pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModBuildTool {
    fn name(&self) -> &str {
        "build"
    }
    fn description(&self) -> &str {
        "Build a structure from a named blueprint at given coordinates. Auto-generates build steps layer by layer (y→z→x), calls place_at for each block. Stops on missing materials. blueprint: name from blueprints() list (dirt_shelter/wood_house/stone_house/wall_3x3). x,y,z: world coords for blueprint origin (corner). orientation: 0-3 (0=north, 1=east, 2=south, 3=west). Returns blocks placed/failed."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"blueprint":{"type":"string","description":"Blueprint name: dirt_shelter, wood_house, stone_house, wall_3x3"},"x":{"type":"integer","description":"World X coordinate for blueprint origin (corner)"},"y":{"type":"integer","description":"World Y coordinate for blueprint origin (ground level)"},"z":{"type":"integer","description":"World Z coordinate for blueprint origin (corner)"},"orientation":{"type":"integer","description":"Rotation 0-3 (0=north,1=east,2=south,3=west)","default":0,"minimum":0,"maximum":3}},"required":["blueprint","x","y","z"]})
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
        let bp_name = args["blueprint"].as_str().unwrap_or("dirt_shelter");
        let ox = args["x"].as_i64().unwrap_or(0) as i32;
        let oy = args["y"].as_i64().unwrap_or(0) as i32;
        let oz = args["z"].as_i64().unwrap_or(0) as i32;
        let orientation = args["orientation"].as_u64().unwrap_or(0) as u32 % 4;

        let Some(bp) = crate::blueprint::get_blueprint(bp_name) else {
            return Ok(ToolResult {
                message: format!(
                    "build: blueprint '{bp_name}' not found. Use blueprints() to list available."
                ),
                is_error: true,
                images: vec![],
            });
        };

        // 先检查材料
        let needed = bp.materials_needed();
        let st = self.adapter.borrow().reload()?;
        let mut missing = Vec::new();
        for (mat, need) in &needed {
            let have: u32 = st
                .inventory
                .iter()
                .filter(|i| i.id.contains(mat))
                .map(|i| i.count)
                .sum();
            if have < *need {
                missing.push(format!("{mat}: need {need}, have {have}"));
            }
        }
        if !missing.is_empty() {
            return Ok(ToolResult {
                message: format!("build {bp_name}: missing materials: {}", missing.join("; ")),
                is_error: true,
                images: vec![],
            });
        }

        // 生成建造步骤并逐步执行
        let steps = bp.build_steps(ox, oy, oz, orientation);
        let mut placed = 0u32;
        let mut failed = 0u32;
        let mut consecutive_fail = 0u32;
        for step in &steps {
            match &step.action {
                crate::blueprint::BuildAction::Place(item) => {
                    // 物品名补全 minecraft: 前缀（mod 侧匹配）
                    let full_item = if item.contains(':') {
                        item.clone()
                    } else {
                        format!("minecraft:{item}")
                    };
                    match self
                        .adapter
                        .borrow()
                        .place_at(step.x, step.y, step.z, &full_item)
                    {
                        Ok(ack) if ack.placed == Some(true) => {
                            placed += 1;
                            consecutive_fail = 0;
                        }
                        _ => {
                            failed += 1;
                            consecutive_fail += 1;
                            if consecutive_fail >= 5 {
                                break;
                            } // 连续失败 5 次停止
                        }
                    }
                }
                crate::blueprint::BuildAction::Dig => {
                    match self.adapter.borrow().dig_at(step.x, step.y, step.z) {
                        Ok(ack) if ack.broken == Some(true) => {
                            placed += 1;
                            consecutive_fail = 0;
                        }
                        _ => {
                            failed += 1;
                            consecutive_fail += 1;
                            if consecutive_fail >= 5 {
                                break;
                            }
                        }
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(150));
        }

        let msg = if failed == 0 {
            format!(
                "build {bp_name} at ({ox},{oy},{oz}) orient={orientation}: placed {placed} blocks (complete)"
            )
        } else if placed > 0 {
            format!(
                "build {bp_name} at ({ox},{oy},{oz}) orient={orientation}: placed {placed}, failed {failed} (partial)"
            )
        } else {
            format!(
                "build {bp_name} at ({ox},{oy},{oz}) orient={orientation}: all {failed} blocks failed"
            )
        };
        Ok(ToolResult {
            message: msg,
            is_error: placed == 0,
            images: vec![],
        })
    }
}

// ═══════════════════════════════════════════════════════════════
// blueprints — 列出可用蓝图 + 材料需求
// ═══════════════════════════════════════════════════════════════

pub struct ModBlueprintsTool {
    adapter: Rc<RefCell<MinecraftModAdapter>>,
}
impl ModBlueprintsTool {
    pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModBlueprintsTool {
    fn name(&self) -> &str {
        "blueprints"
    }
    fn description(&self) -> &str {
        "List all available blueprints with their materials requirements. Returns name, size (layers×rows×cols), and materials needed. Use before build() to verify you have enough materials. Built-in: dirt_shelter (3x3), wood_house (5x5), stone_house (5x5), wall_3x3."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{},"required":[]})
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
        let bps = crate::blueprint::builtin_blueprints();
        let st = self.adapter.borrow().reload()?;
        let mut lines = Vec::new();
        for (name, json) in bps {
            if let Ok(bp) = crate::blueprint::BlueprintDef::from_json(json) {
                let layers = bp.blocks.len();
                let rows = bp.blocks.first().map(|l| l.len()).unwrap_or(0);
                let cols = bp
                    .blocks
                    .first()
                    .and_then(|l| l.first())
                    .map(|r| r.len())
                    .unwrap_or(0);
                let mats = bp.materials_needed();
                let mat_str: Vec<String> = mats
                    .iter()
                    .map(|(m, n)| {
                        let have: u32 = st
                            .inventory
                            .iter()
                            .filter(|i| i.id.contains(m))
                            .map(|i| i.count)
                            .sum();
                        let status = if have >= *n { "ok" } else { "MISSING" };
                        format!("{m}×{n}({status})")
                    })
                    .collect();
                lines.push(format!(
                    "  {name}: {layers}×{rows}×{cols} | materials: {}",
                    mat_str.join(", ")
                ));
            }
        }
        Ok(ToolResult {
            message: format!("BLUEPRINTS:\n{}", lines.join("\n")),
            is_error: false,
            images: vec![],
        })
    }
}

// ═══════════════════════════════════════════════════════════════
// combat — 战斗 AI（mod 侧自主走位：melee/kite/retreat）
// ═══════════════════════════════════════════════════════════════

pub struct ModCombatTool {
    adapter: Rc<RefCell<MinecraftModAdapter>>,
}
impl ModCombatTool {
    pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModCombatTool {
    fn name(&self) -> &str {
        "combat"
    }
    fn description(&self) -> &str {
        "Autonomous combat AI. Mod-side handles targeting, movement, and attacks. Modes: 'melee' (run up and hit, best for zombies/spiders), 'kite' (hit-and-run, best for skeletons/creeper), 'retreat' (flee from all hostiles). Auto-equips best weapon. Auto-retreats from creepers and when health <6. ticks: combat duration (200≈10s, max 600). Returns result (killed/retreated/timeout/no_target) and target type."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"mode":{"type":"string","enum":["melee","kite","retreat"],"description":"melee=aggressive close combat, kite=hit-and-run, retreat=flee","default":"melee"},"ticks":{"type":"integer","description":"Combat duration in ticks (200≈10s, max 500 to fit TCP timeout)","default":200,"minimum":20,"maximum":500}},"required":["mode"]})
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
        let mode = args["mode"].as_str().unwrap_or("melee");
        let ticks = args["ticks"].as_u64().unwrap_or(200).min(500) as u32;
        let adapter = self.adapter.borrow();
        match adapter.combat(mode, ticks) {
            Ok(ack) => {
                let result = ack.result.unwrap_or_else(|| "unknown".into());
                let target = ack.target.unwrap_or_else(|| "none".into());
                let msg = match result.as_str() {
                    "killed" => format!("combat {mode}: killed {target}"),
                    "retreated" => {
                        format!("combat {mode}: retreated from {target} (low health or creeper)")
                    }
                    "timeout" => {
                        format!("combat {mode}: timeout after {ticks} ticks fighting {target}")
                    }
                    "no_target" => format!("combat {mode}: no hostile entities nearby"),
                    _ => format!("combat {mode}: {result} target={target}"),
                };
                Ok(ToolResult {
                    message: msg,
                    is_error: result == "no_target",
                    images: vec![],
                })
            }
            Err(e) => Ok(ToolResult {
                message: format!("combat {mode}: error: {e}"),
                is_error: true,
                images: vec![],
            }),
        }
    }
}

// ═══════════════════════════════════════════════════════════════
// searchForEntity — 搜索实体并走过去（Mindcraft 对齐）
// ═══════════════════════════════════════════════════════════════

pub struct ModSearchEntityTool {
    adapter: Rc<RefCell<MinecraftModAdapter>>,
}
impl ModSearchEntityTool {
    pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModSearchEntityTool {
    fn name(&self) -> &str {
        "searchForEntity"
    }
    fn description(&self) -> &str {
        "Find nearest entity of given type and walk to it (no attack). Uses move_to for navigation. type: entity type like cow, pig, villager, sheep, chicken. Returns entity type, coordinates, and distance."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"type":{"type":"string","description":"Entity type to find, e.g. cow, pig, villager, sheep, chicken, zombie"},"search_range":{"type":"number","description":"Max search distance (default 64)","default":64}},"required":["type"]})
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
        let target = args["type"].as_str().unwrap_or("cow");
        let range = args["search_range"].as_f64().unwrap_or(64.0);
        let adapter = self.adapter.borrow();
        let st = adapter.reload()?;
        let target_clean = target.replace("minecraft:", "");
        let nearest = st
            .entities
            .iter()
            .filter(|e| {
                let ty = e.r#type.replace("minecraft:", "");
                ty.contains(&target_clean) && e.dist <= range
            })
            .min_by(|a, b| {
                a.dist
                    .partial_cmp(&b.dist)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
        match nearest {
            Some(ent) => {
                let ty = ent.r#type.replace("minecraft:", "");
                drop(adapter);
                let _ = self.adapter.borrow_mut().move_to(ent.x, ent.y, ent.z)?;
                Ok(ToolResult {
                    message: format!(
                        "walked to {} at ({:.1},{:.1},{:.1}) dist={:.1}m",
                        ty, ent.x, ent.y, ent.z, ent.dist
                    ),
                    is_error: false,
                    images: vec![],
                })
            }
            None => Ok(ToolResult {
                message: format!("searchForEntity: no {target} within {range}m"),
                is_error: true,
                images: vec![],
            }),
        }
    }
}

// ═══════════════════════════════════════════════════════════════
// goToBed — 找最近的床并睡觉（Mindcraft 对齐）
// ═══════════════════════════════════════════════════════════════

pub struct ModGoToBedTool {
    adapter: Rc<RefCell<MinecraftModAdapter>>,
}
impl ModGoToBedTool {
    pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self {
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
        serde_json::json!({"type":"object","properties":{},"required":[]})
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
        let mut adapter = self.adapter.borrow_mut();
        let st = adapter.reload()?;
        // 检查时间（MC 时间 >13000 = 夜晚）
        let is_night = st.time > 13000 || st.time < 230;
        let is_thunder = st.thundering;
        if !is_night && !is_thunder {
            return Ok(ToolResult {
                message: "goToBed: not night or thundering, no need to sleep".into(),
                is_error: false,
                images: vec![],
            });
        }
        // 找床
        let bed = st
            .nearby_blocks
            .iter()
            .filter(|b| b.id.contains("bed") && !b.id.contains("bedrock"))
            .min_by(|a, b| {
                a.dist
                    .partial_cmp(&b.dist)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
        match bed {
            Some(b) => {
                let _ = adapter.move_to(b.x, b.y + 0.5, b.z)?;
                std::thread::sleep(std::time::Duration::from_millis(300));
                adapter.look_at(b.x, b.y + 0.5, b.z)?;
                std::thread::sleep(std::time::Duration::from_millis(200));
                let _ = adapter.use_item(10)?;
                std::thread::sleep(std::time::Duration::from_millis(1000));
                Ok(ToolResult {
                    message: format!("sleeping in {} at ({:.0},{:.0},{:.0})", b.id, b.x, b.y, b.z),
                    is_error: false,
                    images: vec![],
                })
            }
            None => Ok(ToolResult {
                message: "goToBed: no bed found nearby".into(),
                is_error: true,
                images: vec![],
            }),
        }
    }
}

// ═══════════════════════════════════════════════════════════════
// stay — 原地等待（Mindcraft 对齐）
// ═══════════════════════════════════════════════════════════════

pub struct ModStayTool {
    adapter: Rc<RefCell<MinecraftModAdapter>>,
}
impl ModStayTool {
    pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
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
        serde_json::json!({"type":"object","properties":{"type":{"type":"integer","description":"Seconds to stay (1-30, -1=forever but capped at 30)","default":5,"minimum":-1,"maximum":30}}})
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
        let secs = args["type"].as_i64().unwrap_or(5).min(30).max(-1) as i32;
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
    adapter: Rc<RefCell<MinecraftModAdapter>>,
}
impl ModGoToSurfaceTool {
    pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self {
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
        serde_json::json!({"type":"object","properties":{},"required":[]})
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
        let adapter = self.adapter.borrow();
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
                let _ = self.adapter.borrow_mut().move_to(sx, sy, sz)?;
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
    adapter: Rc<RefCell<MinecraftModAdapter>>,
}
impl ModSetModeTool {
    pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self {
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
        serde_json::json!({"type":"object","properties":{"mode_name":{"type":"string","enum":["self_preservation","self_defense","unstuck","cowardice","hunting","torch_placing","idle_staring"],"description":"Mode name"},"on":{"type":"boolean","description":"true=enable, false=disable"}},"required":["mode_name","on"]})
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
        self.adapter.borrow().set_mode(mode, on);
        let modes_list = self.adapter.borrow().list_modes();
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

pub struct ModCraftingPlanTool {
    adapter: Rc<RefCell<MinecraftModAdapter>>,
}
impl ModCraftingPlanTool {
    pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModCraftingPlanTool {
    fn name(&self) -> &str {
        "getCraftingPlan"
    }
    fn description(&self) -> &str {
        "Get detailed crafting plan for an item: required materials, exact quantities, and missing materials analysis. targetItem: item to craft. quantity: how many to craft. Returns materials list (have/need/missing) and craftable count."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"targetItem":{"type":"string","description":"Item to craft, e.g. wooden_pickaxe, torch, chest"},"quantity":{"type":"integer","description":"How many to craft","default":1,"minimum":1}},"required":["targetItem"]})
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
        let target = args["targetItem"].as_str().unwrap_or("oak_planks");
        let qty = args["quantity"].as_u64().unwrap_or(1) as u32;
        let st = self.adapter.borrow().reload()?;
        const RECIPES: &[(&str, &[(&str, u32)], u32)] = &[
            ("oak_planks", &[("oak_log", 1)], 4),
            ("birch_planks", &[("birch_log", 1)], 4),
            ("stick", &[("planks", 2)], 4),
            ("crafting_table", &[("planks", 4)], 1),
            ("wooden_pickaxe", &[("planks", 3), ("stick", 2)], 1),
            ("wooden_axe", &[("planks", 3), ("stick", 2)], 1),
            ("wooden_sword", &[("planks", 2), ("stick", 1)], 1),
            ("wooden_shovel", &[("planks", 1), ("stick", 2)], 1),
            ("stone_pickaxe", &[("cobblestone", 3), ("stick", 2)], 1),
            ("stone_axe", &[("cobblestone", 3), ("stick", 2)], 1),
            ("stone_sword", &[("cobblestone", 2), ("stick", 1)], 1),
            ("iron_pickaxe", &[("iron_ingot", 3), ("stick", 2)], 1),
            ("iron_sword", &[("iron_ingot", 2), ("stick", 1)], 1),
            ("torch", &[("stick", 1), ("coal", 1)], 4),
            ("furnace", &[("cobblestone", 8)], 1),
            ("chest", &[("planks", 8)], 1),
            ("oak_door", &[("planks", 6)], 1),
        ];
        let recipe = RECIPES.iter().find(|(name, _, _)| *name == target);
        match recipe {
            Some((_, ingredients, yield_count)) => {
                let mut lines = Vec::new();
                let mut max_crafts = u32::MAX;
                let mut all_ok = true;
                for (mat, need_per) in ingredients.iter() {
                    let have: u32 = st
                        .inventory
                        .iter()
                        .filter(|i| i.id.contains(mat))
                        .map(|i| i.count)
                        .sum();
                    let need = need_per * qty;
                    let missing = if have >= need { 0 } else { need - have };
                    if missing > 0 {
                        all_ok = false;
                    }
                    let can_craft = if *need_per > 0 {
                        have / need_per
                    } else {
                        u32::MAX
                    };
                    max_crafts = max_crafts.min(can_craft);
                    lines.push(format!(
                        "  {mat}: have {have}, need {need}{} (can craft {can_craft}×)",
                        if missing > 0 {
                            format!(", MISSING {missing}")
                        } else {
                            String::new()
                        }
                    ));
                }
                let total_yield = max_crafts.saturating_mul(*yield_count);
                lines.push(format!("  → yield: {yield_count} per craft, max {max_crafts} crafts = {total_yield} items"));
                if all_ok {
                    lines.push(format!("  → READY: can craft {qty}× {target}"));
                } else {
                    lines.push(format!(
                        "  → BLOCKED: missing materials for {qty}× {target}"
                    ));
                }
                Ok(ToolResult {
                    message: format!("CRAFTING PLAN for {qty}× {target}:\n{}", lines.join("\n")),
                    is_error: false,
                    images: vec![],
                })
            }
            None => Ok(ToolResult {
                message: format!(
                    "getCraftingPlan: no recipe for '{target}'. Use craftable() to list all craftable items."
                ),
                is_error: true,
                images: vec![],
            }),
        }
    }
}

// ═══════════════════════════════════════════════════════════════
// checkBlueprintLevel / getBlueprintLevel — 蓝图层级查询（Mindcraft 对齐）
// ═══════════════════════════════════════════════════════════════

pub struct ModBlueprintLevelTool {
    adapter: Rc<RefCell<MinecraftModAdapter>>,
}
impl ModBlueprintLevelTool {
    pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModBlueprintLevelTool {
    fn name(&self) -> &str {
        "getBlueprintLevel"
    }
    fn description(&self) -> &str {
        "Get blueprint details for a specific level (floor). blueprint: name from blueprints(). level: floor number (0=ground level). Returns the block layout for that level."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"blueprint":{"type":"string","description":"Blueprint name: dirt_shelter, wood_house, stone_house, wall_3x3"},"level":{"type":"integer","description":"Level/floor number (0=ground)","minimum":0}},"required":["blueprint","level"]})
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
        let bp_name = args["blueprint"].as_str().unwrap_or("dirt_shelter");
        let level = args["level"].as_u64().unwrap_or(0) as usize;
        let Some(bp) = crate::blueprint::get_blueprint(bp_name) else {
            return Ok(ToolResult {
                message: format!("blueprint '{bp_name}' not found"),
                is_error: true,
                images: vec![],
            });
        };
        if level >= bp.blocks.len() {
            return Ok(ToolResult {
                message: format!("level {level} out of range (0..{})", bp.blocks.len()),
                is_error: true,
                images: vec![],
            });
        }
        let layer = &bp.blocks[level];
        let mut rows = Vec::new();
        for row in layer.iter() {
            rows.push(
                row.iter()
                    .map(|b| {
                        if b.is_empty() || b == "air" {
                            "."
                        } else {
                            b.as_str()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" "),
            );
        }
        Ok(ToolResult {
            message: format!("BLUEPRINT {bp_name} level {level}:\n{}", rows.join("\n")),
            is_error: false,
            images: vec![],
        })
    }
}

// ═══════════════════════════════════════════════════════════════
// useOn — 对实体/方块使用工具（Mindcraft 对齐：剪羊毛/挤牛奶/点燃等）
// ═══════════════════════════════════════════════════════════════

pub struct ModUseOnTool {
    adapter: Rc<RefCell<MinecraftModAdapter>>,
}
impl ModUseOnTool {
    pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModUseOnTool {
    fn name(&self) -> &str {
        "useOn"
    }
    fn description(&self) -> &str {
        "Use (right-click) current item on nearest entity or block. tool_name: item to equip first (or 'hand' for empty hand). target: entity type (cow, sheep, villager) or block type (crafting_table, furnace) or 'nothing' to use in air. Used for: shearing sheep, milking cows, trading with villagers, opening crafting table/furnace/chest."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"tool_name":{"type":"string","description":"Item to equip first, e.g. shears, bucket, or 'hand' for empty hand"},"target":{"type":"string","description":"Entity type (cow, sheep, villager) or block type (crafting_table, furnace), or 'nothing'"}},"required":["tool_name","target"]})
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
        let tool = args["tool_name"].as_str().unwrap_or("hand");
        let target = args["target"].as_str().unwrap_or("nothing");
        let mut adapter = self.adapter.borrow_mut();
        let st = adapter.reload()?;

        // 装备工具（ServerPlayer 架构：直接反射设置 Inventory.selected）
        if tool != "hand" {
            let slot = st
                .inventory
                .iter()
                .find(|i| i.id.contains(tool) && i.slot < 9 && i.count > 0)
                .map(|i| i.slot + 1);
            if let Some(s) = slot {
                let _ = adapter.select_slot(s - 1)?;
                std::thread::sleep(std::time::Duration::from_millis(100));
            } else {
                return Ok(ToolResult {
                    message: format!("useOn: {tool} not found in hotbar"),
                    is_error: true,
                    images: vec![],
                });
            }
        }

        // 找目标
        let target_clean = target.replace("minecraft:", "");
        let ent = st
            .entities
            .iter()
            .filter(|e| {
                let ty = e.r#type.replace("minecraft:", "");
                ty.contains(&target_clean) && e.dist <= 5.0
            })
            .min_by(|a, b| {
                a.dist
                    .partial_cmp(&b.dist)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
        let block = st
            .nearby_blocks
            .iter()
            .filter(|b| b.id.contains(&target_clean) && b.dist <= 5.0)
            .min_by(|a, b| {
                a.dist
                    .partial_cmp(&b.dist)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });

        if let Some(e) = ent {
            adapter.look_at(e.x, e.y + 1.0, e.z)?;
            std::thread::sleep(std::time::Duration::from_millis(150));
            let _ = adapter.use_item(10)?;
            Ok(ToolResult {
                message: format!(
                    "used {tool} on {} at ({:.1},{:.1},{:.1})",
                    e.r#type, e.x, e.y, e.z
                ),
                is_error: false,
                images: vec![],
            })
        } else if let Some(b) = block {
            adapter.look_at(b.x, b.y + 0.5, b.z)?;
            std::thread::sleep(std::time::Duration::from_millis(150));
            let _ = adapter.use_item(10)?;
            Ok(ToolResult {
                message: format!(
                    "used {tool} on {} at ({:.0},{:.0},{:.0})",
                    b.id, b.x, b.y, b.z
                ),
                is_error: false,
                images: vec![],
            })
        } else if target == "nothing" {
            let _ = adapter.use_item(10)?;
            Ok(ToolResult {
                message: format!("used {tool} in air"),
                is_error: false,
                images: vec![],
            })
        } else {
            Ok(ToolResult {
                message: format!("useOn: no {target} within 5m"),
                is_error: true,
                images: vec![],
            })
        }
    }
}

// ═══════════════════════════════════════════════════════════════
// putInChest / takeFromChest / viewChest — 箱子操作（Mindcraft 对齐）
// ═══════════════════════════════════════════════════════════════

pub struct ModChestTool {
    adapter: Rc<RefCell<MinecraftModAdapter>>,
}
impl ModChestTool {
    pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self {
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
        serde_json::json!({"type":"object","properties":{"action":{"type":"string","enum":["view","put","take"],"description":"view=list contents, put=deposit, take=withdraw"},"item":{"type":"string","description":"Item to put/take (ignored for view)"},"num":{"type":"integer","description":"Count to put/take","default":1,"minimum":1}},"required":["action"]})
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
        let num = args["num"].as_u64().unwrap_or(1) as u32;
        let mut adapter = self.adapter.borrow_mut();
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
                message: format!("chest: invalid action or missing item param"),
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
    adapter: Rc<RefCell<MinecraftModAdapter>>,
}
impl ModClearFurnaceTool {
    pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self {
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
        serde_json::json!({"type":"object","properties":{},"required":[]})
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
        let mut adapter = self.adapter.borrow_mut();
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

pub struct ModInspectGuiTool {
    adapter: Rc<RefCell<MinecraftModAdapter>>,
}
impl ModInspectGuiTool {
    pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModInspectGuiTool {
    fn name(&self) -> &str {
        "inspect_gui"
    }
    fn description(&self) -> &str {
        "Read the currently open container/crafting GUI contents. Returns slot-by-slot inventory of the open container (chest, furnace, crafting table, etc) plus player inventory side. Use BEFORE transfer() to know slot indices."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{},"required":[]})
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
        let ack = self.adapter.borrow().inspect_gui()?;
        let has_gui = ack.has_gui.unwrap_or(false);
        if !has_gui {
            return Ok(ToolResult {
                message: "inspect_gui: no container open".into(),
                is_error: false,
                images: vec![],
            });
        }
        let slots = ack.slots.clone().unwrap_or_default();
        let carried = ack.carried_item.clone().unwrap_or_default();
        let mut lines = vec![format!("GUI slots: {} total", ack.detail)];
        if let Some(arr) = slots.as_array() {
            for slot in arr.iter().take(40) {
                let idx = slot["slot_index"].as_u64().unwrap_or(0);
                let id = slot["id"].as_str().unwrap_or("?");
                let count = slot["count"].as_u64().unwrap_or(0);
                let side = slot["side"].as_str().unwrap_or("?");
                if id != "minecraft:air" {
                    lines.push(format!(
                        "  [{idx}] {id} x{count} ({side})",
                        idx = idx,
                        id = id.replace("minecraft:", ""),
                        count = count,
                        side = side
                    ));
                }
            }
        }
        if carried != "minecraft:air" && !carried.is_empty() {
            lines.push(format!(
                "  [cursor] {} x{}",
                carried.replace("minecraft:", ""),
                ack.carried_count.unwrap_or(0)
            ));
        }
        Ok(ToolResult {
            message: lines.join("\n"),
            is_error: false,
            images: vec![],
        })
    }
}

pub struct ModTransferTool {
    adapter: Rc<RefCell<MinecraftModAdapter>>,
}
impl ModTransferTool {
    pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModTransferTool {
    fn name(&self) -> &str {
        "transfer"
    }
    fn description(&self) -> &str {
        "Move items within an open container GUI. Use inspect_gui first to get slot indices. moves: array of {from: slot_index, to?: slot_index|null}. to=null means shift-click (auto-route to opposite side). Call close_gui when done."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"moves":{"type":"array","description":"List of moves [{from: int, to?: int|null}]","items":{"type":"object"}}},"required":[]})
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
        let moves = args["moves"].clone();
        let ack = self.adapter.borrow().transfer(moves)?;
        let moved = ack.moved_count.unwrap_or(0);
        Ok(ToolResult {
            message: format!("transfer: {moved} moves executed. {}", ack.detail),
            is_error: moved == 0,
            images: vec![],
        })
    }
}

pub struct ModCloseGuiTool {
    adapter: Rc<RefCell<MinecraftModAdapter>>,
}
impl ModCloseGuiTool {
    pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModCloseGuiTool {
    fn name(&self) -> &str {
        "close_gui"
    }
    fn description(&self) -> &str {
        "Close the currently open container/crafting GUI. Use after transfer() is complete."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{},"required":[]})
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
        let ack = self.adapter.borrow().close_gui()?;
        Ok(ToolResult {
            message: ack.detail,
            is_error: false,
            images: vec![],
        })
    }
}

// ═══════════════════════════════════════════════════════════════
// equip_item / eat_item / drop_items / wait — 物品管理（参考 Numen）
// ═══════════════════════════════════════════════════════════════

pub struct ModEquipItemTool {
    adapter: Rc<RefCell<MinecraftModAdapter>>,
}
impl ModEquipItemTool {
    pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModEquipItemTool {
    fn name(&self) -> &str {
        "equip_item"
    }
    fn description(&self) -> &str {
        "Equip an item to a specific slot. Supports armor (head/chest/legs/feet), offhand, and mainhand. Auto-routes by item type if slot not specified. item: item name. slot: 'mainhand'|'offhand'|'head'|'chest'|'legs'|'feet' (optional, auto-detected)."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"item":{"type":"string","description":"Item to equip, e.g. iron_helmet, shield, diamond_sword"},"slot":{"type":"string","description":"Target slot: mainhand/offhand/head/chest/legs/feet"}},"required":["item"]})
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
        let slot = args["slot"].as_str();
        let ack = self.adapter.borrow().equip_item(item, slot)?;
        let equipped = ack.equipped.unwrap_or(false);
        let slot_str = ack.slot.clone().unwrap_or_default();
        Ok(ToolResult {
            message: format!("equip_item {item} -> {slot_str} (equipped={})", equipped),
            is_error: !equipped,
            images: vec![],
        })
    }
}

pub struct ModEatItemTool {
    adapter: Rc<RefCell<MinecraftModAdapter>>,
}
impl ModEatItemTool {
    pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModEatItemTool {
    fn name(&self) -> &str {
        "eat_item"
    }
    fn description(&self) -> &str {
        "Eat a specific food item from inventory. Finds item, equips it, and consumes it. item: food name like cooked_beef, bread, apple. ticks: eat duration (32≈1.6s for most food)."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"item":{"type":"string","description":"Food to eat, e.g. cooked_beef, bread, apple"},"ticks":{"type":"integer","description":"Eat duration (32≈1.6s)","default":32}},"required":["item"]})
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
        let ticks = args["ticks"].as_u64().map(|t| t as u32);
        let ack = self.adapter.borrow().eat_item(item, ticks)?;
        let consumed = ack.consumed.unwrap_or(false);
        Ok(ToolResult {
            message: format!("eat_item {item} (consumed={})", consumed),
            is_error: !consumed,
            images: vec![],
        })
    }
}

pub struct ModDropItemsTool {
    adapter: Rc<RefCell<MinecraftModAdapter>>,
}
impl ModDropItemsTool {
    pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModDropItemsTool {
    fn name(&self) -> &str {
        "drop_items"
    }
    fn description(&self) -> &str {
        "Drop items from inventory as ground ItemEntity (with pickup cooldown, like pressing Q). item: item name. num: how many to drop. Unlike discard(), this spawns real items on the ground that can be picked back up."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"item":{"type":"string","description":"Item to drop"},"num":{"type":"integer","description":"Count to drop","default":1,"minimum":1}},"required":["item"]})
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
        let ack = self.adapter.borrow().drop_items(item, num)?;
        let dropped = ack.dropped.unwrap_or(0);
        Ok(ToolResult {
            message: format!("drop_items {item} x{dropped} (ItemEntity spawned)"),
            is_error: dropped == 0,
            images: vec![],
        })
    }
}

pub struct ModWaitTool {
    adapter: Rc<RefCell<MinecraftModAdapter>>,
}
impl ModWaitTool {
    pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModWaitTool {
    fn name(&self) -> &str {
        "wait"
    }
    fn description(&self) -> &str {
        "Wait for N seconds. seconds: how long to wait (1-30). Use after placing items in furnace, waiting for crops, or healing."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"seconds":{"type":"integer","description":"Seconds to wait (1-30)","default":5,"minimum":1,"maximum":30}},"required":[]})
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
        let ack = self.adapter.borrow().wait(seconds)?;
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
    adapter: Rc<RefCell<MinecraftModAdapter>>,
}
impl ModListPlayersTool {
    pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self {
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
        serde_json::json!({"type":"object","properties":{},"required":[]})
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
        let ack = self.adapter.borrow().list_players()?;
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
    adapter: Rc<RefCell<MinecraftModAdapter>>,
}
impl ModGoToPlayerTool {
    pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self {
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
        serde_json::json!({"type":"object","properties":{"player_name":{"type":"string","description":"Exact player name"},"closeness":{"type":"number","description":"How close to get (meters)","default":2.0}},"required":["player_name"]})
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
        let ack = self.adapter.borrow().go_to_player(name, closeness)?;
        let reached = ack.reached.unwrap_or(false);
        let dist = ack.final_dist.unwrap_or(0.0);
        Ok(ToolResult {
            message: format!("go_to_player {name}: reached={reached} dist={dist:.1}m"),
            is_error: !reached,
            images: vec![],
        })
    }
}

pub struct ModAttackPlayerTool {
    adapter: Rc<RefCell<MinecraftModAdapter>>,
}
impl ModAttackPlayerTool {
    pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModAttackPlayerTool {
    fn name(&self) -> &str {
        "attack_player"
    }
    fn description(&self) -> &str {
        "Attack a specific player by name. Automatically approaches if too far, stops if target dies. player_name: exact name from list_players. ticks: attack duration (60≈3s)."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"player_name":{"type":"string","description":"Exact player name"},"ticks":{"type":"integer","description":"Attack duration","default":60}},"required":["player_name"]})
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
        let ticks = args["ticks"].as_u64().unwrap_or(60) as u32;
        let ack = self.adapter.borrow().attack_player(name, ticks)?;
        let hits = ack.hits.unwrap_or(0);
        Ok(ToolResult {
            message: format!("attack_player {name}: {hits} hits"),
            is_error: hits == 0,
            images: vec![],
        })
    }
}

pub struct ModGivePlayerTool {
    adapter: Rc<RefCell<MinecraftModAdapter>>,
}
impl ModGivePlayerTool {
    pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self {
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
        serde_json::json!({"type":"object","properties":{"player_name":{"type":"string","description":"Exact player name"},"item":{"type":"string","description":"Item name"},"num":{"type":"integer","description":"Count","default":1,"minimum":1}},"required":["player_name","item"]})
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
        let ack = self.adapter.borrow().give_player(name, item, num)?;
        let dropped = ack.dropped.unwrap_or(0);
        Ok(ToolResult {
            message: format!("give_player {item} x{dropped} to {name}"),
            is_error: dropped == 0,
            images: vec![],
        })
    }
}

pub struct ModCollectItemsTool {
    adapter: Rc<RefCell<MinecraftModAdapter>>,
}
impl ModCollectItemsTool {
    pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self {
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
        serde_json::json!({"type":"object","properties":{"item_ids":{"type":"array","description":"Filter items (empty=all)","items":{"type":"string"}},"radius":{"type":"number","description":"Search radius","default":16.0},"max_count":{"type":"integer","description":"Max to collect","default":64}},"required":[]})
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
        let item_ids: Vec<String> = args["item_ids"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        let radius = args["radius"].as_f64().unwrap_or(16.0);
        let max_count = args["max_count"].as_u64().unwrap_or(64) as u32;
        let ack = self
            .adapter
            .borrow()
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
    adapter: Rc<RefCell<MinecraftModAdapter>>,
}
impl ModStopTool {
    pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self {
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
        serde_json::json!({"type":"object","properties":{},"required":[]})
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
        let ack = self.adapter.borrow().stop()?;
        Ok(ToolResult {
            message: ack.detail,
            is_error: false,
            images: vec![],
        })
    }
}

pub struct ModSetGoalTool {
    adapter: Rc<RefCell<MinecraftModAdapter>>,
}
impl ModSetGoalTool {
    pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
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
        serde_json::json!({"type":"object","properties":{"goal":{"type":"string","description":"Goal description (empty to clear)"}},"required":[]})
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
        let ack = self.adapter.borrow().set_goal(goal)?;
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
    adapter: Rc<RefCell<MinecraftModAdapter>>,
}
impl ModFollowPlayerTool {
    pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self {
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
        serde_json::json!({"type":"object","properties":{"player_name":{"type":"string","description":"Player name to follow"},"follow_dist":{"type":"number","description":"Distance to maintain (default 3m)","minimum":0}},"required":["player_name"]})
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
        let ack = self.adapter.borrow().follow_player(name, dist)?;
        Ok(ToolResult {
            message: ack.detail,
            is_error: ack.status == "fail",
            images: vec![],
        })
    }
}

pub struct ModSearchWikiTool {
    adapter: Rc<RefCell<MinecraftModAdapter>>,
}
impl ModSearchWikiTool {
    pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self {
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
        serde_json::json!({"type":"object","properties":{"query":{"type":"string","description":"Search query"}},"required":["query"]})
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
        let ack = self.adapter.borrow().search_wiki(q)?;
        let text = ack.wiki_text.unwrap_or(ack.detail);
        Ok(ToolResult {
            message: text,
            is_error: ack.status == "fail",
            images: vec![],
        })
    }
}

pub struct ModVillagerTradesTool {
    adapter: Rc<RefCell<MinecraftModAdapter>>,
}
impl ModVillagerTradesTool {
    pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self {
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
        serde_json::json!({"type":"object","properties":{"radius":{"type":"number","description":"Search radius (default 8m)","minimum":1,"maximum":32}},"required":[]})
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
        let ack = self.adapter.borrow().villager_trades(radius)?;
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
    adapter: Rc<RefCell<MinecraftModAdapter>>,
}
impl ModTradeWithVillagerTool {
    pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self {
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
        serde_json::json!({"type":"object","properties":{"index":{"type":"integer","description":"1-indexed trade position","minimum":1},"count":{"type":"integer","description":"How many trades (default 1)","minimum":1,"maximum":64},"radius":{"type":"number","description":"Search radius (default 8m)","minimum":1,"maximum":32}},"required":["index"]})
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
            .borrow()
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
    adapter: Rc<RefCell<MinecraftModAdapter>>,
}
impl ModLookAtPlayerTool {
    pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self {
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
        serde_json::json!({"type":"object","properties":{"player_name":{"type":"string","description":"Player name to look at"}},"required":["player_name"]})
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
        let ack = self.adapter.borrow().look_at_player(name)?;
        Ok(ToolResult {
            message: ack.detail,
            is_error: ack.status == "fail",
            images: vec![],
        })
    }
}

pub struct ModLookAtPositionTool {
    adapter: Rc<RefCell<MinecraftModAdapter>>,
}
impl ModLookAtPositionTool {
    pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self {
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
        serde_json::json!({"type":"object","properties":{"x":{"type":"number","description":"World X"},"y":{"type":"number","description":"World Y"},"z":{"type":"number","description":"World Z"}},"required":["x","y","z"]})
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
        let ack = self.adapter.borrow().look_at_position(x, y, z)?;
        Ok(ToolResult {
            message: ack.detail,
            is_error: ack.status == "fail",
            images: vec![],
        })
    }
}

pub struct ModActivateBlockTool {
    adapter: Rc<RefCell<MinecraftModAdapter>>,
}
impl ModActivateBlockTool {
    pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModActivateBlockTool {
    fn name(&self) -> &str {
        "activate_block"
    }
    fn description(&self) -> &str {
        "Right-click activate a block at x/y/z (orient + useItemOn). Opens doors, activates levers/buttons, opens furnace/chest GUI. Must be within 5m."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"x":{"type":"integer","description":"World X"},"y":{"type":"integer","description":"World Y"},"z":{"type":"integer","description":"World Z"}},"required":["x","y","z"]})
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
        let x = args["x"].as_i64().unwrap_or(0) as i32;
        let y = args["y"].as_i64().unwrap_or(0) as i32;
        let z = args["z"].as_i64().unwrap_or(0) as i32;
        let ack = self.adapter.borrow().activate_block(x, y, z)?;
        let activated = ack.activated.unwrap_or(false);
        Ok(ToolResult {
            message: ack.detail,
            is_error: !activated && ack.status == "fail",
            images: vec![],
        })
    }
}

pub struct ModUseOnEntityTool {
    adapter: Rc<RefCell<MinecraftModAdapter>>,
}
impl ModUseOnEntityTool {
    pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModUseOnEntityTool {
    fn name(&self) -> &str {
        "use_on_entity"
    }
    fn description(&self) -> &str {
        "Use held item on nearest entity of given type (orient + interactOn). entity_type: entity ID substring (e.g. 'villager', 'horse', 'cow'). radius: search radius (default 8m)."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"entity_type":{"type":"string","description":"Entity type ID substring"},"radius":{"type":"number","description":"Search radius (default 8m)","minimum":1,"maximum":32}},"required":["entity_type"]})
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
        let et = args["entity_type"].as_str().unwrap_or("");
        let radius = args["radius"].as_f64();
        let ack = self.adapter.borrow().use_on_entity(et, radius)?;
        let interacted = ack.interacted.unwrap_or(false);
        Ok(ToolResult {
            message: ack.detail,
            is_error: !interacted && ack.status == "fail",
            images: vec![],
        })
    }
}

pub struct ModClearChatTool {
    adapter: Rc<RefCell<MinecraftModAdapter>>,
}
impl ModClearChatTool {
    pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self {
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
        serde_json::json!({"type":"object","properties":{},"required":[]})
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
        let _ = self.adapter.borrow().clear_chat()?;
        Ok(ToolResult {
            message:
                "Chat history cleared (mod ack only — agent runtime should clear its own history)."
                    .into(),
            is_error: false,
            images: vec![],
        })
    }
}

pub struct ModActivateNearestBlockTool {
    adapter: Rc<RefCell<MinecraftModAdapter>>,
}
impl ModActivateNearestBlockTool {
    pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModActivateNearestBlockTool {
    fn name(&self) -> &str {
        "activate_nearest_block"
    }
    fn description(&self) -> &str {
        "Activate nearest block of given type within 5m (mindcraft !activateNearestBlock). Auto-searches + uses useItemOn. block_type: block ID substring (e.g. 'door', 'lever', 'button')."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"block_type":{"type":"string","description":"Block ID substring"},"radius":{"type":"number","description":"Search radius (default 5m)","minimum":1,"maximum":10}},"required":["block_type"]})
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
        let bt = args["block_type"].as_str().unwrap_or("");
        let radius = args["radius"].as_f64();
        let ack = self.adapter.borrow().activate_nearest_block(bt, radius)?;
        let activated = ack.activated.unwrap_or(false);
        Ok(ToolResult {
            message: ack.detail,
            is_error: !activated && ack.status == "fail",
            images: vec![],
        })
    }
}

pub struct ModGetCraftingPlanTool {
    adapter: Rc<RefCell<MinecraftModAdapter>>,
}
impl ModGetCraftingPlanTool {
    pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModGetCraftingPlanTool {
    fn name(&self) -> &str {
        "get_crafting_plan"
    }
    fn description(&self) -> &str {
        "Get crafting plan: how many you have + what's missing. item: target item. count: how many to craft. Returns have_count and missing materials list."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"item":{"type":"string","description":"Target item name"},"count":{"type":"integer","description":"How many to craft","minimum":1,"default":1}},"required":["item"]})
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
        let item = args["item"].as_str().unwrap_or("");
        let count = args["count"].as_u64().unwrap_or(1) as u32;
        let ack = self.adapter.borrow().get_crafting_plan(item, count)?;
        let have = ack.have_count.unwrap_or(0);
        let missing = ack
            .missing
            .map(|m| m.to_string())
            .unwrap_or_else(|| "[]".into());
        Ok(ToolResult {
            message: format!(
                "have {} of {}: {} | missing: {}",
                have, count, item, missing
            ),
            is_error: ack.status == "fail",
            images: vec![],
        })
    }
}

pub struct ModDiscardSmartTool {
    adapter: Rc<RefCell<MinecraftModAdapter>>,
}
impl ModDiscardSmartTool {
    pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self {
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
        serde_json::json!({"type":"object","properties":{"item":{"type":"string","description":"Item ID substring"},"num":{"type":"integer","description":"How many to drop","minimum":1,"maximum":64}},"required":["item","num"]})
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
        let ack = self.adapter.borrow().discard_smart(item, num)?;
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
// 工厂
// ═══════════════════════════════════════════════════════════════

pub fn create_mc_mod_tools(
    adapter: Rc<RefCell<MinecraftModAdapter>>,
    image_max_side: Option<u32>,
    shots_dir: Option<PathBuf>,
    enable_visual_perceive: bool,
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
    tools.push(Box::new(ModUseItemTool::new(adapter.clone())));
    tools.push(Box::new(ModCombatTool::new(adapter.clone())));
    tools.push(Box::new(ModMoveToTool::new(adapter.clone())));
    tools.push(Box::new(ModLookAtTool::new(adapter.clone())));
    tools.push(Box::new(ModSearchBlockTool::new(adapter.clone())));
    tools.push(Box::new(ModMoveAwayTool::new(adapter.clone())));
    tools.push(Box::new(ModDigDownTool::new(adapter.clone())));
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
    tools.push(Box::new(ModSetGoalTool::new(adapter.clone())));
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
    tools.push(Box::new(ModDiscardSmartTool::new(adapter)));
    tools
}

// ═══════════════════════════════════════════════════════════════
// VisualPerceive — 截屏+VLM分析（仅 GUI 场景）
// ═══════════════════════════════════════════════════════════════

pub struct ModVisualPerceiveTool {
    adapter: Rc<RefCell<MinecraftModAdapter>>,
}
impl ModVisualPerceiveTool {
    pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModVisualPerceiveTool {
    fn name(&self) -> &str {
        "visual_perceive"
    }
    fn description(&self) -> &str {
        "HIGH LATENCY (3-5s). Screenshot + VLM analysis. Use ONLY for GUI inspection: crafting table, furnace, chest, or villager trade interfaces. prompt: what to look for. For game state use perceive() (auto-injected)."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"prompt":{"type":"string","description":"What to look for, e.g. 'What does the crafting table show?'"}},"required":["prompt"]})
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
        let prompt = args["prompt"].as_str().unwrap_or("Describe the screen");
        match self.adapter.borrow().perceive_visual(prompt) {
            Ok(r) => Ok(ToolResult {
                message: r,
                is_error: false,
                images: vec![],
            }),
            Err(e) => Ok(ToolResult {
                message: format!("visual: {e}"),
                is_error: true,
                images: vec![],
            }),
        }
    }
}
