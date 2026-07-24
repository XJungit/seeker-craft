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
//!
//! 端点适配说明（重要）：本 demo 默认走的 [llm] 后端（如本地 OC-DSV4F 代理）
//! 背后的上游**不支持多轮 tool-calling 历史**——只要发出的 messages 含
//! assistant.tool_calls 或 role:"tool" 就返回 invalid_request_error / Upstream
//! request failed。此限制在 craft-agent-model 的 OpenAiLlmClient::chat_tools
//! 内通过 fold_tool_history() 适配：发送前把 tool 历史折叠为纯文本
//! （删 role:tool、剥 tool_calls、结果并入 content）。agent 核心的多轮
//! 协议不受影响（它读自身内存的 messages）。换用原生支持多轮 tool 的
//! 端点时，可将该折叠改为可配置关闭以保留完整多轮上下文。

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
        .unwrap_or(10);
    let goal: String = args
        .iter()
        .find(|a| a.starts_with("--goal="))
        .map(|s| s.trim_start_matches("--goal=").to_string())
        .unwrap_or_else(|| "你每轮都会收到[已知世界记忆·邻近]（bot 自动扫描到的资源点/结构/容器坐标）。请优先利用这些已知坐标：先 perceive 确认状态，然后去最近的 dark_oak_log 砍 8 个原木；砍完后若记得附近有合适位置，用 place 放一个 crafting_table 并记住它的坐标（用 memory 工具 anchor 命名）。完成后 chat 汇报你用了哪些记忆坐标。".to_string());

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

    // 共享世界记忆库（适配器自动扫描回填 + 工具显式记录 + Agent 每轮注入）。
    let world_mem = craft_agent::core::memory::WorldMemory::new();

    // 连接 azalea adapter：azalea 内部用独立 OS 线程跑自己的 runtime，
    // 此处仅用一次性局部 runtime 把 async connect 跑完，拿到句柄后立即 drop。
    let adapter = {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        rt.block_on(ArcAzaleaAdapter::connect_with_memory(
            "localhost:4444",
            "craftbot",
            world_mem.clone(),
        ))
        .expect("azalea adapter 连接失败（确认服为纯 vanilla 26.2）")
    };

    // 临时验证桩：等 bot 扫描回填世界记忆，打印采样（确认 A 的扫描来源有数据）
    std::thread::sleep(std::time::Duration::from_secs(6));
    println!(
        "[verify] 扫描到 {} 条世界记忆；采样: {}",
        world_mem.len(),
        world_mem
            .to_json()
            .chars()
            .take(300)
            .collect::<String>()
    );
    if let Some(a) = world_mem.find_anchor("__self__") {
        println!("[verify] __self__ 锚点: {:?}", a.pos);
    }

    // 注册 azalea 工具集（持有 adapter 引用）。
    let mut registry = ToolRegistry::new();
    for tool in create_mc_azalea_tools(adapter, world_mem.clone()) {
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
         可用工具：\n\
         - perceive()：读坐标/背包/附近玩家（无参数）。\n\
         - goto(x,y,z)：A* 导航到坐标。\n\
         - mine_below()：挖脚下方块（向下探矿，会持续挖直到你改指令）。\n\
         - mine(x,y,z)：挖掉指定世界坐标的方块（精确挖掘）。\n\
         - interact_block(x,y,z)：对着指定坐标方块交互（放置/右键激活）。\n\
         - attack(target)：攻击最近的生物（自卫/狩猎），target 可填 nearest。\n\
          - craft(item,count)：2×2 背包合成（无需工作台），如 craft(\"oak_planks\",4) / craft(\"stick\")。\n\
          - craft_3x3(item,count)：3×3 工作台合成（需先右键打开工作台），如 craft_3x3(\"furnace\") / craft_3x3(\"chest\")。\n\
          - smelt(output,fuel,count)：熔炼（需先右键打开熔炉），如 smelt(\"iron_ingot\",\"coal\") / smelt(\"charcoal\",\"oak_log\")。\n\
          - gather(item,count)：走到最近方块并挖掘（早期采集），如 gather(\"oak_log\",4) / gather(\"stone\",8)。\n\
          - place(item,x,y,z)：把手持物品放到坐标旁（如 place(\"crafting_table\",x,y,z) 造工作台）。\n\
          - open(x,y,z)：打开坐标处容器（工作台/熔炉），随后可 craft_3x3 / smelt。\n\
           - auto_craft(item,count)：高层一键造任意已登记物品（推荐），如 auto_craft(\"chest\",1) / auto_craft(\"iron_ingot\",3)，bot 自主采集+合成+熔炼+放置容器。\n\
           - enchant(item,level)：附魔（需先 open 打开附魔台，且背包有 item 与青金石 lapis_lazuli），level 取 1/2/3，如 enchant(\"iron_sword\",2)。\n\
           - interact_entity(kind)：右键交互最近的实体（如 villager）。先走到村民附近再用。\n\
           - trade(offer)：与最近的村民交易，选第 offer 个报价（0 起）。需先靠近村民。\n\
           - chat(content)：发聊天消息，用于向玩家汇报进度。\n\
         行为准则：\n\
         1) 下探任务：连续调 mine_below 2~3 次后，调一次 chat 汇报当前 Y 坐标与进度，\n\
              再继续 mine_below。穿插 chat 汇报，不要无脑连续调同一工具超过 3 次。\n\
         2) 若 perceive 返回含 \"[卡住N轮]\" 提示（Y 坐标不变，已挖到基岩或脚下无可破坏方块），\n\
              必须停止下探，用 chat 向玩家说明情况后，以纯文本宣布任务完成/无法继续——\n\
              不得继续调 mine_below，也不得假装还在挖。\n\
         3) perceive 可随时调用确认状态，不必每轮都调。\n\
          4) 任务确实无法推进时，允许纯文本结束（说明原因），这不算错误。\n\
          5) 你每轮会收到 [已知世界记忆·邻近] 段落，列出自动扫描到的资源点/结构/容器坐标。\n\
             做空间决策时优先直接复用这些已知坐标（如 goto 到记忆里的 dark_oak_log），\n\
             不要盲目乱走。也可用 memory 工具显式记录/查询锚点。\n\
             注意：被砍光/挖完的资源点会被标记为「已耗尽」，不要再回头去采。",
    );
    let cfg = AgentConfig::new(system_prompt, max_iter)
        .with_compaction(compaction)
        // azalea 路线无 mod 专属知识：关闭 MC_KNOWLEDGE_BASE 与 world_info，
        // 仅用工具自描述，避免 LLM 误调 azalea 不存在的 collect/combat 等工具。
        .with_knowledge_base(None)
        .with_world_info(None)
        .with_knowledge_tool(false);

    let mut agent = Agent::new(Box::new(Lp { llm }), registry, cfg).with_world_memory(world_mem);

    println!(
        "\n=== AZALEA localhost:4444 | LLM={} ctx={} iter={} goal={} ===",
        llm_backend.model, llm_backend.context_window, max_iter, goal
    );
    let log = agent.run(goal)?;
    for line in &log {
        println!("{line}");
    }
    // 临时验证桩：run 后再次 dump，观察 B（砍树→资源记忆变化）/C（depleted 标记）是否生效
    let wm = agent.world_memory();
    println!(
        "[verify] run 后世界记忆 {} 条；含 depleted 的资源点: {}",
        wm.len(),
        wm.query(Some(craft_agent::core::memory::MemoryKind::Resource), None)
            .into_iter()
            .filter(|c| c.depleted)
            .count()
    );
    Ok(())
}

#[cfg(not(feature = "azalea-bot"))]
fn main() {
    eprintln!("需要 --features azalea-bot");
}
