//! Minecraft 真机适配器（仅 `real` 特性编译）。
//!
//! 实现 `craft_agent::core::adapter::GameAdapter`：
//! - [`capture`]：默认 xcap 直接捕获 MC 窗口（方法 A，窗口化）；全屏模式改用方法 C
//!   捕获主显示器整屏（独占全屏 D3D 下按窗口标题捕获会截黑帧）。
//! - [`perceive`]：hotbar(9) + hud(1) 规则布局 → SoM 编号渲染 → VLM 场景描述，
//!   产出统一 `WorldState`（含 `marked_elements` 供决策层 Click 查表）。
//! - [`execute`]：Click 绝对定位（坐标钳制）/ Look 相对移动转视角 / Move 按键保持 /
//!   AimAndMine 长按挖矿。**不发送 ESC**（ESC 会开暂停菜单，属保留约束）。
//!
//! **为何推荐全屏（P1.5 默认）**：MC 窗口化仅在自己是【前台窗口】且鼠标被捕获时才读输入，
//! 失去焦点即暂停/不收键鼠；全屏则始终前台、不暂停、指针被游戏独占捕获（raw input），
//! 视角转动自然可用，彻底消灭焦点/暂停纠缠。全屏下 MC 窗口==主显示器，局部坐标即屏幕坐标。

use crate::WINDOW_MARGIN;
use crate::to_screen_coords;
use anyhow::{Context, Result, anyhow};
use craft_agent::core::adapter::GameAdapter;
use craft_agent::core::types::{Action, Element, ExecResult, Screenshot, WorldState};
use craft_agent_model::som::render::render_marks;
use craft_agent_model::som::{mc_hotbar_marks, mc_hud_marks};
use craft_agent_model::vision::VisionClient;
use enigo::{Button, Coordinate, Direction, Enigo, Key, Keyboard, Mouse, Settings};
use std::cell::RefCell;
use std::thread;
use std::time::Duration;

/// 每个 Move tick 的按键保持时长（毫秒）。MC 移动靠"按住 W"累积位移。
const STEP_MS: u64 = 50;
/// AimAndMine 每 tick 的挖矿保持时长（毫秒）。MC 挖矿靠"按住左键"。
const MINE_MS: u64 = 200;

/// Minecraft 真机适配器。
pub struct MinecraftAdapter {
    vision: Box<dyn VisionClient>,
    enigo: Enigo,
    /// 全屏模式：capture 改用方法 C（主显示器整屏），无需按窗口标题查找。
    fullscreen: bool,
    /// capture/perceive 是 `&self`，但需缓存窗口信息供 execute(`&mut self`) 使用 → 内部可变性。
    /// `(wx, wy, ww, wh)`：窗口左上角物理像素坐标与尺寸。全屏时为 (0,0,mw,mh)。
    rect: RefCell<Option<(i32, i32, u32, u32)>>,
    /// 最近一次 perceive 得到的标记元素表（execute 的 Click 据此查坐标）。
    elements: RefCell<Vec<Element>>,
}

impl MinecraftAdapter {
    /// 构造【窗口化】适配器。注入视觉客户端（mock 或真实 VLM，便于离线单测注入 mock）。
    ///
    /// 会开启进程级 DPI 感知（enigo 鼠标 / xcap 窗口坐标同源走物理像素），
    /// 并创建 enigo（需 Windows 显示会话，否则报错）。
    pub fn new(vision: Box<dyn VisionClient>) -> Result<Self> {
        let _ = enigo::set_dpi_awareness();
        let enigo =
            Enigo::new(&Settings::default()).context("创建 enigo 失败（需 Windows 显示会话）")?;
        Ok(Self {
            vision,
            enigo,
            fullscreen: false,
            rect: RefCell::new(None),
            elements: RefCell::new(Vec::new()),
        })
    }

    /// 构造【全屏】适配器。capture 走方法 C（主显示器整屏），适配 MC 独占全屏，
    /// 消除窗口化的焦点/暂停/指针捕获问题（P1.5 端到端闭环推荐）。
    pub fn new_fullscreen(vision: Box<dyn VisionClient>) -> Result<Self> {
        let mut a = Self::new(vision)?;
        a.fullscreen = true;
        Ok(a)
    }

    /// 查找标题含 "minecraft" 的窗口（不区分大小写）。仅窗口化捕获使用。
    fn find_mc_window(&self) -> Result<xcap::Window> {
        let windows = xcap::Window::all().context("枚举窗口失败")?;
        for w in windows {
            let title = w.title().context("读取窗口标题失败")?;
            if title.to_lowercase().contains("minecraft") {
                return Ok(w);
            }
        }
        Err(anyhow!(
            "未找到标题含 'Minecraft' 的窗口，请先打开 MC（窗口化、固定分辨率）"
        ))
    }

    /// 全屏捕获（方法 C）：直接捕获主显示器整屏。
    ///
    /// 全屏下 MC 独占 D3D，按窗口标题捕获（方法 A）可能截黑帧；主显示器整屏则稳定。
    /// 全屏时 MC 窗口即主显示器，故缓存 rect=(0,0,mw,mh)，局部坐标==屏幕坐标。
    fn capture_fullscreen(&self) -> Result<Screenshot> {
        let monitors = xcap::Monitor::all().context("枚举显示器失败")?;
        let mon = monitors
            .into_iter()
            .next()
            .context("未找到主显示器（方法 C）")?;
        let img = mon
            .capture_image()
            .context("xcap 捕获主显示器失败（方法 C）")?;
        let mw = img.width();
        let mh = img.height();
        self.rect.replace(Some((0, 0, mw, mh)));
        let mut png = Vec::new();
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .context("编码截图 PNG 失败")?;
        Ok(png)
    }
}

impl GameAdapter for MinecraftAdapter {
    fn capture(&self) -> Result<Screenshot> {
        let _ = enigo::set_dpi_awareness();
        if self.fullscreen {
            return self.capture_fullscreen();
        }
        let w = self.find_mc_window()?;
        let (wx, wy, ww, wh) = (w.x()?, w.y()?, w.width()?, w.height()?);
        // 缓存窗口信息（供 execute 坐标换算/钳制）
        self.rect.replace(Some((wx, wy, ww, wh)));

        // 方法 A：xcap 直捕窗口自身帧缓冲（遮挡免疫），编码为 PNG 供 VLM/SoM 消费。
        let img = w.capture_image().context("xcap 捕获窗口失败（方法 A）")?;
        let mut png = Vec::new();
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .context("编码截图 PNG 失败")?;
        Ok(png)
    }

    fn perceive(&self) -> Result<WorldState> {
        let png = self.capture()?;
        let (_wx, _wy, ww, wh) = self
            .rect
            .borrow()
            .ok_or_else(|| anyhow!("capture 未成功获取窗口尺寸"))?;

        // 规则布局：hotbar(9 槽) + hud(1 区)，纯几何无检测模型
        let mut elements = mc_hotbar_marks(ww, wh);
        elements.extend(mc_hud_marks(ww, wh));

        // 叠加编号渲染 → 编号图喂给 VLM（让 VLM 说"点③"而非模糊方位）
        let marked_png = render_marks(&png, &elements).context("SoM 渲染编号失败（需系统字体）")?;
        let marked_text: String = elements
            .iter()
            .map(|e| {
                format!(
                    "{} {}",
                    craft_agent_model::som::element_mark_char(e.id),
                    e.label
                )
            })
            .collect::<Vec<_>>()
            .join("  ");

        let scene_desc = self
            .vision
            .describe(&marked_png, &marked_text)
            .context("VLM 场景描述失败")?;

        self.elements.replace(elements.clone());
        Ok(WorldState {
            scene_desc,
            marked_elements: elements,
            detected_targets: vec![], // 3D 目标检测留待 P2
            self_hint: String::new(),
            screenshot: png,
        })
    }

    fn execute(&mut self, action: Action) -> Result<ExecResult> {
        let (wx, wy, ww, wh) = self
            .rect
            .borrow()
            .ok_or_else(|| anyhow!("请先 capture/perceive 获取窗口尺寸再执行动作"))?;

        match action {
            Action::Click { element_id } => {
                let els = self.elements.borrow();
                let el = els.iter().find(|e| e.id == element_id).ok_or_else(|| {
                    anyhow!("未知元素 id={element_id}，请先 perceive 获取 marked_elements")
                })?;
                // 局部中心 → 屏幕绝对坐标，钳制在窗口内留 margin
                let (sx, sy) = to_screen_coords(wx, wy, ww, wh, el.center, WINDOW_MARGIN);
                self.enigo.move_mouse(sx, sy, Coordinate::Abs)?;
                thread::sleep(Duration::from_millis(30));
                self.enigo.button(Button::Left, Direction::Click)?;
                Ok(ExecResult {
                    ok: true,
                    detail: format!("click element {element_id} @ ({sx},{sy})"),
                })
            }
            Action::Look { dx, dy } => {
                // 相对鼠标移动转动 MC 视角（raw mouse input）
                self.enigo.move_mouse(dx, dy, Coordinate::Rel)?;
                Ok(ExecResult {
                    ok: true,
                    detail: format!("look dx={dx} dy={dy}"),
                })
            }
            Action::Move { dir, ticks } => {
                let key = dir_to_key(dir);
                self.enigo.key(key, Direction::Press)?;
                thread::sleep(Duration::from_millis(
                    (ticks as u64).saturating_mul(STEP_MS),
                ));
                self.enigo.key(key, Direction::Release)?;
                Ok(ExecResult {
                    ok: true,
                    detail: format!("move {dir:?} x{ticks}"),
                })
            }
            Action::AimAndMine { target } => {
                // P1.4：3D 目标检测未接入，准星指向即挖；detected_targets 有匹配时将来先 Look 对准。
                // 明确不发送 ESC（ESC 会开暂停菜单，属保留约束）。挖矿时长用固定值（本动作无 ticks 字段）。
                self.enigo.button(Button::Left, Direction::Press)?;
                thread::sleep(Duration::from_millis(MINE_MS * 10));
                self.enigo.button(Button::Left, Direction::Release)?;
                Ok(ExecResult {
                    ok: true,
                    detail: format!("mine '{target}' (3D aim in P2)"),
                })
            }
        }
    }
}

/// 移动方向 → enigo 按键。
fn dir_to_key(dir: craft_agent::core::types::Direction) -> Key {
    use craft_agent::core::types::Direction::*;
    match dir {
        Forward => Key::W,
        Back => Key::S,
        Left => Key::A,
        Right => Key::D,
        Up => Key::Space,   // 跳跃
        Down => Key::Shift, // 潜行
    }
}
