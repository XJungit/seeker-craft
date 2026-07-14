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
use craft_agent::core::types::{Action, Element, ExecResult, Screenshot, Target, WorldState};
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
    /// 最近一次 perceive 检测到的 3D 目标（execute 的 AimAndMine 据此对准）。
    targets: RefCell<Vec<Target>>,
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
            targets: RefCell::new(Vec::new()),
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

/// 绕过 enigo 的绝对定位转换，直接用 SendInput + MOUSEEVENTF_MOVE（无 ABSOLUTE flag）
/// 发送真正的相对鼠标位移，触发 MC 独占全屏下的 raw input 视角旋转。
///
/// 根因：MC 独占全屏时鼠标被锁定在画面中心，enigo 的 `move_mouse(dx, dy, Coordinate::Rel)`
/// 内部把当前光标+deltas→算出绝对坐标→`SetCursorPos`/`MOUSEEVENTF_ABSOLUTE` 发送，
/// 这只会把光标钉回中心、不产生视角差。而 SendInput 裸 `MOUSEEVENTF_MOVE`（无 ABSOLUTE）
/// 触发 MC raw input 的路由，这才是 AutoHotkey / Java Robot 能在 MC 中转视角的机制。
#[cfg(windows)]
fn raw_mouse_rel(dx: i32, dy: i32) -> Result<()> {
    use std::mem::size_of;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_MOVE, MOUSEINPUT, SendInput,
    };

    let input = INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                mouseData: 0,
                dwFlags: MOUSEEVENTF_MOVE,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    unsafe {
        SendInput(1, &input as *const INPUT, size_of::<INPUT>() as i32);
    }
    Ok(())
}

/// 尝试把 Minecraft 窗口抢回【前台】，使 enigo 的 SendInput 能投到 MC 消息队列。
///
/// 根因：从终端 `cargo run` 时终端是前台窗口，MC（即便独占全屏）被系统挂起 / 收不到输入；
/// enigo 合成的鼠标键盘事件只投到【前台窗口】，故 MC 视角/按键无响应。
/// 绕过 Windows 前台锁的标准做法：`AttachThreadInput` 挂到当前前台线程后 `SetForegroundWindow`
/// + `SetFocus`（独占全屏被挂起时先 `ShowWindow(SW_RESTORE)` 唤醒）。
#[cfg(windows)]
fn focus_minecraft() -> Result<()> {
    use windows_sys::Win32::Foundation::{HWND, LPARAM};
    use windows_sys::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetForegroundWindow, GetWindowTextLengthW, GetWindowTextW,
        GetWindowThreadProcessId, IsIconic, SW_RESTORE, SetForegroundWindow, ShowWindow,
    };
    use windows_sys::core::BOOL;

    unsafe extern "system" fn enum_cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
        unsafe {
            let found = &mut *(lparam as *mut HWND);
            let len = GetWindowTextLengthW(hwnd);
            if len > 0 {
                let mut buf = vec![0u16; (len + 1) as usize];
                let n = GetWindowTextW(hwnd, buf.as_mut_ptr(), len + 1);
                if n > 0 {
                    let title = String::from_utf16_lossy(&buf[..n as usize]).to_lowercase();
                    if title.contains("minecraft") {
                        *found = hwnd;
                        return 0i32; // 找到即停止枚举（非零继续）
                    }
                }
            }
            1i32
        }
    }

    unsafe {
        let mut found: HWND = std::ptr::null_mut();
        EnumWindows(Some(enum_cb), &mut found as *mut _ as LPARAM);
        if found.is_null() {
            return Err(anyhow!("未找到标题含 'minecraft' 的窗口，无法置前台"));
        }
        let fg = GetForegroundWindow();
        let mut fg_pid = 0u32;
        let fg_thread = GetWindowThreadProcessId(fg, &mut fg_pid);
        let mut _mc_pid = 0u32;
        let _mc_thread = GetWindowThreadProcessId(found, &mut _mc_pid);
        let my_thread = GetCurrentThreadId();
        // 挂到当前前台线程，绕过 Windows 前台锁；失败也不致命（仅置前台可能无效）。
        let _ = AttachThreadInput(my_thread, fg_thread, 1i32);
        if IsIconic(found) != 0 {
            ShowWindow(found, SW_RESTORE);
        }
        SetForegroundWindow(found);
        SetFocus(found);
        let _ = AttachThreadInput(my_thread, fg_thread, 0i32);
        eprintln!("[focus] MC窗口已置前台");
        Ok(())
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
        // 全屏模式：独占全屏后台会被系统挂起 → 截图可能是黑帧/旧帧；先抢回前台拿实时画面。
        // 窗口化 MC 后台仍正常渲染，无需抢焦点（避免只读测试时无故夺走终端前台）。
        #[cfg(windows)]
        if self.fullscreen
            && let Err(e) = focus_minecraft()
        {
            eprintln!("[warn] 置 MC 前台失败：{e}（截图可能非实时）");
        }
        let png = self.capture()?;
        let (_wx, _wy, ww, wh) = self
            .rect
            .borrow()
            .ok_or_else(|| anyhow!("capture 未成功获取窗口尺寸"))?;

        // 规则布局：hotbar(9 槽) + hud(1 区)，纯几何无检测模型
        let mut elements = mc_hotbar_marks(ww, wh);
        elements.extend(mc_hud_marks(ww, wh));

        // 叠加编号渲染 → 编号图喂给 VLM（让 VLM 说"点③"而非模糊方位）
        let _marked_png = render_marks(&png, &elements).context("SoM 渲染编号失败（需系统字体）")?;
        let _elements_str: String = elements
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

        // 合并场景描述 + 目标检测为一次 VLM 调用
        let combined_prompt = format!(
            "列出3D物体坐标。每行: label: (x,y)。x范围0~{ww}, y范围0~{wh}。\n\
             可识别的物体: tree stone water animal ore grass dirt。\n\
             不要UI/HUD/hotbar。不要解释。标注: {}",
            _elements_str
        );
        let reply = self.vision.chat(&png, &combined_prompt)
            .context("VLM 场景描述失败")?;

        // 从回复中提取场景描述（坐标部分之前的内容）
        let scene_desc = reply.clone();
        // 解析目标检测坐标
        let targets = parse_vlm_targets(&reply, ww, wh).unwrap_or_default();

        self.elements.replace(elements.clone());
        self.targets.replace(targets.clone());
        Ok(WorldState {
            scene_desc,
            marked_elements: elements,
            detected_targets: targets,
            self_hint: String::new(),
            screenshot: png,
        })
    }

    fn execute(&mut self, action: Action) -> Result<ExecResult> {
        // 关键：发输入前把 MC 抢回前台。否则（终端前台的背景下）enigo 的 SendInput 投不到 MC，
        // 视角/按键全无响应。独占全屏被挂起时 focus_minecraft 内部会 ShowWindow(SW_RESTORE) 唤醒。
        #[cfg(windows)]
        if let Err(e) = focus_minecraft() {
            eprintln!("[warn] 置 MC 前台失败：{e}（enigo 输入可能不生效）");
        }
        // 给 MC 一点时间接管网消息泵（尤其从挂起唤醒后），再发输入。
        #[cfg(windows)]
        thread::sleep(Duration::from_millis(60));
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
                if !self.fullscreen {
                    let cx = wx + (ww / 2) as i32;
                    let cy = wy + (wh / 2) as i32;
                    self.enigo.move_mouse(cx, cy, Coordinate::Abs)?;
                    thread::sleep(Duration::from_millis(100));
                    // 按住再松开, 确保 MC 注册点击
                    self.enigo.button(Button::Left, Direction::Press)?;
                    thread::sleep(Duration::from_millis(100));
                    self.enigo.button(Button::Left, Direction::Release)?;
                    thread::sleep(Duration::from_millis(200));
                }
                #[cfg(windows)]
                raw_mouse_rel(dx, dy)?;
                #[cfg(not(windows))]
                self.enigo.move_mouse(dx, dy, Coordinate::Rel)?;
                Ok(ExecResult {
                    ok: true,
                    detail: format!("look dx={dx} dy={dy}"),
                })
            }
            Action::Move { dir, ticks } => {
                // 发键前务必让 MC 获得焦点
                #[cfg(windows)]
                if let Err(e) = focus_minecraft() {
                    eprintln!("[warn] Move前 focus_minecraft 失败: {e}");
                }
                thread::sleep(Duration::from_millis(100));
                let key = dir_to_key(dir);
                eprintln!("[move] 按下 {:?} {}ms", key, (ticks as u64).saturating_mul(STEP_MS));
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
                // 模糊匹配：从 "water offset=(-328,-153)" 中提取 "water"
                let clean_target = target
                    .split_whitespace()
                    .next()
                    .unwrap_or(&target)
                    .to_lowercase();
                let t = self
                    .targets
                    .borrow()
                    .iter()
                    .find(|t| t.label.to_lowercase().contains(&clean_target))
                    .cloned();
                match t {
                    Some(t) => {
                        let (dx, dy) = t.offset_from_crosshair;
                        if dx != 0 || dy != 0 {
                            // 窗口化 MC: 点击窗口中心激活鼠标捕获
                            // MC 需要足够的响应时间 (~300ms) 才能可靠捕获鼠标
                            if !self.fullscreen {
                                let rect = self.rect.borrow();
                                if let Some((wx, wy, ww, wh)) = *rect {
                                    let cx = wx + (ww / 2) as i32;
                                    let cy = wy + (wh / 2) as i32;
                                    // 移光标到窗口中心
                                    self.enigo.move_mouse(cx, cy, Coordinate::Abs)?;
                                    thread::sleep(Duration::from_millis(100));
                                    // 按住左键 100ms 再松开 (确保 MC 注册点击事件)
                                    self.enigo.button(Button::Left, Direction::Press)?;
                                    thread::sleep(Duration::from_millis(100));
                                    self.enigo.button(Button::Left, Direction::Release)?;
                                    // 等待 MC 捕获鼠标
                                    thread::sleep(Duration::from_millis(200));
                                }
                            }
                            // 灵敏度缩放: 像素偏移 ≠ 鼠标delta, 1px ≈ 0.15°
                            let sdx = (dx as f32 * 0.5) as i32;
                            let sdy = (dy as f32 * 0.5) as i32;
                            eprintln!("[aim] raw_mouse_rel({sdx}, {sdy}) ← ({dx},{dy}) → {t_label}", t_label = t.label, dy = dy, dx = dx);
                            #[cfg(windows)]
                            raw_mouse_rel(sdx, sdy)?;
                            #[cfg(not(windows))]
                            self.enigo.move_mouse(dx, dy, Coordinate::Rel)?;
                            thread::sleep(Duration::from_millis(150));
                        }
                        // 挖矿
                        self.enigo.button(Button::Left, Direction::Press)?;
                        thread::sleep(Duration::from_millis(MINE_MS * 10));
                        self.enigo.button(Button::Left, Direction::Release)?;
                        Ok(ExecResult {
                            ok: true,
                            detail: format!(
                                "瞄准 {} (offset={},{}) 并挖掘了2秒。目标方块可能已被破坏。",
                                t.label, dx, dy
                            ),
                        })
                    }
                    None => {
                        self.enigo.button(Button::Left, Direction::Press)?;
                        thread::sleep(Duration::from_millis(MINE_MS * 10));
                        self.enigo.button(Button::Left, Direction::Release)?;
                        Ok(ExecResult {
                            ok: true,
                            detail: format!("mine (no target '{clean_target}', blind)"),
                        })
                    }
                }
            }
        }
    }
}

/// 从 VLM 回复中解析目标坐标
fn parse_vlm_targets(reply: &str, screen_w: u32, screen_h: u32) -> Result<Vec<Target>> {
    let mut targets = Vec::new();
    let re = regex::Regex::new(r"(\S+?):\s*.*?\((\d+),\s*(\d+)\)")
        .context("编译 VLM 检测正则失败")?;
    let (screen_cx, screen_cy) = (screen_w as f32 / 2.0, screen_h as f32 / 2.0);

    for cap in re.captures_iter(reply) {
        let raw_label = cap[1].trim().to_string();
        // 处理 "左侧大石头：stone" → 取英文部分，或全取
        let label = raw_label
            .rsplit(&['：', ':'][..])
            .next()
            .unwrap_or(&raw_label)
            .trim()
            .trim_matches('*')
            .trim_matches('#')
            .to_lowercase();
        let cx: i32 = cap[2].parse().unwrap_or(0);
        let cy: i32 = cap[3].parse().unwrap_or(0);
        // 钳制到屏幕范围内（VLM 坐标估算不精确, 不浪费有效检测）
        let cx = cx.max(0).min(screen_w as i32 - 1);
        let cy = cy.max(0).min(screen_h as i32 - 1);
        // debug: 打印所有匹配
        eprintln!("[parse] raw={raw_label:?} → label={label:?} ({cx},{cy})");
        // 过滤 UI 元素
        if label.starts_with("hotbar") || label.starts_with("hud")
            || label.is_empty() || label.len() > 30
        {
            eprintln!("[parse] ^ 过滤(UI)");
            continue;
        }
        if cx == 0 && cy == 0 {
            eprintln!("[parse] ^ 过滤(0,0)");
            continue;
        }
        let half = 20i32;
        targets.push(Target {
            label,
            bbox: [cx - half, cy - half, half * 2, half * 2],
            offset_from_crosshair: (cx - screen_cx as i32, cy - screen_cy as i32),
        });
    }
    Ok(targets)
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
