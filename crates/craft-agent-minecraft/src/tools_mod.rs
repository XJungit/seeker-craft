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
        "Rotate camera: dx>0 turns right, dx<0 turns left (300≈90°). dy>0 looks down, dy<0 looks up. Use for precise aiming before collect/mine."
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
        "Hold left-click to mine the targeted block. Returns whether logs were actually collected. Wood=60ticks(3s), stone=120ticks(6s). Use collect() for automatic gathering instead."
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
        "AUTO find, aim, walk to, and mine blocks. Your primary tool for gathering resources. target: block ID (e.g. oak_log, birch_log, stone, coal_ore). count: how many to collect. Each block takes ~3-5s. Returns collected count."
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

        let max_attempts = (want * 5).max(15);
        let mut got = before;
        for _ in 0..max_attempts {
            if got >= before + want {
                break;
            }

            // Fast path: already looking at the target and close enough → mine directly
            let st = adapter.reload()?;
            if let Some(ref tb) = st.targeted_block
                && tb.id.contains(target)
                && tb.dist <= 4.0
            {
                adapter.execute(Action::Mine { ticks: 60 })?;
                std::thread::sleep(std::time::Duration::from_millis(150));
                let st2 = adapter.reload()?;
                got = st2
                    .inventory
                    .iter()
                    .filter(|i| i.id.contains(target))
                    .map(|i| i.count)
                    .sum();
                continue;
            }

            let Some((block, yaw_diff)) = find_nearest(&adapter, target) else {
                return Ok(ToolResult {
                    message: format!("collected {target}: {before}→{got} (no more nearby)"),
                    is_error: got == before,
                    images: vec![],
                });
            };
            if yaw_diff.abs() > 10.0 {
                adapter.execute(Action::Look {
                    dx: yaw_diff as i32,
                    dy: 0,
                })?;
                std::thread::sleep(std::time::Duration::from_millis(80));
                continue;
            }
            if block.dist > 3.5 {
                let walk = ((block.dist * 10.0) as u32).min(80).max(5);
                adapter.execute(Action::Press {
                    keys: "w".into(),
                    ticks: walk,
                })?;
                std::thread::sleep(std::time::Duration::from_millis(200));
                continue;
            }
            adapter.execute(Action::Mine { ticks: 60 })?;
            std::thread::sleep(std::time::Duration::from_millis(150));
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
        "Craft an item from inventory materials. Mod handles the recipe automatically (2x2 or 3x3). No visual input needed. item: what to craft (oak_planks, stick, crafting_table, wooden_pickaxe, wooden_axe, wooden_sword, torch). count: how many. Recipe list: 1 log→4 planks, 2 planks→4 sticks, 4 planks→1 crafting_table, 3 planks+2 sticks→wooden_pickaxe/axe, 2 planks+1 stick→wooden_sword, 1 stick+1 coal→4 torches."
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
    tools.push(Box::new(ModAttackTool::new(adapter)));
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
