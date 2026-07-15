//! Minecraft 工具 — 每个工具自己完整执行

#[cfg(feature = "real")]
use base64::{Engine as _, engine::general_purpose::STANDARD};
use craft_agent::core::tool::{GameTool, ToolEffects, ToolResult, ToolUpdateFn};
use craft_agent_model::config::VisionMode;
use craft_agent_model::vision::VisionClient;
#[cfg(feature = "real")]
use craft_agent_model::vision::real::downscale_png;
use enigo::{Keyboard, Mouse};
use serde_json::Value;
use std::cell::Cell;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

// ── Perceive ──

pub struct PerceiveTool {
    vlm: Arc<dyn VisionClient>,
    capture: Box<dyn Fn() -> anyhow::Result<Vec<u8>>>,
    /// 视觉后端模式：vlm=独立视觉模型转文字；multimodal=截图直传决策 LLM 由它看像素。
    mode: VisionMode,
    /// 多模态模式截图最长边（像素）；None=不缩放。
    image_max_side: Option<u32>,
    /// 截图落盘目录（相对路径，如 sessions/mc_run.shots）。Some 时每次 perceive 把截图
    /// 写到 `step-NNN.png`，并在 message 中嵌入相对路径，供 viewer 逐张核对模型看到的是否真实。
    /// None 时不落盘（无 session 的纯内存运行 / 单测）。
    shots_dir: Option<PathBuf>,
    /// 截图序号计数器（内部可变，execute 用 &self）。
    counter: Cell<u32>,
}
impl PerceiveTool {
    pub fn new(
        vlm: Arc<dyn VisionClient>,
        capture: Box<dyn Fn() -> anyhow::Result<Vec<u8>>>,
        mode: VisionMode,
        image_max_side: Option<u32>,
        shots_dir: Option<PathBuf>,
    ) -> Self {
        Self {
            vlm,
            capture,
            mode,
            image_max_side,
            shots_dir,
            counter: Cell::new(0),
        }
    }

    /// 把截图落盘到 `shots_dir/step-NNN.png`，返回相对路径字符串（供 message 嵌入 + viewer 解析）。
    /// 落盘失败只告警不致命，主流程继续。
    fn save_shot(&self, png: &[u8]) -> Option<String> {
        let dir = self.shots_dir.as_ref()?;
        let n = self.counter.get() + 1;
        self.counter.set(n);
        let fname = format!("step-{n:03}.png");
        let rel = dir.join(&fname);
        let rel_str = rel.to_string_lossy().to_string();
        if std::fs::create_dir_all(dir).is_ok() && std::fs::write(&rel, png).is_ok() {
            Some(rel_str)
        } else {
            eprintln!("[warn] perceive 截图落盘失败: {}", rel.display());
            None
        }
    }
}
impl GameTool for PerceiveTool {
    fn name(&self) -> &str {
        "perceive"
    }
    fn description(&self) -> &str {
        match self.mode {
            VisionMode::Vlm => {
                "拍照观察周围。prompt用英文, 问清楚: 前方有什么方块和生物? 树在哪? 石头在哪? 有没有怪物?"
            }
            VisionMode::Multimodal => {
                "拍照观察周围。截图会作为下一条 user 消息的图片直接发给你(请你自己看像素), 直接根据画面回答: 前方有什么方块/生物? 准星对准的是什么? 树/石头/怪物在哪? prompt 可留空或补充具体问题。"
            }
        }
    }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"prompt":{"type":"string","description":"英文提示词, 如: Describe the Minecraft scene. List trees, stones, animals, monsters, water near the crosshair."}}})
    }
    fn effects(&self) -> ToolEffects {
        ToolEffects::network()
    } // 调 VLM, 网络 I/O
    fn execute(
        &self,
        _id: &str,
        args: Value,
        _on_update: Option<ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        let png = (self.capture)()?;
        match self.mode {
            VisionMode::Vlm => {
                // 独立 VLM：截图 → 文字描述 → 文字进历史（决策 LLM 读不到图）。
                let prompt = args["prompt"].as_str().unwrap_or(
                    "Describe the Minecraft scene. List all visible blocks and entities near the crosshair.",
                );
                let desc = self.vlm.chat(&png, prompt)?;
                // 落盘=VLM 实际看到的原始截图，便于事后核对。
                let shot_rel = self.save_shot(&png);
                let message = match shot_rel {
                    Some(p) => format!("{desc}\n\n[截图已落盘 {p}]"),
                    None => desc,
                };
                Ok(ToolResult {
                    message,
                    is_error: false,
                    images: vec![],
                })
            }
            VisionMode::Multimodal => {
                // 多模态 LLM 直读：把截图 base64 内联，作为 images 字段返回，
                // 由 agent 主循环作为下一条 user 角色消息发给决策 LLM（而非挂在
                // tool 角色，兼容性最好），决策 LLM 自己看像素。
                let scaled = match self.image_max_side {
                    Some(ms) => downscale_png(&png, ms)?.0,
                    None => png.clone(),
                };
                let data_uri = format!("data:image/png;base64,{}", STANDARD.encode(&scaled));
                // 落盘=发给 LLM 的缩放后截图（与模型输入一致），便于事后逐张核对。
                let shot_rel = self.save_shot(&scaled);
                let message = match shot_rel {
                    Some(p) => format!(
                        "[截图已附上（已落盘 {p}），请直接看图回答：前方有什么方块/生物？准星对准的是什么？树、石头、怪物分别在哪里？]"
                    ),
                    None => "[截图已附上，请直接看图回答：前方有什么方块/生物？准星对准的是什么？树、石头、怪物分别在哪里？]".into(),
                };
                Ok(ToolResult {
                    message,
                    is_error: false,
                    images: vec![data_uri],
                })
            }
        }
    }
}

// ── Press ──

pub struct PressTool {
    enigo: Rc<RefCell<enigo::Enigo>>,
    focus: Box<dyn Fn()>,
}
impl PressTool {
    pub fn new(enigo: Rc<RefCell<enigo::Enigo>>, focus: Box<dyn Fn()>) -> Self {
        Self { enigo, focus }
    }
}
impl GameTool for PressTool {
    fn name(&self) -> &str {
        "press"
    }
    fn description(&self) -> &str {
        "按下按键。w/a/s/d=前/左/后/右移动(要采集必须 press w 走向目标才能 mine, 站在原地挖不到); space=跳; shift=潜行; e=开关背包/合成界面, 打开会遮挡视野, 除非要合成否则不要按 e, 严禁反复开关键(开→关→开是无效循环); 1-9=切换快捷栏; ctrl=疾跑; q=丢弃; f=切换主副手"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"keys":{"type":"string","description":"按键字母, 如 w, space, e, shift, 1-9"},"ticks":{"type":"integer","description":"持续时间, 20≈1秒, 40≈2秒","default":20}},"required":["keys"]})
    }
    fn effects(&self) -> ToolEffects {
        ToolEffects::write()
    } // 改变游戏状态
    fn execute(
        &self,
        _id: &str,
        args: Value,
        _on_update: Option<ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        (self.focus)();
        let keys = args["keys"].as_str().unwrap_or("w");
        let ticks = args["ticks"].as_u64().unwrap_or(20);
        let ms = ticks * 50;
        // 统一解析为 enigo::Key（未知键名走 Unicode 兜底），用 RAII 守卫保证无论
        // 正常返回还是 panic，按键都会 Release，不会卡在按下状态。
        let key = key_from_str(keys)
            .unwrap_or_else(|| enigo::Key::Unicode(keys.chars().next().unwrap_or('w')));
        let _guard = KeyGuard::hold(self.enigo.clone(), key)
            .map_err(|e| anyhow::anyhow!("press {keys} 失败: {e}"))?;
        std::thread::sleep(std::time::Duration::from_millis(ms));
        Ok(ToolResult {
            message: format!("press {keys} {ms}ms"),
            is_error: false,
            images: vec![],
        })
    }
}

fn key_from_str(s: &str) -> Option<enigo::Key> {
    Some(match s {
        "w" | "W" => enigo::Key::W,
        "a" | "A" => enigo::Key::A,
        "s" | "S" => enigo::Key::S,
        "d" | "D" => enigo::Key::D,
        "space" | "Space" | " " => enigo::Key::Space,
        "shift" | "Shift" => enigo::Key::Shift,
        "ctrl" | "Ctrl" => enigo::Key::Control,
        "e" | "E" => enigo::Key::E,
        "q" | "Q" => enigo::Key::Q,
        "tab" | "Tab" => enigo::Key::Tab,
        "escape" | "Escape" | "esc" => enigo::Key::Escape,
        // digits: enigo 0.6.1 has Key::Unicode('1') but not Key::N1
        _ => return None,
    })
}

// ── Look ──

pub struct LookTool {
    focus: Box<dyn Fn()>,
}
impl LookTool {
    pub fn new(focus: Box<dyn Fn()>) -> Self {
        Self { focus }
    }
}
impl GameTool for LookTool {
    fn name(&self) -> &str {
        "look"
    }
    fn description(&self) -> &str {
        "转动视角。dx>0右转(300≈90度), dy>0低头, dy<0抬头。"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"dx":{"type":"integer"},"dy":{"type":"integer"}},"required":["dx","dy"]})
    }
    fn effects(&self) -> ToolEffects {
        ToolEffects::read()
    } // 只读, 不改动世界
    fn execute(
        &self,
        _id: &str,
        args: Value,
        _on_update: Option<ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        (self.focus)();
        let dx = args["dx"].as_i64().unwrap_or(0) as i32;
        let dy = args["dy"].as_i64().unwrap_or(0) as i32;
        #[cfg(windows)]
        crate::adapter::raw_mouse_rel(dx, dy)?;
        Ok(ToolResult {
            message: format!("look dx={dx} dy={dy}"),
            is_error: false,
            images: vec![],
        })
    }
}

// ── Mine ──

pub struct MineTool {
    enigo: Rc<RefCell<enigo::Enigo>>,
    focus: Box<dyn Fn()>,
}
impl MineTool {
    pub fn new(enigo: Rc<RefCell<enigo::Enigo>>, focus: Box<dyn Fn()>) -> Self {
        Self { enigo, focus }
    }
}
impl GameTool for MineTool {
    fn name(&self) -> &str {
        "mine"
    }
    fn description(&self) -> &str {
        "按住左键挖掘。必须先: ①look 转向目标让准星对准 ②press w 走近直到准星压住树干(原地挖不到空气) ③再 mine。木头=60ticks(3秒), 石头=120ticks(6秒), 矿石=200ticks(10秒)。"
    }
    fn parameters(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"ticks":{"type":"integer","description":"挖掘时长, 20≈1秒","default":60}}})
    }
    fn effects(&self) -> ToolEffects {
        ToolEffects::write()
    } // 改变游戏状态
    fn execute(
        &self,
        _id: &str,
        args: Value,
        _on_update: Option<ToolUpdateFn>,
    ) -> anyhow::Result<ToolResult> {
        (self.focus)();
        let ticks = args["ticks"].as_u64().unwrap_or(60);
        let ms = ticks * 50;
        // RAII 守卫：睡眠期间左键保持按下，返回/panic 时自动 Release。
        let _guard = MouseGuard::hold(self.enigo.clone(), enigo::Button::Left)
            .map_err(|e| anyhow::anyhow!("mine 失败: {e}"))?;
        std::thread::sleep(std::time::Duration::from_millis(ms));
        Ok(ToolResult {
            message: format!("mine {ms}ms"),
            is_error: false,
            images: vec![],
        })
    }
}

// ── 输入释放守卫（RAII）──
//
// enigo 的 Press/Release 必须成对；若工具在 sleep 期间 panic或提前返回，
// 守卫的 Drop 仍会发送 Release，避免按键/鼠标卡在按下状态。

/// 按住一个键盘按键，Drop 时自动 Release。
struct KeyGuard {
    enigo: Rc<RefCell<enigo::Enigo>>,
    key: enigo::Key,
}
impl KeyGuard {
    fn hold(enigo: Rc<RefCell<enigo::Enigo>>, key: enigo::Key) -> anyhow::Result<Self> {
        enigo
            .borrow_mut()
            .key(key, enigo::Direction::Press)
            .map_err(|e| anyhow::anyhow!("按下按键失败: {e}"))?;
        Ok(Self { enigo, key })
    }
}
impl Drop for KeyGuard {
    fn drop(&mut self) {
        let _ = self
            .enigo
            .borrow_mut()
            .key(self.key, enigo::Direction::Release);
    }
}

/// 按住一个鼠标按键，Drop 时自动 Release。
struct MouseGuard {
    enigo: Rc<RefCell<enigo::Enigo>>,
    button: enigo::Button,
}
impl MouseGuard {
    fn hold(enigo: Rc<RefCell<enigo::Enigo>>, button: enigo::Button) -> anyhow::Result<Self> {
        enigo
            .borrow_mut()
            .button(button, enigo::Direction::Press)
            .map_err(|e| anyhow::anyhow!("按下鼠标失败: {e}"))?;
        Ok(Self { enigo, button })
    }
}
impl Drop for MouseGuard {
    fn drop(&mut self) {
        let _ = self
            .enigo
            .borrow_mut()
            .button(self.button, enigo::Direction::Release);
    }
}

// ── 工厂 ──

/// 构建四个真实 Minecraft 工具。
///
/// 每个会发键鼠的工具都持有同一个 `focus` 回调：发输入前把 Minecraft 窗口抢回前台，
/// 否则 enigo 的 SendInput 投不到 MC（终端前台时尤其明显）。focus 失败只告警不致命。
fn make_focus() -> Box<dyn Fn()> {
    Box::new(|| {
        #[cfg(windows)]
        {
            if let Err(e) = crate::adapter::focus_minecraft() {
                eprintln!("[warn] 聚焦 Minecraft 失败: {e}（工具输入可能不生效）");
            }
        }
        #[cfg(not(windows))]
        {
            eprintln!("[warn] 非 Windows 平台，未实现窗口聚焦（工具可能不生效）");
        }
    })
}

pub fn create_mc_tools(
    vlm: Arc<dyn VisionClient>,
    capture: Box<dyn Fn() -> anyhow::Result<Vec<u8>>>,
    enigo: Rc<RefCell<enigo::Enigo>>,
    perceive_mode: VisionMode,
    perceive_image_max_side: Option<u32>,
    shots_dir: Option<PathBuf>,
) -> Vec<Box<dyn GameTool>> {
    vec![
        Box::new(PerceiveTool::new(
            vlm,
            capture,
            perceive_mode,
            perceive_image_max_side,
            shots_dir,
        )),
        Box::new(PressTool::new(enigo.clone(), make_focus())),
        Box::new(LookTool::new(make_focus())),
        Box::new(MineTool::new(enigo, make_focus())),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, ImageFormat, Rgb, RgbImage};
    use std::sync::Arc;

    struct FakeVlm;
    impl VisionClient for FakeVlm {
        fn chat(&self, _s: &Vec<u8>, _p: &str) -> anyhow::Result<String> {
            Ok("fake-vision".into())
        }
    }

    /// 造一张 w×h 的 PNG 当作"截图"（real 特性下 image 可用）。
    fn fake_png(w: u32, h: u32) -> Vec<u8> {
        let img = RgbImage::from_pixel(w, h, Rgb([10u8, 20, 30]));
        let dynimg = DynamicImage::ImageRgb8(img);
        let mut out = Vec::new();
        dynimg
            .write_to(&mut std::io::Cursor::new(&mut out), ImageFormat::Png)
            .unwrap();
        out
    }

    #[test]
    fn multimodal_perceive_returns_inline_image() {
        // 多模态模式：截图应以 base64 data URI 内联返回，且经过最长边缩放。
        let capture = Box::new(|| Ok(fake_png(200, 100)));
        let tool = PerceiveTool::new(
            Arc::new(FakeVlm),
            capture,
            VisionMode::Multimodal,
            Some(128),
            None,
        );
        let res = tool.execute("c1", serde_json::json!({}), None).unwrap();
        assert!(!res.is_error);
        assert_eq!(res.images.len(), 1, "多模态模式应返回 1 张内联截图");
        assert!(
            res.images[0].starts_with("data:image/png;base64,"),
            "图像段必须是 base64 data URI: {}",
            &res.images[0][..40]
        );
        // 200×100 缩到最长边 128 → 128×64，体积应小于原图
        let b64 = res.images[0].trim_start_matches("data:image/png;base64,");
        let decoded = STANDARD.decode(b64).unwrap();
        assert!(decoded.len() < fake_png(200, 100).len(), "缩放后应更小");
    }

    #[test]
    fn vlm_perceive_returns_text_only_and_does_not_inline_image() {
        // vlm 模式：返回文字描述，不应带图像段。
        let capture = Box::new(|| Ok(fake_png(200, 100)));
        let tool = PerceiveTool::new(Arc::new(FakeVlm), capture, VisionMode::Vlm, None, None);
        let res = tool.execute("c1", serde_json::json!({}), None).unwrap();
        assert!(!res.is_error);
        assert!(res.images.is_empty(), "vlm 模式不应返回图像段");
        assert_eq!(res.message, "fake-vision", "vlm 模式应返回 VLM 文字");
    }

    #[test]
    fn create_mc_tools_count_and_perceive_mode_passed() {
        // 工厂应产出 4 个工具，且 perceive 的 mode 透传进 PerceiveTool。
        let capture = Box::new(|| Ok(fake_png(64, 64)));
        let tools = create_mc_tools(
            Arc::new(FakeVlm),
            capture,
            Rc::new(RefCell::new(
                enigo::Enigo::new(&enigo::Settings::default()).unwrap(),
            )),
            VisionMode::Multimodal,
            Some(768),
            None,
        );
        assert_eq!(tools.len(), 4, "应注册 4 个工具");
        assert_eq!(tools[0].name(), "perceive");
    }

    #[test]
    fn multimodal_perceive_saves_shot_to_disk_and_embeds_path() {
        // 给定 shots_dir，多模态模式应把截图落盘为 step-001.png 并在 message 嵌入相对路径。
        let tmp = std::env::temp_dir().join(format!("ca_shot_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let capture = Box::new(|| Ok(fake_png(200, 100)));
        let tool = PerceiveTool::new(
            Arc::new(FakeVlm),
            capture,
            VisionMode::Multimodal,
            Some(128),
            Some(tmp.clone()),
        );
        let res = tool.execute("c1", serde_json::json!({}), None).unwrap();
        let shot = tmp.join("step-001.png");
        assert!(shot.exists(), "截图应落盘到 {}", shot.display());
        let written = std::fs::read(&shot).unwrap();
        assert!(
            written.len() < fake_png(200, 100).len(),
            "落盘应是缩放后的更小图"
        );
        assert!(
            res.message.contains("step-001.png"),
            "message 应嵌入落盘路径: {}",
            res.message
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn vlm_perceive_saves_shot_to_disk_too() {
        // vlm 模式同样落盘（便于事后核对），且不返回图像段。
        let tmp = std::env::temp_dir().join(format!("ca_shot_test_vlm_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let capture = Box::new(|| Ok(fake_png(120, 80)));
        let tool = PerceiveTool::new(
            Arc::new(FakeVlm),
            capture,
            VisionMode::Vlm,
            None,
            Some(tmp.clone()),
        );
        let res = tool.execute("c1", serde_json::json!({}), None).unwrap();
        assert!(res.images.is_empty(), "vlm 模式不应返回图像段");
        assert!(tmp.join("step-001.png").exists(), "vlm 模式也应落盘截图");
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
