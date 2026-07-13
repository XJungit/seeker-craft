//! P1.5+ 多步连续闭环：Agent 自动跑 N 步，每步
//! `capture → perceive(VLM) → decide(LLM) → execute(enigo)`
//!
//! 用法（窗口化 MC，先 F3+P 关失去焦点暂停）：
//!   cargo run -p craft-agent-minecraft --example agent_multi_step --features real [-- --steps=10]
//!
//! 每步输出：
//!   ① 场景摘要（VLM scene_desc 前 80 字）
//!   ② LLM 决策的 Action
//!   ③ 执行结果
//!   ④ 累加耗时

#[cfg(feature = "real")]
fn main() -> anyhow::Result<()> {
    use craft_agent::agent::Agent;
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

    // 读 VLM + LLM 后端配置
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

    let skills_hint = "\
- 你面前是 Minecraft 主世界，可以自由移动、转向、挖掘。
- 快捷栏在屏幕底部，共有 9 格（编号 1~9），当前手持物品见快捷栏高亮。
- 可用动作（**必须选一个，不要空转**）：
  a) 点击编号元素（如合成台、物品栏）→ Click
  b) 前进/后退/左移/右移/跳跃/潜行一段时间 → Move（ticks 用 20~60，约 1~3 秒）
  c) 转视角观察环境 → Look（**dx,dy 绝对值至少 100，最大 500**，正 dx=右转、正 dy=下看）
  d) **看到树/矿石/石头时，对准并挖掘 → AimAndMine（这是最优先）**
- **重要**：如果场景描述中提到有树、橡树、石头、矿石，立刻执行 AimAndMine。
- 如果周围没有明显目标，向前移动探索或左右观察寻找资源。";

    println!("\n=== P1.6 多步连续闭环 ===");
    println!("VLM: {} | LLM: {}", vlm_backend.model, llm_backend.model);
    println!("步数: {steps}");
    println!();

    let t_start = Instant::now();
    let mut successes = 0u32;

    for step in 1..=steps {
        print!("[{step}/{steps}] ");
        let t_step = Instant::now();

        let result = match agent.step_with(|state| llm.decide(state, skills_hint)) {
            Ok(r) => {
                successes += 1;
                r
            }
            Err(e) => {
                // 报错后等一秒再继续（如 MiniCPM 502）
                eprintln!("❌ 报错: {e}");
                std::thread::sleep(std::time::Duration::from_secs(1));
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
        "\n=== 完成 {steps} 步 ({successes}/{steps} 成功) 耗时 {:.1}s ===",
        total.as_secs_f64()
    );

    // 总结报告：列出所有成功的执行结果，方便用户看 LLM 都做了什么
    Ok(())
}

#[cfg(not(feature = "real"))]
fn main() {
    eprintln!("本示例需要 --features real 编译（接入 xcap/enigo/VLM/LLM）。");
    std::process::exit(2);
}
