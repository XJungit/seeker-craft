//! P2 Agent — MiniCPM/Agnes(眼睛) + LongCat(大脑)
//!
//! 设计参考 Pi/Claude Code/SillyTavern：短提示词 + while循环 + 丰富工具结果。
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
        .and_then(|s| s.trim_start_matches("--steps=").parse().ok()).unwrap_or(8);

    let cfg = AgentConfig::load("config/agent.toml")?;
    let vlm_backend = cfg.vlm.active_backend()?;
    let llm_group = cfg.llm.as_ref()
        .ok_or_else(|| anyhow::anyhow!("缺少 [llm]"))?;
    let llm_backend = llm_group.active_backend()?;

    let vision: Box<dyn VisionClient> = Box::new(OpenAiVisionClient::from_config(vlm_backend)?);
    let llm = OpenAiLlmClient::from_config(llm_backend)?;
    let adapter = MinecraftAdapter::new(vision)?;
    let mut agent = Agent::new(adapter);

    // ── 工具定义 ──
    let tools: Value = serde_json::json!([
        {"type":"function","function":{
            "name":"perceive",
            "description":"拍照并用视觉AI识别你面前的3D世界物体（树、石头、水、动物、矿石等），返回物体的名称和位置。",
            "parameters":{"type":"object","properties":{}}
        }},
        {"type":"function","function":{
            "name":"aim_and_mine",
            "description":"转动视角对准指定目标，然后按住左键挖掘2秒。只有perceive确认目标存在后才调用。",
            "parameters":{"type":"object","properties":{
                "target":{"type":"string","description":"目标名称（tree/stone/water/dirt/ore等）"}
            },"required":["target"]}
        }},
        {"type":"function","function":{
            "name":"move_forward",
            "description":"按住W键向前移动。用于探索新区域或靠近远处的目标。",
            "parameters":{"type":"object","properties":{
                "ticks":{"type":"integer","description":"移动时长，80≈4秒，40≈2秒","default":80}
            }}
        }},
        {"type":"function","function":{
            "name":"look",
            "description":"转动视角观察四周。dx>0右转，dy>0下看。用于环顾寻找目标。",
            "parameters":{"type":"object","properties":{
                "dx":{"type":"integer","description":"水平转动量，300≈90°"},
                "dy":{"type":"integer","description":"垂直转动量"}
            },"required":["dx","dy"]}
        }}
    ]);

    // ── 系统提示词（短 + 循环 + 示例思维）──
    let system_prompt = "\
你是一个 Minecraft 生存模式 AI 玩家。你运行在一个 while 循环中：

while True:
    perceive()           -- 看周围有什么
    思考下一步做什么
    aim_and_mine / move_forward / look   -- 执行一个动作
    观察结果
    继续

目标: 收集资源(优先木材和石头), 探索世界。

示例思维链:
- perceive 看到面前有橡树 --> aim_and_mine tree 挖它
- 挖完后 perceive --> 发现树消失了, 前方是平原 --> move_forward 前进探索
- perceive --> 什么都没看到 --> look dx=300 右转观察
- perceive --> 看到左前方有石头 --> aim_and_mine stone

不要在同一位置停滞超过两轮。不要问问题, 直接行动。
如果你连续两次 perceive 都看不到目标, 立刻 move_forward 或 look。
";

    let mut messages: Vec<Value> = vec![serde_json::json!({
        "role": "system",
        "content": system_prompt
    })];

    println!("\n=== Agent === VLM:{} | LLM:{} 轮次:{max_turns}\n",
        vlm_backend.model, llm_backend.model);

    let t_start = Instant::now();

    for turn in 1..=max_turns {
        print!("[{turn}/{max_turns}] ");
        let t_step = Instant::now();

        // --- 调用 LongCat ---
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

        // 记录 assistant tool_call
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

        // --- 执行工具 + 生成描述性结果 ---
        let result_msg = match name.as_str() {
            "perceive" => match agent.step_tools(|_| Ok(AgentTool::Perceive)) {
                Ok(_) => {
                    let empty_targets = vec![];
                    let state = agent.last_state.as_ref();
                    let targets = state.map(|s| &s.detected_targets).unwrap_or(&empty_targets);
                    let scene = state.map(|s| s.scene_desc.as_str()).unwrap_or("?");
                    let summary = if targets.is_empty() {
                        format!("你拍照观察了周围。VLM报告：{} 没有发现3D世界物体（树/石头/矿石等）。\
                                 你应该 look 转动视角或 move_forward 前进探索。",
                            &scene[..scene.len().min(80)])
                    } else {
                        let list: Vec<_> = targets.iter().map(|t|
                            format!("{}", t.label)
                        ).collect();
                        format!("你拍照观察了周围。VLM检测到这些3D物体：{}。\
                                 你应该选择一个目标，调用 aim_and_mine 挖掘。",
                            list.join("、"))
                    };
                    println!("{}", summary);
                    summary
                }
                Err(e) => { eprintln!("{}", e); continue; }
            },

            "aim_and_mine" => {
                let target = args["target"].as_str().unwrap_or("?").to_string();
                match agent.step_tools(|_| Ok(AgentTool::Act(types::Action::AimAndMine { target: target.clone() }))) {
                    Ok(r) => {
                        let msg = format!("你转动视角对准了「{}」，并按住左键挖掘了2秒。{}",
                            target, r.detail);
                        println!("{}", msg);
                        msg
                    }
                    Err(e) => { eprintln!("{}", e); continue; }
                }
            },

            "move_forward" => {
                let ticks = args["ticks"].as_u64().unwrap_or(80) as u32;
                let secs = ticks as f32 * 0.05;
                match agent.step_tools(|_| Ok(AgentTool::Act(types::Action::Move {
                    dir: types::Direction::Forward, ticks
                }))) {
                    Ok(_) => {
                        let msg = format!("你按住W向前移动了{:.1}秒。前方的场景应该已经变化。", secs);
                        println!("{}", msg);
                        msg
                    }
                    Err(e) => { eprintln!("{}", e); continue; }
                }
            },

            "look" => {
                let dx = args["dx"].as_i64().unwrap_or(200) as i32;
                let dy = args["dy"].as_i64().unwrap_or(0) as i32;
                match agent.step_tools(|_| Ok(AgentTool::Act(types::Action::Look { dx, dy }))) {
                    Ok(_) => {
                        let dir = if dx > 0 { "右" } else if dx < 0 { "左" } else { "" };
                        let msg = format!("你向{dir}转动了视角(dx={dx},dy={dy})。现在看到的是新方向。");
                        println!("{}", msg);
                        msg
                    }
                    Err(e) => { eprintln!("{}", e); continue; }
                }
            },
            _ => { eprintln!("unknown: {name}"); continue; }
        };

        // 工具结果
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
