//! Agent — 全量 mod 控制入口（craft-agent-bridge 桥接 mod）。
//!
//! 运行（先开 MC 并加载 craft-agent-bridge，再另开终端跑本程序）：
//! ```bash
//! cargo run -p craft-agent-minecraft --example agent_multi_step_mod --features mod-bridge \
//!   -- --steps=40 --goal="收集木头做工作台" --session=sessions/mc_run_mod.jsonl
//! ```
//! - 感知：mod 结构化状态（精确物品栏/方块/实体坐标），不依赖 VLM 看图猜。
//! - 动作：mod 进程内精确控制（look/press/mine/look_at），不抢鼠标键盘、可后台运行。
//! - `--steps` 最大决策迭代；`--goal` 目标；`--session` 可选断点续跑。

#[cfg(feature = "mod-bridge")]
fn main() -> anyhow::Result<()> {
    use craft_agent::agent::{Agent, AgentConfig, CompactionConfig, LlmProvider};
    use craft_agent::core::message::{AssistantResponse, Message};
    use craft_agent::core::session::Session;
    use craft_agent::core::tool::ToolRegistry;
    use craft_agent_minecraft::adapter_mod::MinecraftModAdapter;
    use craft_agent_minecraft::bridge::DEFAULT_PORT;
    use craft_agent_minecraft::tools_mod::create_mc_mod_tools;
    use craft_agent_model::config::AgentConfig as ModelConfig;
    use craft_agent_model::decision::real::OpenAiLlmClient;
    use craft_agent_model::vision::VisionClient;
    use craft_agent_model::vision::real::OpenAiVisionClient;
    use serde_json::Value;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    let _ = dotenvy::dotenv();
    let args: Vec<String> = std::env::args().collect();
    let max_iter: u32 = args
        .iter()
        .find(|a| a.starts_with("--steps="))
        .and_then(|s| s.trim_start_matches("--steps=").parse().ok())
        .unwrap_or(40);
    let goal: String = args
        .iter()
        .find(|a| a.starts_with("--goal="))
        .map(|s| s.trim_start_matches("--goal=").to_string())
        .unwrap_or_else(|| "收集木头做工作台".to_string());
    let session_path: Option<String> = args
        .iter()
        .find(|a| a.starts_with("--session="))
        .map(|s| s.trim_start_matches("--session=").to_string());
    let use_vision = args.iter().any(|a| a == "--vision");

    let model_cfg = ModelConfig::load("config/agent.toml")?;
    let perceive_cfg = model_cfg.perceive.unwrap_or_default();
    let shots_dir: Option<PathBuf> = session_path.as_ref().map(|p| {
        let p = Path::new(p);
        let parent = p.parent().unwrap_or_else(|| Path::new("."));
        let stem = p
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "session".to_string());
        parent.join(format!("{stem}.shots"))
    });

    let llm_group = model_cfg
        .llm
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("缺少 [llm]"))?;
    let llm_backend = llm_group.active_backend()?;
    let llm = Arc::new(OpenAiLlmClient::from_config(llm_backend)?);

    // VLM 视觉客户端（仅 --vision 时启用）
    let vision: Option<Arc<dyn VisionClient>> = if use_vision {
        let vlm_backend = model_cfg.vlm.active_backend()?;
        let vc = OpenAiVisionClient::from_config(vlm_backend)?;
        println!(
            "=== VLM 已启用: {}（仅在 visual_perceive 时调用） ===",
            vlm_backend.model
        );
        Some(Arc::new(vc))
    } else {
        println!("=== VLM 未启用（加 --vision 可启用视觉补充） ===");
        None
    };

    // Provider：转发 OpenAI 兼容带工具 chat。
    struct Lp {
        llm: Arc<OpenAiLlmClient>,
    }
    impl LlmProvider for Lp {
        fn complete(&self, m: &[Value], t: &[Value]) -> anyhow::Result<AssistantResponse> {
            self.llm
                .chat_tools(&Value::Array(m.to_vec()), &Value::Array(t.to_vec()))
        }
    }

    // 连接本机 MC 桥接 mod（MC 必须先启动并加载 craft-agent-bridge）。
    let adapter = Arc::new(Mutex::new(MinecraftModAdapter::connect_with_vision(
        "127.0.0.1",
        DEFAULT_PORT,
        vision,
    )?));

    let mut registry = ToolRegistry::new();
    for tool in create_mc_mod_tools(
        adapter.clone(),
        perceive_cfg.image_max_side,
        shots_dir.clone(),
        use_vision,
        None,
    ) {
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

    let mut system_prompt = String::from(
        "你是 Minecraft AI 玩家，通过服务端 mod 桥接（ServerPlayer 架构）精确控制角色。每轮会自动注入游戏状态（perceive），无需手动调用 perceive。\n\
         感知返回的是结构化状态（精确物品栏数量/方块与生物的世界坐标与距离/玩家坐标朝向/血量饥饿），直接据数据决策，不要靠看图猜。\n\
         核心工具: collect(target,count) 自动找→走→挖; move_to(x,y,z) 精确导航; look_at(x,y,z) 瞄准坐标; craft(item,count) 合成; place(item) 放置; combat(mode,ticks) 战斗。\n\
         禁止调用 look(dx,dy)/press(keys)/mine(ticks)/craftable() —— 这些工具未注册，会返回 Unknown tool。",
    );
    if use_vision {
        system_prompt.push_str(" visual_perceive(prompt) 工具可截屏看图，仅在需要识别 GUI 界面/合成台/背包画面时使用，平时用自动注入的 perceive 数据即可。");
    }
    system_prompt.push_str(
        "\n采集木头标准流程: collect(\"oak_log\", 8) → craft(\"oak_planks\", 32) → craft(\"crafting_table\", 1) → look_at(脚下地面坐标) → place(\"crafting_table\")。\n\
         collect 会自动找最近的目标方块、走过去、挖掘，回执里返回实际挖到的数量。如果数量不足，换个位置再 collect。\n\
         除非已合成工作台（物品栏出现 crafting_table 或已放置），否则每个回合必须以 tool_call 收尾，不得只用文本结束。",
    );
    let cfg = AgentConfig::new(system_prompt, max_iter)
        .with_compaction(compaction)
        .with_auto_perceive(true);

    let mut agent = match session_path {
        Some(p) => {
            let path = Path::new(&p);
            let sess = if path.exists() {
                Session::open(path)?
            } else {
                let mut s = Session::new("minecraft-mod");
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                s.save_to(path)?;
                s
            };
            Agent::new(Box::new(Lp { llm }), registry, cfg).with_session(sess)
        }
        None => Agent::new(Box::new(Lp { llm }), registry, cfg),
    };

    println!(
        "\n=== BRIDGE:127.0.0.1:{} LLM:{} (ctx={}) {} iterations, goal: {} ===",
        DEFAULT_PORT, llm_backend.model, llm_backend.context_window, max_iter, goal
    );
    match &shots_dir {
        Some(d) => println!("=== 截图落盘: {} （viewer 可逐张核对） ===\n", d.display()),
        None => println!("=== 截图不落盘（无 --session） ===\n"),
    }
    let t0 = Instant::now();
    let log = agent.run(goal)?;
    for line in &log {
        println!("{line}");
    }
    let turns = agent
        .messages
        .iter()
        .filter(|m| matches!(m, Message::Assistant(_)))
        .count();
    println!(
        "\n=== {} assistant turns, {} total messages, {:.1}s ===",
        turns,
        agent.messages.len(),
        t0.elapsed().as_secs_f64()
    );
    Ok(())
}

#[cfg(not(feature = "mod-bridge"))]
fn main() {
    eprintln!("需要 --features mod-bridge");
}
