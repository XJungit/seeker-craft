//! Phase 6 验证：LLM 主循环驱动 azalea bot（端到端自主）。
//!
//! 运行（先开纯 vanilla 26.2 局域网服，端口 4444）：
//! ```bash
//! cargo run -p craft-agent-minecraft --example agent_azalea_demo --features azalea-bot \
//!   -- --goal="挖矿下探" --steps=20
//! ```
//! 行为：connect azalea adapter -> 注册 azalea 工具集 -> agent.run(goal)
//!       LLM 通过 perceive/goto/mine_below/chat 工具驱动 bot。
//!
//! 架构对齐 mod 路线（agent_multi_step_mod.rs）：main 保持纯同步，
//! LLM 客户端用 reqwest::blocking（from_config 不能在 tokio runtime 内构建）。
//! 仅连接阶段用一次性局部 runtime 跑完 async connect，之后全程同步。

#[cfg(feature = "azalea-bot")]
fn main() -> anyhow::Result<()> {
    use craft_agent::agent::{Agent, AgentConfig, CompactionConfig, LlmProvider};
    use craft_agent::core::message::AssistantResponse;
    use craft_agent::core::tool::ToolRegistry;
    use craft_agent_minecraft::adapter_azalea::ArcAzaleaAdapter;
    use craft_agent_minecraft::tools_azalea::create_mc_azalea_tools;
    use craft_agent_model::config::AgentConfig as ModelConfig;
    use craft_agent_model::decision::real::OpenAiLlmClient;
    use serde_json::Value;
    use std::sync::Arc;

    let _ = dotenvy::dotenv();
    let args: Vec<String> = std::env::args().collect();
    let max_iter: u32 = args
        .iter()
        .find(|a| a.starts_with("--steps="))
        .and_then(|s| s.trim_start_matches("--steps=").parse().ok())
        .unwrap_or(20);
    let goal: String = args
        .iter()
        .find(|a| a.starts_with("--goal="))
        .map(|s| s.trim_start_matches("--goal=").to_string())
        .unwrap_or_else(|| "挖矿下探".to_string());

    // 同步构建 LLM 客户端（必须在任何 tokio runtime 之外，因内部用 reqwest::blocking）。
    let model_cfg = ModelConfig::load("config/agent.toml")?;
    let llm_group = model_cfg.llm.as_ref().ok_or_else(|| anyhow::anyhow!("缺少 [llm]"))?;
    let llm_backend = llm_group.active_backend()?;
    let llm = Arc::new(OpenAiLlmClient::from_config(llm_backend)?);

    struct Lp {
        llm: Arc<OpenAiLlmClient>,
    }
    impl LlmProvider for Lp {
        fn complete(
            &self,
            m: &[Value],
            t: &[Value],
        ) -> anyhow::Result<AssistantResponse> {
            self.llm
                .chat_tools(&Value::Array(m.to_vec()), &Value::Array(t.to_vec()))
        }
    }

    // 连接 azalea adapter：azalea 内部用独立 OS 线程跑自己的 runtime，
    // 此处仅用一次性局部 runtime 把 async connect 跑完，拿到句柄后立即 drop。
    let adapter = {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        rt.block_on(ArcAzaleaAdapter::connect("localhost:4444", "craftbot"))
            .expect("azalea adapter 连接失败（确认服为纯 vanilla 26.2）")
    };

    // 注册 azalea 工具集（持有 adapter 引用）。
    let mut registry = ToolRegistry::new();
    for tool in create_mc_azalea_tools(adapter) {
        registry.register(tool);
    }

    let cw = llm_backend.context_window;
    let reserve = (cw as f64 * 0.20) as u32;
    let keep_recent = (cw as f64 * 0.60) as u32;
    let compaction = CompactionConfig {
        context_window: cw,
        reserve,
        keep_recent,
        compaction_model: None,
        compaction_provider: None,
        compaction_thinking: false,
    };

    let system_prompt = String::from(
        "你是 Minecraft AI 玩家，通过 azalea 客户端协议控制 bot（纯 vanilla 26.2）。\n\
         每轮自动注入游戏状态（perceive），无需手动调用。\n\
         可用工具：\n\
          - perceive()：读坐标/背包/附近玩家（无参数）。\n\
          - goto(x,y,z)：A* 导航到坐标。\n\
          - mine_below()：挖脚下方块（向下探矿）。\n\
          - chat(content)：发聊天消息。\n\
         策略：用 perceive 看状态，用 mine_below 下探，用 chat 回报进度。除非任务完成，否则每轮必须以工具调用收尾。",
    );
    let cfg = AgentConfig::new(system_prompt, max_iter)
        .with_compaction(compaction)
        // azalea 路线无 mod 专属知识：关闭 MC_KNOWLEDGE_BASE 与 world_info，
        // 仅用工具自描述，避免 LLM 误调 azalea 不存在的 collect/combat 等工具。
        .with_knowledge_base(None)
        .with_world_info(None);

    let mut agent = Agent::new(Box::new(Lp { llm }), registry, cfg);

    println!(
        "\n=== AZALEA localhost:4444 | LLM={} ctx={} iter={} goal={} ===",
        llm_backend.model, llm_backend.context_window, max_iter, goal
    );
    let log = agent.run(goal)?;
    for line in &log {
        println!("{line}");
    }
    Ok(())
}

#[cfg(not(feature = "azalea-bot"))]
fn main() {
    eprintln!("需要 --features azalea-bot");
}
