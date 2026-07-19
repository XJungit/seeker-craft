//! 感知工具：perceive（读游戏状态）与 visual_perceive（GUI 截图+VLM）。

use crate::tool_args::schema;
use crate::tools_mod::MinecraftModAdapter;
use crate::tools_mod::SafeLockAdapter;
use base64::Engine as _;
use craft_agent::core::adapter::GameAdapter;
use craft_agent::core::tool::{GameTool, ToolEffects, ToolResult, ToolUpdateFn};
use craft_agent_model::vision::real::downscale_png;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};

pub struct ModPerceiveTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
    image_max_side: Option<u32>,
    shots_dir: Option<PathBuf>,
    counter: AtomicU32,
}
impl ModPerceiveTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>, im: Option<u32>, sd: Option<PathBuf>) -> Self {
        Self {
            adapter: a,
            image_max_side: im,
            shots_dir: sd,
            counter: AtomicU32::new(0),
        }
    }
    fn save_shot(&self, png: &[u8]) -> Option<String> {
        let dir = self.shots_dir.as_ref()?;
        let n = self.counter.load(Ordering::Relaxed) + 1;
        self.counter.store(n, Ordering::Relaxed);
        let rel = dir.join(format!("step-{n:03}.png"));
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
        "Read full game state via mod (latency <100ms, no side effects). Returns: position(x/y/z), yaw/pitch, health/hunger, gamemode/biome/dimension, light levels, weather, ALL inventory items (hotbar slots 1-9 + main inventory), targeted block (what crosshair points at), nearby blocks (top 30 by relevance), nearby entities. This data is auto-injected each turn — you rarely need to call this manually. Usage: perceive() — no arguments needed."
    }
    fn parameters(&self) -> Value {
        schema::no_args()
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
        let ws = self.adapter.lock_adapter()?.perceive()?;
        // mod perceive 返回纯结构化数据（无截图）。
        // screenshot 为空时不保存文件、不生成 images，避免 0 字节空文件误导。
        // 截图留给 visual_perceive 工具（需要时手动调用）。
        let images = if !ws.screenshot.is_empty() {
            let scaled: Vec<u8> = match self.image_max_side {
                Some(ms) => downscale_png(&ws.screenshot, ms)
                    .map(|r| r.0)
                    .unwrap_or_default(),
                None => ws.screenshot.to_vec(),
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

pub struct ModVisualPerceiveTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModVisualPerceiveTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
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
        schema::object()
            .str_req(
                "prompt",
                "What to look for, e.g. 'What does the crafting table show?'",
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
        let prompt = args["prompt"].as_str().unwrap_or("Describe the screen");
        match self.adapter.lock_adapter()?.perceive_visual(prompt) {
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
