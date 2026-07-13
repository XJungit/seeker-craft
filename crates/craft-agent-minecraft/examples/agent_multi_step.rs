//! P2 Agent 自主决策闭环：LLM 决定什么时候看、做什么。
//! `need vision? → perceive(VLM) → plan → act → repeat`
//!
//! 用法（窗口化 MC，先 F3+P 关失去焦点暂停）：
//!   cargo run -p craft-agent-minecraft --example agent_multi_step --features real [-- --steps=10]
//!
//! LLM 拿到 WorldState 后，可以：
//!   1. 输出 {"tool":"perceive"} → 调用 VLM 看画面
//!   2. 输出 {"action":"AimAndMine","target":"tree"} → 直接执行
//!   3. 输出 {"action":"Look","dx":200,"dy":0} → 转视角探索

#[cfg(feature = "real")]
fn main() -> anyhow::Result<()> {
    use craft_agent::agent::{Agent, AgentTool};
    use craft_agent_minecraft::adapter::MinecraftAdapter;
    use craft_agent_model::config::AgentConfig;
    use craft_agent_model::decision::DecisionClient;
    use craft_agent_model::decision::real::OpenAiLlmClient;
    use craft_agent_model::vision::VisionClient;
    use craft_agent_model::vision::real::OpenAiVisionClient;
    use std::time::Instant;

    let _ = dotenvy::dotenv();

    let args: Vec<String> = std::env::args().collect();
    let fullscreen = args.iter().any(|a| a == "--fullscreen");
    let steps: u32 = args
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

    let tool_hint = "\
你是一个 Minecraft Agent。你可以使用以下工具：
- **perceive**：拍照并用 VLM 识别画面中的物体（树/石/水/动物等）
- **动作**：Look/Move/AimAndMine/Click

规则：
1. 如果你的 WorldState 为空或已过时，先调用 perceive。
2. 如果检测到树/石头/矿石，用 AimAndMine 对准挖掘。
3. 如果周围没有明显目标，Look 左右观察或 Move 前进探索。
4. 不要连续两次 perceive，看到东西后要行动。";

    // 用工具调用格式构建 LLM prompt
    fn build_tool_prompt(state: Option<&craft_agent::core::types::WorldState>) -> String {
        if let Some(s) = state {
            format!(
                "[当前状态]\n\
                 场景: {}\n\
                 检测目标({}): {}\n\
                 可交互 UI({}): {}\n\
                 自身: {}\n\n\
                 [请决策] 立刻给出一个 JSON 动作，不要解释：\n\
                 格式: {{\"action\":\"AimAndMine\",\"target\":\"tree\"}} 或\n\
                       {{\"action\":\"Look\",\"dx\":200,\"dy\":0}} 或\n\
                       {{\"tool\":\"perceive\"}}\n\
                 不要输出其他内容。",
                s.scene_desc,
                s.detected_targets.len(),
                s.detected_targets.iter().map(|t| format!("{} offset=({},{})", t.label, t.offset_from_crosshair.0, t.offset_from_crosshair.1)).collect::<Vec<_>>().join(", "),
                s.marked_elements.len(),
                s.self_hint,
            )
        } else {
            "你还没有观察过世界。第一步请调用 perceive。输出 {{\"tool\":\"perceive\"}}".into()
        }
    }

    println!("\n=== P2 Agent 自主决策闭环 ===");
    println!("VLM: {} | LLM: {}", vlm_backend.model, llm_backend.model);
    println!("步数: {steps}\n");

    let t_start = Instant::now();

    for step in 1..=steps {
        print!("[{step}/{steps}] ");
        let t_step = Instant::now();

        // LLM 决策：根据 last_state 输出 tool 或 action
        let tool = match llm.chat_text(&build_tool_prompt(agent.last_state.as_ref())) {
            Ok(reply) => {
                let json = craft_agent_model::decision::extract_json(&reply).unwrap_or_default();
                eprintln!("[llm] {}", reply.trim().chars().take(120).collect::<String>());
                if json.get("tool").and_then(|v| v.as_str()) == Some("perceive") {
                    AgentTool::Perceive
                } else if let Some(action) = craft_agent_model::decision::value_to_action(&json).ok() {
                    AgentTool::Act(action)
                } else {
                    // 解析失败 → 先观察
                    AgentTool::Perceive
                }
            }
            Err(e) => {
                eprintln!("LLM 错误: {e}");
                std::thread::sleep(std::time::Duration::from_secs(1));
                continue;
            }
        };

        let result = match agent.step_tools(|_| Ok(tool)) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("❌ {e}");
                continue;
            }
        };

        let step_ms = t_step.elapsed().as_millis();
        println!(
            "  {}  [{step_ms}ms] (累计 {:.1}s)",
            result.detail,
            t_start.elapsed().as_secs_f64()
        );
    }

    let total = t_start.elapsed();
    println!(
        "\n=== 完成 {steps} 步 耗时 {:.1}s ===",
        total.as_secs_f64()
    );
    Ok(())
}

#[cfg(not(feature = "real"))]
fn main() {
    eprintln!("本示例需要 --features real 编译（接入 xcap/enigo/VLM/LLM）。");
    std::process::exit(2);
}
