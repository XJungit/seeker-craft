//! Azalea bot 动作层：移动、挖矿、放置、聊天。
//! 所有方法在 26.2 上已验证（见 examples/azalea_connect.rs）。

use super::{AzaleaBot, BotCommand};

impl AzaleaBot {
    /// 走到目标方块坐标（异步触发，由 handler tick 推进 pathfinder）。
    pub fn goto(&self, x: i32, y: i32, z: i32) {
        self.push_cmd(BotCommand::Goto { x, y, z });
    }

    /// 挖掉指定方块。
    pub fn mine(&self, x: i32, y: i32, z: i32) {
        self.push_cmd(BotCommand::Mine { x, y, z });
    }

    /// 挖掉 bot 脚下方块（最常用：向下挖矿井）。
    pub fn mine_below(&self) {
        self.push_cmd(BotCommand::MineBelow);
    }

    /// 对着指定方块交互（放置方块 / 右键交互）。
    pub fn block_interact(&self, x: i32, y: i32, z: i32) {
        self.push_cmd(BotCommand::BlockInteract { x, y, z });
    }

    /// 发送聊天消息（验证过的发送链路；也用作 LLM 指令回显）。
    pub fn chat(&self, content: &str) {
        self.push_cmd(BotCommand::Chat {
            content: content.to_string(),
        });
    }
}
