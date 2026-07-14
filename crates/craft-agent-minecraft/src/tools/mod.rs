//! Minecraft 工具 — 每个工具自己完整执行

use craft_agent::core::tool::{GameTool, ToolEffects, ToolResult};
use craft_agent_model::vision::VisionClient;
use enigo::{Keyboard, Mouse};
use serde_json::Value;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

// ── Perceive ──

pub struct PerceiveTool {
    vlm: Arc<dyn VisionClient>,
    capture: Box<dyn Fn() -> anyhow::Result<Vec<u8>>>,
}
impl PerceiveTool {
    pub fn new(vlm: Arc<dyn VisionClient>, capture: Box<dyn Fn() -> anyhow::Result<Vec<u8>>>) -> Self { Self { vlm, capture } }
}
impl GameTool for PerceiveTool {
    fn name(&self) -> &str { "perceive" }
    fn description(&self) -> &str {
        "拍照观察周围。prompt用英文, 问清楚: 前方有什么方块和生物? 树在哪? 石头在哪? 有没有怪物?"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"prompt":{"type":"string","description":"英文提示词, 如: Describe the Minecraft scene. List trees, stones, animals, monsters, water near the crosshair."}}})
    }
    fn effects(&self) -> ToolEffects { ToolEffects::read_only() }
    fn execute(&self, _id: &str, args: Value) -> anyhow::Result<ToolResult> {
        let prompt = args["prompt"].as_str()
            .unwrap_or("Describe the Minecraft scene. List all visible blocks and entities near the crosshair.");
        let png = (self.capture)()?;
        Ok(ToolResult { message: self.vlm.chat(&png, prompt)?, is_error: false })
    }
}

// ── Press ──

pub struct PressTool {
    enigo: Rc<RefCell<enigo::Enigo>>,
    focus: Box<dyn Fn()>,
}
impl PressTool {
    pub fn new(enigo: Rc<RefCell<enigo::Enigo>>, focus: Box<dyn Fn()>) -> Self { Self { enigo, focus } }
}
impl GameTool for PressTool {
    fn name(&self) -> &str { "press" }
    fn description(&self) -> &str {
        "按下按键。支持: w/a/s/d(移动), space(跳), shift(潜行), e(背包), 1-9(快捷栏), ctrl(疾跑), q(丢弃), f(副手)"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"keys":{"type":"string","description":"按键字母, 如 w, space, e, shift, 1-9"},"ticks":{"type":"integer","description":"持续时间, 20≈1秒, 40≈2秒","default":20}},"required":["keys"]})
    }
    fn execute(&self, _id: &str, args: Value) -> anyhow::Result<ToolResult> {
        (self.focus)();
        let keys = args["keys"].as_str().unwrap_or("w");
        let ticks = args["ticks"].as_u64().unwrap_or(20) as u64;
        let ms = ticks * 50;
        let key = key_from_str(keys);

        let mut e = self.enigo.borrow_mut();
        if let Some(k) = key {
            e.key(k, enigo::Direction::Press)?;
            std::thread::sleep(std::time::Duration::from_millis(ms));
            e.key(k, enigo::Direction::Release)?;
        } else {
            // Unicode fallback
            let ch = keys.chars().next().unwrap_or('w');
            e.key(enigo::Key::Unicode(ch), enigo::Direction::Press)?;
            std::thread::sleep(std::time::Duration::from_millis(ms));
            e.key(enigo::Key::Unicode(ch), enigo::Direction::Release)?;
        }
        Ok(ToolResult { message: format!("press {keys} {ms}ms"), is_error: false })
    }
}

fn key_from_str(s: &str) -> Option<enigo::Key> {
    Some(match s {
        "w" | "W" => enigo::Key::W, "a" | "A" => enigo::Key::A,
        "s" | "S" => enigo::Key::S, "d" | "D" => enigo::Key::D,
        "space" | "Space" | " " => enigo::Key::Space,
        "shift" | "Shift" => enigo::Key::Shift,
        "ctrl" | "Ctrl" => enigo::Key::Control,
        "e" | "E" => enigo::Key::E, "q" | "Q" => enigo::Key::Q,
        "tab" | "Tab" => enigo::Key::Tab,
        "escape" | "Escape" | "esc" => enigo::Key::Escape,
        // digits: enigo 0.6.1 has Key::Unicode('1') but not Key::N1
        _ => return None,
    })
}

// ── Look ──

pub struct LookTool;
impl GameTool for LookTool {
    fn name(&self) -> &str { "look" }
    fn description(&self) -> &str { "转动视角。dx>0右转(300≈90度), dy>0低头, dy<0抬头。" }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"dx":{"type":"integer"},"dy":{"type":"integer"}},"required":["dx","dy"]})
    }
    fn execute(&self, _id: &str, args: Value) -> anyhow::Result<ToolResult> {
        let dx = args["dx"].as_i64().unwrap_or(0) as i32;
        let dy = args["dy"].as_i64().unwrap_or(0) as i32;
        #[cfg(windows)]
        crate::adapter::raw_mouse_rel(dx, dy)?;
        Ok(ToolResult { message: format!("look dx={dx} dy={dy}"), is_error: false })
    }
}

// ── Mine ──

pub struct MineTool {
    enigo: Rc<RefCell<enigo::Enigo>>,
    focus: Box<dyn Fn()>,
}
impl MineTool {
    pub fn new(enigo: Rc<RefCell<enigo::Enigo>>, focus: Box<dyn Fn()>) -> Self { Self { enigo, focus } }
}
impl GameTool for MineTool {
    fn name(&self) -> &str { "mine" }
    fn description(&self) -> &str { "按住左键挖掘。先perceive确认对准目标。木头=60ticks(3秒), 石头=120ticks(6秒), 矿石=200ticks(10秒)。" }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"ticks":{"type":"integer","description":"挖掘时长, 20≈1秒","default":60}}})
    }
    fn effects(&self) -> ToolEffects { ToolEffects::destructive() }
    fn execute(&self, _id: &str, args: Value) -> anyhow::Result<ToolResult> {
        (self.focus)();
        let ticks = args["ticks"].as_u64().unwrap_or(60) as u64;
        let ms = ticks * 50;
        let mut e = self.enigo.borrow_mut();
        e.button(enigo::Button::Left, enigo::Direction::Press)?;
        std::thread::sleep(std::time::Duration::from_millis(ms));
        e.button(enigo::Button::Left, enigo::Direction::Release)?;
        Ok(ToolResult { message: format!("mine {ms}ms"), is_error: false })
    }
}

// ── 工厂 ──

pub fn create_mc_tools(
    vlm: Arc<dyn VisionClient>,
    capture: Box<dyn Fn() -> anyhow::Result<Vec<u8>>>,
    enigo: Rc<RefCell<enigo::Enigo>>,
) -> Vec<Box<dyn GameTool>> {
    let f = || {}; // no-op focus — adapter handles it internally
    vec![
        Box::new(PerceiveTool::new(vlm, capture)),
        Box::new(PressTool::new(enigo.clone(), Box::new(f))),
        Box::new(LookTool),
        Box::new(MineTool::new(enigo, Box::new(f))),
    ]
}
