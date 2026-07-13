//! Phase 0 真机验证脚手架：enigo 视角控制 + xcap 截图
//!
//! 必须在**真实运行 Minecraft（窗口化、固定分辨率）**时由人眼观察。
//! 用法：
//!   cargo run -- view      # 测试 enigo 相对鼠标移动能否转动 MC 视角
//!   cargo run -- capture   # 测试 xcap 能否截到 MC 窗口并定位坐标基准
//!
//! 依赖：enigo 0.6 / xcap 0.9（cargo 自动解析版本）。
//! 注意：MC 视角用 raw mouse input，需验证 enigo 的 Relative 移动能否驱动。
//! 若 view 模式视角不转 → 退回 windows crate 直接调 SendInput（见下方注释）。

use anyhow::{Result, anyhow};
use enigo::{Button, Enigo, Mouse, Settings};
use image::imageops;
use std::thread;
use std::time::Duration;

fn find_mc_window() -> Result<xcap::Window> {
    let windows = xcap::Window::all()?;
    for w in windows {
        let title = w.title()?;
        if title.to_lowercase().contains("minecraft") {
            return Ok(w);
        }
    }
    Err(anyhow!(
        "未找到标题含 'Minecraft' 的窗口，请先打开 MC（窗口化、固定分辨率）"
    ))
}

/// 获取系统 DPI 缩放比例（Windows 注册表）
fn get_dpi_scale() -> f64 {
    let key = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER);
    if let Ok(subkey) = key.open_subkey(r"Control Panel\Desktop\WindowMetrics")
        && let Ok(dpi_raw) = subkey.get_value::<u32, _>("AppliedDPI")
    {
        return dpi_raw as f64 / 96.0;
    }
    1.0
}

fn mode_capture() -> Result<()> {
    // 声明进程 DPI 感知
    // 效果：① enigo 鼠标坐标走物理像素 ② xcap Window::capture_image() 返回完整画面
    // 实测：开启后窗口直捕从 629x658(裁断) → 1091x724(150% DPI 下接近完整)
    let _ = enigo::set_dpi_awareness();

    let w = find_mc_window()?;
    let title = w.title()?;
    let id = w.id()?;
    let wx = w.x()?;
    let wy = w.y()?;
    let ww = w.width()?;
    let wh = w.height()?;
    println!(
        "[窗口] title='{}'  id={}  pos=({}, {})  size={}x{}",
        title, id, wx, wy, ww, wh
    );

    let reg_dpi = get_dpi_scale();
    println!("[DPI] 注册表 AppliedDPI = {:.2}x", reg_dpi);

    // ══════════════════════════════════════════
    // 方法 A（主力）：xcap 直接捕获窗口
    //   抓的是窗口自身帧缓冲 → 即使被其他应用遮挡也能拿到完整界面（对 VLM 输入至关重要）
    //   set_dpi_awareness 后实测质量大幅提升（629x658 → 1091x724）
    //   判定标准：截图尺寸 ≥ 窗口报告尺寸的 85%（余量为 Windows DWM 边框/阴影）
    //   注：方法 C 是"全屏截屏后裁剪"，窗口被挡则挡的部分去不掉，仅作兜底
    // ══════════════════════════════════════════
    let mut primary_ok = false;
    if let Ok(img) = w.capture_image() {
        let aw = img.width();
        let ah = img.height();
        let cov_w = aw as f64 / ww as f64;
        let cov_h = ah as f64 / wh as f64;
        println!(
            "[方法A] 窗口直捕 {}x{}  覆盖率={:.0}x{:.0}%",
            aw,
            ah,
            cov_w * 100.0,
            cov_h * 100.0
        );

        if cov_w > 0.85 && cov_h > 0.85 {
            img.save("mc_capture.png")?;
            println!("  → 采用方法A（覆盖率达标），输出 mc_capture.png");
            primary_ok = true;
        } else {
            img.save("mc_capture_window.png")?;
            println!("  → 方法A不达标，保存为对照 mc_capture_window.png");
        }
    }

    // ══════════════════════════════════════════
    // 方法 C（兜底）：Monitor 全屏截取 + 按窗口 rect 裁切
    //   仅在方法 A 不达标时启用
    //   坐标规则：set_dpi_awareness 后 xcap 窗口坐标 ≈ 物理像素，直接用不乘 scale_factor
    // ══════════════════════════════════════════
    if !primary_ok {
        let monitors = xcap::Monitor::all()?;
        let cx = wx + (ww / 2) as i32;
        let cy = wy + (wh / 2) as i32;

        let chosen = monitors.iter().find(|m| {
            let mx = m.x().unwrap_or(0);
            let my = m.y().unwrap_or(0);
            let mw = m.width().unwrap_or(0) as i32;
            let mh = m.height().unwrap_or(0) as i32;
            cx >= mx && cx <= mx + mw && cy >= my && cy <= my + mh
        });
        let m = chosen.unwrap_or(&monitors[0]);
        let sf = m.scale_factor().unwrap_or(1.0);
        let mw_phys = m.width()?;
        let mh_phys = m.height()?;
        println!(
            "[显示器] scale_factor={:.2}  物理尺寸={}x{}",
            sf, mw_phys, mh_phys
        );

        let mut full = m.capture_image()?;
        let fw = full.width();
        let fh = full.height();

        // 原始坐标直接用（set_dpi_awareness 已对齐到物理像素空间）
        let (crop_x, crop_y, mut crop_w, mut crop_h) = (wx as u32, wy as u32, ww, wh);

        // 边界钳制
        if crop_x + crop_w > fw {
            crop_w = fw - crop_x;
        }
        if crop_y + crop_h > fh {
            crop_h = fh - crop_y;
        }
        println!(
            "[方法C] 全屏裁切 rect=({}, {}, {}x{})  from {}x{}  (sf={:.2}, 坐标未×)",
            crop_x, crop_y, crop_w, crop_h, fw, fh, sf
        );

        let cropped = imageops::crop(&mut full, crop_x, crop_y, crop_w, crop_h).to_image();
        cropped.save("mc_capture.png")?;
        println!(
            "  → 采用方法C，输出 mc_capture.png  ({}x{})",
            cropped.width(),
            cropped.height()
        );
    }

    Ok(())
}

fn mode_view() -> Result<()> {
    let mut enigo = Enigo::new(&Settings::default())?;
    println!("[提示] 请先用鼠标点击 MC 窗口使其获得焦点（不要锁定点），程序将做相对鼠标移动");
    thread::sleep(Duration::from_secs(2));

    println!("[enigo] 就绪，准备做 5 次相对移动 dx=200, dy=0 ...");
    for i in 1..=5 {
        enigo.move_mouse(200, 0, enigo::Coordinate::Rel)?;
        println!(
            "[enigo] 第 {} 次 move_mouse(200,Rel) 完成 —— 请观察准星/视角是否转动",
            i
        );
        thread::sleep(Duration::from_secs(1));
    }

    enigo.button(Button::Left, enigo::Direction::Click)?;
    println!("[enigo] 左键 Click 完成");

    // 若视角不转 → 退回 windows crate 的 SendInput：
    //   use windows::Win32::UI::Input::KeyboardAndMouse::*;
    //   unsafe { SendInput(...); }
    println!("[完成] 若视角未转动 → 改用 windows/SendInput 重测");
    Ok(())
}

fn mode_coord() -> Result<()> {
    // DPI 感知：enigo 绝对坐标走物理像素，与 xcap 窗口坐标（已对齐物理）同源
    let _ = enigo::set_dpi_awareness();

    let w = find_mc_window()?;
    let wx = w.x()?;
    let wy = w.y()?;
    let ww = w.width()?;
    let wh = w.height()?;
    println!(
        "[窗口] pos=({}, {})  size={}x{}  (set_dpi_awareness 后应为物理像素)",
        wx, wy, ww, wh
    );

    let mut enigo = Enigo::new(&Settings::default())?;

    // 安全边距：MC 光标移出窗口会自暂停，所有点击目标必须钳制在窗口内并留 margin
    let margin: i32 = 20;
    let clamp_x = |x: i32| x.max(wx + margin).min(wx + ww as i32 - margin);
    let clamp_y = |y: i32| y.max(wy + margin).min(wy + wh as i32 - margin);

    // 绝对定位测试点：中心 / 左上 1/4 / 右下 3/4（均经钳制，避免触边暂停）
    let tests: [(&str, i32, i32); 3] = [
        (
            "中心",
            clamp_x(wx + (ww / 2) as i32),
            clamp_y(wy + (wh / 2) as i32),
        ),
        (
            "左上1/4",
            clamp_x(wx + (ww / 4) as i32),
            clamp_y(wy + (wh / 4) as i32),
        ),
        (
            "右下3/4",
            clamp_x(wx + (ww * 3 / 4) as i32),
            clamp_y(wy + (wh * 3 / 4) as i32),
        ),
    ];

    for (name, tx, ty) in tests {
        println!(
            "[enigo] 移动光标到 {} ({}, {}) —— 请在 MC 窗口确认光标是否落在该位置",
            name, tx, ty
        );
        enigo.move_mouse(tx, ty, enigo::Coordinate::Abs)?;
        thread::sleep(Duration::from_secs(2));
    }

    // 在最后一个点（右下 3/4）点击，验证落点
    let (_, lx, ly) = tests[2];
    println!(
        "[enigo] 在 ({}, {}) 点击左键 —— 观察 MC 是否在该位置响应",
        lx, ly
    );
    enigo.button(Button::Left, enigo::Direction::Click)?;

    println!("[完成] 若三个点光标落位均正确、点击响应符合预期 → 第3项通过");
    println!("[提示] 若整体偏移固定量（如都偏左 N 像素）→ 可能需对坐标做补偿，把偏移告诉我");
    Ok(())
}

fn main() -> Result<()> {
    let mode = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "capture".to_string());
    match mode.as_str() {
        "view" => mode_view(),
        "capture" => mode_capture(),
        "coord" => mode_coord(),
        other => Err(anyhow!("未知模式 '{}'，请用 view / capture / coord", other)),
    }
}
