//! SoM 端到端演示（需 `--features real`）：
//! 1) 合成一张"游戏内截图"（深色背景 + 底部 9 个 hotbar 槽位）；
//! 2) 用 [`mc_hotbar_marks`] 生成编号；
//! 3) [`render_marks`] 叠加编号框；
//! 4) 把编号图存盘，并（传 `--select` 且设了 VLM key）让 VLM 选号验证闭环。
//!
//! 运行：
//! `cargo run -p craft-agent-model --example som_demo --features real [-- --select]`

#[cfg(feature = "real")]
fn main() -> anyhow::Result<()> {
    use craft_agent_model::som::{mc_hotbar_marks, render::render_marks, select_mark_id};
    use craft_agent_model::vision::real::OpenAiVisionClient;
    use image::{DynamicImage, ImageFormat, Rgb, RgbImage};

    let (w, h) = (1091u32, 724u32);

    // 1) 合成截图：深灰背景 + 底部 9 个浅色槽位（模拟 hotbar）
    let mut img = RgbImage::from_pixel(w, h, Rgb([20u8, 20, 28]));
    let marks = mc_hotbar_marks(w, h);
    for m in &marks {
        let [x, y, ww, hh] = m.bbox;
        for py in y.max(0)..(y + hh).min(h as i32) {
            for px in x.max(0)..(x + ww).min(w as i32) {
                img.put_pixel(px as u32, py as u32, Rgb([90u8, 90, 110]));
            }
        }
    }
    let mut raw = Vec::new();
    DynamicImage::ImageRgb8(img).write_to(&mut std::io::Cursor::new(&mut raw), ImageFormat::Png)?;

    // 2) 渲染编号叠加
    let marked = render_marks(&raw, &marks)?;
    let out_path = std::env::temp_dir().join("som_demo_marked.png");
    std::fs::write(&out_path, &marked)?;
    println!(
        "[演示] 已生成编号图 {}（{} 个 hotbar 槽位编号）",
        out_path.display(),
        marks.len()
    );

    // 3) 若传 --select 且设了 VLM key，让 VLM 选号验证闭环
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--select") {
        match OpenAiVisionClient::from_env() {
            Ok(client) => {
                let id = select_mark_id(&client, &marked, "把第 3 个槽位（中间偏左）选中")?;
                println!("[演示] VLM 选中编号 = {id}（期望 3）");
            }
            Err(e) => println!("[演示] 跳过选号（无 VLM key）：{e}"),
        }
    } else {
        println!("[演示] 未传 --select，只生成编号图。加 --select 可让 VLM 选号验证闭环。");
    }
    Ok(())
}

#[cfg(not(feature = "real"))]
fn main() {
    eprintln!(
        "本示例需要 --features real 才能渲染/选号。\n运行：cargo run -p craft-agent-model --example som_demo --features real [-- --select]"
    );
}
