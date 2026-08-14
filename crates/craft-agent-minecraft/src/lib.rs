//! Craft-Agent Minecraft 适配器（Azalea 客户端路线）。
//!
//! 这是一个**独立 crate**，依赖核心抽象 [`craft-agent`](craft_agent)（GameAdapter /
//! WorldState / Action）。
//! 这样核心与 LLM 决策解耦，符合"换游戏 = 换 Adapter crate"的通用框架设计——
//! 后续加 `craft-agent-browser` 等不会污染核心。bot 的 LLM 决策由 DSH（Cordis 创造模式）经 viewer 桥驱动。
//!
//! 唯一路线：Azalea 客户端协议层（Rust 全栈 bot 连入普通 MC 服务器，MC 26.2 原生支持）。
//! 旧 Fabric mod TCP 桥接（mod-bridge）与真机 VLM 键鼠（real）路线已从源码删除。

#[cfg(feature = "azalea-bot")]
pub mod adapter_azalea;
/// Azalea 客户端协议层适配器：Rust 全栈 bot 连入 MC 服务器，
/// 原生支持 26.2，替代原 Fabric mod TCP 桥。仅 `azalea-bot` 特性编译。
#[cfg(feature = "azalea-bot")]
pub mod azalea;
#[cfg(feature = "azalea-bot")]
pub mod tools_azalea;

/// 蓝图系统：预定义可复用建筑模板（P2-1）。无 azalea 依赖，纯数据结构 + JSON。
pub mod blueprint;

/// LLM 自定义动作库（P2-4：newAction 等价物）。无 azalea 依赖，纯数据结构 + JSON。
pub mod action_lib;
