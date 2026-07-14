//! 离线测试的假适配器：不依赖任何游戏/显示，返回固定 WorldState。
//!
//! 用于在无 Minecraft 环境下验证主循环、类型系统与 trait 设计。
//! 真实实现见后续 `MinecraftAdapter`（xcap + ort + VLM + enigo）。

use crate::core::adapter::GameAdapter;
use crate::core::types::{Action, Element, ExecResult, Screenshot, Target, WorldState};
use anyhow::Result;

#[derive(Clone)]
pub struct FakeGameAdapter;

impl GameAdapter for FakeGameAdapter {
    fn capture(&self) -> Result<Screenshot> {
        Ok(vec![0u8; 4 * 640 * 360])
    }

    fn perceive(&self) -> Result<WorldState> {
        Ok(WorldState {
            scene_desc: "fake scene: an oak tree is ahead, crafting table nearby".into(),
            marked_elements: vec![Element {
                id: 1,
                label: "crafting_table".into(),
                bbox: [100, 100, 50, 50],
                center: (125, 125),
            }],
            detected_targets: vec![Target {
                label: "oak_tree".into(),
                bbox: [300, 200, 80, 120],
                offset_from_crosshair: (40, -10),
            }],
            self_hint: "health=20 hunger=18".into(),
            screenshot: vec![0u8; 4 * 640 * 360],
        })
    }

    fn perceive_with_prompt(&self, prompt: &str) -> Result<String> {
        Ok(format!("fake vlm reply for: {:.50}...", prompt))
    }

    fn execute(&mut self, action: Action) -> Result<ExecResult> {
        Ok(ExecResult {
            ok: true,
            detail: format!("fake executed: {:?}", action),
        })
    }
}
