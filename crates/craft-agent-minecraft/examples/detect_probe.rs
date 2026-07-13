//! P2 检测验证探针：加载 YOLO-World 对 MC 截图执行真实 3D 目标检测。
//!
//! 用法：
//!   cargo run -p craft-agent-minecraft --example detect_probe --features real
//!   cargo run -p craft-agent-minecraft --example detect_probe --features real -- --fullscreen
//!
//! 输出：检测到的目标列表（label, bbox, offset_from_crosshair）。

#[cfg(feature = "real")]
fn main() -> anyhow::Result<()> {
    #[cfg(windows)]
    unsafe {
        windows_sys::Win32::UI::HiDpi::SetProcessDpiAwareness(
            windows_sys::Win32::UI::HiDpi::PROCESS_PER_MONITOR_DPI_AWARE,
        );
    }

    use craft_agent_minecraft::detect::ObjectDetector;
    use std::path::Path;
    use std::time::Instant;

    let args: Vec<String> = std::env::args().collect();
    let fullscreen = args.iter().any(|a| a == "--fullscreen");

    // 1. 加载 YOLO-World 检测器
    let models_dir = Path::new("models");
    println!("[detect_probe] 加载 YOLO-World 模型...");
    let t0 = Instant::now();
    let mut detector = ObjectDetector::load(models_dir)?;
    println!(
        "[detect_probe] 模型加载完成 ({:.2}s)",
        t0.elapsed().as_secs_f32()
    );

    // 2. 截图 —— 全屏模式直接截，窗口模式用 Win32 找 MC 窗口精确裁剪
    println!("[detect_probe] 捕获 MC 截图...");
    let (png, screen_w, screen_h) = if fullscreen {
        let monitors = xcap::Monitor::all()?;
        let mon = monitors
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("无显示器"))?;
        let img = mon.capture_image()?;
        let (w, h) = (img.width(), img.height());
        let mut png = Vec::new();
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)?;
        (png, w, h)
    } else {
        capture_mc_window()?
    };
    println!(
        "[detect_probe] 截图尺寸: {}x{} ({:.0}KB)  (w*h={})",
        screen_w,
        screen_h,
        png.len() as f64 / 1024.0,
        screen_w.saturating_mul(screen_h)
    );

    // 3. 保存截图到文件，方便肉眼对比检测结果
    std::fs::write("mc_screenshot.png", &png)?;
    println!("[detect_probe] 截图已保存到 mc_screenshot.png");

    // 4. 执行检测（debug 模式：打印分数分布 + top-20 候选）
    println!("[detect_probe] 运行 YOLO-World 检测 (debug 模式)...");
    let t1 = Instant::now();
    detector.detect_debug(&png, screen_w, screen_h)?;
    let elapsed = t1.elapsed().as_secs_f32();
    println!("[detect_probe] 检测完成 ({:.3}s)", elapsed);

    Ok(())
}

#[cfg(not(feature = "real"))]
fn main() {
    eprintln!(
        "请使用 --features real 编译：cargo build -p craft-agent-minecraft --example detect_probe --features real"
    );
}

/// Win32: xcap 窗口帧缓冲（遮挡免疫）。DPI 感知已在 main 顶部设置。
#[cfg(windows)]
fn capture_mc_window() -> anyhow::Result<(Vec<u8>, u32, u32)> {
    for attempt in 0..=1 {
        let windows = xcap::Window::all()?;
        let mc = windows
            .into_iter()
            .filter(|w| {
                w.title()
                    .map(|t| t.to_lowercase().contains("minecraft"))
                    .unwrap_or(false)
            })
            .max_by_key(|w| (w.width().unwrap_or(0) as u64) * (w.height().unwrap_or(0) as u64))
            .ok_or_else(|| anyhow::anyhow!("未找到标题含 minecraft 的窗口"))?;

        let title = mc.title().unwrap_or_default();
        let (wx, wy, ww, wh) = (mc.x()?, mc.y()?, mc.width()?, mc.height()?);
        let minimized = mc.is_minimized().unwrap_or(false);
        eprintln!(
            "[xcap] attempt={} \"{}\"  pos=({},{})  size={}x{}  minimized={}",
            attempt, title, wx, wy, ww, wh, minimized
        );

        if minimized && attempt == 0 {
            eprintln!("[xcap] MC 窗口已最小化，正在强制还原...");
            #[cfg(windows)]
            unsafe {
                use windows_sys::Win32::UI::WindowsAndMessaging::{
                    SW_RESTORE, SetForegroundWindow, ShowWindow,
                };
                let hwnd = mc.id()? as isize as windows_sys::Win32::Foundation::HWND;
                ShowWindow(hwnd, SW_RESTORE);
                SetForegroundWindow(hwnd);
            }
            std::thread::sleep(std::time::Duration::from_millis(1500));
            continue;
        }

        if ww < 200 || wh < 100 {
            anyhow::bail!("MC 窗口太小 ({}x{})，请先还原并拉大!", ww, wh);
        }

        // xcap 直捕窗口帧缓冲（遮挡免疫）
        let img = mc.capture_image()?;
        let (w, h) = (img.width(), img.height());
        let mut png = Vec::new();
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)?;
        return Ok((png, w, h));
    }
    anyhow::bail!("无法获取 MC 窗口截图")
}

#[cfg(not(windows))]
fn capture_mc_window() -> anyhow::Result<(Vec<u8>, u32, u32)> {
    anyhow::bail!("窗口捕获仅支持 Windows")
}
