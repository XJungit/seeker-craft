//! 真实 LLM 决策探针：构造一个 WorldState → 调 OpenAI 兼容后端 → 打印解析出的 Action。
//!
//! 用途：离线（无需开 MC）验证决策腿——LLM 能否从结构化世界状态里稳定产出
//! 合法的 Action JSON，并被 `value_to_action` 正确解析。
//!
//! 两种后端来源：
//! 1) **配置文件（推荐）**：`--config <toml>`，用其中 `[llm].active` 选定的后端
//! 2) **环境变量（快速测试）**：不带 --config 时读 LLM_API_KEY / LLM_API_BASE / LLM_MODEL
//!
//! 用法（在 workspace 根目录运行）：
//! ```bash
//! cargo run -p craft-agent-model --example llm_probe --features real -- --config config/agent.toml
//! # 或环境变量方式
//! LLM_API_KEY=<key> cargo run -p craft-agent-model --example llm_probe --features real
//! ```

#[cfg(feature = "real")]
fn main() -> anyhow::Result<()> {
    use craft_agent::core::types::{Element, Target, WorldState};
    use craft_agent_model::config::AgentConfig;
    use craft_agent_model::decision::DecisionClient;
    use craft_agent_model::decision::real::OpenAiLlmClient;

    // 解析参数：仅识别 --config <path>
    let mut config_path: Option<String> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == "--config" || a == "-c" {
            config_path = args.next();
        }
    }

    // 构造一个典型的"砍树"场景世界状态（模拟 VLM 感知输出）
    let state = WorldState {
        scene_desc: "玩家站在草地上，正前方约 3 格处有一棵橡木树，天色为白天。".into(),
        marked_elements: vec![Element {
            id: 1,
            label: "hotbar_axe".into(),
            bbox: [820, 1040, 40, 40],
            center: (840, 1060),
        }],
        detected_targets: vec![Target {
            label: "oak_tree".into(),
            bbox: [900, 500, 120, 260],
            offset_from_crosshair: (18, -6),
        }],
        self_hint: "血量满 20/20，饥饿满，快捷栏第 1 格是斧头（已选中），背包空。".into(),
        screenshot: vec![],
    };
    let skills_hint =
        "chop_tree: 对准树干 AimAndMine 直到掉落木头；若树不在准星附近，先 Look 对准。";

    // 构造客户端：优先配置文件，否则环境变量
    let client = match &config_path {
        Some(cfg_path) => {
            let cfg = AgentConfig::load(cfg_path)?;
            let group = cfg
                .llm
                .ok_or_else(|| anyhow::anyhow!("配置文件缺少 [llm] 段"))?;
            let backend = group.active_backend()?;
            println!(
                "[探针] 使用 LLM 配置后端 active=\"{}\"  model={}  url={}",
                group.active,
                backend.model,
                backend.chat_endpoint()
            );
            OpenAiLlmClient::from_config(backend)?
        }
        None => {
            println!("[探针] 使用环境变量后端（LLM_*）");
            OpenAiLlmClient::from_env()?
        }
    };

    println!("[探针] 场景：砍树凑木头。调用决策中 …");
    let t0 = std::time::Instant::now();
    let action = client.decide(&state, skills_hint)?;
    println!("[探针] 完成，用时 {:.1}s\n", t0.elapsed().as_secs_f32());
    println!("解析出的 Action = {action:?}");
    Ok(())
}

#[cfg(not(feature = "real"))]
fn main() {
    eprintln!(
        "请加 --features real 编译运行：cargo run -p craft-agent-model --example llm_probe --features real -- --config config/agent.toml"
    );
}
