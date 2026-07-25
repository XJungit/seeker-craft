//! Craft-Agent：通用游戏 Agent 框架核心（Minecraft 首落，纯视觉路线）
//!
//! 本 crate 定义跨游戏通用的核心抽象：
//! - [`core::types`]：WorldState / Action / Element / Target 等数据结构
//! - [`core::adapter`]：GameAdapter trait（截图 / 感知 / 执行）
//! - [`core::world_model`]：WorldModel trait（预留世界模型接口）
//! - [`agent`]：Agent 主循环骨架
//! - [`adapters::fake`]：离线测试用的假适配器（不依赖任何游戏/显示）

pub mod adapters;
pub mod agent;
pub mod core;
pub mod profile;
