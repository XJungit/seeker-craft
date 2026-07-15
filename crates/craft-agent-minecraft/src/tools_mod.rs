//! Minecraft mod 工具集（仅 `mod-bridge` 特性编译）。
//!
//! 心智模型参考 Mindcraft 的 47 个工具设计，但适配我们的 mod TCP 桥接架构：
//!   - 高层工具封装完整操作闭环（LLM 一个调用搞定）
//!   - 低层工具保留给需要精确控制的场景
//!   - 工具描述明确告知参数含义和预期耗时

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

// ── Perceive（自动注入精确状态）──

pub struct ModPerceiveTool {
    adapter: Rc<RefCell<MinecraftModAdapter>>,
    image_max_side: Option<u32>,
    shots_dir: Option<PathBuf>,
    counter: Cell<u32>,
}
impl ModPerceiveTool {
    pub fn new(
        adapter: Rc<RefCell<MinecraftModAdapter>>,
        image_max_side: Option<u32>,
        shots_dir: Option<PathBuf>,
    ) -> Self {
        Self {
            adapter,
            image_max_side,
            shots_dir,
            counter: Cell::new(0),
        }
    }
    fn save_shot(&self, png: &[u8]) -> Option<String> {
        let dir = self.shots_dir.as_ref()?;
        let n = self.counter.get() + 1;
        self.counter.set(n);
        let fname = format!("step-{n:03}.png");
        let rel = dir.join(&fname);
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
        "Read precise game state from mod (<0.1s, free). Returns coordinates, yaw/pitch, health/hunger, full inventory, targeted block, nearby blocks with coordinates, nearby entities. This data is auto-injected each turn — you rarely need to call this manually."
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
                vec![format!(
                    "data:image/png;base64,{}",
                    base64::engine::general_purpose::STANDARD.encode(&scaled)
                )]
            }
        } else {
            vec![]
        };
        let shot_rel = if !ws.screenshot.is_empty() {
            self.save_shot(&ws.screenshot)
        } else {
            None
        };
        let message = match shot_rel {
            Some(p) => format!("{}\n\n[screenshot saved to {}]", ws.scene_desc, p),
            None => ws.scene_desc,
        };
        Ok(ToolResult {
            message,
            is_error: false,
            images,
        })
    }
}

// ── Look（精确转视角）──

pub struct ModLookTool {
    adapter: Rc<RefCell<MinecraftModAdapter>>,
}
impl ModLookTool {
    pub fn new(adapter: Rc<RefCell<MinecraftModAdapter>>) -> Self {
        Self { adapter }
    }
}
impl GameTool for ModLookTool {
    fn name(&self) -> &str {
        "look"
    }
    fn description(&self) -> &str {
        "Rotate camera: dx>0=turn right (300≈90°). ⚠️ dy>0=look UP(sky), dy<0=look DOWN(ground). Prefer look_at() for precise block targeting."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"dx":{"type":"integer","description":"Horizontal rotation amount"},"dy":{"type":"integer","description":"Vertical rotation amount"}},"required":["dx","dy"]})
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

// ── Press（按键移动/跳跃）──

pub struct ModPressTool {
    adapter: Rc<RefCell<MinecraftModAdapter>>,
}
impl ModPressTool {
    pub fn new(adapter: Rc<RefCell<MinecraftModAdapter>>) -> Self {
        Self { adapter }
    }
}
impl GameTool for ModPressTool {
    fn name(&self) -> &str {
        "press"
    }
    fn description(&self) -> &str {
        "Hold a key for N ticks (20≈1s). keys: w/a/s/d=movement, space=jump, shift=sneak, e=inventory, 1-9=hotbar slot. Use for fine movement control when collect doesn't fit."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"keys":{"type":"string","description":"Key(s) to hold, e.g. 'w', 'space', 'w+d'"},"ticks":{"type":"integer","description":"Duration in ticks, 20≈1 second","default":20}},"required":["keys"]})
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
        let ticks = args["ticks"].as_u64().unwrap_or(20) as u32;
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

// ── Mine（精确挖掘）──

pub struct ModMineTool {
    adapter: Rc<RefCell<MinecraftModAdapter>>,
}
impl ModMineTool {
    pub fn new(adapter: Rc<RefCell<MinecraftModAdapter>>) -> Self {
        Self { adapter }
    }
}
impl GameTool for ModMineTool {
    fn name(&self) -> &str {
        "mine"
    }
    fn description(&self) -> &str {
        "Hold left-click to mine targeted block. Mod automatically keeps mining until the block breaks (max ticks as safety timeout). No fixed duration — returns true if block broken. Use collect() for automatic gathering instead."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"ticks":{"type":"integer","description":"Mining duration in ticks","default":60}}})
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
        let ticks = args["ticks"].as_u64().unwrap_or(60) as u32;
        let r = self.adapter.borrow_mut().execute(Action::Mine { ticks })?;
        Ok(ToolResult {
            message: r.detail,
            is_error: !r.ok,
            images: vec![],
        })
    }
}

// ── VisualPerceive（视觉补充）──

pub struct ModVisualPerceiveTool {
    adapter: Rc<RefCell<MinecraftModAdapter>>,
}
impl ModVisualPerceiveTool {
    pub fn new(adapter: Rc<RefCell<MinecraftModAdapter>>) -> Self {
        Self { adapter }
    }
}
impl GameTool for ModVisualPerceiveTool {
    fn name(&self) -> &str {
        "visual_perceive"
    }
    fn description(&self) -> &str {
        "HIGH LATENCY (3-5s) — Screenshot + visual model analysis. ONLY use when you need to see GUI (crafting table interface, furnace, chest inventory). For game state, use perceive (already auto-injected, <0.1s). Must specify what to look for in prompt."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"prompt":{"type":"string","description":"What to look for in the screenshot. Example: 'What does the crafting table interface show?'"}},"required":["prompt"]})
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
        let prompt = args["prompt"]
            .as_str()
            .unwrap_or("Describe the current screen");
        match self.adapter.borrow().perceive_visual(prompt) {
            Ok(reply) => Ok(ToolResult {
                message: reply,
                is_error: false,
                images: vec![],
            }),
            Err(e) => Ok(ToolResult {
                message: format!("visual perceive failed: {e}"),
                is_error: true,
                images: vec![],
            }),
        }
    }
}

// ═══ 高层工具（Mindcraft 风格，封装操作闭环）═══

/// 根据方块类型和手持工具估算挖掘 tick 数。
/// MC 硬度 × 工具效率倍率 → 实际 tick（20 tick = 1 秒）。
fn mine_ticks_for(block_id: &str, held_item: &str) -> u32 {
    // 基础 tick（徒手，不含工具加速）
    let base: u32 = if block_id.contains("_log") || block_id.contains("planks") {
        100 // 木头硬度 2.0 → 3s, 给 5s
    } else if block_id.contains("stone") || block_id.contains("cobble") {
        200 // 石头硬度 1.5 → 徒手 7.5s, 给 10s
    } else if block_id.contains("_ore") {
        if block_id.contains("coal") || block_id.contains("copper") {
            200
        } else if block_id.contains("iron") {
            250
        } else {
            250
        }
    } else if block_id.contains("dirt") || block_id.contains("grass") || block_id.contains("sand") {
        30
    } else if block_id.contains("leaves") {
        20
    } else {
        100
    };

    // 工具加速：根据手持物品类型乘以倍率
    let tool_mult: f64 = if held_item.contains("_axe") {
        if held_item.contains("wooden") {
            0.5
        } else if held_item.contains("stone") {
            0.25
        } else {
            0.2
        }
    } else if held_item.contains("_pickaxe") {
        if held_item.contains("wooden") {
            0.6
        } else if held_item.contains("stone") {
            0.3
        } else {
            0.2
        }
    } else if held_item.contains("_shovel") {
        0.4
    } else if held_item.contains("_sword") && block_id.contains("leaves") {
        0.2
    } else if held_item.contains("shears") && block_id.contains("leaves") {
        0.07 // 剪刀极快
    } else {
        1.0 // 徒手
    };

    (base as f64 * tool_mult).max(5.0) as u32
}

/// 辅助：从适配器拿到最近状态并找最近目标方块。
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

// ── Collect（核心采集工具）──

pub struct ModCollectTool {
    adapter: Rc<RefCell<MinecraftModAdapter>>,
}
impl ModCollectTool {
    pub fn new(adapter: Rc<RefCell<MinecraftModAdapter>>) -> Self {
        Self { adapter }
    }
}
impl GameTool for ModCollectTool {
    fn name(&self) -> &str {
        "collect"
    }
    fn description(&self) -> &str {
        "AUTO find, walk to, and mine target blocks. Uses mod-side move_to for smooth navigation (no camera oscillation). Mod-side mining auto-stops when block breaks (no fixed tick guessing). Your primary gathering tool. count: how many to collect."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"target":{"type":"string","description":"Block ID to collect, e.g. oak_log, stone, coal_ore"},"count":{"type":"integer","description":"Number to collect","default":1}},"required":["target"]})
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
        let want = args["count"].as_u64().unwrap_or(1) as u32;
        let mut adapter = self.adapter.borrow_mut();

        let st = adapter.reload()?;
        let before: u32 = st
            .inventory
            .iter()
            .filter(|i| i.id.contains(target))
            .map(|i| i.count)
            .sum();
        let mine_ticks = mine_ticks_for(target, &st.held_item);

        let max_attempts = (want * 5).max(15);
        let mut got = before;
        for _ in 0..max_attempts {
            if got >= before + want {
                break;
            }

            // Fast path: already looking at target and close → mine directly
            let st = adapter.reload()?;
            if let Some(ref tb) = st.targeted_block
                && tb.id.contains(target)
                && tb.dist <= 4.0
            {
                adapter.execute(Action::Mine { ticks: mine_ticks })?;
                std::thread::sleep(std::time::Duration::from_millis(200));
                let st2 = adapter.reload()?;
                got = st2
                    .inventory
                    .iter()
                    .filter(|i| i.id.contains(target))
                    .map(|i| i.count)
                    .sum();
                continue;
            }

            // Use mod-side move_to: handles aiming + walking to exact world coords, no oscillation
            let Some((block, _yaw_diff)) = find_nearest(&adapter, target) else {
                return Ok(ToolResult {
                    message: format!("collected {target}: {before}→{got} (no more nearby)"),
                    is_error: got == before,
                    images: vec![],
                });
            };

            // Walk to block position using mod's built-in pathfinding
            adapter.move_to(block.x, block.y + 0.5, block.z)?;
            std::thread::sleep(std::time::Duration::from_millis(300));

            // Mine with appropriate ticks for this block type
            adapter.execute(Action::Mine { ticks: mine_ticks })?;
            std::thread::sleep(std::time::Duration::from_millis(200));

            let st2 = adapter.reload()?;
            got = st2
                .inventory
                .iter()
                .filter(|i| i.id.contains(target))
                .map(|i| i.count)
                .sum();
        }
        Ok(ToolResult {
            message: format!("collected {target}: {before}→{got} (wanted +{want})"),
            is_error: got < before + want,
            images: vec![],
        })
    }
}

// ── Craft（合成）──

pub struct ModCraftTool {
    adapter: Rc<RefCell<MinecraftModAdapter>>,
}
impl ModCraftTool {
    pub fn new(adapter: Rc<RefCell<MinecraftModAdapter>>) -> Self {
        Self { adapter }
    }
}
impl GameTool for ModCraftTool {
    fn name(&self) -> &str {
        "craft"
    }
    fn description(&self) -> &str {
        "Craft items directly via inventory manipulation. Mod handles: finding recipe, consuming materials, adding result. item: target item name like oak_planks, stick, crafting_table, wooden_pickaxe, torch. count: how many. Always check craftable() first if unsure."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"item":{"type":"string","description":"Item to craft, e.g. crafting_table, oak_planks, stick, wooden_pickaxe"},"count":{"type":"integer","description":"How many to craft","default":1}},"required":["item"]})
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

        adapter.craft(item, count)?;
        std::thread::sleep(std::time::Duration::from_millis(200));

        let st2 = adapter.reload()?;
        let after: u32 = st2
            .inventory
            .iter()
            .filter(|i| i.id.contains(item))
            .map(|i| i.count)
            .sum();

        let got = after.saturating_sub(before);
        if got >= count {
            Ok(ToolResult {
                message: format!("crafted {item} x{got} ({before}→{after})"),
                is_error: false,
                images: vec![],
            })
        } else {
            Ok(ToolResult {
                message: format!(
                    "craft {item}: only got {got}/{count}. Likely missing materials. Inventory count: {before}→{after}. Check you have enough ingredients (e.g. planks→need logs first)."
                ),
                is_error: true,
                images: vec![],
            })
        }
    }
}

// ── Place（放置方块）──

pub struct ModPlaceTool {
    adapter: Rc<RefCell<MinecraftModAdapter>>,
}
impl ModPlaceTool {
    pub fn new(adapter: Rc<RefCell<MinecraftModAdapter>>) -> Self {
        Self { adapter }
    }
}
impl GameTool for ModPlaceTool {
    fn name(&self) -> &str {
        "place"
    }
    fn description(&self) -> &str {
        "Place a block from your inventory at the targeted position. Switches to the hotbar slot containing the item, then right-clicks. item: block name to place (e.g. crafting_table, torch, oak_planks)."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"item":{"type":"string","description":"Block to place, e.g. crafting_table, torch"}},"required":["item"]})
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
        let item = args["item"].as_str().unwrap_or("crafting_table");
        let mut adapter = self.adapter.borrow_mut();

        let st = adapter.reload()?;
        let slot = st
            .inventory
            .iter()
            .filter(|i| i.id.contains(item) && i.slot < 9 && i.count > 0)
            .map(|i| i.slot)
            .next()
            .unwrap_or(0);

        // 切换到该槽位
        adapter.execute(Action::Press {
            keys: format!("{}", slot + 1),
            ticks: 3,
        })?;
        std::thread::sleep(std::time::Duration::from_millis(100));
        // 右键放置
        adapter.right_click(5)?;

        Ok(ToolResult {
            message: format!("placed {item} (slot {})", slot + 1),
            is_error: false,
            images: vec![],
        })
    }
}

// ── Equip（装备物品）──

pub struct ModEquipTool {
    adapter: Rc<RefCell<MinecraftModAdapter>>,
}
impl ModEquipTool {
    pub fn new(adapter: Rc<RefCell<MinecraftModAdapter>>) -> Self {
        Self { adapter }
    }
}
impl GameTool for ModEquipTool {
    fn name(&self) -> &str {
        "equip"
    }
    fn description(&self) -> &str {
        "Equip an item to your main hand by switching hotbar slot. slot: hotbar number 1-9 containing the item. Use before mining with proper tools."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"slot":{"type":"integer","description":"Hotbar slot 1-9 to equip"}},"required":["slot"]})
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
        let slot = args["slot"].as_u64().unwrap_or(1) as u32;
        let mut adapter = self.adapter.borrow_mut();
        adapter.execute(Action::Press {
            keys: format!("{slot}"),
            ticks: 3,
        })?;
        Ok(ToolResult {
            message: format!("equipped slot {slot}"),
            is_error: false,
            images: vec![],
        })
    }
}

// ── Item（消耗/使用物品）──

pub struct ModUseItemTool {
    adapter: Rc<RefCell<MinecraftModAdapter>>,
}
impl ModUseItemTool {
    pub fn new(adapter: Rc<RefCell<MinecraftModAdapter>>) -> Self {
        Self { adapter }
    }
}
impl GameTool for ModUseItemTool {
    fn name(&self) -> &str {
        "use_item"
    }
    fn description(&self) -> &str {
        "Right-click to use/eat the item in your main hand. Works for food (eat), placing blocks, opening doors, etc. Hold for ticks duration."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"ticks":{"type":"integer","description":"How long to hold right-click. 20=1s for eating, 5=quick use.","default":20}}})
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
        let mut adapter = self.adapter.borrow_mut();
        adapter.right_click(ticks)?;
        Ok(ToolResult {
            message: format!("right-click for {ticks} ticks (use item / eat food / open chest)"),
            is_error: false,
            images: vec![],
        })
    }
}

// ── MoveTo（导航到世界坐标）──

pub struct ModMoveToTool {
    adapter: Rc<RefCell<MinecraftModAdapter>>,
}
impl ModMoveToTool {
    pub fn new(adapter: Rc<RefCell<MinecraftModAdapter>>) -> Self {
        Self { adapter }
    }
}
impl GameTool for ModMoveToTool {
    fn name(&self) -> &str {
        "move_to"
    }
    fn description(&self) -> &str {
        "Navigate to world coordinates. Mod re-aims every tick toward the target while walking — no camera oscillation. Stops within 1.5m of target. Uses obstacle detection (strafe around walls, jump over blocks). Takes ~2-5s depending on distance."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"x":{"type":"number","description":"Target X coordinate"},"y":{"type":"number","description":"Target Y coordinate (block Y + 0.5 for center)"},"z":{"type":"number","description":"Target Z coordinate"}},"required":["x","y","z"]})
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
        self.adapter.borrow_mut().move_to(x, y, z)?;
        Ok(ToolResult {
            message: format!("moving to ({:.1},{:.1},{:.1})", x, y, z),
            is_error: false,
            images: vec![],
        })
    }
}

// ── LookAt（精确看向世界坐标）──

pub struct ModLookAtTool {
    adapter: Rc<RefCell<MinecraftModAdapter>>,
}
impl ModLookAtTool {
    pub fn new(adapter: Rc<RefCell<MinecraftModAdapter>>) -> Self {
        Self { adapter }
    }
}
impl GameTool for ModLookAtTool {
    fn name(&self) -> &str {
        "look_at"
    }
    fn description(&self) -> &str {
        "Face a specific world coordinate precisely. Uses mod's absolute look-at to snap crosshair to target. x/y/z: block coordinates from NEARBY BLOCKS. Much more accurate than look(dx,dy) for precise aiming."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"x":{"type":"number","description":"Target X coordinate"},"y":{"type":"number","description":"Target Y coordinate"},"z":{"type":"number","description":"Target Z coordinate"}},"required":["x","y","z"]})
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

// ── Craftable（查询可合成物品）──

/// 已知配方及其所需材料（与 mod 侧 craft 命令支持的配方一致）。
const RECIPES: &[(&str, &[(&str, u32)])] = &[
    ("oak_planks", &[("oak_log", 1)]),
    ("birch_planks", &[("birch_log", 1)]),
    ("stick", &[("planks", 2)]),
    ("crafting_table", &[("planks", 4)]),
    ("wooden_pickaxe", &[("planks", 3), ("stick", 2)]),
    ("wooden_axe", &[("planks", 3), ("stick", 2)]),
    ("wooden_sword", &[("planks", 2), ("stick", 1)]),
    ("wooden_shovel", &[("planks", 1), ("stick", 2)]),
    ("torch", &[("stick", 1), ("coal", 1)]),
    ("furnace", &[("cobblestone", 8)]),
];

pub struct ModCraftableTool {
    adapter: Rc<RefCell<MinecraftModAdapter>>,
}
impl ModCraftableTool {
    pub fn new(adapter: Rc<RefCell<MinecraftModAdapter>>) -> Self {
        Self { adapter }
    }
}
impl GameTool for ModCraftableTool {
    fn name(&self) -> &str {
        "craftable"
    }
    fn description(&self) -> &str {
        "Query what items you can craft with current inventory. Returns list of craftable recipes with quantities. Use before calling craft() to ensure you have materials."
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
        let mut craftable = Vec::new();

        for (item, ingredients) in RECIPES {
            let can_craft = ingredients.iter().all(|(mat, need)| {
                let have: u32 = st
                    .inventory
                    .iter()
                    .filter(|i| i.id.contains(mat))
                    .map(|i| i.count)
                    .sum();
                have >= *need
            });
            if can_craft {
                let max_count = ingredients
                    .iter()
                    .map(|(mat, need)| {
                        let have: u32 = st
                            .inventory
                            .iter()
                            .filter(|i| i.id.contains(mat))
                            .map(|i| i.count)
                            .sum();
                        have / need
                    })
                    .min()
                    .unwrap_or(0);
                craftable.push(format!("  {item} x{max_count}"));
            }
        }

        if craftable.is_empty() {
            Ok(ToolResult {
                message: "CRAFTABLE: nothing (gather more materials first)".into(),
                is_error: false,
                images: vec![],
            })
        } else {
            Ok(ToolResult {
                message: format!("CRAFTABLE:\n{}", craftable.join("\n")),
                is_error: false,
                images: vec![],
            })
        }
    }
}

// ── SearchForBlock（Mindcraft !searchForBlock 风格）──

pub struct ModSearchBlockTool {
    adapter: Rc<RefCell<MinecraftModAdapter>>,
}
impl ModSearchBlockTool {
    pub fn new(adapter: Rc<RefCell<MinecraftModAdapter>>) -> Self {
        Self { adapter }
    }
}
impl GameTool for ModSearchBlockTool {
    fn name(&self) -> &str {
        "searchForBlock"
    }
    fn description(&self) -> &str {
        "Find the nearest block of a given type and navigate to it. Does NOT mine the block — just walks to it. Use this before placing blocks nearby or for exploration. Returns what was found and distance moved."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"type":{"type":"string","description":"Block type to search for, e.g. oak_log, stone, crafting_table"}},"required":["type"]})
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
        let adapter = self.adapter.borrow_mut();
        let Some((block, _)) = find_nearest(&adapter, target) else {
            return Ok(ToolResult {
                message: format!("searchForBlock: no {target} found nearby"),
                is_error: true,
                images: vec![],
            });
        };
        adapter.move_to(block.x, block.y + 0.5, block.z)?;
        let dist = format!("{:.1}m", block.dist);
        Ok(ToolResult {
            message: format!(
                "walked to nearest {} at ({:.0},{:.0},{:.0}), distance was {}",
                target, block.x, block.y, block.z, dist
            ),
            is_error: false,
            images: vec![],
        })
    }
}

// ── 工厂 ──

/// 构建完整的 mod 工具集。enable_visual_perceive 仅在配置了 VLM 时为 true。
pub fn create_mc_mod_tools(
    adapter: Rc<RefCell<MinecraftModAdapter>>,
    image_max_side: Option<u32>,
    shots_dir: Option<PathBuf>,
    enable_visual_perceive: bool,
) -> Vec<Box<dyn GameTool>> {
    let mut tools: Vec<Box<dyn GameTool>> = vec![
        Box::new(ModPerceiveTool::new(
            adapter.clone(),
            image_max_side,
            shots_dir,
        )),
        // 低层精确工具
        Box::new(ModLookTool::new(adapter.clone())),
        Box::new(ModPressTool::new(adapter.clone())),
        Box::new(ModMineTool::new(adapter.clone())),
    ];
    // 视觉补充
    if enable_visual_perceive {
        tools.push(Box::new(ModVisualPerceiveTool::new(adapter.clone())));
    }
    // 高层 Mindcraft 风格工具
    tools.push(Box::new(ModCollectTool::new(adapter.clone())));
    tools.push(Box::new(ModCraftTool::new(adapter.clone())));
    tools.push(Box::new(ModPlaceTool::new(adapter.clone())));
    tools.push(Box::new(ModEquipTool::new(adapter.clone())));
    tools.push(Box::new(ModUseItemTool::new(adapter.clone())));
    tools.push(Box::new(ModAttackTool::new(adapter.clone())));
    // 导航与精确瞄准
    tools.push(Box::new(ModMoveToTool::new(adapter.clone())));
    tools.push(Box::new(ModLookAtTool::new(adapter.clone())));
    // 搜索工具
    tools.push(Box::new(ModSearchBlockTool::new(adapter.clone())));
    // 查询工具
    tools.push(Box::new(ModCraftableTool::new(adapter)));
    tools
}

// ── Attack（攻击实体）──

pub struct ModAttackTool {
    adapter: Rc<RefCell<MinecraftModAdapter>>,
}
impl ModAttackTool {
    pub fn new(adapter: Rc<RefCell<MinecraftModAdapter>>) -> Self {
        Self { adapter }
    }
}
impl GameTool for ModAttackTool {
    fn name(&self) -> &str {
        "attack"
    }
    fn description(&self) -> &str {
        "Attack the nearest hostile entity by holding left-click. ticks: duration (30≈1.5s). Use when hostile mobs are nearby (shown in NEARBY ENTITIES section)."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"ticks":{"type":"integer","description":"Attack duration in ticks","default":30}}})
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
        self.adapter.borrow().attack(ticks)?;
        Ok(ToolResult {
            message: format!("attacked for {ticks} ticks"),
            is_error: false,
            images: vec![],
        })
    }
}
