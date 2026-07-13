//! P1.4 真机验证示例：MinecraftAdapter 走一遍 capture → perceive（→ 可选 execute）。
//!
//! 用法（需先打开 Minecraft 窗口化、固定分辨率，并在前台）：
//!   cargo run -p craft-agent-minecraft --example mc_step --features real
//!       → 只读：截一张 MC 窗口、调 VLM 出场景描述，不碰鼠标键盘。安全可反复跑。
//!         VLM 后端默认读 config/agent.toml 的 active（现=minicpm，~0.7s）。
//!   cargo run -p craft-agent-minecraft --example mc_step --features real -- --config path/to/agent.toml
//!       → 指定后端配置（active 决定用哪个 VLM）。
//!   cargo run -p craft-agent-minecraft --example mc_step --features real -- --env
//!       → 强制从环境变量读取（AGNES_API_KEY / AGNES_API_BASE / AGNES_MODEL）。
//!   cargo run -p craft-agent-minecraft --example mc_step --features real -- --act
//!       → 在只读基础上，额外执行一次 Look(50,0) 演示视角转动（请人眼观察准星）。

#[cfg(feature = "real")]
fn main() -> anyhow::Result<()> {
    use craft_agent::core::adapter::GameAdapter;
    use craft_agent::core::types::Action;
    use craft_agent_minecraft::adapter::MinecraftAdapter;
    use craft_agent_model::config::AgentConfig;
    use craft_agent_model::vision::real::OpenAiVisionClient;
    use std::fs;

    let args: Vec<String> = std::env::args().collect();
    let act = args.iter().any(|a| a == "--act");
    let use_env = args.iter().any(|a| a == "--env");
    let cfg_path = args
        .iter()
        .find(|a| a.starts_with("--config="))
        .map(|s| s.trim_start_matches("--config=").to_string());

    let vision = if use_env {
        println!("[mc_step] 构造 VLM 客户端（from_env: AGNES_API_*）...");
        OpenAiVisionClient::from_env()?
    } else {
        let p = cfg_path.unwrap_or_else(|| "config/agent.toml".to_string());
        println!("[mc_step] 构造 VLM 客户端（config={p} 的 active 后端）...");
        let cfg = AgentConfig::load(fs::read_to_string(&p)?)?;
        let backend = cfg.vlm.active_backend()?;
        OpenAiVisionClient::from_config(backend)?
    };
    let mut adapter = MinecraftAdapter::new(Box::new(vision))?;

    println!("[mc_step] capture：截 MC 窗口（方法 A）...");
    let png = adapter.capture()?;
    println!("  → 截图 {} 字节（PNG）", png.len());

    println!("[mc_step] perceive：布局编号 + VLM 场景描述...");
    let ws = adapter.perceive()?;
    println!("  scene: {}", ws.scene_desc);
    println!("  marked_elements: {}", ws.marked_elements.len());
    for e in &ws.marked_elements {
        println!(
            "    #{} {}  bbox={:?} center={:?}",
            e.id, e.label, e.bbox, e.center
        );
    }

    if act {
        println!("[mc_step] --act：演示 Look(50,0)（请观察 MC 准星/视角是否转动）...");
        let r = adapter.execute(Action::Look { dx: 50, dy: 0 })?;
        println!("  → {}", r.detail);
    } else {
        println!("[mc_step] 只读模式：未执行任何鼠标/键盘动作。加 --act 才执行一次 Look。");
    }
    Ok(())
}

#[cfg(not(feature = "real"))]
fn main() {
    eprintln!("本示例需要 --features real 编译（接入 xcap/enigo/VLM）。");
    std::process::exit(2);
}
