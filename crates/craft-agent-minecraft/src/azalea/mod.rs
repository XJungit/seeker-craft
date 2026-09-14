//! Azalea 客户端协议层适配器（Phase 3）。
//!
//! 用 Azalea（Rust 全栈 Minecraft bot，原生支持 26.2）替代原 Fabric mod TCP 桥。
//! 通过客户端协议连入普通 MC 服务器（含局域网），由 LLM 驱动 bot 执行动作。
//!
//! 设计要点：
//! - Azalea 的 `Client` 仅在 handler 闭包内可用，外部无法持有。因此采用
//!   **命令队列**模式：`AzaleaBot` 把动作指令 push 进共享队列，handler 每 tick
//!   从队列 drain 并执行（用闭包内的 `bot`）。
//! - handler 是 `fn` 指针（azalea 要求不捕获），故队列/事件通道挂在
//!   自定义 `BotState`（Arc<Mutex<...>>，实现 Component + Default + Clone）上。
//! - 所有动作在 26.2 上已逐一验证（见 examples/azalea_connect.rs Phase 2 POC）。

pub mod action_manager;
pub mod auto_craft;
pub mod bot;
pub mod chest;
pub mod commands;
pub mod craft;
pub mod ext_state;
pub mod gather;
pub mod handler;
pub mod harvest;
pub mod inventory;
pub mod place;
pub mod recipe_book;
pub mod recipes;
pub mod scan;
pub mod sleep;
pub mod smart_actions;
pub mod table_flow;
pub mod till;
pub mod trade;

pub use action_manager::{ActionManager, Priority, SubmitOutcome, cmd_signature, timeout_ticks};
pub use bot::AzaleaBot;
pub use commands::{BotCommand, QueuedCommand, parse_chat_command};
pub use handler::BotState;
pub use inventory::{
    auto_equip_best_axe, auto_equip_best_pickaxe, block_drops_item, block_required_pickaxe_tier,
    do_consume, do_discard, do_equip, force_hold_in_hotbar, has_any_axe_in_inventory,
    has_any_pickaxe_in_inventory, is_hard_block, is_log_block, pickaxe_tier, wait_for_held_item,
};

use azalea_registry::builtin::EntityKind;
use std::collections::HashMap;
use std::sync::Arc;

/// 整数方块坐标（watchdog / 冷却表用）。
pub type ChunkPos = (i32, i32, i32);
/// make_obsidian 状态机：(剩余数, 阶段, 黑曜石坐标)。
pub type ObsidianTask = Option<(u32, u8, Option<ChunkPos>)>;
/// 感知聚合：(实体名, (数量, 最近距离, 坐标))。
pub type EntityAgg = HashMap<String, (u32, f64, ChunkPos)>;

/// 当前 Unix 时间戳（毫秒）。
pub(crate) fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn normalize_entity_target(target: &str) -> String {
    let normalized = target.trim().to_ascii_lowercase();
    normalized
        .strip_prefix("minecraft:")
        .unwrap_or(&normalized)
        .to_string()
}

fn entity_kind_name(kind: EntityKind) -> String {
    let name = kind.to_str();
    name.strip_prefix("minecraft:").unwrap_or(name).to_string()
}

/// 转发给外部的 bot 事件（供 harness / LLM 消费）。
#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)] // State 携带完整快照，装箱使所有调用点解包，收益低
pub enum BotEvent {
    /// 连入世界成功。
    Spawn { position: azalea::Vec3 },
    /// 收到游戏聊天（LLM 指令入口）。
    Chat { content: String },
    /// P93：长时工具进度流式事件（goto/mine 执行中每 20 tick 推一次）。
    /// 供 viewer 可视化与 harness 超时前干预；agent 工具结果语义不变（仍为最终结果）。
    Progress { command: String, detail: String },
    /// 与服务端断开。
    Disconnect { reason: String },
    /// 周期性状态快照（位置/维度 + 背包 + 生命/饱食 + 主手 + 群系 + 附近方块/实体 + 任务统计）。
    State {
        position: azalea::Vec3,
        /// 全量非空格：格式 `oak_log:3, cobblestone:64, wooden_pickaxe:1`
        inventory: String,
        /// hotbar（槽 36-44）摘要：`coal x52, dirt x19` 或 `空`（P124 新增）。
        /// 让 LLM 知道装备/切换无需先清背包——避免误判"背包满"陷入 discard 循环。
        hotbar: String,
        /// 已穿戴盔甲摘要：`头盔: iron_helmet, 胸甲: 无, 护腿: 无, 靴子: 无`（P56 新增）
        armor: String,
        /// 已穿戴盔甲结构化列表：[头盔, 胸甲, 护腿, 靴子]，未穿为 "无"。
        /// 与 `armor` 文本摘要同源（P188 新增，供 WorldState.armor 结构化字段）。
        armor_list: Vec<String>,
        player_count: usize,
        /// 朝向（yaw 度数，0=+Z 南，-90=+X 东，90=-X 西，±180=-Z 北）。
        yaw: f64,
        pitch: f64,
        /// 脚下方块名，如 "stone" / "grass_block" / "air"
        block_under: String,
        /// 正前方 1 格视线方块名
        block_ahead: String,
        /// 生命值 (0~20)
        health: f32,
        /// 饱食度 (0~20)
        food: u32,
        /// 饱和值 (隐藏数值，0~20)
        saturation: f32,
        /// 主手物品，如 "wooden_pickaxe" / "air"
        held_item: String,
        /// 生物群系，如 "plains" / "forest"
        biome: String,
        /// 附近方块概览（3x3 地面）：`grass_block:5, stone:3, air:1`
        nearby: String,
        /// 10x10 范围方块扫描：所有非空气方块类型及计数
        nearby_blocks: String,
        /// 附近实体列表：玩家、动物、怪物等
        nearby_entities: String,
        /// 头顶连续实心方块数（P83）：从 bot 头部向上数，遇到空气/未加载停止（上限 64）。
        /// 0 = 头顶即空气（洞穴/地表）；N>10 = 深埋，需 mine_above 挖出。
        overhead_solid: u32,
        /// 结构化游戏状态 JSON（前端面板可视化用），构建于 tick handler 中。
        game_state: serde_json::Value,
    },
}
#[cfg(test)]
mod normalize_item_tests {
    use super::inventory::normalize_item_id;

    /// 单复数容错（P126b）：oak_plank → oak_planks、wheat_seed → wheat_seeds。
    /// 已带 minecraft: 前缀、已复数、无单复数关系的 id 一律原样。
    #[test]
    fn regression_normalize_item_id_plural_fallback() {
        assert_eq!(normalize_item_id("oak_plank"), "minecraft:oak_planks");
        assert_eq!(normalize_item_id("spruce_plank"), "minecraft:spruce_planks");
        assert_eq!(normalize_item_id("wheat_seed"), "minecraft:wheat_seeds");
        assert_eq!(
            normalize_item_id("beetroot_seed"),
            "minecraft:beetroot_seeds"
        );
        // 已带前缀：只做复数容错，不再拼前缀
        assert_eq!(
            normalize_item_id("minecraft:oak_plank"),
            "minecraft:oak_planks"
        );
        assert_eq!(
            normalize_item_id("minecraft:oak_planks"),
            "minecraft:oak_planks"
        );
        assert_eq!(
            normalize_item_id("minecraft:wheat_seed"),
            "minecraft:wheat_seeds"
        );
        // 已复数 / 无关 id：不变
        assert_eq!(normalize_item_id("oak_planks"), "minecraft:oak_planks");
        assert_eq!(normalize_item_id("wheat_seeds"), "minecraft:wheat_seeds");
        assert_eq!(normalize_item_id("stone"), "minecraft:stone");
        assert_eq!(normalize_item_id("stick"), "minecraft:stick");
        assert_eq!(normalize_item_id("oak_sapling"), "minecraft:oak_sapling");
        assert_eq!(normalize_item_id("bamboo"), "minecraft:bamboo");
    }
}

#[cfg(test)]
mod entity_target_tests {
    use super::*;

    #[test]
    fn normalizes_namespaced_entity_target() {
        assert_eq!(normalize_entity_target(" minecraft:COW "), "cow");
        assert_eq!(normalize_entity_target(" COW "), "cow");
    }

    #[test]
    fn entity_kind_name_uses_registry_snake_case() {
        assert_eq!(entity_kind_name(EntityKind::CaveSpider), "cave_spider");
    }
}

/// 便捷类型：共享的 bot 句柄。
pub type SharedBot = Arc<AzaleaBot>;
