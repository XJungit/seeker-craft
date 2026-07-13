//! P2 Agent 自主决策闭环 — OpenAI function calling 原生支持
//! `LLM 工具调用 → 执行 → 结果回传 → 继续`
//!
//! 用法（窗口化 MC，先 F3+P 关失去焦点暂停）：
//!   cargo run -p craft-agent-minecraft --example agent_multi_step --features real [-- --steps=10]

#[cfg(feature = "real")]
fn main() -> anyhow::Result<()> {
    use craft_agent::agent::{Agent, AgentTool};
    use craft_agent_minecraft::adapter::MinecraftAdapter;
    use craft_agent_model::config::AgentConfig;
    use craft_agent_model::decision::real::OpenAiLlmClient;
    use craft_agent_model::vision::VisionClient;
    use craft_agent_model::vision::real::OpenAiVisionClient;
    use serde_json::Value;
    use std::time::Instant;

    let _ = dotenvy::dotenv();

    let args: Vec<String> = std::env::args().collect();
    let fullscreen = args.iter().any(|a| a == "--fullscreen");
    let max_turns: u32 = args
        .iter()
        .find(|a| a.starts_with("--steps="))
        .and_then(|s| s.trim_start_matches("--steps=").parse().ok())
        .unwrap_or(10);

    let cfg = AgentConfig::load("config/agent.toml")?;
    let vlm_backend = cfg.vlm.active_backend()?;
    let llm_group = cfg
        .llm
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("config/agent.toml 缺少 [llm] 段"))?;
    let llm_backend = llm_group.active_backend()?;

    let vision: Box<dyn VisionClient> = Box::new(OpenAiVisionClient::from_config(vlm_backend)?);
    let llm = OpenAiLlmClient::from_config(llm_backend)?;

    let adapter = if fullscreen {
        MinecraftAdapter::new_fullscreen(vision)?
    } else {
        MinecraftAdapter::new(vision)?
    };
    let mut agent = Agent::new(adapter);

    // ─── OpenAI function calling 工具定义 ───
    let tools: Value = serde_json::json!([
        {
            "type": "function",
            "function": {
                "name": "perceive",
                "description": "拍照并识别画面中的 3D 物体（树/石/水/动物等），返回坐标列表",
                "parameters": {
                    "type": "object",
                    "properties": {}
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "look",
                "description": "转动视角。正 dx=右转，正 dy=下看",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "dx": { "type": "integer", "description": "水平转动像素数，-500~500" },
                        "dy": { "type": "integer", "description": "垂直转动像素数，-300~300" }
                    },
                    "required": ["dx", "dy"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "move_forward",
                "description": "向前移动探索",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "ticks": { "type": "integer", "description": "移动时长（20=1秒）", "default": 30 }
                    }
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "aim_and_mine",
                "description": "对准并挖掘目标物体（树/石头/矿石）。需要先用 perceive 确认目标位置。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "target": { "type": "string", "description": "目标名称（如 tree/stone/water）" }
                    },
                    "required": ["target"]
                }
            }
        }
    ]);

    // 系统消息
    let system_msg = serde_json::json!({
        "role": "system",
        "content": "你是一个 Minecraft Agent，在生存模式下收集资源。你可以调用工具来观察世界和行动。\
            看到树、石头、矿石时优先采集。没有明确目标时探索周围。"
    });

    let mut messages: Vec<Value> = vec![system_msg];

    // MiniCPM 不支持 function calling，先用 prompt 模式
    let use_fn_calling = llm_backend.model.to_lowercase().contains("longcat")
        || llm_backend.model.to_lowercase().contains("gpt");
    println!("模型: {}  function_calling={use_fn_calling}", llm_backend.model);
    println!("轮次: {max_turns}\n");

    fn build_tool_prompt(state: Option<&craft_agent::core::types::WorldState>) -> String {
        if let Some(s) = state {
            let targets_str = if s.detected_targets.is_empty() {
                "无".to_string()
            } else {
                s.detected_targets.iter().map(|t| 
                    format!("{} offset=({},{})", t.label, t.offset_from_crosshair.0, t.offset_from_crosshair.1)
                ).collect::<Vec<_>>().join(", ")
            };
            format!(
                "场景: {}\n检测目标: {}\n\n\
                 只选一个行动：\n\
                 有目标→ {{\"tool\":\"aim_and_mine\",\"target\":\"具体目标名\"}}\n\
                 没目标→ {{\"tool\":\"look\",\"dx\":200,\"dy\":0}}\n\
                 不确定→ {{\"tool\":\"perceive\"}}\n\
                 注意: target 只填物体名(如 water/stone/tree)，不要填坐标。",
                s.scene_desc, targets_str,
            )
        } else {
            "首次行动，请输出 {{\"tool\":\"perceive\"}} 观察世界。只输出这个 JSON。".into()
        }
    }

    let t_start = Instant::now();

    for turn in 1..=max_turns {
        print!("[{turn}/{max_turns}] ");
        let t_step = Instant::now();

        let (name, args) = if use_fn_calling {
            // ─── function calling 模式 ───
            let m = serde_json::Value::Array(messages.clone());
            match llm.chat_tools(&m, &tools) {
                Ok(calls) => {
                    calls.into_iter().next().unwrap_or_else(|| ("perceive".into(), "{}".into()))
                }
                Err(e) => {
                    eprintln!("LLM 错误: {e}");
                    std::thread::sleep(std::time::Duration::from_secs(1));
                    continue;
                }
            }
        } else {
            // ─── prompt 模式 ───
            let prompt = build_tool_prompt(agent.last_state.as_ref());
            match llm.chat_text(&prompt) {
                Ok(reply) => {
                    eprintln!("[llm] {}", reply.trim().chars().take(120).collect::<String>());
                    let json = craft_agent_model::decision::extract_json(&reply).unwrap_or_default();
                    let name = json.get("tool").and_then(|v| v.as_str()).unwrap_or("perceive").to_string();
                    let args = json.to_string();
                    (name, args)
                }
                Err(e) => {
                    eprintln!("LLM 错误: {e}");
                    std::thread::sleep(std::time::Duration::from_secs(1));
                    continue;
                }
            }
        };

        eprintln!("[fn-call] {name}({args})");
        let args: Value = serde_json::from_str(&args).unwrap_or_default();

        // ─── 工具执行（两种模式共用）───

            match name.as_str() {
                "perceive" => {
                    match agent.step_tools(|_| Ok(AgentTool::Perceive)) {
                        Ok(r) => {
                            let detail = r.detail;
                            println!("  {}", detail);
                            messages.push(serde_json::json!({
                                "role": "assistant",
                                "content": null,
                                "tool_calls": [{"function": {"name": "perceive", "arguments": "{}"}}]
                            }));
                            messages.push(serde_json::json!({
                                "role": "tool",
                                "tool_call_id": "call_perceive",
                                "content": detail
                            }));
                        }
                        Err(e) => {
                            eprintln!("perceive 错误: {e}");
                        }
                    }
                }
                "look" => {
                    let dx = args["dx"].as_i64().unwrap_or(0) as i32;
                    let dy = args["dy"].as_i64().unwrap_or(0) as i32;
                    match agent.step_tools(|_| Ok(AgentTool::Act(
                        craft_agent::core::types::Action::Look { dx, dy }
                    ))) {
                        Ok(r) => {
                            println!("  {}", r.detail);
                            messages.push(serde_json::json!({
                                "role": "tool", "tool_call_id": "call_look",
                                "content": format!("视角转动 dx={dx} dy={dy}")
                            }));
                        }
                        Err(e) => eprintln!("look 错误: {e}"),
                    }
                }
                "move_forward" => {
                    let ticks = args["ticks"].as_u64().unwrap_or(30) as u32;
                    match agent.step_tools(|_| Ok(AgentTool::Act(
                        craft_agent::core::types::Action::Move {
                            dir: craft_agent::core::types::Direction::Forward,
                            ticks,
                        }
                    ))) {
                        Ok(r) => {
                            println!("  {}", r.detail);
                            messages.push(serde_json::json!({
                                "role": "tool", "tool_call_id": "call_move",
                                "content": format!("前进 {ticks} ticks")
                            }));
                        }
                        Err(e) => eprintln!("move 错误: {e}"),
                    }
                }
                "aim_and_mine" => {
                    let target = args["target"].as_str().unwrap_or("").to_string();
                    match agent.step_tools(|_| Ok(AgentTool::Act(
                        craft_agent::core::types::Action::AimAndMine { target: target.clone() }
                    ))) {
                        Ok(r) => {
                            println!("  {}", r.detail);
                            messages.push(serde_json::json!({
                                "role": "tool", "tool_call_id": "call_mine",
                                "content": format!("挖掘 {target}: {}", r.detail)
                            }));
                        }
                        Err(e) => eprintln!("mine 错误: {e}"),
                    }
                }
                other => {
                    eprintln!("  未知工具: {other}");
                }
            }

        let step_ms = t_step.elapsed().as_millis();
        let total_s = t_start.elapsed().as_secs_f64();
        println!("  [{step_ms}ms] (累计 {total_s:.1}s)");
    }

    let total = t_start.elapsed();
    println!("\n=== 完成 {max_turns} 轮 耗时 {:.1}s ===", total.as_secs_f64());
    Ok(())
}

#[cfg(not(feature = "real"))]
fn main() {
    eprintln!("本示例需要 --features real 编译（接入 xcap/enigo/VLM/LLM）。");
    std::process::exit(2);
}
