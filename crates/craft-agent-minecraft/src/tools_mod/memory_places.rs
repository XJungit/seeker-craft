// ═══════════════════════════════════════════════════════════════
// 记忆/地点工具（rememberHere / goToRememberedPlace / savedPlaces）
// 从 tools_mod.rs 拆分到本子模块（重构 ②）
// ═══════════════════════════════════════════════════════════════

use crate::tool_args::schema;
use crate::tools_mod::MinecraftModAdapter;
use crate::tools_mod::SafeLockAdapter;
use craft_agent::core::tool::{GameTool, ToolEffects, ToolResult, ToolUpdateFn};
use serde_json::Value;
use std::sync::Arc;
use std::sync::Mutex;

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
        Ok(ToolResult {
            message: self.adapter.lock_adapter()?.list_places(),
            is_error: false,
            images: vec![],
        })
    }
}
