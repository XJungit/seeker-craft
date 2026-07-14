//! Minecraft 工具 — 每个工具自己执行 (pi 风格)
//!
//! 工具拥有自己的资源, Agent 只调 execute()。

use craft_agent::core::tool::{GameTool, ToolEffects, ToolResult};
use craft_agent_model::vision::VisionClient;
use enigo::{Keyboard, Mouse};
use serde_json::Value;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

pub struct PerceiveTool {
    vlm: Arc<dyn VisionClient>,
    capture: Box<dyn Fn() -> anyhow::Result<Vec<u8>>>,
}
impl PerceiveTool {
    pub fn new(vlm: Arc<dyn VisionClient>, capture: Box<dyn Fn() -> anyhow::Result<Vec<u8>>>) -> Self {
        Self { vlm, capture }
    }
}
impl GameTool for PerceiveTool {
    fn name(&self) -> &str { "perceive" }
    fn description(&self) -> &str { "拍照观察。prompt用英文问清楚周围有什么。" }
    fn parameters(&self) -> Value { serde_json::json!({"type":"object","properties":{"prompt":{"type":"string"}}}) }
    fn effects(&self) -> ToolEffects { ToolEffects::read_only() }
    fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let prompt = args["prompt"].as_str().unwrap_or("Describe Minecraft scene. List visible blocks.");
        let png = (self.capture)()?;
        Ok(ToolResult { message: self.vlm.chat(&png, prompt)?, is_error: false })
    }
}

pub struct PressTool {
    enigo: Rc<RefCell<enigo::Enigo>>,
}
impl PressTool {
    pub fn new(enigo: Rc<RefCell<enigo::Enigo>>) -> Self { Self { enigo } }
}
impl GameTool for PressTool {
    fn name(&self) -> &str { "press" }
    fn description(&self) -> &str { "按键。w/a/s/d移动, space跳, e背包。" }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"keys":{"type":"string"},"ticks":{"type":"integer","default":40}},"required":["keys"]})
    }
    fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let keys = args["keys"].as_str().unwrap_or("w");
        let ticks = args["ticks"].as_u64().unwrap_or(40) as u64;
        let ms = ticks * 50;
        let mut e = self.enigo.borrow_mut();
        for ch in keys.chars().filter(|c| !c.is_whitespace() && *c != '+') {
            let _ = e.key(enigo::Key::Unicode(ch), enigo::Direction::Press);
        }
        std::thread::sleep(std::time::Duration::from_millis(ms));
        for ch in keys.chars().filter(|c| !c.is_whitespace() && *c != '+') {
            let _ = e.key(enigo::Key::Unicode(ch), enigo::Direction::Release);
        }
        Ok(ToolResult { message: format!("press {keys} {ms}ms"), is_error: false })
    }
}

pub struct LookTool;
impl LookTool {
    pub fn new() -> Self { Self {} }
}
impl GameTool for LookTool {
    fn name(&self) -> &str { "look" }
    fn description(&self) -> &str { "转动视角。dx>0右, dy>0低头。300≈90度。" }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"dx":{"type":"integer"},"dy":{"type":"integer"}},"required":["dx","dy"]})
    }
    fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let dx = args["dx"].as_i64().unwrap_or(0) as i32;
        let dy = args["dy"].as_i64().unwrap_or(0) as i32;
        #[cfg(windows)]
        crate::adapter::raw_mouse_rel(dx, dy)?;
        Ok(ToolResult { message: format!("look dx={dx} dy={dy}"), is_error: false })
    }
}

pub struct MineTool {
    enigo: Rc<RefCell<enigo::Enigo>>,
}
impl MineTool {
    pub fn new(enigo: Rc<RefCell<enigo::Enigo>>) -> Self { Self { enigo } }
}
impl GameTool for MineTool {
    fn name(&self) -> &str { "mine" }
    fn description(&self) -> &str { "按住左键挖掘。先perceive确认对准再用。" }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"ticks":{"type":"integer","description":"60≈3秒","default":60}}})
    }
    fn effects(&self) -> ToolEffects { ToolEffects::destructive() }
    fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let ticks = args["ticks"].as_u64().unwrap_or(60) as u64;
        let ms = ticks * 50;
        let mut e = self.enigo.borrow_mut();
        e.button(enigo::Button::Left, enigo::Direction::Press)?;
        std::thread::sleep(std::time::Duration::from_millis(ms));
        e.button(enigo::Button::Left, enigo::Direction::Release)?;
        Ok(ToolResult { message: format!("mine {ms}ms"), is_error: false })
    }
}

pub fn create_mc_tools(
    vlm: Arc<dyn VisionClient>,
    capture: Box<dyn Fn() -> anyhow::Result<Vec<u8>>>,
    enigo: Rc<RefCell<enigo::Enigo>>,
) -> Vec<Box<dyn GameTool>> {
    vec![
        Box::new(PerceiveTool::new(vlm, capture)),
        Box::new(PressTool::new(enigo.clone())),
        Box::new(LookTool::new()),
        Box::new(MineTool::new(enigo)),
    ]
}
