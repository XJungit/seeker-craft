//! 步骤序列执行引擎：LLM 生成 JSON 计划，Rust 解释执行。
//! 支持顺序/条件/循环/wait，组合已有工具完成复杂任务。

use crate::adapter_mod::MinecraftModAdapter;
use crate::tool_args::schema;
use craft_agent::core::tool::{GameTool, ToolEffects, ToolResult, ToolUpdateFn};
use serde_json::Value;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

pub struct ModExecutePlanTool {
    adapter: Arc<Mutex<MinecraftModAdapter>>,
}
impl ModExecutePlanTool {
    pub fn new(a: Arc<Mutex<MinecraftModAdapter>>) -> Self {
        Self { adapter: a }
    }
}

impl GameTool for ModExecutePlanTool {
    fn name(&self) -> &str {
        "execute_plan"
    }
    fn description(&self) -> &str {
        "Execute a plan (JSON array of steps). Supports: tool calls, if-then-else conditions, loops, and wait. Each step: {\"tool\":\"name\",\"args\":{...}} or {\"if\":{\"state\":\"...\",\"op\":\"...\",\"value\":...},\"then\":[...],\"else\":[...]} or {\"loop\":{\"times\":N},\"do\":[...]} or {\"wait\":{\"seconds\":N}}. Condition state: health, hunger, has_item, has_entity, inventory_full. Example: [{\"tool\":\"nav_to\",\"args\":{\"x\":12,\"y\":64,\"z\":8}},{\"if\":{\"state\":\"has_item\",\"args\":{\"item\":\"iron_ore\",\"count\":3}},\"then\":[{\"tool\":\"goal_execute\",\"args\":{\"type\":\"smelt\",\"param\":\"raw_iron\",\"count\":3}}]}]. Usage: execute_plan(plan=\"[{\\\"tool\\\":\\\"nav_to\\\",\\\"args\\\":{\\\"x\\\":12,\\\"y\\\":64,\\\"z\\\":8}}]\")"
    }
    fn parameters(&self) -> Value {
        schema::object()
            .str_req("plan", "JSON array of steps")
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
        let plan_str = args["plan"].as_str().unwrap_or("[]");
        let plan: Vec<Value> = serde_json::from_str(plan_str)
            .map_err(|e| anyhow::anyhow!("invalid plan JSON: {e}"))?;

        let mut step_results: Vec<String> = Vec::new();
        for (i, step) in plan.iter().enumerate() {
            match execute_step(&self.adapter, step) {
                Ok(result) => {
                    step_results.push(format!("step{}: {result}", i + 1));
                    if result.starts_with("FAILED") {
                        return Ok(ToolResult {
                            message: format!(
                                "Plan failed at step {}: {}\nSuccessful steps:\n{}",
                                i + 1,
                                result,
                                step_results[..i].join("\n")
                            ),
                            is_error: true,
                            images: vec![],
                        });
                    }
                }
                Err(e) => {
                    return Ok(ToolResult {
                        message: format!(
                            "Plan error at step {}: {}\nSuccessful steps:\n{}",
                            i + 1,
                            e,
                            step_results[..i].join("\n")
                        ),
                        is_error: true,
                        images: vec![],
                    });
                }
            }
        }

        Ok(ToolResult {
            message: format!("Plan completed:\n{}", step_results.join("\n")),
            is_error: false,
            images: vec![],
        })
    }
}

fn execute_step(
    adapter: &Arc<Mutex<MinecraftModAdapter>>,
    step: &Value,
) -> Result<String, anyhow::Error> {
    // Tool call step
    if let Some(tool) = step.get("tool").and_then(|v| v.as_str()) {
        return execute_tool(adapter, tool, step.get("args").unwrap_or(&Value::Null));
    }

    // If-then-else step
    if let Some(cond) = step.get("if") {
        let condition_met = evaluate_condition(adapter, cond)?;
        let branch = if condition_met {
            step.get("then").unwrap_or(&Value::Null)
        } else {
            step.get("else").unwrap_or(&Value::Null)
        };
        if let Some(steps) = branch.as_array() {
            for s in steps.iter() {
                execute_step(adapter, s)?;
            }
        }
        return Ok(if condition_met {
            "condition: true"
        } else {
            "condition: false"
        }
        .into());
    }

    // Loop step
    if let Some(loop_def) = step.get("loop") {
        let times = loop_def.get("times").and_then(|v| v.as_u64()).unwrap_or(1);
        let empty_vec: Vec<Value> = Vec::new();
        let body = loop_def
            .get("do")
            .and_then(|v| v.as_array())
            .unwrap_or(&empty_vec);
        for _i in 0..times {
            for s in body {
                execute_step(adapter, s)?;
            }
        }
        return Ok(format!("loop: {times} iterations"));
    }

    // Wait step
    if let Some(wait) = step.get("wait") {
        let secs = wait.get("seconds").and_then(|v| v.as_u64()).unwrap_or(1);
        std::thread::sleep(Duration::from_secs(secs));
        return Ok(format!("waited {secs}s"));
    }

    Err(anyhow::anyhow!("unknown step type: {step}"))
}

fn execute_tool(
    adapter: &Arc<Mutex<MinecraftModAdapter>>,
    tool: &str,
    args: &Value,
) -> Result<String, anyhow::Error> {
    let a = adapter.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
    match tool {
        "nav_to" => {
            let x = args["x"].as_f64().unwrap_or(0.0);
            let y = args["y"].as_f64().unwrap_or(0.0);
            let z = args["z"].as_f64().unwrap_or(0.0);
            let ack = a.nav_to(x, y, z)?;
            wait_for_nav(adapter, 200)?;
            Ok(format!("nav_to -> {}", ack.detail))
        }
        "collect" => {
            let target = args["target"].as_str().unwrap_or("oak_log");
            let count = args["count"].as_u64().unwrap_or(1) as u32;
            let _ = a.collect_start(target, count)?;
            wait_for_collect(adapter, 300)?;
            let status = a.collect_status()?;
            Ok(format!("collect -> {}", status.detail))
        }
        "combat" => {
            let mode = args["mode"].as_str().unwrap_or("melee");
            let ticks = args["ticks"].as_u64().unwrap_or(100) as u32;
            let _ = a.combat_start(mode, ticks)?;
            std::thread::sleep(Duration::from_millis(ticks as u64 * 50 + 500));
            let status = a.combat_status()?;
            Ok(format!("combat -> {}", status.detail))
        }
        "goal_execute" => {
            let goal_type = args["type"].as_str().unwrap_or("craft");
            let param = args["param"].as_str().unwrap_or("");
            let count = args["count"].as_u64().unwrap_or(1) as u32;
            let _ack = a.goal_execute(goal_type, param, count)?;
            wait_for_goal(adapter, 600)?;
            let status = a.goal_status()?;
            Ok(format!("goal_execute -> {}", status.detail))
        }
        "wait" => {
            let secs = args["seconds"].as_u64().unwrap_or(1);
            std::thread::sleep(Duration::from_secs(secs));
            Ok(format!("waited {secs}s"))
        }
        other => Err(anyhow::anyhow!("unknown tool in plan: {other}")),
    }
}

fn evaluate_condition(
    adapter: &Arc<Mutex<MinecraftModAdapter>>,
    cond: &Value,
) -> Result<bool, anyhow::Error> {
    let state = cond.get("state").and_then(|v| v.as_str()).unwrap_or("");
    let op = cond.get("op").and_then(|v| v.as_str()).unwrap_or("gte");
    let val = cond.get("value");

    let a = adapter.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
    let st = a.reload()?;

    match state {
        "health" => {
            let threshold = val.and_then(|v| v.as_f64()).unwrap_or(10.0);
            Ok(compare(st.health as f64, op, threshold))
        }
        "hunger" => {
            let threshold = val.and_then(|v| v.as_f64()).unwrap_or(10.0);
            Ok(compare(st.hunger as f64, op, threshold))
        }
        "has_item" => {
            let item = cond
                .get("args")
                .and_then(|a| a.get("item"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let need = cond
                .get("args")
                .and_then(|a| a.get("count"))
                .and_then(|v| v.as_u64())
                .unwrap_or(1);
            let have: u32 = st
                .inventory
                .iter()
                .filter(|i| i.id.contains(item))
                .map(|i| i.count)
                .sum();
            Ok(compare(have as f64, op, need as f64))
        }
        "has_entity" => {
            let etype = cond
                .get("args")
                .and_then(|a| a.get("type"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let has = st.entities.iter().any(|e| e.r#type.contains(etype));
            Ok(has)
        }
        "inventory_full" => {
            let free = st.inventory.iter().filter(|i| i.id.contains("air")).count();
            Ok(free <= 2)
        }
        _ => Ok(false),
    }
}

fn compare(a: f64, op: &str, b: f64) -> bool {
    match op {
        "lt" => a < b,
        "lte" => a <= b,
        "gt" => a > b,
        "gte" => a >= b,
        "eq" => (a - b).abs() < 0.01,
        "neq" => (a - b).abs() >= 0.01,
        _ => a >= b,
    }
}

fn wait_for_nav(
    adapter: &Arc<Mutex<MinecraftModAdapter>>,
    max_ticks: u32,
) -> Result<(), anyhow::Error> {
    for _ in 0..max_ticks {
        let a = adapter.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let status = a.nav_status()?;
        if status.detail.contains("arrived")
            || status.detail.contains("idle")
            || status.detail.contains("failed")
        {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Ok(())
}

fn wait_for_collect(
    adapter: &Arc<Mutex<MinecraftModAdapter>>,
    max_ticks: u32,
) -> Result<(), anyhow::Error> {
    for _ in 0..max_ticks {
        let a = adapter.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let status = a.collect_status()?;
        if status.detail.contains("done") || status.detail.contains("idle") {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Ok(())
}

fn wait_for_goal(
    adapter: &Arc<Mutex<MinecraftModAdapter>>,
    max_ticks: u32,
) -> Result<(), anyhow::Error> {
    for _ in 0..max_ticks {
        let a = adapter.lock().map_err(|e| anyhow::anyhow!("{e}"))?;
        let status = a.goal_status()?;
        if status.detail.contains("done")
            || status.detail.contains("idle")
            || status.detail.contains("failed")
        {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Ok(())
}
