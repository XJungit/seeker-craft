//! Craft-Agent：通用游戏 Agent 框架核心（Minecraft 首落，Azalea 客户端协议路线）
//!
//! 本 crate 定义跨游戏通用的核心抽象：
//! - [`core::types`]：WorldState / Action / MinecraftAction / ExecResult 等数据结构
//! - [`core::adapter`]：GameAdapter trait（感知 / 执行；截图接口保留，azalea 路线返回空占位）
//! - [`core::memory`]：WorldMemory 空间记忆（区块索引 + 命名锚点 + TTL 遗忘）
//! - [`core::semantic_memory`]：跨会话语义记忆（strategy/fact/insight/preference）
//! - [`core::session`]：会话持久化（JSONL + 滚动归档 + 恢复）
//! - [`core::tool`]：GameTool trait / ToolRegistry / 工具效果分组（READ 并行、WRITE 串行）
//! - [`core::skill`]：技能抽取与检索
//! - [`core::world_model`]：WorldModel trait（预留世界模型接口）
//! - [`agent`]：Agent 主循环（run_one_turn：压缩 / 自动感知 / 模式 / 工具批量执行 / 死循环检测）
//! - [`profile`]：3 层 prompt 合并（_default → defaults/{mode} → {individual}）
//! - [`task`]：结构化任务系统（InventoryHas / AtPosition / Killed 等完成条件）
//!
//! 运行时适配器在 `craft-agent-minecraft` crate：Azalea 客户端协议层
//! （结构化状态感知，非截图+VLM 视觉路线）。视觉截图仅作为可选 VLM 补充。

pub mod adapters;
pub mod agent;
pub mod core;
pub mod profile;
pub mod task;
