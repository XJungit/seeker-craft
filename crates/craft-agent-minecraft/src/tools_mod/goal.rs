//! 目标引擎工具（GoalEngine）：LLM 发目标，Mod 自动执行。
//! 替代 LLM 手动调多个工具，一步完成复合操作。

use crate::tool_args::schema;
use crate::tools_mod::MinecraftModAdapter;
use crate::tools_mod::SafeLockAdapter;
use craft_agent::core::tool::{GameTool, ToolEffects, ToolResult, ToolUpdateFn};
use serde_json::Value;
use std::sync::Arc;
use std::sync::Mutex;

pub struct ModGoalExecuteTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModGoalExecuteTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModGoalExecuteTool {
    fn name(&self) -> &str {
        "goal_execute"
    }
    fn description(&self) -> &str {
        "Execute a compound goal automatically. Mod handles all sub-steps: material checking, gathering, crafting, smelting, equipping. type: goal type (craft/get/hunt/smelt/enchant/build/explore/defend). param: item name for craft/get/smelt/enchant/build; ignored for hunt/explore/defend. count: how many (default 1). Use goal_status to check progress. Usage: goal_execute(type=\"craft\", param=\"iron_pickaxe\")  goal_execute(type=\"get\", param=\"stone\", count=20)  goal_execute(type=\"hunt\")  goal_execute(type=\"smelt\", param=\"raw_iron\", count=3)  goal_execute(type=\"build\", param=\"oak_planks\")  goal_execute(type=\"explore\")  goal_execute(type=\"defend\")"
    }
    fn parameters(&self) -> Value {
        schema::object()
            .str_req("type", "Goal type: craft, get, hunt, smelt, enchant, build, explore, defend")
            .str_opt(
                "param",
                "Item name (e.g. iron_pickaxe, stone, raw_iron) for craft/get/smelt/enchant/build",
                "",
            )
            .int_opt("count", "How many to craft/get/smelt", 1, 1, 64)
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
        let goal_type = args["type"].as_str().unwrap_or("craft");
        let param = args["param"].as_str().unwrap_or("");
        let count = args["count"].as_u64().unwrap_or(1) as u32;
        let ack = self
            .adapter
            .lock_adapter()?
            .goal_execute(goal_type, param, count)?;
        Ok(ToolResult {
            message: ack.detail,
            is_error: ack.status != "ok",
            images: vec![],
        })
    }
}

pub struct ModGoalStatusTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModGoalStatusTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}
impl GameTool for ModGoalStatusTool {
    fn name(&self) -> &str {
        "goal_status"
    }
    fn description(&self) -> &str {
        "Check the status of the active goal engine. Returns current state (idle/running/done/failed) with result message. Use after goal_execute to check if the compound task completed."
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
        let ack = self.adapter.lock_adapter()?.goal_status()?;
        Ok(ToolResult {
            message: ack.detail,
            is_error: false,
            images: vec![],
        })
    }
}
