//! Craft-Agent Minecraft 适配器（P1.4）：xcap 截图 + VLM 感知 + enigo 键鼠执行。
//!
//! 这是一个**独立 crate**，依赖核心抽象 [`craft-agent`](craft_agent)（GameAdapter /
//! WorldState / Action）与模型层 [`craft-agent-model`](craft_agent_model)（VisionClient / SoM）。
//! 这样核心与模型互不反向依赖，符合"换游戏 = 换 Adapter crate"的通用框架设计——
//! 后续加 `craft-agent-browser` 等不会污染核心。
//!
//! 真机依赖（xcap / enigo / image / craft-agent-model 的 real 特性）全部 gated 在 `real`
//! 特性后；默认构建只含纯函数（如 [`to_screen_coords`]），可离线单测、CI 无显示亦可编译。

#[cfg(feature = "real")]
pub mod adapter;
#[cfg(feature = "real")]
pub mod tools;

#[cfg(feature = "mod-bridge")]
pub mod adapter_mod;
#[cfg(feature = "mod-bridge")]
pub mod blueprint;
/// MC 桥接 mod 适配器（全量 mod 控制，见 §"治本缺口"）。仅 `mod-bridge` 特性编译。
#[cfg(feature = "mod-bridge")]
pub mod bridge;
#[cfg(feature = "mod-bridge")]
pub mod survival;
#[cfg(feature = "mod-bridge")]
pub mod survival_decisions;
#[cfg(feature = "mod-bridge")]
pub mod damage_source;
#[cfg(feature = "mod-bridge")]
pub mod stamina;
#[cfg(feature = "mod-bridge")]
pub mod task_base;
#[cfg(feature = "mod-bridge")]
pub mod combat_dsl;
#[cfg(feature = "mod-bridge")]
pub mod mindcraft_ext;
#[cfg(feature = "mod-bridge")]
pub mod tools_mod;

/// 点击/移动目标与窗口边缘的安全间距（像素）。MC 光标移出窗口会自暂停，所有坐标必须内缩。
pub const WINDOW_MARGIN: i32 = 20;

/// 把窗口内局部坐标（元素中心，相对窗口左上角）转成屏幕绝对坐标（加窗口偏移），
/// 并钳制在窗口内（留 [`WINDOW_MARGIN`]），避免触边导致 MC 自暂停或点错。
///
/// 设计要点（对齐 enigo_mc_test 实测结论）：
/// - `set_dpi_awareness` 后 xcap 窗口坐标已是物理像素，这里**不乘 scale_factor**。
/// - 不依赖任何显示/游戏，纯函数，离线可测。
pub fn to_screen_coords(
    wx: i32,
    wy: i32,
    ww: u32,
    wh: u32,
    local: (i32, i32),
    margin: i32,
) -> (i32, i32) {
    let sx = wx + local.0;
    let sy = wy + local.1;
    let min_x = wx + margin;
    let max_x = wx + ww as i32 - margin;
    let min_y = wy + margin;
    let max_y = wy + wh as i32 - margin;
    (sx.max(min_x).min(max_x), sy.max(min_y).min(max_y))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_coords_inside_window_and_clamped() {
        // 窗口在 (100,50)，大小 800x600，margin 20
        let (x, y) = to_screen_coords(100, 50, 800, 600, (400, 300), WINDOW_MARGIN);
        assert_eq!((x, y), (500, 350), "正常局部坐标应加窗口偏移");

        // 越过右边界 → 钳制到 max_x
        let (x, _) = to_screen_coords(100, 50, 800, 600, (900, 0), WINDOW_MARGIN);
        assert_eq!(x, 100 + 800 - 20, "应钳制到右边界内");

        // 越过左边界（负值）→ 钳制到 min_x
        let (x, _) = to_screen_coords(100, 50, 800, 600, (-900, 0), WINDOW_MARGIN);
        assert_eq!(x, 100 + 20, "应钳制到左边界内");

        // 越过下边界 → 钳制到 max_y
        let (_, y) = to_screen_coords(100, 50, 800, 600, (0, 900), WINDOW_MARGIN);
        assert_eq!(y, 50 + 600 - 20, "应钳制到下边界内");
    }
}
