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
    let agent_cfg = AgentConfig::new(
        PromptBuilder::new()
            .identity("Minecraft 生存模式 AI。你可以完全控制游戏。")
            .role_desc(
                "工具: perceive(拍照观察) / look(转动视角) / press(按键移动) / mine(挖掘)。\
                 你决定VLM的提示词, 你决定往哪看, 你决定按什么键, 你决定前进距离和挖掘时间。"
            )
            .add_example("perceive看到树在右前方 -> look dx=100 dy=-20 -> perceive -> 树在准星了 -> mine")
            .add_example("perceive什么都没看到 -> look dx=300 dy=0 右转 -> perceive")
            .add_example("前方是开阔地 -> press keys=w ticks=80 前进4秒 -> perceive")
            .add_example("perceive看到左前方有石头 -> look dx=-150 dy=20 -> perceive -> mine ticks=120")
            .jailbreak(
                "策略: 先简短思考(一行), 再输出tool_call。\
                 例如: '我看到三棵树,选最近那棵' 然后tool_call。\
                 每次perceive决定自己的prompt。所有距离和时间你自己决定。\
                 看到目标就调整视角对准后mine。看不到目标就look或press w探索。"
            ),
        max_turns,
    );

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
