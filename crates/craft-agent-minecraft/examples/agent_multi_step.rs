//! P2 Agent — Pi 风格: Agent::run() 内部执行所有工具。
//!
//! 用法:
//!   cargo run -p craft-agent-minecraft --example agent_multi_step --features real -- --steps=10

#[cfg(feature = "real")]
fn main() -> anyhow::Result<()> {
    use craft_agent::agent::{Agent, AgentConfig, DecideFn};
    use craft_agent_minecraft::adapter::MinecraftAdapter;
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

    // Agent 配置 (Pi 风格: 短提示词 + 工具定义)
    let agent_cfg = AgentConfig {
        system_prompt: "\
你是 Minecraft AI 玩家。你在一个 while 循环中运行:
  while True:
    1. perceive() -- 拍照看周围
    2. 决定: aim_and_mine(挖掘) / move_forward(探索) / look(观察)
    3. 观察结果, 继续

目标: 收集木材和石头。看到树挖树, 看到石头挖石头。没目标就前进。

示例思维:
- perceive 看到树 -> aim_and_mine tree
- 挖完后 perceive -> 树消失, 前方是平原 -> move_forward
- perceive -> 没看到目标 -> look dx=300 右转
- perceive -> 左前方有石头 -> aim_and_mine stone

不要停滞。不要问问题。直接行动。".into(),

        tools: vec![
            serde_json::json!({"type":"function","function":{
                "name":"perceive","description":"拍照识别3D物体(树/石头/水等),返回物体列表。"
            }}),
            serde_json::json!({"type":"function","function":{
                "name":"aim_and_mine","description":"转动视角对准目标并挖掘2秒。",
                "parameters":{"type":"object","properties":{"target":{"type":"string"}},"required":["target"]}
            }}),
            serde_json::json!({"type":"function","function":{
                "name":"move_forward","description":"按住W向前移动探索。",
                "parameters":{"type":"object","properties":{"ticks":{"type":"integer","description":"80≈4秒","default":80}}}
            }}),
            serde_json::json!({"type":"function","function":{
                "name":"look","description":"转动视角。dx>0右转,dy>0下看。",
                "parameters":{"type":"object","properties":{"dx":{"type":"integer"},"dy":{"type":"integer"}},"required":["dx","dy"]}
            }}),
        ],
        max_turns,
    };

    let vision: Box<dyn VisionClient> = Box::new(OpenAiVisionClient::from_config(vlm_backend)?);
    let llm = Arc::new(OpenAiLlmClient::from_config(llm_backend)?);
    let adapter = MinecraftAdapter::new(vision)?;
    let mut agent = Agent::new(adapter, agent_cfg);

    // LLM 决策闭包 (唯一需要注入的外部依赖)
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
