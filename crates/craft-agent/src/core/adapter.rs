//! 游戏适配器抽象（通用性核心，与 game-agent-design.md §4.5 对齐）

use crate::core::types::{Action, ExecResult, Screenshot, WorldState};
use anyhow::Result;

/// 游戏适配器：屏蔽"怎么截图、怎么理解、怎么操作"。
///
/// 换游戏 = 换 Adapter 实现，内核（记忆/规划/决策/反思）零改动。
/// - `MinecraftAdapter`：xcap 截图 + ort 检测 + VLM API + enigo 键鼠
/// - `BrowserAdapter`：headless 截图 + DOM（网页游戏，后续）
/// - `DesktopAdapter`：xcap + enigo（其他桌面游戏）
pub trait GameAdapter {
    /// 截图（返回 RGBA 原始字节）
    fn capture(&self) -> Result<Screenshot>;

    /// 截图 → 检测 + VLM → 统一世界状态
    fn perceive(&self) -> Result<WorldState>;

    /// 执行抽象动作（键鼠）
    fn execute(&mut self, action: Action) -> Result<ExecResult>;
}
