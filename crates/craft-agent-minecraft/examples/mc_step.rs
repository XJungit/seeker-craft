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
//!       → 在只读基础上，额外执行一次 Look(400,0) 演示视角转动（请人眼观察准星）。
//!         发输入前程序会自动把 MC 置为前台（绕过 Windows 前台锁），确保收到 enigo 合成输入。
//!   cargo run -p craft-agent-minecraft --example mc_step --features real -- --fullscreen
//!       → MC 全屏模式：capture 改用方法 C（主显示器整屏），消除窗口化焦点/暂停纠缠。
//!         端到端闭环推荐；配合 --act 验证全屏下 Look 视角转动。

#[cfg(feature = "real")]
fn main() -> anyhow::Result<()> {
    use craft_agent::core::adapter::GameAdapter;
    use craft_agent::core::types::Action;
    use craft_agent_minecraft::adapter::MinecraftAdapter;
    use craft_agent_model::config::AgentConfig;
    use craft_agent_model::vision::real::OpenAiVisionClient;

    // 加载项目根 .env（含 MINICPM_API_KEY 等密钥；已被 gitignore 不会进版本库）。
    // dotenvy 只补充「未设置」的环境变量，不覆盖已存在的（如 setx 持久化的同名变量）。
    let _ = dotenvy::dotenv();

    let args: Vec<String> = std::env::args().collect();
    let act = args.iter().any(|a| a == "--act");
    let use_env = args.iter().any(|a| a == "--env");
    let fullscreen = args.iter().any(|a| a == "--fullscreen");
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
        // 注意：AgentConfig::load 自己按路径读文件，不要先 fs::read_to_string 再传字符串，
        // 否则会把整份配置内容当成文件名去 open → Windows os error 123。
        let cfg = AgentConfig::load(&p)?;
        let backend = cfg.vlm.active_backend()?;
        OpenAiVisionClient::from_config(backend)?
    };
    let mut adapter = if fullscreen {
        println!("[mc_step] 全屏模式：capture 走方法 C（主显示器整屏）...");
        MinecraftAdapter::new_fullscreen(Box::new(vision))?
    } else {
        MinecraftAdapter::new(Box::new(vision))?
    };

    println!(
        "[mc_step] capture：截 MC {}...",
        if fullscreen {
            "主显示器整屏（方法 C）"
        } else {
            "窗口（方法 A）"
        }
    );
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
    println!("  detected_targets: {}", ws.detected_targets.len());
    for t in &ws.detected_targets {
        println!(
            "    {}  bbox={:?}  offset={:?}",
            t.label, t.bbox, t.offset_from_crosshair
        );
    }

    if act {
        println!(
            "[mc_step] --act：演示 Look(400,0)（程序会先把 MC 置前台，请观察视角是否转动）..."
        );
        let r = adapter.execute(Action::Look { dx: 400, dy: 0 })?;
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
