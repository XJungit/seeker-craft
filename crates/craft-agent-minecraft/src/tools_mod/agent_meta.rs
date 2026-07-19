//! Agent-local 元工具（Numen 风格：无需 mod 通信）。

use crate::tool_args::schema;
use craft_agent::core::tool::{GameTool, ToolEffects, ToolResult, ToolUpdateFn};
use serde_json::Value;

pub struct NumenTodoWriteTool;
impl GameTool for NumenTodoWriteTool {
    fn name(&self) -> &str {
        "todowrite"
    }
    fn description(&self) -> &str {
        "Write a todo list for your next steps. Each todo must have: content (what to do), status (pending/in_progress/completed/cancelled), priority (high/medium/low). Keep EXACTLY ONE todo in_progress at a time. The list replaces ALL previous todos — send the full updated list each time. Usage: todowrite(todos=[{content:\"Mine iron\",status:\"in_progress\",priority:\"high\"},{content:\"Smelt ore\",status:\"pending\",priority:\"medium\"}])"
    }
    fn parameters(&self) -> Value {
        schema::object()
            .str_req("todos", "JSON array of todo objects: [{\"content\":\"...\",\"status\":\"pending|in_progress|completed|cancelled\",\"priority\":\"high|medium|low\"}]")
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
        // 回显给 LLM，不做持久化（LLM 管理自己的 todo list）
        let raw = serde_json::to_string_pretty(&args).unwrap_or_default();
        Ok(ToolResult {
            message: format!("todos updated. Current plan:\n{raw}"),
            is_error: false,
            images: vec![],
        })
    }
}

pub struct NumenStatusTool;
impl GameTool for NumenStatusTool {
    fn name(&self) -> &str {
        "agent_status"
    }
    fn description(&self) -> &str {
        "Report your current agent-internal status: mode flags, saved places, and active goals. No arguments needed. Usage: agent_status()"
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
            message: "agent_status: use perceive() for game state, list_modes()/savedPlaces()/get_goal() for specific info".into(),
            is_error: false,
            images: vec![],
        })
    }
}
