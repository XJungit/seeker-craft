//! 核心世界状态与动作定义（与 game-agent-design.md §4 对齐）

use std::sync::Arc;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// 截图像素（RGBA 原始字节）。Arc 使 WorldState 克隆零成本（O(1) refcount 而不拷贝 MB 级数据）。
pub type Screenshot = Arc<Vec<u8>>;

pub(crate) mod screenshot_serde {
    use super::*;
    pub fn serialize<S: Serializer>(v: &Arc<Vec<u8>>, s: S) -> Result<S::Ok, S::Error> {
        v.as_ref().serialize(s)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Arc<Vec<u8>>, D::Error> {
        Vec::<u8>::deserialize(d).map(Arc::new)
    }
}

/// 可交互元素（2D 界面，如背包/合成界面按钮）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Element {
    pub id: u32,
    pub label: String,
    /// `[x, y, w, h]`，以游戏窗口左上角为基准
    pub bbox: [i32; 4],
    pub center: (i32, i32),
}

/// 3D 世界中的检测目标（如树、矿石）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Target {
    pub label: String,
    pub bbox: [i32; 4],
    /// 目标中心相对屏幕准星的偏移 `(dx, dy)`
    pub offset_from_crosshair: (i32, i32),
}

/// 统一世界状态：感知层输出，决策层输入
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldState {
    /// VLM 场景描述（"前方 3 格有橡木树，背包有 2 木头…"）
    pub scene_desc: String,
    /// 标记元素表（可点编号）
    pub marked_elements: Vec<Element>,
    /// 3D 目标检测结果
    pub detected_targets: Vec<Target>,
    /// 血量/饥饿/背包等 HUD 视觉读取
    pub self_hint: String,
    /// 原始截图（RGBA）
    #[serde(with = "screenshot_serde")]
    pub screenshot: Screenshot,
}

/// 移动方向（WASD 映射）
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum Direction {
    Forward,
    Back,
    Left,
    Right,
    Up,
    Down,
}

/// 抽象动作：决策层输出
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Action {
    /// 点击带编号的可交互元素
    Click { element_id: u32 },
    /// 对准并挖掘某目标 (已废弃, 用 look+mining 替代)
    AimAndMine { target: String },
    /// WASD 移动若干 tick
    Move { dir: Direction, ticks: u32 },
    /// 转视角（相对移动）
    Look { dx: i32, dy: i32 },
    /// 按下任意按键
    Press { keys: String, ticks: u32 },
    /// 原地挖掘
    Mine { ticks: u32 },
}

/// 动作执行结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecResult {
    pub ok: bool,
    pub detail: String,
}
