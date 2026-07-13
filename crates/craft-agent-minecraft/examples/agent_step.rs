//! P1.5 端到端单步闭环：Agent 驱动 MinecraftAdapter 完整走一遍
//! `capture → perceive(VLM) → decide(LLM) → execute(enigo)`
//!
//! 用法（窗口化 MC，先 F3+P 关失去焦点暂停）：
//!   cargo run -p craft-agent-minecraft --example agent_step --features real
//!   cargo run -p craft-agent-minecraft --example agent_step --features real -- --fullscreen
//!
//! 输出：LLM 决策的 Action + 执行结果。视角/按键/点击由适配器自动完成。

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

    // 加载 .env（密钥等）
    let _ = dotenvy::dotenv();

    let args: Vec<String> = std::env::args().collect();
    let fullscreen = args.iter().any(|a| a == "--fullscreen");

    // 从配置读取 VLM 和 LLM 后端
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
        println!("[agent_step] 全屏模式：capture 方法 C（主显示器整屏）");
        MinecraftAdapter::new_fullscreen(vision)?
    } else {
        println!("[agent_step] 窗口化模式：capture 方法 A（MC 窗口帧缓冲，遮挡免疫）");
        MinecraftAdapter::new(vision)?
    };
    let mut agent = Agent::new(adapter);

    // 技能提示：告诉 LLM 当前 Minecraft 版本可执行的动作
    let skills_hint = "\
- 你面前是 Minecraft 主世界，可以自由移动、转向、挖掘。
- 快捷栏在屏幕底部，共有 9 格（编号 1~9），当前手持物品见快捷栏高亮。
- 可用动作（**必须选一个，不要空转**）：
  a) 点击编号元素（如合成台、物品栏）→ Click
  b) 前进/后退/左移/右移/跳跃/潜行一段时间 → Move（ticks 用 20~60，约 1~3 秒）
  c) 转视角观察环境 → Look（**dx,dy 绝对值至少 100，最大 500**，正 dx=右转、正 dy=下看）
  d) 看到树/矿石/石头时，对准并挖掘 → AimAndMine
- **重要**：Look 的 dx,dy 必须绝对值 >= 100，否则视角变化太小看不到。
- 如果视野中有树木，优先砍树。
- 如果周围没有明显目标，用 Move 向前探索一段时间。";

    println!("\n=== 端到端单步闭环 P1.5 ===");
    println!("VLM 后端: {} ({})", vlm_backend.model, vlm_backend.base_url);
    println!("LLM 后端: {} ({})", llm_backend.model, llm_backend.base_url);
    println!();

    let t0 = Instant::now();

    // 单步：capture → perceive(VLM) → decide(LLM) → execute(enigo)
    let result = agent.step_with(|state| llm.decide(state, skills_hint))?;

    let elapsed = t0.elapsed();
    println!("  执行结果: {} (ok={})", result.detail, result.ok);
    println!("  总耗时: {:.1}s", elapsed.as_secs_f64());
    println!("=== 闭环完成 ===");
    Ok(())
}

#[cfg(not(feature = "real"))]
fn main() {
    eprintln!("本示例需要 --features real 编译（接入 xcap/enigo/VLM/LLM）。");
    std::process::exit(2);
}
