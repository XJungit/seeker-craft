//! Azalea bot 感知层。
//!
//! 坐标通过 `last_position` 同步读取（handler 每 tick 更新）。
//! 背包/附近玩家等快照由 handler 周期性以 `BotEvent::State` 推送（见 mod.rs）。

use super::AzaleaBot;
use azalea::Vec3;

impl AzaleaBot {
    /// 最近一次已知坐标（handler 每 tick 更新）。None 表示尚未连入。
    pub fn position(&self) -> Option<Vec3> {
        *self.last_position.lock().unwrap()
    }

    /// 当前 tick 数。
    pub fn ticks(&self) -> u32 {
        // 通过 last_position 不可得，预留接口；实际 tick 在事件流中体现。
        0
    }
}
