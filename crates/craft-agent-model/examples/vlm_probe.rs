//! 真实 VLM 探针：读一张 PNG 截图 → 调 OpenAI 兼容后端 → 打印描述。
//!
//! 用途：验证 Rust 侧 `OpenAiVisionClient` 能打通真实 API 并读懂 MC 截图。
//!
//! 两种后端来源：
//! 1) **配置文件（推荐）**：`--config <toml>`，用其中 `[vlm].active` 选定的后端
//! 2) **环境变量（快速测试）**：不带 --config 时读 AGNES_API_KEY / AGNES_API_BASE / AGNES_MODEL
//!
//! 用法（在 workspace 根目录运行）：
//! ```bash
//! # 配置文件方式（换后端只改 toml 里的 active）
//! cargo run -p craft-agent-model --example vlm_probe --features real -- <png路径> --config config/agent.toml
//! # 环境变量方式
//! cargo run -p craft-agent-model --example vlm_probe --features real -- <png路径> ["自定义prompt"]
//! ```

#[cfg(feature = "real")]
fn main() -> anyhow::Result<()> {
    use craft_agent_model::config::AgentConfig;
    use craft_agent_model::vision::real::{OpenAiVisionClient, downscale_png};

    // 解析参数：第一个非 flag 位置参数=png；--config <path>=配置；其余非 flag=prompt
    let mut png_path: Option<String> = None;
    let mut config_path: Option<String> = None;
    let mut prompt: Option<String> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--config" | "-c" => config_path = args.next(),
            _ if png_path.is_none() => png_path = Some(a),
            _ if prompt.is_none() => prompt = Some(a),
            _ => {}
        }
    }
    let path = png_path
        .ok_or_else(|| anyhow::anyhow!("用法: vlm_probe <png路径> [--config <toml>] [prompt]"))?;
    let prompt = prompt.unwrap_or_else(|| {
        "以游戏 AI agent 的视角分析这张 Minecraft 截图：1) 当前界面；\
         2) 可交互元素/物体及大致位置；3) 玩家状态；\
         4) 若目标是砍树凑木头，下一步该做什么。用简洁中文分点。"
            .to_string()
    });

    let png = std::fs::read(&path)?;
    println!("[探针] 读入 {} ({} 字节)", path, png.len());

    // 构造客户端：优先配置文件，否则环境变量
    let client = match &config_path {
        Some(cfg_path) => {
            let cfg = AgentConfig::load(cfg_path)?;
            let backend = cfg.vlm.active_backend()?;
            println!(
                "[探针] 使用配置后端 active=\"{}\"  model={}  url={}",
                cfg.vlm.active,
                backend.model,
                backend.chat_endpoint()
            );
            // VLM 输入优化直观对比：打印缩放前后的尺寸与体积
            match backend.max_side {
                Some(ms) => {
                    let (scaled, (w, h)) = downscale_png(&png, ms)?;
                    println!(
                        "[探针] 输入缩放 max_side={ms}：{} 字节 → {}×{} {} 字节（约 {:.0}%）",
                        png.len(),
                        w,
                        h,
                        scaled.len(),
                        scaled.len() as f64 / png.len() as f64 * 100.0
                    );
                }
                None => println!("[探针] 未配置 max_side，按原图发送"),
            }
            OpenAiVisionClient::from_config(backend)?
        }
        None => {
            println!("[探针] 使用环境变量后端（AGNES_*）");
            OpenAiVisionClient::from_env()?
        }
    };

    println!("[探针] 调用中 …");
    let t0 = std::time::Instant::now();
    let out = client.chat_image_png(&png, &prompt)?;
    println!("[探针] 完成，用时 {:.1}s\n", t0.elapsed().as_secs_f32());
    println!("{out}");
    Ok(())
}

#[cfg(not(feature = "real"))]
fn main() {
    eprintln!(
        "请加 --features real 编译运行：cargo run -p craft-agent-model --example vlm_probe --features real -- <png>"
    );
}
