//! Agent — pi_agent_rust 架构
//! cargo run -p craft-agent-minecraft --example agent_multi_step --features real -- --steps=10

#[cfg(feature = "real")]
fn main() -> anyhow::Result<()> {
    use craft_agent::agent::{Agent, AgentConfig, LlmProvider};
    use craft_agent::core::tool::ToolRegistry;
    use craft_agent_minecraft::adapter::MinecraftAdapter;
    use craft_agent_minecraft::tools::create_mc_tools;
    use craft_agent_model::decision::real::OpenAiLlmClient;
    use craft_agent_model::vision::{VisionClient, real::OpenAiVisionClient};
    use craft_agent_model::config::AgentConfig as ModelConfig;
    use serde_json::Value;
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::Arc;
    use std::time::Instant;

    let _ = dotenvy::dotenv();
    let args: Vec<String> = std::env::args().collect();
    let max_turns: u32 = args.iter().find(|a| a.starts_with("--steps="))
        .and_then(|s| s.trim_start_matches("--steps=").parse().ok()).unwrap_or(10);

    let model_cfg = ModelConfig::load("config/agent.toml")?;
    let vlm_backend = model_cfg.vlm.active_backend()?;
    let llm_group = model_cfg.llm.as_ref().ok_or_else(|| anyhow::anyhow!("缺少 [llm]"))?;
    let llm_backend = llm_group.active_backend()?;

    let llm = Arc::new(OpenAiLlmClient::from_config(llm_backend)?);

    struct Lp { llm: Arc<OpenAiLlmClient> }
    impl LlmProvider for Lp {
        fn complete(&self, m: &[Value], t: &[Value]) -> anyhow::Result<(Option<String>, Vec<(String,String)>)> {
            self.llm.chat_tools(&Value::Array(m.to_vec()), &Value::Array(t.to_vec()))
        }
    }

    let enigo = Rc::new(RefCell::new(enigo::Enigo::new(&enigo::Settings::default())?));
    let adapter = Rc::new(RefCell::new(MinecraftAdapter::new(
        Box::new(OpenAiVisionClient::from_config(vlm_backend)?)
    )?));
    let vlm_arc: Arc<dyn VisionClient> = Arc::new(OpenAiVisionClient::from_config(vlm_backend)?);

    let a = adapter.clone();
    let capture: Box<dyn Fn() -> anyhow::Result<Vec<u8>>> = Box::new(move || a.borrow().capture_screen());

    let mut registry = ToolRegistry::new();
    for tool in create_mc_tools(vlm_arc, capture, enigo) {
        registry.register(tool);
    }

    let mut agent = Agent::new(Box::new(Lp { llm }), registry, AgentConfig {
        prompt: "\
你是 Minecraft AI 玩家。循环: perceive -> 思考 -> act -> perceive -> ...
工具: perceive(拍照,prompt英文) / look(dx,dy) / press(keys,ticks) / mine(ticks)
策略: perceive看周围 -> 有目标就look对准 -> mine挖掘。没目标就look/press探索。
先思考一行,再tool_call。".into(),
        max_turns,
        max_messages: 50,
    });

    println!("\n=== VLM:{} LLM:{} ===\n", vlm_backend.model, llm_backend.model);
    let t0 = Instant::now();
    for e in agent.run()? { println!("{}", e); }
    println!("\n=== {}轮 {:.1}s ===", agent.session.len(), t0.elapsed().as_secs_f64());
    Ok(())
}

#[cfg(not(feature = "real"))]
fn main() { eprintln!("需要 --features real"); }
