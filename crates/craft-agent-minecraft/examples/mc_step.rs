//! P1.4 真机验证示例：MinecraftAdapter 走一遍 capture → perceive（→ 可选 execute）。
//!
//! 用法（需先打开 Minecraft 窗口化、固定分辨率，并在前台）：
//!   cargo run -p craft-agent-minecraft --example mc_step --features real
//!       → 只读：截一张 MC 窗口、调 VLM 出场景描述，不碰鼠标键盘。安全可反复跑。
//!   cargo run -p craft-agent-minecraft --example mc_step --features real -- --act
//!       → 在只读基础上，额外执行一次 Look(50,0) 演示视角转动（请人眼观察准星）。
//!
//! VLM 后端从环境变量读取：AGNES_API_KEY / AGNES_API_BASE / AGNES_MODEL
//! （或改用 config/agent.toml 的 minicpm，速度更快）。

#[cfg(feature = "real")]
fn main() -> anyhow::Result<()> {
    use craft_agent::core::adapter::GameAdapter;
    use craft_agent::core::types::Action;
    use craft_agent_minecraft::adapter::MinecraftAdapter;
    use craft_agent_model::vision::real::OpenAiVisionClient;

    let act = std::env::args().any(|a| a == "--act");

    println!("[mc_step] 构造 VLM 客户端（from_env）...");
    let vision = OpenAiVisionClient::from_env()?;
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
