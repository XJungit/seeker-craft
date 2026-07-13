//! P2 Agent 自主决策 — LongCat function calling
//! MiniCPM(VLM 眼睛) + LongCat(LLM 大脑)
//!
//! 用法：
//!   cargo run -p craft-agent-minecraft --example agent_multi_step --features real -- --steps=10

#[cfg(feature = "real")]
fn main() -> anyhow::Result<()> {
    use craft_agent::agent::{Agent, AgentTool};
    use craft_agent::core::types;
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
    let max_turns: u32 = args.iter().find(|a| a.starts_with("--steps="))
        .and_then(|s| s.trim_start_matches("--steps=").parse().ok()).unwrap_or(10);

    let cfg = AgentConfig::load("config/agent.toml")?;
    let vlm_backend = cfg.vlm.active_backend()?;
    let llm_group = cfg.llm.as_ref()
        .ok_or_else(|| anyhow::anyhow!("config/agent.toml 缺少 [llm] 段"))?;
    let llm_backend = llm_group.active_backend()?;

    let vision: Box<dyn VisionClient> = Box::new(OpenAiVisionClient::from_config(vlm_backend)?);
    let llm = OpenAiLlmClient::from_config(llm_backend)?;

    let adapter = if fullscreen { MinecraftAdapter::new_fullscreen(vision)? }
        else { MinecraftAdapter::new(vision)? };
    let mut agent = Agent::new(adapter);

    // ─── Function calling 工具定义 ───
    let tools: Value = serde_json::json!([
        {"type":"function","function":{"name":"perceive","description":"拍照识别画面中的物体和方块","parameters":{"type":"object","properties":{}}}},
        {"type":"function","function":{"name":"look","description":"转动视角。dx>0 右转，dy>0 下看","parameters":{"type":"object","properties":{"dx":{"type":"integer"},"dy":{"type":"integer"}},"required":["dx","dy"]}}},
        {"type":"function","function":{"name":"move_forward","description":"向前移动","parameters":{"type":"object","properties":{"ticks":{"type":"integer","default":30}},"required":[]}}},
        {"type":"function","function":{"name":"aim_and_mine","description":"对准目标并挖掘（树/石头/矿石/水）","parameters":{"type":"object","properties":{"target":{"type":"string"}},"required":["target"]}}}
    ]);

    let mut messages: Vec<Value> = vec![serde_json::json!({
        "role": "system",
        "content": "Minecraft Agent。规则：1) perceive看到目标→立刻aim_and_mine 2) 没目标→look探索 3) 不连续perceive。看到stone,tree,water任一个就挖。"
    })];

    println!("\n=== LongCat Agent ===");
    println!("VLM: {} | LLM: {}", vlm_backend.model, llm_backend.model);
    println!("轮次: {max_turns}\n");

    let t_start = Instant::now();

    for turn in 1..=max_turns {
        print!("[{turn}/{max_turns}] ");
        let t_step = Instant::now();

        let m = Value::Array(messages.clone());
        let calls = match llm.chat_tools(&m, &tools) {
            Ok(c) => c,
            Err(e) => { eprintln!("LLM: {e}"); std::thread::sleep(std::time::Duration::from_secs(1)); continue; }
        };

        if calls.is_empty() { break; }
        let (name, args_str) = &calls[0];
        eprintln!("[tool] {name}({args_str})");
        let args: Value = serde_json::from_str(args_str).unwrap_or_default();

        let result_msg = match name.as_str() {
            "perceive" => match agent.step_tools(|_| Ok(AgentTool::Perceive)) {
                Ok(r) => {
                    // 把检测到的目标注入 tool result
                    let targets = agent.last_state.as_ref()
                        .map(|s| s.detected_targets.iter()
                            .map(|t| format!("{} offset=({},{})", t.label, t.offset_from_crosshair.0, t.offset_from_crosshair.1))
                            .collect::<Vec<_>>().join("; "))
                        .unwrap_or_default();
                    let result = if targets.is_empty() {
                        format!("perceive: 无目标。{}", r.detail)
                    } else {
                        format!("perceive: 检测到 [{}]。{}", targets, r.detail)
                    };
                    println!("  {}", result);
                    result
                }
                Err(e) => { eprintln!("  {}", e); continue; }
            },
            "look" => {
                let dx = args["dx"].as_i64().unwrap_or(200) as i32;
                let dy = args["dy"].as_i64().unwrap_or(0) as i32;
                match agent.step_tools(|_| Ok(AgentTool::Act(types::Action::Look { dx, dy }))) {
                    Ok(r) => { println!("  {}", r.detail); format!("look dx={dx} dy={dy}") }
                    Err(e) => { eprintln!("look: {e}"); continue; }
                }
            }
            "move_forward" => {
                let ticks = args["ticks"].as_u64().unwrap_or(30) as u32;
                match agent.step_tools(|_| Ok(AgentTool::Act(types::Action::Move { dir: types::Direction::Forward, ticks }))) {
                    Ok(r) => { println!("  {}", r.detail); format!("move {ticks} ticks") }
                    Err(e) => { eprintln!("move: {e}"); continue; }
                }
            }
            "aim_and_mine" => {
                let target = args["target"].as_str().unwrap_or("").to_string();
                match agent.step_tools(|_| Ok(AgentTool::Act(types::Action::AimAndMine { target: target.clone() }))) {
                    Ok(r) => { println!("  {}", r.detail); format!("aim_mine {target}") }
                    Err(e) => { eprintln!("mine: {e}"); continue; }
                }
            }
            _ => { eprintln!("  unknown: {name}"); continue; }
        };

        messages.push(serde_json::json!({"role":"tool","content":result_msg}));

        let step_ms = t_step.elapsed().as_millis();
        println!("  [{step_ms}ms] (累计 {:.1}s)", t_start.elapsed().as_secs_f64());
    }

    println!("\n=== 完成 {max_turns} 轮 耗时 {:.1}s ===", t_start.elapsed().as_secs_f64());
    Ok(())
}

#[cfg(not(feature = "real"))]
fn main() {
    eprintln!("本示例需要 --features real");
}
