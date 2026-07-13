//! 诊断：验证 enigo 在本机能否真正移动鼠标光标（不碰 MC，仅移动系统光标并读回）。
//!
//! 用法：
//!   cargo run -p craft-agent-minecraft --example mouse_probe --features real
//!
//! 预期：移动前/后光标 x 相差约 50；说明 enigo 能发出真实鼠标移动。
//! 若读数不变 → enigo 在本机失效（需切 windows crate SendInput 兜底）。

#[cfg(feature = "real")]
fn main() -> anyhow::Result<()> {
    use enigo::{Coordinate, Enigo, Mouse};

    let mut enigo = Enigo::new(&enigo::Settings::default())?;
    let (x0, y0) = enigo.location()?;
    println!("[probe] 移动前光标 = ({x0}, {y0})");

    enigo.move_mouse(50, 0, Coordinate::Rel)?;
    let (x1, y1) = enigo.location()?;
    println!("[probe] +50 后光标 = ({x1}, {y1})  (期望 x 增 ~50)");

    enigo.move_mouse(-50, 0, Coordinate::Rel)?; // 还原
    let (x2, y2) = enigo.location()?;
    println!("[probe] 还原后光标 = ({x2}, {y2})  (期望回到起始)");

    let moved = (x1 - x0).abs();
    println!(
        "[probe] 结论：enigo {} 移动光标（Δx={moved}）",
        if moved > 10 { "能" } else { "不能" }
    );
    Ok(())
}

#[cfg(not(feature = "real"))]
fn main() {
    eprintln!("本示例需要 --features real 编译（接入 enigo）。");
    std::process::exit(2);
}
