//! 控制台 Agent Demo：直接运行 agent loop，输出到控制台（无 Web UI）。
//!
//! 运行：
//! ```bash
//! cargo run -p craft-agent-minecraft --example agent_console_demo --features azalea-bot -- --goal="收集木头" --steps=5
//! ```
//!
//! 用途：调试 agent 循环，观察 LLM 决策与工具调用。

#[cfg(feature = "azalea-bot")]
fn main() -> anyhow::Result<()> {
    use craft_agent::agent::{Agent, AgentConfig, CompactionConfig, LlmProvider};
    use craft_agent::core::message::AssistantResponse;
    use craft_agent::core::tool::ToolRegistry;
    use craft_agent_minecraft::adapter_azalea::ArcAzaleaAdapter;
    use craft_agent_minecraft::tools_azalea::create_mc_azalea_tools_full;
    use craft_agent_model::config::AgentConfig as ModelConfig;
    use craft_agent_model::decision::real::OpenAiLlmClient;
    use craft_agent_minecraft::action_lib::ActionLibrary;
    use craft_agent_minecraft::blueprint::BlueprintLibrary;
    use serde_json::Value;
    use std::sync::Arc;
    use std::time::Duration;

    let _ = dotenvy::dotenv();
    let args: Vec<String> = std::env::args().collect();
    let max_steps: u32 = args
        .iter()
        .find(|a| a.starts_with("--steps="))
        .and_then(|s| s.trim_start_matches("--steps=").parse().ok())
        .unwrap_or(5);
    let goal: String = args
        .iter()
        .find(|a| a.starts_with("--goal="))
        .map(|s| s.trim_start_matches("--goal=").to_string())
        .unwrap_or_else(|| "收集木头做工作台".to_string());
    let mc_addr = "localhost:4444".to_string();
    let username = "craftbot_console".to_string();

    println!("=== Console Agent Demo ===");
    println!("Goal: {goal}");
    println!("Steps: {max_steps}");
    println!("MC: {mc_addr}");
    println!("User: {username}");

    // 构建 LLM 客户端
    let model_cfg = ModelConfig::load("data/config/agent.toml")?;
    let llm_group = model_cfg.llm.as_ref().ok_or_else(|| anyhow::anyhow!("缺少 [llm]"))?;
    let llm_backend = llm_group.active_backend()?;
    let llm = Arc::new(OpenAiLlmClient::from_config(llm_backend)?);

    struct Lp { llm: Arc<OpenAiLlmClient> }
    impl LlmProvider for Lp {
        fn complete(&self, m: &[Value], t: &[Value]) -> anyhow::Result<AssistantResponse> {
            self.llm.chat_tools(&Value::Array(m.to_vec()), &Value::Array(t.to_vec()))
        }
    }

    // 连接 azalea adapter
    let world_mem = craft_agent::core::memory::WorldMemory::new();
    let adapter = {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
        rt.block_on(ArcAzaleaAdapter::connect_with_memory(&mc_addr, &username, world_mem.clone()))
            .map_err(|e| anyhow::anyhow!("连接失败: {e}"))?
    };

    // 注册工具
    let blueprints = BlueprintLibrary::load_dir(std::path::Path::new("data/blueprints"));
    let actions = ActionLibrary::load_dir(std::path::Path::new("data/actions"));
    let mut registry = ToolRegistry::new();
    for tool in create_mc_azalea_tools_full(adapter, world_mem.clone(), blueprints, actions) {
        registry.register(tool);
    }

    let cw = llm_backend.context_window;
    let compaction = CompactionConfig {
        context_window: cw,
        reserve: (cw as f64 * 0.20) as u32,
        keep_recent: (cw as f64 * 0.60) as u32,
        compaction_model: None,
        compaction_provider: None,
        compaction_thinking: false,
    };

    let system_prompt = "你是 Minecraft AI 控制台 bot。\
        可用工具：perceive() 读状态、goto(x,y,z) 导航、mine_below() 下挖、\
        mine(x,y,z) 挖指定方块、gather(item,count) 采集、craft(item,count) 合成、\
        chat(content) 发消息。\
        任务：用 perceive 确认状态，用工具行动，每步都用 chat 汇报进度。".to_string();

    let cfg = AgentConfig::new(system_prompt, max_steps)
        .with_compaction(compaction)
        .with_knowledge_base(None)
        .with_world_info(None)
        .with_knowledge_tool(false);

    let mut agent = Agent::new(Box::new(Lp { llm }), registry, cfg).with_world_memory(world_mem);

    println!("Starting agent loop...");
    let start = std::time::Instant::now();
    let log = agent.run(goal)?;
    let elapsed = start.elapsed();

    println!("\n=== Agent 完成 ({:.1}s) ===", elapsed.as_secs_f64());
    for line in &log {
        println!("  {line}");
    }

    // 等待后台事件处理完成
    std::thread::sleep(Duration::from_secs(2));
    Ok(())
}

#[cfg(not(feature = "azalea-bot"))]
fn main() {
    eprintln!("需要 --features azalea-bot");
}
