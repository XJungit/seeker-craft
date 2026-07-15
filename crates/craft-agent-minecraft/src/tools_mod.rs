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

pub struct ModPerceiveTool { adapter: Rc<RefCell<MinecraftModAdapter>>, image_max_side: Option<u32>, shots_dir: Option<PathBuf>, counter: Cell<u32> }
impl ModPerceiveTool { pub fn new(a: Rc<RefCell<MinecraftModAdapter>>, im: Option<u32>, sd: Option<PathBuf>) -> Self { Self { adapter: a, image_max_side: im, shots_dir: sd, counter: Cell::new(0) } }
    fn save_shot(&self, png: &[u8]) -> Option<String> {
        let dir = self.shots_dir.as_ref()?; let n = self.counter.get() + 1; self.counter.set(n);
        let rel = dir.join(&format!("step-{n:03}.png"));
        if std::fs::create_dir_all(dir).is_ok() && std::fs::write(&rel, png).is_ok() { Some(rel.to_string_lossy().to_string()) } else { None }
    }
}
impl GameTool for ModPerceiveTool {
    fn name(&self) -> &str { "perceive" }
    fn description(&self) -> &str {
        "Read full game state via mod (latency <100ms, no side effects). Returns: position(x/y/z), yaw/pitch, health/hunger, gamemode/biome/dimension, light levels, weather, ALL inventory items (hotbar slots 1-9 + main inventory), targeted block (what crosshair points at), nearby blocks (top 30 by relevance), nearby entities. This data is auto-injected each turn — you rarely need to call this manually."
    }
    fn parameters(&self) -> Value { serde_json::json!({"type":"object","properties":{},"required":[]}) }
    fn effects(&self) -> ToolEffects { ToolEffects::read() }
    fn execute(&self, _id: &str, _args: Value, _on_update: Option<ToolUpdateFn>) -> anyhow::Result<ToolResult> {
        let ws = self.adapter.borrow().perceive()?;
        let images = if !ws.screenshot.is_empty() {
            let scaled = match self.image_max_side { Some(ms) => downscale_png(&ws.screenshot, ms).map(|r| r.0).unwrap_or_default(), None => ws.screenshot.clone() };
            if scaled.is_empty() { vec![] } else { vec![format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(&scaled))] }
        } else { vec![] };
        let message = match self.save_shot(&ws.screenshot) { Some(p) => format!("{}\n\n[screenshot saved to {}]", ws.scene_desc, p), None => ws.scene_desc };
        Ok(ToolResult { message, is_error: false, images })
    }
}

// ═══════════════════════════════════════════════════════════════
// collect — 自动采集（找→走→挖，核心工具）
// ═══════════════════════════════════════════════════════════════

pub struct ModCollectTool { adapter: Rc<RefCell<MinecraftModAdapter>> }
impl ModCollectTool { pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self { Self { adapter: a } } }
impl GameTool for ModCollectTool {
    fn name(&self) -> &str { "collect" }
    fn description(&self) -> &str {
        "AUTO find nearest target block, walk to it via mod-side move_to (no camera oscillation), mine until block breaks (mod auto-detects block destruction). Each block takes 2-10s depending on tool and block hardness. target: block ID substring (e.g. oak_log, stone, coal_ore). count: how many to collect (max 64 per call). Returns actual count collected. If no more blocks nearby, stops early."
    }
    fn parameters(&self) -> Value { serde_json::json!({"type":"object","properties":{"target":{"type":"string","description":"Block ID substring, e.g. oak_log, stone, coal_ore"},"count":{"type":"integer","description":"Number to collect (1-64)","default":1,"minimum":1,"maximum":64}},"required":["target"]}) }
    fn effects(&self) -> ToolEffects { ToolEffects::write() }
    fn execute(&self, _id: &str, args: Value, _on_update: Option<ToolUpdateFn>) -> anyhow::Result<ToolResult> {
        let target = args["target"].as_str().unwrap_or("oak_log"); let want = args["count"].as_u64().unwrap_or(1).min(64) as u32;
        let mut adapter = self.adapter.borrow_mut();
        let st = adapter.reload()?; let before: u32 = st.inventory.iter().filter(|i| i.id.contains(target)).map(|i| i.count).sum();
        let mine_ticks = mine_ticks_for(target, &st.held_item);
        let max_attempts = (want * 5).max(15); let mut got = before;
        for _ in 0..max_attempts {
            if got >= before + want { break; }
            let st = adapter.reload()?;
            if let Some(ref tb) = st.targeted_block && tb.id.contains(target) && tb.dist <= 4.0 {
                adapter.execute(Action::Mine { ticks: mine_ticks })?; std::thread::sleep(std::time::Duration::from_millis(200));
                got = adapter.reload()?.inventory.iter().filter(|i| i.id.contains(target)).map(|i| i.count).sum(); continue;
            }
            let Some((block, _)) = find_nearest(&adapter, target) else { return Ok(ToolResult { message: format!("collected {target}: {before}→{got} (no more nearby)"), is_error: got == before, images: vec![] }); };
            adapter.move_to(block.x, block.y + 0.5, block.z)?; std::thread::sleep(std::time::Duration::from_millis(300));
            adapter.execute(Action::Mine { ticks: mine_ticks })?; std::thread::sleep(std::time::Duration::from_millis(200));
            got = adapter.reload()?.inventory.iter().filter(|i| i.id.contains(target)).map(|i| i.count).sum();
        }
        Ok(ToolResult { message: format!("collected {target}: {before}→{got} (wanted +{want})"), is_error: got < before + want, images: vec![] })
    }
}

// ═══════════════════════════════════════════════════════════════
// craft — 合成（mod Inventory 直接操作）
// ═══════════════════════════════════════════════════════════════

pub struct ModCraftTool { adapter: Rc<RefCell<MinecraftModAdapter>> }
impl ModCraftTool { pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self { Self { adapter: a } } }
impl GameTool for ModCraftTool {
    fn name(&self) -> &str { "craft" }
    fn description(&self) -> &str {
        "Craft items via direct inventory manipulation. Mod handlers cover: planks (log→4planks), sticks (2planks→4sticks), crafting_table (4planks→1), wooden tools (pickaxe/axe/shovel/hoe/sword), stone tools (pickaxe/axe/shovel/hoe/sword), torch (stick+coal→4), furnace (8cobble→1), chest (8planks→1), door/sign/fence/bowl/ladder. item: target item name. count: how many. Check craftable() first to verify materials."
    }
    fn parameters(&self) -> Value { serde_json::json!({"type":"object","properties":{"item":{"type":"string","description":"Item to craft, e.g. oak_planks, stick, crafting_table, wooden_pickaxe, torch"},"count":{"type":"integer","description":"How many to craft","default":1,"minimum":1}},"required":["item"]}) }
    fn effects(&self) -> ToolEffects { ToolEffects::write() }
    fn execute(&self, _id: &str, args: Value, _on_update: Option<ToolUpdateFn>) -> anyhow::Result<ToolResult> {
        let item = args["item"].as_str().unwrap_or("oak_planks"); let count = args["count"].as_u64().unwrap_or(1) as u32;
        let mut adapter = self.adapter.borrow_mut();
        let st = adapter.reload()?; let before: u32 = st.inventory.iter().filter(|i| i.id.contains(item)).map(|i| i.count).sum();
        adapter.craft(item, count)?; std::thread::sleep(std::time::Duration::from_millis(200));
        let after: u32 = adapter.reload()?.inventory.iter().filter(|i| i.id.contains(item)).map(|i| i.count).sum();
        let got = after.saturating_sub(before);
        Ok(ToolResult { message: format!("crafted {item}: {before}→{after} (+{got})"), is_error: got < count, images: vec![] })
    }
}

// ═══════════════════════════════════════════════════════════════
// place — 放置方块（自动切快捷栏+右键）
// ═══════════════════════════════════════════════════════════════

pub struct ModPlaceTool { adapter: Rc<RefCell<MinecraftModAdapter>> }
impl ModPlaceTool { pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self { Self { adapter: a } } }
impl GameTool for ModPlaceTool {
    fn name(&self) -> &str { "place" }
    fn description(&self) -> &str {
        "Place a block from inventory at crosshair position. Finds item in hotbar (slots 1-9), switches to that slot, right-clicks to place. item: block name like crafting_table, torch, oak_planks, dirt. Must be looking at a valid surface (ground, wall) before calling."
    }
    fn parameters(&self) -> Value { serde_json::json!({"type":"object","properties":{"item":{"type":"string","description":"Block to place, e.g. crafting_table, torch, dirt"}},"required":["item"]}) }
    fn effects(&self) -> ToolEffects { ToolEffects::write() }
    fn execute(&self, _id: &str, args: Value, _on_update: Option<ToolUpdateFn>) -> anyhow::Result<ToolResult> {
        let item = args["item"].as_str().unwrap_or("crafting_table"); let mut adapter = self.adapter.borrow_mut();
        let st = adapter.reload()?; let slot = st.inventory.iter().filter(|i| i.id.contains(item) && i.slot < 9 && i.count > 0).map(|i| i.slot).next().unwrap_or(0);
        adapter.execute(Action::Press { keys: format!("{}", slot + 1), ticks: 3 })?; std::thread::sleep(std::time::Duration::from_millis(100));
        adapter.right_click(5)?;
        Ok(ToolResult { message: format!("placed {item} (hotbar slot {})", slot + 1), is_error: false, images: vec![] })
    }
}

// ═══════════════════════════════════════════════════════════════
// equip / use_item / attack — 物品/战斗
// ═══════════════════════════════════════════════════════════════

pub struct ModEquipTool { adapter: Rc<RefCell<MinecraftModAdapter>> }
impl ModEquipTool { pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self { Self { adapter: a } } }
impl GameTool for ModEquipTool {
    fn name(&self) -> &str { "equip" }
    fn description(&self) -> &str { "Switch active hotbar slot. slot: 1-9 (corresponds to HOTBAR display). Use to hold a specific item before mining or placing." }
    fn parameters(&self) -> Value { serde_json::json!({"type":"object","properties":{"slot":{"type":"integer","description":"Hotbar slot number 1-9","minimum":1,"maximum":9}},"required":["slot"]}) }
    fn effects(&self) -> ToolEffects { ToolEffects::write() }
    fn execute(&self, _id: &str, args: Value, _on_update: Option<ToolUpdateFn>) -> anyhow::Result<ToolResult> {
        let slot = args["slot"].as_u64().unwrap_or(1).min(9) as u32;
        self.adapter.borrow_mut().execute(Action::Press { keys: format!("{slot}"), ticks: 3 })?;
        Ok(ToolResult { message: format!("equipped slot {slot}"), is_error: false, images: vec![] })
    }
}

pub struct ModUseItemTool { adapter: Rc<RefCell<MinecraftModAdapter>> }
impl ModUseItemTool { pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self { Self { adapter: a } } }
impl GameTool for ModUseItemTool {
    fn name(&self) -> &str { "use_item" }
    fn description(&self) -> &str { "Right-click to use item in hand. For eating food (32 ticks≈1.6s), opening doors (5 ticks), placing blocks (5 ticks). ticks: hold duration. Prefer consume() for eating specific foods." }
    fn parameters(&self) -> Value { serde_json::json!({"type":"object","properties":{"ticks":{"type":"integer","description":"Hold duration (20≈1s). 32=eat, 5=quick use","default":20}}}) }
    fn effects(&self) -> ToolEffects { ToolEffects::write() }
    fn execute(&self, _id: &str, args: Value, _on_update: Option<ToolUpdateFn>) -> anyhow::Result<ToolResult> {
        let ticks = args["ticks"].as_u64().unwrap_or(20) as u32;
        self.adapter.borrow_mut().right_click(ticks)?;
        Ok(ToolResult { message: format!("right-click {ticks} ticks"), is_error: false, images: vec![] })
    }
}

pub struct ModAttackTool { adapter: Rc<RefCell<MinecraftModAdapter>> }
impl ModAttackTool { pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self { Self { adapter: a } } }
impl GameTool for ModAttackTool {
    fn name(&self) -> &str { "attack" }
    fn description(&self) -> &str { "Attack nearest entity in crosshair direction. ticks: hold duration (30≈1.5s). Use when hostile mobs appear in NEARBY ENTITIES." }
    fn parameters(&self) -> Value { serde_json::json!({"type":"object","properties":{"ticks":{"type":"integer","description":"Attack duration (30≈1.5s)","default":30}}}) }
    fn effects(&self) -> ToolEffects { ToolEffects::write() }
    fn execute(&self, _id: &str, args: Value, _on_update: Option<ToolUpdateFn>) -> anyhow::Result<ToolResult> {
        let ticks = args["ticks"].as_u64().unwrap_or(30) as u32;
        self.adapter.borrow().attack(ticks)?;
        Ok(ToolResult { message: format!("attacked {ticks} ticks"), is_error: false, images: vec![] })
    }
}

// ═══════════════════════════════════════════════════════════════
// move_to / look_at / look / press / mine — 精确控制
// ═══════════════════════════════════════════════════════════════

pub struct ModMoveToTool { adapter: Rc<RefCell<MinecraftModAdapter>> }
impl ModMoveToTool { pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self { Self { adapter: a } } }
impl GameTool for ModMoveToTool {
    fn name(&self) -> &str { "move_to" }
    fn description(&self) -> &str { "Navigate to world coordinates. Mod re-aims every tick toward target (no oscillation), strafes around walls, jumps over blocks. Stops within 1.5m of target. Time: 2-10s depending on distance (≈15 ticks/m). Use coordinates from NEARBY BLOCKS section." }
    fn parameters(&self) -> Value { serde_json::json!({"type":"object","properties":{"x":{"type":"number","description":"Target X coordinate"},"y":{"type":"number","description":"Target Y (use block.y + 0.5 for block center)"},"z":{"type":"number","description":"Target Z coordinate"}},"required":["x","y","z"]}) }
    fn effects(&self) -> ToolEffects { ToolEffects::write() }
    fn execute(&self, _id: &str, args: Value, _on_update: Option<ToolUpdateFn>) -> anyhow::Result<ToolResult> {
        let x = args["x"].as_f64().unwrap_or(0.0); let y = args["y"].as_f64().unwrap_or(0.0); let z = args["z"].as_f64().unwrap_or(0.0);
        self.adapter.borrow_mut().move_to(x, y, z)?;
        Ok(ToolResult { message: format!("moved to ({:.1},{:.1},{:.1})", x, y, z), is_error: false, images: vec![] })
    }
}

pub struct ModLookAtTool { adapter: Rc<RefCell<MinecraftModAdapter>> }
impl ModLookAtTool { pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self { Self { adapter: a } } }
impl GameTool for ModLookAtTool {
    fn name(&self) -> &str { "look_at" }
    fn description(&self) -> &str { "Snap crosshair to a world coordinate. Integer coords auto-offset to block center (+0.5). Returns what block was actually hit (or 'nothing'). Force-refreshes raycast so targeted_block is accurate immediately." }
    fn parameters(&self) -> Value { serde_json::json!({"type":"object","properties":{"x":{"type":"number","description":"Target X coordinate"},"y":{"type":"number","description":"Target Y (auto-centers if integer)"},"z":{"type":"number","description":"Target Z coordinate"}},"required":["x","y","z"]}) }
    fn effects(&self) -> ToolEffects { ToolEffects::read() }
    fn execute(&self, _id: &str, args: Value, _on_update: Option<ToolUpdateFn>) -> anyhow::Result<ToolResult> {
        let x = args["x"].as_f64().unwrap_or(0.0); let y = args["y"].as_f64().unwrap_or(0.0); let z = args["z"].as_f64().unwrap_or(0.0);
        self.adapter.borrow_mut().look_at(x, y, z)?;
        Ok(ToolResult { message: format!("looking at ({:.1},{:.1},{:.1})", x, y, z), is_error: false, images: vec![] })
    }
}

pub struct ModLookTool { adapter: Rc<RefCell<MinecraftModAdapter>> }
impl ModLookTool { pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self { Self { adapter: a } } }
impl GameTool for ModLookTool {
    fn name(&self) -> &str { "look" }
    fn description(&self) -> &str { "Rotate camera relative to current view. dx>0=turn RIGHT (300≈90°). dy>0=look UP (toward sky). dy<0=look DOWN (toward ground). Sensitivity factor: 0.3°/unit. Prefer look_at() for precise coordinate targeting — this is approximate." }
    fn parameters(&self) -> Value { serde_json::json!({"type":"object","properties":{"dx":{"type":"integer","description":"Horizontal rotation (300≈90° right, -300≈90° left)"},"dy":{"type":"integer","description":"Vertical rotation (+=up/sky, -=down/ground, 65≈20°). Use NEGATIVE to look at ground!"}},"required":["dx","dy"]}) }
    fn effects(&self) -> ToolEffects { ToolEffects::read() }
    fn execute(&self, _id: &str, args: Value, _on_update: Option<ToolUpdateFn>) -> anyhow::Result<ToolResult> {
        let dx = args["dx"].as_i64().unwrap_or(0) as i32; let dy = args["dy"].as_i64().unwrap_or(0) as i32;
        let r = self.adapter.borrow_mut().execute(Action::Look { dx, dy })?;
        Ok(ToolResult { message: r.detail, is_error: !r.ok, images: vec![] })
    }
}

pub struct ModPressTool { adapter: Rc<RefCell<MinecraftModAdapter>> }
impl ModPressTool { pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self { Self { adapter: a } } }
impl GameTool for ModPressTool {
    fn name(&self) -> &str { "press" }
    fn description(&self) -> &str { "Hold keyboard key(s) for N ticks (20≈1s). keys: w(forward)/a(left)/s(back)/d(right) for movement, space(jump), shift(sneak), e(inventory), 1-9(hotbar). Walk distance ≈ ticks/15 blocks. For precise navigation, use move_to() instead." }
    fn parameters(&self) -> Value { serde_json::json!({"type":"object","properties":{"keys":{"type":"string","description":"Key(s) to hold: w,a,s,d,space,shift,e,1-9"},"ticks":{"type":"integer","description":"Duration (20≈1s, 30≈5 blocks walk)","default":20,"minimum":1,"maximum":200}},"required":["keys"]}) }
    fn effects(&self) -> ToolEffects { ToolEffects::write() }
    fn execute(&self, _id: &str, args: Value, _on_update: Option<ToolUpdateFn>) -> anyhow::Result<ToolResult> {
        let keys = args["keys"].as_str().unwrap_or("w").to_string(); let ticks = args["ticks"].as_u64().unwrap_or(20).min(200) as u32;
        let r = self.adapter.borrow_mut().execute(Action::Press { keys, ticks })?;
        Ok(ToolResult { message: r.detail, is_error: !r.ok, images: vec![] })
    }
}

pub struct ModMineTool { adapter: Rc<RefCell<MinecraftModAdapter>> }
impl ModMineTool { pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self { Self { adapter: a } } }
impl GameTool for ModMineTool {
    fn name(&self) -> &str { "mine" }
    fn description(&self) -> &str { "Hold left-click to mine targeted block. Mod auto-detects block breaking — stops immediately when block disappears (no fixed tick waste). ticks parameter is safety timeout only. Use collect() for automatic gathering instead." }
    fn parameters(&self) -> Value { serde_json::json!({"type":"object","properties":{"ticks":{"type":"integer","description":"Safety timeout in ticks (mod auto-stops when block breaks). 140 for wood, 300 for stone.","default":140}}}) }
    fn effects(&self) -> ToolEffects { ToolEffects::write() }
    fn execute(&self, _id: &str, args: Value, _on_update: Option<ToolUpdateFn>) -> anyhow::Result<ToolResult> {
        let ticks = args["ticks"].as_u64().unwrap_or(140) as u32;
        let r = self.adapter.borrow_mut().execute(Action::Mine { ticks })?;
        Ok(ToolResult { message: r.detail, is_error: !r.ok, images: vec![] })
    }
}

// ═══════════════════════════════════════════════════════════════
// craftable — 查询可合成列表
// ═══════════════════════════════════════════════════════════════

pub struct ModCraftableTool { adapter: Rc<RefCell<MinecraftModAdapter>> }
impl ModCraftableTool { pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self { Self { adapter: a } } }
impl GameTool for ModCraftableTool {
    fn name(&self) -> &str { "craftable" }
    fn description(&self) -> &str { "Query all craftable items from current inventory. Returns list of {item: max_count}. Covers: planks, sticks, crafting_table, wooden/stone tools, torch, furnace, chest, doors, signs, fences. Call before craft() to verify materials." }
    fn parameters(&self) -> Value { serde_json::json!({"type":"object","properties":{},"required":[]}) }
    fn effects(&self) -> ToolEffects { ToolEffects::read() }
    fn execute(&self, _id: &str, _args: Value, _on_update: Option<ToolUpdateFn>) -> anyhow::Result<ToolResult> {
        let st = self.adapter.borrow().reload()?;
        const RECIPES: &[(&str, &[(&str, u32)])] = &[
            ("oak_planks", &[("oak_log", 1)]), ("birch_planks", &[("birch_log", 1)]),
            ("stick", &[("planks", 2)]), ("crafting_table", &[("planks", 4)]),
            ("wooden_pickaxe", &[("planks", 3), ("stick", 2)]), ("wooden_axe", &[("planks", 3), ("stick", 2)]),
            ("wooden_sword", &[("planks", 2), ("stick", 1)]), ("wooden_shovel", &[("planks", 1), ("stick", 2)]),
            ("stone_pickaxe", &[("cobblestone", 3), ("stick", 2)]), ("stone_axe", &[("cobblestone", 3), ("stick", 2)]),
            ("torch", &[("stick", 1), ("coal", 1)]), ("furnace", &[("cobblestone", 8)]),
            ("chest", &[("planks", 8)]), ("oak_door", &[("planks", 6)]),
        ];
        let mut out = Vec::new();
        for (item, ingredients) in RECIPES {
            let ok = ingredients.iter().all(|(mat, need)| st.inventory.iter().filter(|i| i.id.contains(mat)).map(|i| i.count).sum::<u32>() >= *need);
            if ok {
                let max = ingredients.iter().map(|(mat, need)| st.inventory.iter().filter(|i| i.id.contains(mat)).map(|i| i.count).sum::<u32>() / need).min().unwrap_or(0);
                out.push(format!("  {item} x{max}"));
            }
        }
        Ok(ToolResult { message: if out.is_empty() { "CRAFTABLE: none".into() } else { format!("CRAFTABLE:\n{}", out.join("\n")) }, is_error: false, images: vec![] })
    }
}

// ═══════════════════════════════════════════════════════════════
// searchForBlock / moveAway / digDown — Mindcraft 对齐导航
// ═══════════════════════════════════════════════════════════════

pub struct ModSearchBlockTool { adapter: Rc<RefCell<MinecraftModAdapter>> }
impl ModSearchBlockTool { pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self { Self { adapter: a } } }
impl GameTool for ModSearchBlockTool {
    fn name(&self) -> &str { "searchForBlock" }
    fn description(&self) -> &str { "Find nearest block of type and walk to it (no mining). Uses move_to for navigation. Returns block type, coordinates, and distance walked. Use to position yourself before manual mining or building." }
    fn parameters(&self) -> Value { serde_json::json!({"type":"object","properties":{"type":{"type":"string","description":"Block type to find, e.g. oak_log, stone, crafting_table, chest"}},"required":["type"]}) }
    fn effects(&self) -> ToolEffects { ToolEffects::write() }
    fn execute(&self, _id: &str, args: Value, _on_update: Option<ToolUpdateFn>) -> anyhow::Result<ToolResult> {
        let target = args["type"].as_str().unwrap_or("oak_log");
        let Some((block, _)) = find_nearest(&self.adapter.borrow(), target) else { return Ok(ToolResult { message: format!("searchForBlock: no {target} nearby"), is_error: true, images: vec![] }); };
        self.adapter.borrow_mut().move_to(block.x, block.y + 0.5, block.z)?;
        Ok(ToolResult { message: format!("walked to {} at ({:.0},{:.0},{:.0}) dist={:.1}m", target, block.x, block.y, block.z, block.dist), is_error: false, images: vec![] })
    }
}

pub struct ModMoveAwayTool { adapter: Rc<RefCell<MinecraftModAdapter>> }
impl ModMoveAwayTool { pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self { Self { adapter: a } } }
impl GameTool for ModMoveAwayTool {
    fn name(&self) -> &str { "moveAway" }
    fn description(&self) -> &str { "Walk backwards N blocks. distance: approximate meters to retreat (≈distance×15 ticks). Max 20 blocks. Use to create space before building or flee danger." }
    fn parameters(&self) -> Value { serde_json::json!({"type":"object","properties":{"distance":{"type":"integer","description":"Blocks to retreat (1-20)","default":3,"minimum":1,"maximum":20}}}) }
    fn effects(&self) -> ToolEffects { ToolEffects::write() }
    fn execute(&self, _id: &str, args: Value, _on_update: Option<ToolUpdateFn>) -> anyhow::Result<ToolResult> {
        let dist = args["distance"].as_u64().unwrap_or(3).min(20) as u32;
        self.adapter.borrow_mut().execute(Action::Press { keys: "s".into(), ticks: dist * 15 })?;
        Ok(ToolResult { message: format!("moved back ~{dist} blocks"), is_error: false, images: vec![] })
    }
}

pub struct ModDigDownTool { adapter: Rc<RefCell<MinecraftModAdapter>> }
impl ModDigDownTool { pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self { Self { adapter: a } } }
impl GameTool for ModDigDownTool {
    fn name(&self) -> &str { "digDown" }
    fn description(&self) -> &str { "Dig straight down N blocks by looking down and mining. Jumps into hole after each block. distance: 1-10 blocks. Warning: no lava/water detection yet — use cautiously." }
    fn parameters(&self) -> Value { serde_json::json!({"type":"object","properties":{"distance":{"type":"integer","description":"Blocks to dig down (1-10)","default":1,"minimum":1,"maximum":10}}}) }
    fn effects(&self) -> ToolEffects { ToolEffects::write() }
    fn execute(&self, _id: &str, args: Value, _on_update: Option<ToolUpdateFn>) -> anyhow::Result<ToolResult> {
        let dist = args["distance"].as_u64().unwrap_or(1).min(10) as u32;
        let mut a = self.adapter.borrow_mut();
        for _ in 0..dist { a.execute(Action::Look { dx: 0, dy: -150 })?; std::thread::sleep(std::time::Duration::from_millis(50)); a.execute(Action::Mine { ticks: 60 })?; std::thread::sleep(std::time::Duration::from_millis(100)); a.execute(Action::Press { keys: "space".into(), ticks: 2 })?; std::thread::sleep(std::time::Duration::from_millis(300)); }
        Ok(ToolResult { message: format!("dug down {dist} blocks"), is_error: false, images: vec![] })
    }
}

pub struct ModConsumeTool { adapter: Rc<RefCell<MinecraftModAdapter>> }
impl ModConsumeTool { pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self { Self { adapter: a } } }
impl GameTool for ModConsumeTool {
    fn name(&self) -> &str { "consume" }
    fn description(&self) -> &str { "Eat/drink a food item. Finds item in hotbar, equips it, right-clicks to consume. item: food name like cooked_beef, bread, apple. ticks: hold time (32≈1.6s for most food)." }
    fn parameters(&self) -> Value { serde_json::json!({"type":"object","properties":{"item":{"type":"string","description":"Food to eat, e.g. cooked_beef, bread, apple, steak"},"ticks":{"type":"integer","description":"Hold time (32≈1.6s for food, 20 for drinks)","default":32}},"required":["item"]}) }
    fn effects(&self) -> ToolEffects { ToolEffects::write() }
    fn execute(&self, _id: &str, args: Value, _on_update: Option<ToolUpdateFn>) -> anyhow::Result<ToolResult> {
        let item = args["item"].as_str().unwrap_or("bread"); let ticks = args["ticks"].as_u64().unwrap_or(32) as u32;
        let mut a = self.adapter.borrow_mut(); let slot = a.reload()?.inventory.iter().find(|i| i.id.contains(item) && i.slot < 9 && i.count > 0).map(|i| i.slot + 1).unwrap_or(1);
        a.execute(Action::Press { keys: format!("{slot}"), ticks: 3 })?; std::thread::sleep(std::time::Duration::from_millis(100));
        a.right_click(ticks)?;
        Ok(ToolResult { message: format!("ate {item} (from slot {slot})"), is_error: false, images: vec![] })
    }
}

// ═══════════════════════════════════════════════════════════════
// 辅助函数
// ═══════════════════════════════════════════════════════════════

fn mine_ticks_for(block_id: &str, _held_item: &str) -> u32 {
    if block_id.contains("_log") || block_id.contains("planks") || block_id.contains("leaves") { 140 }
    else if block_id.contains("stone") || block_id.contains("cobble") { 300 }
    else if block_id.contains("_ore") { 400 }
    else if block_id.contains("dirt") || block_id.contains("grass") || block_id.contains("sand") { 40 }
    else { 200 }
}

fn find_nearest(adapter: &MinecraftModAdapter, target: &str) -> Option<(crate::bridge::NearbyBlock, f64)> {
    let st = adapter.reload().ok()?;
    let block = st.nearby_blocks.iter().filter(|b| b.id.contains(target)).min_by(|a, b| a.dist.partial_cmp(&b.dist).unwrap_or(std::cmp::Ordering::Equal))?;
    let dx = block.x - st.position[0]; let dz = block.z - st.position[2];
    let target_yaw = (-dx).atan2(dz).to_degrees(); let mut yaw_diff = target_yaw - st.yaw;
    while yaw_diff > 180.0 { yaw_diff -= 360.0; } while yaw_diff < -180.0 { yaw_diff += 360.0; }
    Some((block.clone(), yaw_diff))
}

// ═══════════════════════════════════════════════════════════════
// rememberHere / goToRememberedPlace / savedPlaces — 位置记忆
// ═══════════════════════════════════════════════════════════════

pub struct ModRememberTool { adapter: Rc<RefCell<MinecraftModAdapter>> }
impl ModRememberTool { pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self { Self { adapter: a } } }
impl GameTool for ModRememberTool {
    fn name(&self) -> &str { "rememberHere" }
    fn description(&self) -> &str { "Save current position with a name for later recall. name: label like 'base', 'cave_entrance', 'tree_farm'." }
    fn parameters(&self) -> Value { serde_json::json!({"type":"object","properties":{"name":{"type":"string","description":"Label for this location"}},"required":["name"]}) }
    fn effects(&self) -> ToolEffects { ToolEffects::read() }
    fn execute(&self, _id: &str, args: Value, _on_update: Option<ToolUpdateFn>) -> anyhow::Result<ToolResult> {
        let name = args["name"].as_str().unwrap_or("here");
        Ok(ToolResult { message: self.adapter.borrow().remember_here(name), is_error: false, images: vec![] })
    }
}

pub struct ModGoPlaceTool { adapter: Rc<RefCell<MinecraftModAdapter>> }
impl ModGoPlaceTool { pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self { Self { adapter: a } } }
impl GameTool for ModGoPlaceTool {
    fn name(&self) -> &str { "goToRememberedPlace" }
    fn description(&self) -> &str { "Walk to a previously saved location. name: label from rememberHere. Uses move_to for navigation." }
    fn parameters(&self) -> Value { serde_json::json!({"type":"object","properties":{"name":{"type":"string","description":"Location label from rememberHere"}},"required":["name"]}) }
    fn effects(&self) -> ToolEffects { ToolEffects::write() }
    fn execute(&self, _id: &str, args: Value, _on_update: Option<ToolUpdateFn>) -> anyhow::Result<ToolResult> {
        let name = args["name"].as_str().unwrap_or("here");
        Ok(ToolResult { message: self.adapter.borrow().go_to_place(name)?, is_error: false, images: vec![] })
    }
}

pub struct ModListPlacesTool { adapter: Rc<RefCell<MinecraftModAdapter>> }
impl ModListPlacesTool { pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self { Self { adapter: a } } }
impl GameTool for ModListPlacesTool {
    fn name(&self) -> &str { "savedPlaces" }
    fn description(&self) -> &str { "List all saved location names and coordinates from rememberHere." }
    fn parameters(&self) -> Value { serde_json::json!({"type":"object","properties":{},"required":[]}) }
    fn effects(&self) -> ToolEffects { ToolEffects::read() }
    fn execute(&self, _id: &str, _args: Value, _on_update: Option<ToolUpdateFn>) -> anyhow::Result<ToolResult> {
        Ok(ToolResult { message: self.adapter.borrow().list_places(), is_error: false, images: vec![] })
    }
}

// ═══════════════════════════════════════════════════════════════
// 工厂
// ═══════════════════════════════════════════════════════════════

pub fn create_mc_mod_tools(adapter: Rc<RefCell<MinecraftModAdapter>>, image_max_side: Option<u32>, shots_dir: Option<PathBuf>, enable_visual_perceive: bool) -> Vec<Box<dyn GameTool>> {
    let mut tools: Vec<Box<dyn GameTool>> = vec![Box::new(ModPerceiveTool::new(adapter.clone(), image_max_side, shots_dir)),
        Box::new(ModLookTool::new(adapter.clone())), Box::new(ModPressTool::new(adapter.clone())), Box::new(ModMineTool::new(adapter.clone()))];
    if enable_visual_perceive { tools.push(Box::new(ModVisualPerceiveTool::new(adapter.clone()))); }
    tools.push(Box::new(ModCollectTool::new(adapter.clone()))); tools.push(Box::new(ModCraftTool::new(adapter.clone()))); tools.push(Box::new(ModPlaceTool::new(adapter.clone())));
    tools.push(Box::new(ModEquipTool::new(adapter.clone()))); tools.push(Box::new(ModUseItemTool::new(adapter.clone()))); tools.push(Box::new(ModAttackTool::new(adapter.clone())));
    tools.push(Box::new(ModMoveToTool::new(adapter.clone()))); tools.push(Box::new(ModLookAtTool::new(adapter.clone())));
    tools.push(Box::new(ModSearchBlockTool::new(adapter.clone()))); tools.push(Box::new(ModMoveAwayTool::new(adapter.clone())));
    tools.push(Box::new(ModDigDownTool::new(adapter.clone()))); tools.push(Box::new(ModConsumeTool::new(adapter.clone())));
    tools.push(Box::new(ModRememberTool::new(adapter.clone()))); tools.push(Box::new(ModGoPlaceTool::new(adapter.clone()))); tools.push(Box::new(ModListPlacesTool::new(adapter.clone())));
    tools.push(Box::new(ModCraftableTool::new(adapter)));
    tools
}

// ═══════════════════════════════════════════════════════════════
// VisualPerceive — 截屏+VLM分析（仅 GUI 场景）
// ═══════════════════════════════════════════════════════════════

pub struct ModVisualPerceiveTool { adapter: Rc<RefCell<MinecraftModAdapter>> }
impl ModVisualPerceiveTool { pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self { Self { adapter: a } } }
impl GameTool for ModVisualPerceiveTool {
    fn name(&self) -> &str { "visual_perceive" }
    fn description(&self) -> &str { "HIGH LATENCY (3-5s). Screenshot + VLM analysis. Use ONLY for GUI inspection: crafting table, furnace, chest, or villager trade interfaces. prompt: what to look for. For game state use perceive() (auto-injected)." }
    fn parameters(&self) -> Value { serde_json::json!({"type":"object","properties":{"prompt":{"type":"string","description":"What to look for, e.g. 'What does the crafting table show?'"}},"required":["prompt"]}) }
    fn effects(&self) -> ToolEffects { ToolEffects::read() }
    fn execute(&self, _id: &str, args: Value, _on_update: Option<ToolUpdateFn>) -> anyhow::Result<ToolResult> {
        let prompt = args["prompt"].as_str().unwrap_or("Describe the screen");
        match self.adapter.borrow().perceive_visual(prompt) { Ok(r) => Ok(ToolResult { message: r, is_error: false, images: vec![] }), Err(e) => Ok(ToolResult { message: format!("visual: {e}"), is_error: true, images: vec![] }) }
    }
}
