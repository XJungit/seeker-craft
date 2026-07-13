//! P2 Agent 自主决策 — LongCat function calling + 完整上下文积累
//! VLM(Agnes 眼睛) + LLM(LongCat 大脑)
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
    let max_turns: u32 = args.iter().find(|a| a.starts_with("--steps="))
        .and_then(|s| s.trim_start_matches("--steps=").parse().ok()).unwrap_or(10);

    let cfg = AgentConfig::load("config/agent.toml")?;
    let vlm_backend = cfg.vlm.active_backend()?;
    let llm_group = cfg.llm.as_ref()
        .ok_or_else(|| anyhow::anyhow!("缺少 [llm]"))?;
    let llm_backend = llm_group.active_backend()?;

    let vision: Box<dyn VisionClient> = Box::new(OpenAiVisionClient::from_config(vlm_backend)?);
    let llm = OpenAiLlmClient::from_config(llm_backend)?;
    let adapter = MinecraftAdapter::new(vision)?;
    let mut agent = Agent::new(adapter);

    let tools: Value = serde_json::json!([
        {"type":"function","function":{
            "name":"perceive",
            "description":"拍照识别画面中的3D世界物体（树/石头/水/动物/矿石），返回物体名称和相对准星的像素偏移。这是你的'眼睛'。",
            "parameters":{"type":"object","properties":{}}
        }},
        {"type":"function","function":{
            "name":"aim_and_mine",
            "description":"对准指定目标并挖掘。先用perceive确认目标存在。持续挖2秒。",
            "parameters":{"type":"object","properties":{"target":{"type":"string","description":"目标名称，如tree/stone/water/dirt"}},"required":["target"]}
        }},
        {"type":"function","function":{
            "name":"move_forward",
            "description":"向前移动探索新区域。在当前位置没有价值目标时使用。",
            "parameters":{"type":"object","properties":{"ticks":{"type":"integer","description":"移动时长，30≈1.5秒，60≈3秒","default":40}}}
        }},
        {"type":"function","function":{
            "name":"look",
            "description":"转动视角观察四周。dx>0右转，dy>0下看。",
            "parameters":{"type":"object","properties":{"dx":{"type":"integer"},"dy":{"type":"integer"}},"required":["dx","dy"]}
        }}
    ]);

    let mut messages: Vec<Value> = vec![serde_json::json!({
        "role": "system",
        "content": "你是Minecraft生存模式Agent。你的目标是收集资源并在世界中探索。\n\n\
## 核心循环\n\
perceive观察 → aim_and_mine挖掘资源 → 目标耗尽后move_forward探索新区 → perceive再次观察\n\n\
## 规则\n\
1. perceive看到树/石头/水/矿石 → 立刻aim_and_mine\n\
2. 同一目标挖2次后它可能已消失 → perceive重新观察\n\
3. 连续perceive都看不到目标 → move_forward前进探索(ticks=40~60) + look环顾\n\
4. 不要在同一个位置停滞超过3轮\n\n\
## 策略\n\
- 优先挖树(tree)获取木材\n\
- 优先挖石头(stone)获取圆石\n\
- 看到水(water)可走过去(move_forward)\n\
- 夜晚看到怪物优先避开"
    })];

    println!("\n=== Agent v2 === VLM:{} | LLM:{} 轮次:{max_turns}\n",
        vlm_backend.model, llm_backend.model);

    let t_start = Instant::now();
    let mut turns_since_move = 0u32;

    for turn in 1..=max_turns {
        print!("[{turn}/{max_turns}] ");
        let t_step = Instant::now();

        // 硬规则：连续3轮没移动 → 强制前进探索
        if turns_since_move >= 3 {
            let ticks = 50u32;
            match agent.step_tools(|_| Ok(AgentTool::Act(types::Action::Move {
                dir: types::Direction::Forward, ticks
            }))) {
                Ok(r) => {
                    turns_since_move = 0;
                    let msg = format!("⚠️强制移动 {}ticks: {}", ticks, r.detail);
                    println!("{}", msg);
                    let step_ms = t_step.elapsed().as_millis();
                    println!("  [{step_ms}ms | {:.1}s]", t_start.elapsed().as_secs_f64());
                    continue;
                }
                Err(e) => eprintln!("move失败: {e}"),
            }
        }

        let m = Value::Array(messages.clone());
        let calls = match llm.chat_tools(&m, &tools) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("LLM: {e}");
                std::thread::sleep(std::time::Duration::from_secs(2));
                continue;
            }
        };

        if calls.is_empty() { break; }
        let (name, args_str) = &calls[0];
        eprint!("[{name}] ");
        let args: Value = serde_json::from_str(args_str).unwrap_or_default();

        // 记录 assistant 的 tool_call 消息（完整上下文积累）
        let call_id = format!("call_{turn}");
        messages.push(serde_json::json!({
            "role": "assistant",
            "content": null,
            "tool_calls": [{
                "id": call_id,
                "type": "function",
                "function": {"name": name, "arguments": args_str}
            }]
        }));

        let result_msg = match name.as_str() {
            "perceive" => {
                turns_since_move += 1;
                match agent.step_tools(|_| Ok(AgentTool::Perceive)) {
                    Ok(_r) => {
                        let targets = agent.last_state.as_ref()
                            .map(|s| s.detected_targets.iter()
                                .map(|t| format!("{} offset=({},{})", t.label, t.offset_from_crosshair.0, t.offset_from_crosshair.1))
                                .collect::<Vec<_>>().join("; "))
                            .unwrap_or_default();
                        let move_hint = if turns_since_move >= 3 {
                            format!(" ⚠️已{turns_since_move}轮未移动，建议move_forward(ticks=50)探索新区!")
                        } else { String::new() };
                        let msg = if targets.is_empty() {
                            format!("perceive: 无目标。move_forward探索或look环顾。{}", move_hint)
                        } else {
                            format!("perceive: [{}]。看到目标应aim_and_mine。{}", targets, move_hint)
                        };
                        println!("{}", msg);
                        msg
                    }
                    Err(e) => { eprintln!("{}", e); continue; }
                }
            }
            "aim_and_mine" => {
                let target = args["target"].as_str().unwrap_or("").to_string();
                match agent.step_tools(|_| Ok(AgentTool::Act(types::Action::AimAndMine { target: target.clone() }))) {
                    Ok(_r) => {
                        let msg = format!("aim_and_mine {}: {}", target, _r.detail);
                        println!("{}", msg);
                        msg
                    }
                    Err(e) => { eprintln!("mine: {e}"); continue; }
                }
            }
            "move_forward" => {
                turns_since_move = 0;
                let ticks = args["ticks"].as_u64().unwrap_or(40) as u32;
                match agent.step_tools(|_| Ok(AgentTool::Act(types::Action::Move {
                    dir: types::Direction::Forward, ticks
                }))) {
                    Ok(r) => {
                        let msg = format!("move_forward {}ticks ({}秒)", ticks, ticks as f32 * 0.05);
                        println!("{}", msg);
                        msg
                    }
                    Err(e) => { eprintln!("move: {e}"); continue; }
                }
            }
            "look" => {
                let dx = args["dx"].as_i64().unwrap_or(200) as i32;
                let dy = args["dy"].as_i64().unwrap_or(0) as i32;
                match agent.step_tools(|_| Ok(AgentTool::Act(types::Action::Look { dx, dy }))) {
                    Ok(r) => {
                        let msg = format!("look dx={dx} dy={dy}");
                        println!("{}", msg);
                        msg
                    }
                    Err(e) => { eprintln!("look: {e}"); continue; }
                }
            }
            _ => { eprintln!("unknown: {name}"); continue; }
        };

        // 完整上下文：tool 结果
        messages.push(serde_json::json!({
            "role": "tool",
            "tool_call_id": call_id,
            "content": result_msg
        }));

        let step_ms = t_step.elapsed().as_millis();
        println!("  [{step_ms}ms | {:.1}s]", t_start.elapsed().as_secs_f64());
    }

    println!("\n=== {}轮 {:.1}s ===", max_turns, t_start.elapsed().as_secs_f64());
    Ok(())
}

#[cfg(not(feature = "real"))]
fn main() {
    eprintln!("本示例需要 --features real");
}
