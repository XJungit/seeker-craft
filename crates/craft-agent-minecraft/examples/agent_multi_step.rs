//! P2 Agent — Pi 风格: Agent::run() + ToolRegistry + PromptBuilder 五层 prompt
//!
//! 用法:
//!   cargo run -p craft-agent-minecraft --example agent_multi_step --features real -- --steps=10

#[cfg(feature = "real")]
fn main() -> anyhow::Result<()> {
    use craft_agent::agent::{Agent, AgentConfig, DecideFn};
    use craft_agent::core::prompt::PromptBuilder;
    use craft_agent::core::tool::ToolRegistry;
    use craft_agent_minecraft::adapter::MinecraftAdapter;
    use craft_agent_minecraft::tools::create_mc_tools;
    use craft_agent_model::decision::real::OpenAiLlmClient;
    use craft_agent_model::vision::VisionClient;
    use craft_agent_model::vision::real::OpenAiVisionClient;
    use craft_agent_model::config::AgentConfig as ModelConfig;
    use serde_json::Value;
    use std::sync::Arc;
    use std::time::Instant;

    let _ = dotenvy::dotenv();
    let args: Vec<String> = std::env::args().collect();
    let max_turns: u32 = args.iter().find(|a| a.starts_with("--steps="))
        .and_then(|s| s.trim_start_matches("--steps=").parse().ok()).unwrap_or(10);

    let model_cfg = ModelConfig::load("config/agent.toml")?;
    let vlm_backend = model_cfg.vlm.active_backend()?;
    let llm_group = model_cfg.llm.as_ref()
        .ok_or_else(|| anyhow::anyhow!("缺少 [llm]"))?;
    let llm_backend = llm_group.active_backend()?;

    let vision: Box<dyn VisionClient> = Box::new(OpenAiVisionClient::from_config(vlm_backend)?);
    let llm = Arc::new(OpenAiLlmClient::from_config(llm_backend)?);
    let adapter = MinecraftAdapter::new(vision)?;

    // 工具注册
    let mut registry = ToolRegistry::new();
    for tool in create_mc_tools() {
        registry.register(tool);
    }

    // Agent 配置 (酒馆风格: 五层 prompt)
    let agent_cfg = AgentConfig {
        prompt: PromptBuilder::new()
            .identity("Minecraft 生存模式 AI。循环: perceive->act->perceive->act。")
            .role_desc("果断的采集者。perceive看到目标后必须立刻aim_and_mine, 不要再次perceive。")
            .add_example("perceive检测到tree -> aim_and_mine tree")
            .add_example("aim_and_mine完成后 -> perceive看新场景")
            .add_example("perceive检测到stone -> aim_and_mine stone")
            .add_example("连续2次perceive无目标 -> move_forward")
            .jailbreak("看到目标立刻挖。不要连续perceive两次。直接tool_call。"),
        max_turns,
    };

    let mut agent = Agent::new(adapter, agent_cfg, registry);

    // LLM 决策闭包
    let decide: Box<DecideFn> = Box::new(move |messages, tool_defs| {
        let m = Value::Array(messages.to_vec());
        let t = Value::Array(tool_defs.to_vec());
        llm.chat_tools(&m, &t)
    });

    println!("\n=== Agent === VLM:{} | LLM:{} 轮次:{}\n",
        vlm_backend.model, llm_backend.model, agent.config.max_turns);

    let t_start = Instant::now();
    let log = agent.run(&*decide)?;
    for entry in &log {
        println!("{}", entry);
    }
    println!("\n=== {}轮 {:.1}s ===", log.len(), t_start.elapsed().as_secs_f64());
    Ok(())
}

#[cfg(not(feature = "real"))]
fn main() {
    eprintln!("本示例需要 --features real");
}
