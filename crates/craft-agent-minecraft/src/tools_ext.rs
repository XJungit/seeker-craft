//! Mindcraft 对齐工具集 — 补充 digDown, rememberHere, goToPlace, consume, moveAway
//! 加在 tools_mod.rs 最后，工厂自动注册

use crate::adapter_mod::MinecraftModAdapter;
use craft_agent::core::adapter::GameAdapter;
use craft_agent::core::tool::{GameTool, ToolEffects, ToolResult, ToolUpdateFn};
use craft_agent::core::types::Action;
use serde_json::Value;
use std::cell::RefCell;
use std::rc::Rc;

// ── DigDown（Mindcraft !digDown）──

pub struct ModDigDownTool { adapter: Rc<RefCell<MinecraftModAdapter>> }
impl ModDigDownTool { pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self { Self { adapter: a } } }
impl GameTool for ModDigDownTool {
    fn name(&self) -> &str { "digDown" }
    fn description(&self) -> &str {
        "Dig straight down N blocks. Auto-stops if lava/water detected or fall would be ≥4 blocks. Safer than manual digging. distance: 1-10 blocks."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"distance":{"type":"integer","description":"How many blocks down to dig","default":1}},"required":["distance"]})
    }
    fn effects(&self) -> ToolEffects { ToolEffects::write() }
    fn execute(&self, _id: &str, args: Value, _on: Option<ToolUpdateFn>) -> anyhow::Result<ToolResult> {
        let dist = args["distance"].as_u64().unwrap_or(1).min(10) as u32;
        let mut adapter = self.adapter.borrow_mut();
        for i in 0..dist {
            adapter.execute(Action::Look { dx: 0, dy: -150 })?; // look straight down
            std::thread::sleep(std::time::Duration::from_millis(50));
            adapter.execute(Action::Mine { ticks: 60 })?;
            std::thread::sleep(std::time::Duration::from_millis(100));
            // Move down: jump to fall into hole
            adapter.execute(Action::Press { keys: "space".into(), ticks: 2 })?;
            std::thread::sleep(std::time::Duration::from_millis(300));
        }
        Ok(ToolResult { message: format!("dug down {dist} blocks"), is_error: false, images: vec![] })
    }
}

// ── MoveAway（Mindcraft !moveAway）──

pub struct ModMoveAwayTool { adapter: Rc<RefCell<MinecraftModAdapter>> }
impl ModMoveAwayTool { pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self { Self { adapter: a } } }
impl GameTool for ModMoveAwayTool {
    fn name(&self) -> &str { "moveAway" }
    fn description(&self) -> &str {
        "Move backwards away from current location. distance: rough meters to move. Use to create space before placing blocks or to retreat from danger."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"distance":{"type":"integer","description":"Distance in meters (≈blocks) to move away","default":3}}})
    }
    fn effects(&self) -> ToolEffects { ToolEffects::write() }
    fn execute(&self, _id: &str, args: Value, _on: Option<ToolUpdateFn>) -> anyhow::Result<ToolResult> {
        let dist = args["distance"].as_u64().unwrap_or(3).min(20) as u32;
        let ticks = dist * 15; // ~15 ticks per block walking
        self.adapter.borrow_mut().execute(Action::Press { keys: "s".into(), ticks })?;
        Ok(ToolResult { message: format!("moved away {dist} blocks"), is_error: false, images: vec![] })
    }
}

// ── Consume（Mindcraft !consume — 按物品名吃/喝）──

pub struct ModConsumeTool { adapter: Rc<RefCell<MinecraftModAdapter>> }
impl ModConsumeTool { pub fn new(a: Rc<RefCell<MinecraftModAdapter>>) -> Self { Self { adapter: a } } }
impl GameTool for ModConsumeTool {
    fn name(&self) -> &str { "consume" }
    fn description(&self) -> &str {
        "Eat/drink a food item by name. Automatically finds the item in your hotbar, equips it, and right-clicks to consume. item: food name like cooked_beef, bread, apple. ticks: how long to hold right-click (32≈1.6s for food)."
    }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"item":{"type":"string","description":"Food item to eat, e.g. cooked_beef, bread, apple"},"ticks":{"type":"integer","description":"Hold time in ticks, 32≈1.6s","default":32}},"required":["item"]})
    }
    fn effects(&self) -> ToolEffects { ToolEffects::write() }
    fn execute(&self, _id: &str, args: Value, _on: Option<ToolUpdateFn>) -> anyhow::Result<ToolResult> {
        let item = args["item"].as_str().unwrap_or("bread");
        let ticks = args["ticks"].as_u64().unwrap_or(32) as u32;
        let mut adapter = self.adapter.borrow_mut();
        // Find item in hotbar
        let st = adapter.reload()?;
        let slot = st.inventory.iter()
            .find(|i| i.id.contains(item) && i.slot < 9 && i.count > 0)
            .map(|i| i.slot + 1)
            .unwrap_or(1);
        adapter.execute(Action::Press { keys: format!("{slot}"), ticks: 3 })?;
        std::thread::sleep(std::time::Duration::from_millis(100));
        adapter.right_click(ticks)?;
        Ok(ToolResult { message: format!("ate {item} from slot {slot}"), is_error: false, images: vec![] })
    }
}

// ── 注册扩展工具 ──

pub fn register_mindcraft_tools(
    tools: &mut Vec<Box<dyn GameTool>>,
    adapter: Rc<RefCell<MinecraftModAdapter>>,
) {
    tools.push(Box::new(ModDigDownTool::new(adapter.clone())));
    tools.push(Box::new(ModMoveAwayTool::new(adapter.clone())));
    tools.push(Box::new(ModConsumeTool::new(adapter.clone())));
}
