//! Set-of-Mark（SoM）：把可交互元素用编号标注到截图上，
//! 让 VLM 输出"点①"而非模糊方位描述。与 game-agent-design.md §4.1 / P1.2 对齐。
//!
//! 分层：
//! - 默认编译（不依赖图像库）：[`parse_mark_id`](编号文本解析)、[`select_mark_id`](VLM 选号)、
//!   [`mc_hotbar_marks`]/[`mc_hud_marks`](MC 规则布局，纯几何)。
//! - `real` 特性（image + imageproc）：[`render::render_marks`](真正叠加编号框)。

use crate::vision::VisionClient;
use anyhow::{Context, Result};
use craft_agent::core::types::{Element, Screenshot};

/// 带圈数字 Unicode（①..⑳，编号 1..=20）映射表，便于把 VLM 的"③"解析回 3。
const CIRCLED: &[char] = &[
    '①', '②', '③', '④', '⑤', '⑥', '⑦', '⑧', '⑨', '⑩', '⑪', '⑫', '⑬', '⑭', '⑮', '⑯', '⑰', '⑱', '⑲',
    '⑳',
];

/// 从 VLM 文本回复里解析出选中的元素编号。
///
/// 兼容两种写法：
/// - 阿拉伯数字（"请点击 3"、"我选2号"）→ 取第一个连续数字序列。
/// - 带圈数字（"③"、"点⑩"）→ 经 [`CIRCLED`] 映射。
///
/// 找不到任何编号时返回 `None`（调用方据此报错或回退）。
pub fn parse_mark_id(resp: &str) -> Option<u32> {
    // 先匹配带圈数字（优先级高于阿拉伯，避免正文里的计数干扰）
    for (i, &c) in CIRCLED.iter().enumerate() {
        if resp.contains(c) {
            return Some((i + 1) as u32);
        }
    }
    // 再匹配普通阿拉伯数字（取第一个连续数字串）
    let digits: String = resp.chars().filter(|c| c.is_ascii_digit()).collect();
    digits.parse::<u32>().ok()
}

/// 把元素编号(1..=20)映射为带圈数字字符，用于拼 `marked_text` 喂给 VLM
/// （与编号图上画的 ①..⑳ 一致）。超出范围用阿拉伯数字兜底。
pub fn element_mark_char(id: u32) -> char {
    if (1..=CIRCLED.len() as u32).contains(&id) {
        CIRCLED[(id - 1) as usize]
    } else {
        std::char::from_digit(id, 10).unwrap_or('?')
    }
}

/// 用 VLM 选号：把编号图 + 指令发给 VLM，解析返回的元素编号。
///
/// 不依赖具体后端（接受 `&dyn VisionClient`），单测可注入 [`MockVisionClient`]。
pub fn select_mark_id(
    client: &dyn VisionClient,
    marked_png: &Screenshot,
    instruction: &str,
) -> Result<u32> {
    let prompt = format!(
        "这是一张 Minecraft 游戏截图，画面上的可交互元素已用数字编号（① ② ③ ...）标出。\
         你要执行的指令是：{instruction}\n\
         请只回答你要点击的元素编号（阿拉伯数字），不要任何解释或标点。"
    );
    let resp = client.chat(marked_png, &prompt)?;
    parse_mark_id(&resp).context("VLM 未返回可解析的元素编号")
}

/// Minecraft 规则布局：底部快捷栏（hotbar）9 格。
///
/// MC 的 hotbar 是固定 UI：屏幕底部 9 个等间距槽位。给定窗口分辨率，
/// 按经验比例算出每格 `[x, y, w, h]`（屏幕坐标系，左上角原点）。
/// 编号 1..=9 与槽位 0..=8 对应（方便后续接 `Inventory.setSelectedSlot`）。
///
/// 这是"规则几何"而非检测模型——Minecraft HUD 布局固定，无需 OmniParser。
pub fn mc_hotbar_marks(screen_w: u32, screen_h: u32) -> Vec<Element> {
    // 经验参数（MC 默认 UI 比例，适用于 16:9 左右窗口）
    let slot = (screen_h as f32 * 0.075).max(20.0) as u32; // 槽位约 7.5% 屏高
    let gap = (slot as f32 * 0.18) as u32; // 槽间距
    let total_w = 9 * slot + 8 * gap; // 假设槽位近正方形
    let x0 = screen_w.saturating_sub(total_w) / 2; // 水平居中（saturating 防溢出，u32 已非负）
    let y0 = screen_h.saturating_sub(slot + (screen_h as f32 * 0.02) as u32); // 距底约 2%
    (0..9)
        .map(|i| {
            let x = x0 + i * (slot + gap);
            Element {
                id: i + 1,
                label: format!("hotbar_{i}"),
                bbox: [x as i32, y0 as i32, slot as i32, slot as i32],
                center: ((x + slot / 2) as i32, (y0 + slot / 2) as i32),
            }
        })
        .collect()
}

/// Minecraft 规则布局：左下角 HUD 大致区域（生命/饥饿/经验）。
///
/// 这些 HUD 元素位置相对固定但尺寸随状态变化，这里给一个宽松的包围盒，
/// 足以让 VLM 区分"点生命心"还是"点经验条"，精确像素由执行层 clamp。
pub fn mc_hud_marks(screen_w: u32, screen_h: u32) -> Vec<Element> {
    let w = (screen_w as f32 * 0.18) as u32; // HUD 占约 18% 屏宽
    let h = (screen_h as f32 * 0.10) as u32; // 占约 10% 屏高
    let x0 = (screen_w as f32 * 0.02) as u32; // 距左 2%
    let y0 = screen_h.saturating_sub(h + (screen_h as f32 * 0.02) as u32); // 距底 2%
    vec![Element {
        id: 10,
        label: "hud_health_hunger_xp".into(),
        bbox: [x0 as i32, y0 as i32, w as i32, h as i32],
        center: ((x0 + w / 2) as i32, (y0 + h / 2) as i32),
    }]
}

#[cfg(feature = "real")]
pub mod render {
    //! SoM 渲染：把 [`Element`] 列表叠加编号标注到 PNG 截图上。
    //! 依赖 image + imageproc（仅 `real` 编译，离线单测用合成图验证）。
    use super::Element;
    use ab_glyph::FontVec;
    use anyhow::Context;
    use image::{DynamicImage, ImageFormat, Rgba};
    use imageproc::drawing::{draw_filled_circle_mut, draw_hollow_rect_mut, draw_text_mut};
    use imageproc::rect::Rect;

    /// 把 `elements` 叠加编号标注到 PNG 截图上，返回新的 PNG 字节。
    ///
    /// 每个元素：半透明青色框 + 左上角橙色实心圆点 + 白色编号数字。
    /// 编号数字用系统字体渲染（见 [`load_font`]）；找不到字体时返回错误。
    pub fn render_marks(png: &[u8], elements: &[Element]) -> anyhow::Result<Vec<u8>> {
        let img = image::load_from_memory_with_format(png, ImageFormat::Png)
            .context("render_marks 解码 PNG 失败（期望 PNG 编码字节）")?;
        let mut rgba = img.to_rgba8();
        let font = load_font().context("未找到可用系统字体，无法渲染编号文字")?;

        for el in elements {
            let [x, y, w, h] = el.bbox;
            let (w, h) = (w as u32, h as u32);
            // 半透明青色框
            draw_hollow_rect_mut(
                &mut rgba,
                Rect::at(x, y).of_size(w, h),
                Rgba([0u8, 200, 255, 160]),
            );
            // 左上角橙色实心圆点（编号底色）
            let (cx, cy) = ((x + 10) as u32, (y + 10) as u32);
            draw_filled_circle_mut(
                &mut rgba,
                (cx as i32, cy as i32),
                9,
                Rgba([255u8, 80, 0, 255]),
            );
            // 白色编号数字
            draw_text_mut(
                &mut rgba,
                Rgba([255u8, 255, 255, 255]),
                (cx as i32).saturating_sub(4),
                (cy as i32).saturating_sub(7),
                14.0f32,
                &font,
                &el.id.to_string(),
            );
        }

        let mut out = Vec::new();
        DynamicImage::ImageRgba8(rgba)
            .write_to(&mut std::io::Cursor::new(&mut out), ImageFormat::Png)
            .context("render_marks 编码 PNG 失败")?;
        Ok(out)
    }

    /// 尝试从常见系统字体路径加载 TrueType 字体（跨平台候选列表）。
    fn load_font() -> Option<FontVec> {
        const CANDIDATES: &[&str] = &[
            "C:/Windows/Fonts/arial.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            "/Library/Fonts/Arial.ttf",
            "/System/Library/Fonts/Supplemental/Arial.ttf",
        ];
        for p in CANDIDATES {
            if let Ok(bytes) = std::fs::read(p)
                && let Ok(font) = FontVec::try_from_vec(bytes)
            {
                return Some(font);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vision::MockVisionClient;

    #[test]
    fn parse_arabic_and_circled() {
        assert_eq!(parse_mark_id("请点击 3"), Some(3));
        assert_eq!(parse_mark_id("我选②号"), Some(2));
        assert_eq!(parse_mark_id("点击⑩"), Some(10));
        assert_eq!(parse_mark_id("这里没有数字"), None);
    }

    #[test]
    fn select_uses_vlm_and_parses_id() {
        // MockVisionClient::chat 返回含 "2"
        let v = MockVisionClient;
        let id = select_mark_id(&v, &vec![0u8; 16], "砍木头").unwrap();
        assert_eq!(id, 2);
    }

    #[test]
    fn hotbar_layout_has_9_marks_near_bottom() {
        let marks = mc_hotbar_marks(1091, 724);
        assert_eq!(marks.len(), 9);
        assert_eq!(marks[0].id, 1);
        assert_eq!(marks[8].id, 9);
        // 所有槽位应在屏幕下半部
        for m in &marks {
            assert!(m.bbox[1] > 500, "hotbar 应在屏幕底部，实测 y={}", m.bbox[1]);
        }
        // 槽位间距应等宽（相邻 x 差一致）
        let step = marks[1].bbox[0] - marks[0].bbox[0];
        assert_eq!(marks[2].bbox[0] - marks[1].bbox[0], step);
    }

    #[test]
    fn hud_layout_single_mark() {
        let marks = mc_hud_marks(1091, 724);
        assert_eq!(marks.len(), 1);
        assert_eq!(marks[0].id, 10);
    }
}

#[cfg(all(test, feature = "real"))]
mod render_tests {
    use super::render::render_marks;
    use super::*;

    fn make_png(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbImage::from_pixel(w, h, image::Rgb([30u8, 30, 30]));
        let mut out = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }

    #[test]
    fn render_overlays_numbers_and_stays_valid_png() {
        let src = make_png(400, 300);
        let els = vec![
            Element {
                id: 1,
                label: "a".into(),
                bbox: [10, 10, 80, 80],
                center: (50, 50),
            },
            Element {
                id: 2,
                label: "b".into(),
                bbox: [200, 150, 80, 80],
                center: (240, 190),
            },
        ];
        let out = render_marks(&src, &els).unwrap();
        assert!(!out.is_empty());
        // 渲染结果仍可被 image 解码（合法 PNG）
        image::load_from_memory_with_format(&out, image::ImageFormat::Png).unwrap();
    }
}
