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
pub mod actions;
pub mod auto_craft;
pub mod chest;
pub mod client;
pub mod craft;
pub mod ext_state;
pub mod gather;
pub mod perception;
pub mod place;
pub mod recipe_book;
pub mod recipes;
pub mod smart_actions;
pub mod table_flow;
pub mod trade;

pub use action_manager::{ActionManager, Priority, SubmitOutcome, cmd_signature, timeout_ticks};

use azalea::BlockPos;
use azalea::pathfinder::goals::{BlockPosGoal, RadiusGoal, YGoal};
use azalea::player::GameProfileComponent;
use azalea::prelude::*;
use azalea_registry::DataRegistryKey;
use azalea_registry::builtin::{BlockKind, EntityKind};
use bevy_ecs::component::Component;
use craft_agent::core::memory::{MemoryKind, MemoryPos, WorldMemory};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;

/// 当前毫秒时间戳（扫描 TTL 用）。
fn now_ms() -> u64 {
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

fn nearby_active_portal(bot: &Client, center: BlockPos) -> bool {
    let Ok(world) = bot.world() else {
        return false;
    };
    let world = world.read();
    for dx in -5..=5 {
        for dy in -5..=5 {
            for dz in -5..=5 {
                let pos = BlockPos::new(center.x + dx, center.y + dy, center.z + dz);
                if let Some(state) = world.get_block_state(pos) {
                    let kind: BlockKind = state.into();
                    if kind == BlockKind::NetherPortal {
                        return true;
                    }
                }
            }
        }
    }
    false
}

/// 把感兴趣的 BlockKind 映射为记忆元数据（item, 标签, 类别）。
/// 返回 None 表示该方块不值得记忆。
fn block_memory_meta(bk: BlockKind) -> Option<(String, &'static str, MemoryKind)> {
    // P5 修复：用 to_str() 拿到 snake_case minecraft id（如 "dark_oak_log"），
    // 原代码用 format!("{bk:?}").to_lowercase() 得到 "darkoaklog"（无下划线），
    // LLM 看到 "darkoaklog" 用 gather("dark_oak_log") 报"未知物品" → 100% 卡死。
    let name_full = bk.to_str();
    let name = name_full.strip_prefix("minecraft:").unwrap_or(name_full);
    // 原木类（oak_log / dark_oak_log / birch_log / ...）和菌丝类（crimson_stem / warped_stem）
    if name.ends_with("_log") || name.ends_with("_stem") {
        return Some((name.to_string(), "树木/原木", MemoryKind::Resource));
    }
    // 矿石类
    if name.ends_with("_ore") || name == "ancient_debris" {
        return Some((name.to_string(), "矿石", MemoryKind::Resource));
    }
    match bk {
        BlockKind::CraftingTable => {
            Some(("crafting_table".into(), "工作台", MemoryKind::Structure))
        }
        BlockKind::Furnace => Some(("furnace".into(), "熔炉", MemoryKind::Structure)),
        BlockKind::Chest => Some(("chest".into(), "箱子", MemoryKind::Container)),
        BlockKind::SmithingTable => {
            Some(("smithing_table".into(), "锻造台", MemoryKind::Structure))
        }
        BlockKind::EnchantingTable => {
            Some(("enchanting_table".into(), "附魔台", MemoryKind::Structure))
        }
        BlockKind::NetherPortal => Some(("nether_portal".into(), "下界传送门", MemoryKind::Portal)),
        BlockKind::Lava => Some(("lava".into(), "岩浆", MemoryKind::Hazard)),
        BlockKind::Water => Some(("water".into(), "水", MemoryKind::Hazard)),
        _ => None,
    }
}

/// 扫描去重 TTL：同一坐标在此时间内不再重新向服务端查询（省开销）。
/// 超过 TTL 后重新 `get_block_state` 校验，让"树被砍/方块被破坏"等世界变化
/// 能反映到记忆（消失的资源点标记 depleted，消失的结构/容器直接遗忘）。
const SCAN_TTL_MS: u64 = 30_000;

/// 扫描 bot 周围半径内的关键方块，回填到 WorldMemory。
/// 用 `scanned`（pos → 上次扫描时间戳）去重 + TTL 重验。
fn record_surroundings(
    bot: &Client,
    mem: &WorldMemory,
    center: &MemoryPos,
    scanned: &Arc<Mutex<HashMap<MemoryPos, u64>>>,
) {
    let world = match bot.world() {
        Ok(w) => w,
        Err(_) => return,
    };
    let radius = 8i32;
    let now = now_ms();
    let mut to_write: Vec<(MemoryPos, String, &'static str, MemoryKind)> = Vec::new();
    let mut to_deplete: Vec<MemoryPos> = Vec::new();
    let mut to_forget: Vec<MemoryPos> = Vec::new();
    {
        let mut scanned_g = scanned.lock().unwrap();
        for dx in -radius..=radius {
            for dy in -radius..=radius {
                for dz in -radius..=radius {
                    let pos = BlockPos::new(center.x + dx, center.y + dy, center.z + dz);
                    let mp = MemoryPos::new(pos.x, pos.y, pos.z);
                    // TTL 内已扫过：跳过（世界变化由 action 路径/B 的 forget 即时处理）
                    if let Some(&last) = scanned_g.get(&mp) {
                        if now.saturating_sub(last) < SCAN_TTL_MS {
                            continue;
                        }
                    }
                    scanned_g.insert(mp, now);
                    let still_memory = world
                        .read()
                        .get_block_state(pos)
                        .map(|s| block_memory_meta(s.into()));
                    match still_memory {
                        Some(Some((item, label, kind))) => {
                            to_write.push((mp, item, label, kind));
                        }
                        // 方块不再是记忆类（被挖/被破坏/变空气）：
                        // 若原记忆是资源点 → 标记 depleted（保留但不再推荐）；否则遗忘。
                        Some(None) | None => {
                            if let Some(c) = mem.get(mp) {
                                if c.kind == MemoryKind::Resource {
                                    to_deplete.push(mp);
                                } else {
                                    to_forget.push(mp);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    for (mp, item, label, kind) in to_write {
        match kind {
            MemoryKind::Resource => mem.record_resource(mp, &item, label, None),
            MemoryKind::Structure => mem.record_structure(mp, &item, label),
            MemoryKind::Container => mem.record_container(mp, label, ""),
            MemoryKind::Portal => mem.record(mp, MemoryKind::Portal, Some(&item), label, None),
            MemoryKind::Hazard => mem.record(mp, MemoryKind::Hazard, Some(&item), label, None),
            _ => mem.record(mp, kind, Some(&item), label, None),
        }
    }
    for p in to_deplete {
        mem.mark_depleted(p, true);
    }
    for p in to_forget {
        mem.forget_pos(p);
    }
}

/// 转发给外部的 bot 事件（供 harness / LLM 消费）。
#[derive(Debug, Clone)]
pub enum BotEvent {
    /// 连入世界成功。
    Spawn { position: azalea::Vec3 },
    /// 收到游戏聊天（LLM 指令入口）。
    Chat { content: String },
    /// 与服务端断开。
    Disconnect { reason: String },
    /// 周期性状态快照（位置/维度 + 背包 + 生命/饱食 + 主手 + 群系 + 附近方块/实体 + 任务统计）。
    State {
        position: azalea::Vec3,
        /// 全量非空格：格式 `oak_log:3, cobblestone:64, wooden_pickaxe:1`
        inventory: String,
        /// 已穿戴盔甲摘要：`头盔: iron_helmet, 胸甲: 无, 护腿: 无, 靴子: 无`（P56 新增）
        armor: String,
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
        /// 结构化游戏状态 JSON（前端面板可视化用），构建于 tick handler 中。
        game_state: serde_json::Value,
    },
}

/// 动作指令：由 `AzaleaBot` 发出，handler 内部用 `bot` 执行。
#[derive(Debug, Clone)]
pub enum BotCommand {
    Goto {
        x: i32,
        y: i32,
        z: i32,
    },
    Mine {
        x: i32,
        y: i32,
        z: i32,
    },
    MineBelow,
    /// 持续向上挖：从 bot 头顶逐格挖到空气，让 bot 跳出竖井/地下脱困。
    /// 用于「被埋在地下/卡在 1x1 竖井」场景——mine_below 是向下挖，
    /// mine_above 反向，向上挖通到地表。
    MineAbove,
    BlockInteract {
        x: i32,
        y: i32,
        z: i32,
    },
    Chat {
        content: String,
    },
    Attack {
        target: String,
    },
    /// 2×2 背包合成（无需工作台）：item 为目标物品 id（如 "oak_planks"），count 为期望数量。
    Craft2x2 {
        item: String,
        count: u32,
    },
    /// 3×3 工作台合成。item 为目标物品 id，count 为期望数量。
    /// table_pos=Some 时使用该坐标的现有工作台；None 时 bot 自动放置+打开+关闭工作台（P1-4）。
    Craft3x3 {
        item: String,
        count: u32,
        table_pos: Option<(i32, i32, i32)>,
    },
    /// 熔炼。output 为目标物品 id（如 "iron_ingot"），fuel 为燃料物品 id（如 "coal"），count 为期望数量。
    /// table_pos=Some 时使用该坐标的现有熔炉；None 时 bot 自动放置+打开+关闭熔炉（P1-4）。
    Smelt {
        output: String,
        fuel: String,
        count: u32,
        table_pos: Option<(i32, i32, i32)>,
    },
    /// 采集：走到最近的指定方块（如 "oak_log" / "stone" / "coal_ore"）并挖掘，直到背包有 count 个。
    Gather {
        item: String,
        count: u32,
    },
    /// P67：自动造黑曜石。bot 需手持 water_bucket，且附近（半径 12）有岩浆源。
    /// 工具会：在岩浆旁放一格水→生成黑曜石→用 diamond_pickaxe 挖下→重复 count 次。
    /// 用于下界传送门框架。若没水/没岩浆/没钻石镐会返回错误。
    MakeObsidian {
        count: u32,
    },
    /// 放置：把手持物品 item 放到世界坐标 (x,y,z) 旁（右键放置）。
    Place {
        item: String,
        x: i32,
        y: i32,
        z: i32,
    },
    /// 打开容器：打开世界坐标 (x,y,z) 处的容器（工作台/熔炉/箱子等）。
    OpenContainer {
        x: i32,
        y: i32,
        z: i32,
    },
    /// 高层自动合成（木链）：采集→2×2→放置工作台→开→3×3，一键造木制品。
    AutoCraft {
        item: String,
        count: u32,
    },
    /// 附魔：在已打开的附魔台中，给 item 附魔（需背包有 item 与青金石 lapis_lazuli）。
    /// level 为 1/2/3，对应附魔台三个选项槽。
    Enchant {
        item: String,
        level: u32,
    },
    /// 村民交易：与最近的村民交易，选第 offer 个报价（0 起）。bot 自动打开村民。
    Trade {
        offer: u32,
    },
    /// 实体右键交互（打开村民/动物/展示框等）：与最近的指定种类实体交互。
    /// kind 为实体种类关键词，如 "villager"。
    InteractEntity {
        kind: String,
    },
    /// 捡起附近掉落物：bot 走 4 个方向扫一圈，让物理引擎自然吸取掉落物。
    /// 无参数。挖矿/战斗后调用一次，避免"挖了 8 个石头但只捡到 3 个"。
    Pickup,
    /// 自动防御：等待 5 秒让 handler 层 self_defense mode 自动攻击附近敌人。
    /// 期间监测血量，若受到严重伤害提前返回建议撤退。
    Defend,
    /// 装备背包中的指定物品到指定槽位。
    /// slot: "hand"/"helmet"/"chestplate"/"leggings"/"boots"
    Equip {
        item: String,
        slot: String,
    },
    /// 丢弃背包中的指定物品。count 为丢弃数量（0 表示全部）。
    Discard {
        item: String,
        count: u32,
    },
    /// 消耗（吃/喝）背包中的指定物品。
    Consume {
        item: String,
    },
    /// 查看容器物品列表（打开→读→关闭）。
    ChestView {
        x: i32,
        y: i32,
        z: i32,
    },
    /// 从容器取出 item（count 个）到 bot 背包。
    ChestWithdraw {
        x: i32,
        y: i32,
        z: i32,
        item: String,
        count: u32,
    },
    /// 把背包中的 item（count 个）存入容器。
    ChestDeposit {
        x: i32,
        y: i32,
        z: i32,
        item: String,
        count: u32,
    },
    /// P68：跟随玩家。target 为玩家名（None 表示跟随最近的其他玩家）。
    /// handler 每 tick 读取该玩家坐标并 goto，实现"跟着我"。
    Follow {
        target: Option<String>,
    },
    /// P68：停止跟随（解除 Follow 模式）。
    StopFollow,
    /// P68：把物品丢在指定玩家脚边（玩家拾取）。item 为物品 id，count 为数量（0=全部）。
    /// target 为玩家名（None 表示最近的其他玩家）。基于现有 Discard 能力，但丢在玩家坐标而非 bot 脚边。
    Give {
        item: String,
        count: u32,
        target: Option<String>,
    },
}

/// 队列中的命令包装：携带结果回传通道（None 表示 fire-and-forget，如聊天指令）。
#[derive(Clone)]
pub struct QueuedCommand {
    pub cmd: BotCommand,
    pub result_tx: Option<std::sync::mpsc::Sender<String>>,
}

/// handler 状态：持有命令队列、事件发送端与最近坐标（跨事件持久，Arc 共享）。
#[derive(Component, Clone)]
pub struct BotState {
    pub cmd_queue: Arc<Mutex<Vec<QueuedCommand>>>,
    pub evt_tx: Arc<mpsc::UnboundedSender<BotEvent>>,
    pub last_position: Arc<Mutex<Option<azalea::Vec3>>>,
    /// 持续下挖标志：收到 MineBelow 后置 true，Tick 内只要未在挖就重复触发，
    /// 对齐 POC 的持续挖矿逻辑（azalea 单次 start_mining 可能因中断失效）。
    pub mining_below: Arc<Mutex<bool>>,
    /// 持续上挖标志：收到 MineAbove 后置 true，Tick 内只要未在挖就重复触发。
    /// 用于地下脱困——头顶方块挖完后 bot 自动跳起，下一格又挖，直到头顶是空气。
    pub mining_above: Arc<Mutex<bool>>,
    /// Y at the start of the current MineAbove command. The synchronous tool
    /// completes only after actual upward movement, never on dispatch alone.
    pub mining_above_start_y: Arc<Mutex<Option<i32>>>,
    /// Direction tried by deterministic staircase ascent. Rotated whenever a
    /// concrete adjacent-up goal makes no progress.
    pub mining_above_direction: Arc<Mutex<usize>>,
    /// Whether mine_above already tried /tp rescue. Reset on new MineAbove.
    pub mine_above_tried_tp: Arc<Mutex<bool>>,
    /// ActionManager：封装 pending 槽 + 按命令类型超时 + 抢占 + 快循环检测。
    /// 取代原硬编码 60-tick 超时（合成/采集/熔炼等长任务被误杀）。
    /// 字段保留 pending/pending_since/busy 的 Arc 引用，供旧代码兼容访问。
    pub action_mgr: ActionManager,
    /// 共享世界记忆库（适配器/工具/Agent 共用；handler 内扫描回填）。
    pub memory: Option<WorldMemory>,
    /// 已扫描记录的坐标 → 上次扫描时间戳（TTL 去重 + 重验世界变化）。
    pub scanned: Arc<Mutex<HashMap<MemoryPos, u64>>>,
    /// P65/P66：goto 卡死看门狗。(last_x, last_y, last_z, stall_count)。
    /// 若连续 goto 超时但 bot 净移动 <1.5 格（无论目标坐标如何变），累计 stall，
    /// 达阈值即强制脱困（地表挖开阻挡方块 / 地下 mine_above）。
    pub goto_watchdog: Arc<Mutex<(i32, i32, i32, u32)>>,
    /// P66：goto 冷却表（按 bot 当前格子）。触发脱困后冷却该格子 N tick，
    /// 期间 goto 直接拒绝，打破脚本/LLM 的 goto 死循环。
    pub goto_cooldown: Arc<Mutex<HashMap<(i32, i32, i32), u64>>>,
    /// P67：全局"原地冻死"看门狗。bot 位置长时间（~20s）不变且循环仍在推进，
    /// 说明卡在某个不动作（如空转 run_script / 无效 interact）。累计到阈值即
    /// 向 LLM 推强警告，逼其换策略（pi-agent 自主止损，覆盖所有非 goto 卡死）。
    pub no_move_ticks: Arc<Mutex<u64>>,
    pub last_seen_pos: Arc<Mutex<(i32, i32, i32)>>,
    /// P67：make_obsidian 状态机。(remaining, phase, obsidian_pos)。phase: 0=找岩浆放水, 1=等黑曜石生成, 2=挖黑曜石。
    pub make_obsidian: Arc<Mutex<Option<(u32, u8, Option<(i32, i32, i32)>)>>>,
    /// P68：跟随模式。Some(target) 表示正在跟随该玩家（None 名=跟随最近玩家）；
    /// None 表示未跟随。handler 每 tick 读取目标坐标 goto。
    pub follow_target: Arc<Mutex<Option<Option<String>>>>,
    /// P77：hunting 模式——攻击动物后自动拾取掉落物的截止 tick（0=无窗口）。
    pub hunt_pickup_until: Arc<Mutex<u64>>,
    /// P77：战斗模式请求自动装备的武器名（防重复 push Equip；None=无待装备）。
    pub combat_equip_pending: Arc<Mutex<Option<String>>>,
}

impl Default for BotState {
    fn default() -> Self {
        // dummy：真实 state 总由 connect() 构造，此处仅满足 trait 约束。
        let (_, rx) = mpsc::unbounded_channel::<BotEvent>();
        drop(rx);
        BotState {
            cmd_queue: Arc::new(Mutex::new(Vec::new())),
            evt_tx: Arc::new(mpsc::unbounded_channel::<BotEvent>().0),
            last_position: Arc::new(Mutex::new(None)),
            goto_watchdog: Arc::new(Mutex::new((0, 0, 0, 0))),
            goto_cooldown: Arc::new(Mutex::new(HashMap::new())),
            no_move_ticks: Arc::new(Mutex::new(0)),
            last_seen_pos: Arc::new(Mutex::new((0, 0, 0))),
            make_obsidian: Arc::new(Mutex::new(None)),
            follow_target: Arc::new(Mutex::new(None)),
            mining_below: Arc::new(Mutex::new(false)),
            mining_above: Arc::new(Mutex::new(false)),
            mining_above_start_y: Arc::new(Mutex::new(None)),
            mining_above_direction: Arc::new(Mutex::new(0)),
            mine_above_tried_tp: Arc::new(Mutex::new(false)),
            action_mgr: ActionManager::new(),
            memory: None,
            scanned: Arc::new(Mutex::new(HashMap::new())),
            hunt_pickup_until: Arc::new(Mutex::new(0)),
            combat_equip_pending: Arc::new(Mutex::new(None)),
        }
    }
}

/// Azalea bot 句柄：连入后持有命令队列与事件通道，提供动作与感知 API。
pub struct AzaleaBot {
    cmd_queue: Arc<Mutex<Vec<QueuedCommand>>>,
    events: Arc<tokio::sync::Mutex<mpsc::UnboundedReceiver<BotEvent>>>,
    /// 最近一次已知坐标（由 handler Tick 更新，供同步读取）。
    pub last_position: Arc<Mutex<Option<azalea::Vec3>>>,
    /// 跨系统/跨 handler 共享的扩展状态（村民报价、配方书等）。
    pub ext: crate::azalea::ext_state::SharedExt,
    /// 共享世界记忆库（与适配器/工具/Agent 共用同一实例）。
    pub memory: Option<craft_agent::core::memory::WorldMemory>,
}

impl AzaleaBot {
    /// 异步接收下一个 bot 事件（供 harness 主循环消费）。
    pub async fn next_event(&self) -> Option<BotEvent> {
        let mut rx = self.events.lock().await;
        rx.recv().await
    }
}

fn parse_chat_coords(rest: &str) -> Option<(i32, i32, i32)> {
    let values: Vec<i32> = rest
        .split_whitespace()
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    (values.len() == 3).then(|| (values[0], values[1], values[2]))
}

/// Parse the small, synchronous chat command surface used for in-game control.
/// Keeping this pure makes malformed commands testable without a live client.
pub fn parse_chat_command(content: &str) -> Option<BotCommand> {
    let content = content.trim();
    if let Some(rest) = content.strip_prefix("autocraft ") {
        let mut parts = rest.split_whitespace();
        return Some(BotCommand::AutoCraft {
            item: parts.next()?.to_string(),
            count: parts
                .next()
                .and_then(|value| value.parse().ok())
                .unwrap_or(1),
        });
    }
    if let Some(rest) = content.strip_prefix("open ") {
        let (x, y, z) = parse_chat_coords(rest)?;
        return Some(BotCommand::OpenContainer { x, y, z });
    }
    if let Some(rest) = content.strip_prefix("place ") {
        let mut parts = rest.split_whitespace();
        let item = parts.next()?.to_string();
        let (x, y, z) = parse_chat_coords(&parts.collect::<Vec<_>>().join(" "))?;
        return Some(BotCommand::Place { item, x, y, z });
    }
    if let Some(rest) = content.strip_prefix("gather ") {
        let mut parts = rest.split_whitespace();
        return Some(BotCommand::Gather {
            item: parts.next()?.to_string(),
            count: parts
                .next()
                .and_then(|value| value.parse().ok())
                .unwrap_or(1),
        });
    }
    if let Some(rest) = content.strip_prefix("craft3 ") {
        let mut parts = rest.split_whitespace();
        return Some(BotCommand::Craft3x3 {
            item: parts.next()?.to_string(),
            count: parts
                .next()
                .and_then(|value| value.parse().ok())
                .unwrap_or(1),
            table_pos: None,
        });
    }
    if let Some(rest) = content.strip_prefix("smelt ") {
        let mut parts = rest.split_whitespace();
        return Some(BotCommand::Smelt {
            output: parts.next()?.to_string(),
            fuel: parts.next().unwrap_or("coal").to_string(),
            count: parts
                .next()
                .and_then(|value| value.parse().ok())
                .unwrap_or(1),
            table_pos: None,
        });
    }
    if let Some(rest) = content.strip_prefix("craft ") {
        let mut parts = rest.split_whitespace();
        return Some(BotCommand::Craft2x2 {
            item: parts.next()?.to_string(),
            count: parts
                .next()
                .and_then(|value| value.parse().ok())
                .unwrap_or(1),
        });
    }
    if let Some(rest) = content.strip_prefix("goto ") {
        let (x, y, z) = parse_chat_coords(rest)?;
        return Some(BotCommand::Goto { x, y, z });
    }
    if let Some(rest) = content.strip_prefix("mine ") {
        let (x, y, z) = parse_chat_coords(rest)?;
        return Some(BotCommand::Mine { x, y, z });
    }
    if content == "minebelow" {
        return Some(BotCommand::MineBelow);
    }
    if content == "attack" {
        return Some(BotCommand::Attack {
            target: "chat".into(),
        });
    }
    if let Some(rest) = content.strip_prefix("enchant ") {
        let mut parts = rest.split_whitespace();
        return Some(BotCommand::Enchant {
            item: parts.next()?.to_string(),
            level: parts
                .next()
                .and_then(|value| value.parse().ok())
                .unwrap_or(1),
        });
    }
    if let Some(rest) = content.strip_prefix("trade ") {
        return rest
            .trim()
            .parse()
            .ok()
            .map(|offer| BotCommand::Trade { offer });
    }
    if let Some(rest) = content.strip_prefix("interact ") {
        let kind = rest.trim();
        if !kind.is_empty() {
            return Some(BotCommand::InteractEntity {
                kind: kind.to_string(),
            });
        }
        return None;
    }
    if content == "follow" {
        return Some(BotCommand::Follow { target: None });
    }
    if let Some(rest) = content.strip_prefix("follow ") {
        return Some(BotCommand::Follow {
            target: (!rest.trim().is_empty()).then(|| rest.trim().to_string()),
        });
    }
    if content == "stopfollow" || content == "stop" {
        return Some(BotCommand::StopFollow);
    }
    if let Some(rest) = content.strip_prefix("give ") {
        let mut parts = rest.split_whitespace();
        let item = parts.next()?.to_string();
        let second = parts.next();
        let (count, target) = match second {
            None => (0, None),
            Some(value) => match value.parse::<u32>() {
                Ok(count) => (count, parts.next().map(str::to_string)),
                Err(_) => (0, Some(value.to_string())),
            },
        };
        return Some(BotCommand::Give {
            item,
            count,
            target,
        });
    }
    if let Some(rest) = content.strip_prefix("equip ") {
        let mut parts = rest.split_whitespace();
        let item = parts.next()?.to_string();
        let slot = parts
            .next()
            .map(str::to_string)
            .unwrap_or_else(|| "hand".to_string());
        return Some(BotCommand::Equip { item, slot });
    }
    if let Some(rest) = content.strip_prefix("discard ") {
        let mut parts = rest.split_whitespace();
        let item = parts.next()?.to_string();
        let count = parts
            .next()
            .and_then(|value| value.parse().ok())
            .unwrap_or(0);
        return Some(BotCommand::Discard { item, count });
    }
    if let Some(rest) = content.strip_prefix("consume ") {
        let item = rest.trim();
        if item.is_empty() {
            return None;
        }
        return Some(BotCommand::Consume {
            item: item.to_string(),
        });
    }
    if let Some(rest) = content.strip_prefix("chestview ") {
        let (x, y, z) = parse_chat_coords(rest)?;
        return Some(BotCommand::ChestView { x, y, z });
    }
    if let Some(rest) = content.strip_prefix("chestwithdraw ") {
        let mut parts = rest.split_whitespace();
        let (x, y, z) = parse_chat_coords(&parts.clone().collect::<Vec<_>>()[0..3].join(" "))?;
        let _ = parts.next();
        let _ = parts.next();
        let _ = parts.next();
        let item = parts.next()?.to_string();
        let count = parts
            .next()
            .and_then(|value| value.parse().ok())
            .unwrap_or(1);
        return Some(BotCommand::ChestWithdraw { x, y, z, item, count });
    }
    if let Some(rest) = content.strip_prefix("chestdeposit ") {
        let mut parts = rest.split_whitespace();
        let (x, y, z) = parse_chat_coords(&parts.clone().collect::<Vec<_>>()[0..3].join(" "))?;
        let _ = parts.next();
        let _ = parts.next();
        let _ = parts.next();
        let item = parts.next()?.to_string();
        let count = parts
            .next()
            .and_then(|value| value.parse().ok())
            .unwrap_or(1);
        return Some(BotCommand::ChestDeposit { x, y, z, item, count });
    }
    if let Some(rest) = content.strip_prefix("makeobsidian") {
        let count = rest
            .split_whitespace()
            .next()
            .and_then(|value| value.parse().ok())
            .unwrap_or(1);
        return Some(BotCommand::MakeObsidian { count });
    }
    if content == "pickup" {
        return Some(BotCommand::Pickup);
    }
    if content == "defend" {
        return Some(BotCommand::Defend);
    }
    None
}

fn nearby_player_position(bot: &Client, target: Option<&str>) -> Option<azalea::Vec3> {
    let bot_pos = bot.position().ok();
    let players = bot.nearby_players().ok()?;
    let mut closest: Option<(f64, azalea::Vec3)> = None;
    for player in players.iter() {
        let name = player
            .component::<GameProfileComponent>()
            .map(|profile| profile.0.name.clone())
            .unwrap_or_default();
        if target.is_some_and(|wanted| name != wanted) {
            continue;
        }
        let Ok(position) = player.position() else {
            continue;
        };
        let distance = bot_pos.map_or(0.0, |origin| {
            ((origin.x - position.x).powi(2)
                + (origin.y - position.y).powi(2)
                + (origin.z - position.z).powi(2))
            .sqrt()
        });
        if closest.as_ref().is_none_or(|(best, _)| distance < *best) {
            closest = Some((distance, position));
        }
    }
    closest.map(|(_, position)| position)
}

impl AzaleaBot {
    /// 离线账号连入指定地址（如 "localhost:4444"），返回就绪的 bot 句柄。
    /// 本方法 spawn 后台 task 运行 azalea 客户端循环，立即返回句柄。
    pub async fn connect(
        address: &str,
        username: &str,
        memory: Option<WorldMemory>,
    ) -> anyhow::Result<AzaleaBot> {
        let account = Account::offline(username);
        let (evt_tx, evt_rx) = mpsc::unbounded_channel::<BotEvent>();
        let evt_tx = Arc::new(evt_tx);
        let cmd_queue: Arc<Mutex<Vec<QueuedCommand>>> = Arc::new(Mutex::new(Vec::new()));
        let last_position: Arc<Mutex<Option<azalea::Vec3>>> = Arc::new(Mutex::new(None));
        let ext: crate::azalea::ext_state::SharedExt =
            Arc::new(Mutex::new(crate::azalea::ext_state::BotExtState::default()));
        // 用本地内置配方库（vanilla 26.2）填充配方书，作为 auto_craft 权威数据源。
        // 服务端下发的 RecipeBookAdd 后续会叠加/覆盖（overlay）。
        ext.lock().unwrap().recipes = crate::azalea::recipe_book::load_builtin();
        let ext_for_bot = ext.clone();

        let state = BotState {
            cmd_queue: cmd_queue.clone(),
            evt_tx: evt_tx.clone(),
            last_position: last_position.clone(),
            follow_target: Arc::new(Mutex::new(None)),
            mining_below: Arc::new(Mutex::new(false)),
            mining_above: Arc::new(Mutex::new(false)),
            mining_above_start_y: Arc::new(Mutex::new(None)),
            mining_above_direction: Arc::new(Mutex::new(0)),
            mine_above_tried_tp: Arc::new(Mutex::new(false)),
            action_mgr: ActionManager::new(),
            memory: memory,
            scanned: Arc::new(Mutex::new(HashMap::new())),
            hunt_pickup_until: Arc::new(Mutex::new(0)),
            combat_equip_pending: Arc::new(Mutex::new(None)),
            goto_watchdog: Arc::new(Mutex::new((0, 0, 0, 0))),
            goto_cooldown: Arc::new(Mutex::new(HashMap::new())),
            no_move_ticks: Arc::new(Mutex::new(0)),
            last_seen_pos: Arc::new(Mutex::new((0, 0, 0))),
            make_obsidian: Arc::new(Mutex::new(None)),
        };

        let addr = address.to_string();
        // azalea 内部用 Rc（!Send），不能在多线程 tokio::spawn 里跑。
        // 起一个专用 current-thread runtime 在独立 OS 线程运行 bot 循环。
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("azalea runtime");
            rt.block_on(async move {
                let _ = ClientBuilder::new()
                    .add_plugins(crate::azalea::ext_state::CraftAgentPlugin { ext: ext.clone() })
                    .set_handler(AzaleaBot::handle)
                    .set_state(state)
                    .start(account, addr.as_str())
                    .await;
            });
        });

        Ok(AzaleaBot {
            cmd_queue,
            events: Arc::new(tokio::sync::Mutex::new(evt_rx)),
            last_position,
            ext: ext_for_bot,
            memory: None,
        })
    }

    /// azalea handler：所有 bot 逻辑在此执行（fn 指针，不捕获外部变量）。
    /// 命令从 `state.cmd_queue` 取出执行，事件经 `state.evt_tx` 转发外部。
    async fn handle(bot: Client, event: Event, state: BotState) -> Client {
        let cmd_queue = state.cmd_queue.clone();
        let evt_tx = state.evt_tx.clone();
        let lp = state.last_position.clone();
        match event {
            Event::Spawn => {
                if let Ok(p) = bot.position() {
                    *lp.lock().unwrap() = Some(p);
                    let _ = evt_tx.send(BotEvent::Spawn { position: p });
                }
            }
            Event::Chat(packet) => {
                // M5 修复：用 content() 方法获取纯文本字符串，而非 Debug 格式化。
                // 旧实现 format!("{:?}", p.content) 产出 "TextComponent { text: \"goto 10 64 10\", ... }"
                // 导致 strip_prefix("goto ") 等聊天命令解析全部失效。
                let content = packet.content();
                // 聊天驱动的即时指令（便于实机调试 / 玩家直接指挥 bot）：
                //   craft <物品> [数量]        2×2 背包合成
                //   craft3 <物品> [数量]       3×3 工作台合成（需已开工作台）
                //   smelt <产物> <燃料> [数量] 熔炼（需已开熔炉）
                //   gather <方块> [数量]       走到最近该方块并挖掘（如 gather oak_log 4）
                //   place <物品> <x> <y> <z>  把手持物品放到坐标旁（如 place crafting_table 10 64 10）
                //   open <x> <y> <z>          打开该坐标的容器（工作台/熔炉）
                //   autocraft <物品> [数量]   高层自动合成（木链，如 autocraft chest 1）
                //   enchant <物品> [等级]     附魔（需已开附魔台且背包有 item 与青金石，如 enchant iron_sword 2）
                //   goto <x> <y> <z> / mine <x> <y> <z> / minebelow / attack
                if let Some(cmd) = parse_chat_command(&content) {
                    let mut q = cmd_queue.lock().unwrap();
                    q.push(QueuedCommand {
                        cmd,
                        result_tx: None,
                    });
                }
                let _ = evt_tx.send(BotEvent::Chat { content });
            }
            Event::Disconnect(reason) => {
                let _ = evt_tx.send(BotEvent::Disconnect {
                    reason: format!("{reason:?}"),
                });
            }
            Event::Tick => {
                if let Ok(p) = bot.position() {
                    *lp.lock().unwrap() = Some(p);
                    // P67：全局"原地冻死"看门狗。每 tick 比对上次记录位置，
                    // 若净移动 <1 格则累加 no_move_ticks，否则清零。
                    // 累计达 400 tick(20s) 且循环仍活跃（有 pending 或队列非空）→
                    // 向 LLM 推强警告，逼其换策略（覆盖 goto 之外的所有卡死：空转脚本/无效 interact 等）。
                    {
                        let cur = (p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32);
                        let mut last = state.last_seen_pos.lock().unwrap();
                        let moved = (cur.0 - last.0).abs() > 1
                            || (cur.1 - last.1).abs() > 1
                            || (cur.2 - last.2).abs() > 1;
                        if moved {
                            *last = cur;
                            *state.no_move_ticks.lock().unwrap() = 0;
                        } else {
                            *state.no_move_ticks.lock().unwrap() += 1;
                        }
                        let nmt = *state.no_move_ticks.lock().unwrap();
                        if nmt == 400 {
                            let queue_len = state.cmd_queue.lock().unwrap().len();
                            let pending = state.action_mgr.is_idle();
                            if queue_len > 0 || !pending {
                                let _ = evt_tx.send(BotEvent::Chat {
                                    content: "【原地冻死警告】你已连续 20 秒几乎没移动，但仍在发指令——这说明卡在某个无效动作（如空转脚本、对空气 interact、反复同动作）。请立即换策略：(1) 若目标是挖矿，用 mine_below/mine_above 真正向下/向上挖；(2) 若被挡，用 mine 挖开阻挡方块；(3) 不要重复调用同一个无效工具。先 perceive 看真实状态。".to_string(),
                                });
                            }
                        }
                    }
                }
                // P68：跟随模式（每 10 tick 推进一次）。读取目标玩家坐标并 goto，
                // 实现"跟着我"。仅在当前无 pending 命令（避免打断采矿/合成等）时生效。
                {
                    let follow = state.follow_target.lock().unwrap().clone();
                    if let Some(target) = follow {
                        let tick_now = bot.ticks_connected() as u64;
                        if tick_now % 10 == 0 && state.action_mgr.is_idle() {
                            let players = bot.nearby_players();
                            if let Ok(players) = players {
                                let mut chosen: Option<(f64, f64, f64, String)> = None;
                                for p in players.iter() {
                                    let uname = p
                                        .component::<GameProfileComponent>()
                                        .map(|g| g.0.name.clone())
                                        .unwrap_or_default();
                                    if let Some(t) = &target {
                                        if &uname != t {
                                            continue;
                                        }
                                    }
                                    if let Ok(pos) = p.position() {
                                        chosen = Some((pos.x, pos.y, pos.z, uname));
                                        if target.is_some() {
                                            break;
                                        }
                                    }
                                }
                                if let Some((px, py, pz, _uname)) = chosen {
                                    // 跟随时走到玩家脚下（略低于玩家，避免卡进身体）。
                                    let _ = bot.goto(BlockPosGoal(BlockPos::new(
                                        px.floor() as i32,
                                        py.floor() as i32,
                                        pz.floor() as i32,
                                    )));
                                } else {
                                    // 目标玩家不在附近：解除跟随并提示。
                                    *state.follow_target.lock().unwrap() = None;
                                    let _ = evt_tx.send(BotEvent::Chat {
                                        content: format!(
                                            "[跟随] 找不到玩家 {}，已自动停止跟随。",
                                            target
                                                .clone()
                                                .unwrap_or_else(|| "最近的玩家".to_string())
                                        ),
                                    });
                                }
                            }
                        }
                    }
                }
                // 串行消费命令队列：每 tick 最多推进「一条」命令，等它完成才取下一条。
                // ActionManager 管理单槽 pending + 按命令类型超时 + 抢占 + 快循环检测。
                {
                    let tick_now = bot.ticks_connected() as u64;
                    // 推进队列：pending 空时从队列 pop 一条
                    if state.action_mgr.is_idle() {
                        let next = {
                            let mut q = cmd_queue.lock().unwrap();
                            // FIFO：取最早入队的命令。Vec::pop 取最后元素是 LIFO，
                            // 这里用 remove(0) 实现真 FIFO（队列长度通常 <5，O(n) 可忽略）。
                            if q.is_empty() {
                                None
                            } else {
                                Some(q.remove(0))
                            }
                        };
                        if let Some(qc) = next {
                            state.action_mgr.occupy(qc, tick_now);
                        }
                    }
                    // 轮询非阻塞命令（Goto/Mine）完成状态 + 按命令类型超时
                    if let Some(qc) = state.action_mgr.peek_pending() {
                        let done =
                            match &qc.cmd {
                                BotCommand::Mine { x, y, z } => {
                                    if let Ok(world) = bot.world() {
                                        let s =
                                            world.read().get_block_state(BlockPos::new(*x, *y, *z));
                                        let is_air =
                                            s.is_none() || s.map(|b| b.is_air()).unwrap_or(false);
                                        // P4 修复：start_mining 只在命令派发时调一次，但挖掘可能被
                                        // 重力/移动/伤害中断后不再恢复。这里每 20 tick 重新发起挖掘，
                                        // 确保方块还在就持续挖（对齐 MineBelow 的持续触发逻辑）。
                                        if !is_air && !bot.is_mining() && tick_now % 20 == 0 {
                                            bot.start_mining(BlockPos::new(*x, *y, *z));
                                        }
                                        is_air
                                    } else {
                                        false
                                    }
                                }
                                BotCommand::Goto { x, y, z } => {
                                    if let Ok(p) = bot.position() {
                                        let d = ((p.x - *x as f64).powi(2)
                                            + (p.y - *y as f64).powi(2)
                                            + (p.z - *z as f64).powi(2))
                                        .sqrt();
                                        // P67：到达判定放宽 1.5→2.5m。probe 实测：bot 常停在目标 1.5-2.5m
                                        // 处（pathfinder 已认为到达）而 done 永不触发 → 空等 60s 超时，
                                        // LLM 误判"路径被阻"反复重试。2.5m 内即算到达。
                                        d < 2.5
                                    } else {
                                        false
                                    }
                                }
                                BotCommand::MineBelow => false,
                                BotCommand::MakeObsidian { .. } => false,
                                BotCommand::MineAbove => {
                                    let start_y = *state.mining_above_start_y.lock().unwrap();
                                    bot.position().ok().zip(start_y).is_some_and(
                                        |(position, start)| position.y.floor() as i32 > start,
                                    )
                                }
                                // 非轮询命令（Equip/Craft/Gather/Place/...）由下方执行块处理，
                                // 这里不能标记 done=true——否则会在执行前就清空 pending，
                                // 导致 do_equip/do_craft 等从未运行（bug 表现：equip 返回"命令完成"但主手没变）。
                                _ => false,
                            };
                        // 按命令类型超时（取代原硬编码 60 tick）
                        let timed_out_cmd = state.action_mgr.check_timeout(tick_now);
                        let timed_out = timed_out_cmd.is_some();
                        // P65：goto 伪到达看门狗。当 goto 目标其实是脚下实心方块，
                        // bot 原地判"到达"(distance<1.5) 却从未真正移动 → 反复重发相同 goto 死循环。
                        // 检测：同一目标"done"了 2 次但 bot 实际位置(从 last_position)未变 → 强制 mine_above 脱困。
                        let mut unstick_now = false;
                        if done && matches!(&qc.cmd, BotCommand::Goto { .. }) {
                            if let BotCommand::Goto { x, y, z } = &qc.cmd {
                                let mut wd = state.goto_watchdog.lock().unwrap();
                                let moved =
                                    state.last_position.lock().unwrap().map_or(true, |lp| {
                                        (lp.x - *x as f64).abs() > 1.0
                                            || (lp.y - *y as f64).abs() > 1.0
                                            || (lp.z - *z as f64).abs() > 1.0
                                    });
                                if !moved && *x == wd.0 && *y == wd.1 && *z == wd.2 {
                                    wd.3 += 1;
                                } else {
                                    *wd = (*x, *y, *z, 0);
                                }
                                if wd.3 >= 2 {
                                    *wd = (0, 0, 0, 0);
                                    // 地下 → 自动转 mine_above；地表 → 也强制上挖一层绕开实心目标
                                    if bot.position().map_or(true, |p| (p.y.floor() as i32) < 62) {
                                        *state.mining_above.lock().unwrap() = true;
                                        *state.mining_above_start_y.lock().unwrap() =
                                            Some(bot.position().map_or(0, |p| p.y.floor() as i32));
                                        *state.mining_above_direction.lock().unwrap() = 0;
                                        *state.mine_above_tried_tp.lock().unwrap() = false;
                                        bot.force_stop_pathfinding();
                                        if let Some(tx) = &qc.result_tx {
                                            let _ = tx.send(
                                                "Action output:\ngoto 反复'到达'但 bot 未移动（目标可能是脚下实心方块）。已自动转 mine_above 向上挖出脱困。".to_string(),
                                            );
                                        }
                                        state.action_mgr.clear_pending();
                                        unstick_now = true;
                                    }
                                }
                            }
                        }
                        if unstick_now {
                            // 已自行处理：强制脱困并清空 pending，跳过下方 result_msg 生成。
                        } else if done || timed_out {
                            // 统一用 Mindcraft 风格 "Action output:\n..." 让 LLM 看到一致的反馈。
                            let result_msg = match &qc.cmd {
                                BotCommand::Goto { x, y, z } if done => {
                                    let (cx, cy, cz) = bot
                                        .position()
                                        .ok()
                                        .map(|p| (p.x, p.y, p.z))
                                        .unwrap_or((0.0, 0.0, 0.0));
                                    let dist = ((cx - *x as f64).powi(2)
                                        + (cy - *y as f64).powi(2)
                                        + (cz - *z as f64).powi(2))
                                    .sqrt();
                                    format!(
                                        "Action output:\nArrived at ({},{},{}). Distance traveled: {:.1}m. Current pos: ({:.0},{:.0},{:.0}).",
                                        x, y, z, dist, cx, cy, cz
                                    )
                                }
                                BotCommand::Goto { x, y, z } => {
                                    // P66 修复：bot 反复 goto 相邻空气块却都 empty path 超时——
                                    // 无论目标坐标怎么变（LLM 每次微调），本质都是"原地导航失败"。
                                    // 改用"净移动"判定：连续 goto 超时且 bot 净移动 <1.5 格即累计 stall，
                                    // 达 3 次强制脱困 + 冷却当前格子，彻底打破 goto 洪泛（pi-agent 自主止损）。
                                    let mut wd = state.goto_watchdog.lock().unwrap();
                                    let (lx, ly, lz, _stall) = *wd;
                                    let cur = bot.position().ok().map(|p| {
                                        (p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32)
                                    });
                                    let moved = cur.map_or(true, |(cx, cy, cz)| {
                                        ((cx - lx).abs() as f64 > 1.5)
                                            || ((cy - ly).abs() as f64 > 1.5)
                                            || ((cz - lz).abs() as f64 > 1.5)
                                    });
                                    if moved {
                                        *wd = (
                                            cur.unwrap_or((0, 0, 0)).0,
                                            cur.unwrap_or((0, 0, 0)).1,
                                            cur.unwrap_or((0, 0, 0)).2,
                                            0,
                                        );
                                    } else {
                                        wd.3 += 1;
                                    }
                                    let stall_count = wd.3;
                                    drop(wd);
                                    if stall_count >= 3 {
                                        // 重置并冷却当前格子 15s（300 tick）：期间任何 goto 直接拒绝。
                                        *state.goto_watchdog.lock().unwrap() = (0, 0, 0, 0);
                                        if let Some((cx, cy, cz)) = cur {
                                            state.goto_cooldown.lock().unwrap().insert(
                                                (cx, cy, cz),
                                                bot.ticks_connected() as u64 + 300,
                                            );
                                        }
                                        // 脱困：地下→mine_above；地表→挖开目标阻挡方块（若 solid）或向上挖一层
                                        if let Ok(p) = bot.position() {
                                            if (p.y.floor() as i32) < 62 {
                                                *state.mining_above.lock().unwrap() = true;
                                                *state.mining_above_start_y.lock().unwrap() =
                                                    Some(p.y.floor() as i32);
                                                *state.mining_above_direction.lock().unwrap() = 0;
                                                *state.mine_above_tried_tp.lock().unwrap() = false;
                                            } else if let Ok(world) = bot.world() {
                                                let world = world.read();
                                                // 挖开目标方块（若非空气）和脚下/身旁可能阻挡的方块
                                                for (bx, by, bz) in [
                                                    (*x, *y, *z),
                                                    (*x, *y - 1, *z),
                                                    (*x, *y + 1, *z),
                                                ] {
                                                    if let Some(bs) = world
                                                        .get_block_state(BlockPos::new(bx, by, bz))
                                                    {
                                                        if !bs.is_air() {
                                                            bot.start_mining(BlockPos::new(
                                                                bx, by, bz,
                                                            ));
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                        bot.force_stop_pathfinding();
                                        format!(
                                            "Action output:\ngoto 已连续 {} 次超时且你几乎没移动——这是导航死循环！已强制停止并冷却当前位置 15s。\
                                             请：1) perceive 看清四周，用 mine 挖开挡路的实心方块再走；\
                                             2) 或 mine_above 上到地表开阔处再 goto；3) 不要重复 goto 旁边同一片区域。",
                                            stall_count
                                        )
                                    } else if let Ok(p) = bot.position() {
                                        if (p.y.floor() as i32) < 62 {
                                            *state.mining_above.lock().unwrap() = true;
                                            *state.mining_above_start_y.lock().unwrap() =
                                                Some(p.y.floor() as i32);
                                            *state.mining_above_direction.lock().unwrap() = 0;
                                            *state.mine_above_tried_tp.lock().unwrap() = false;
                                            bot.force_stop_pathfinding();
                                            format!(
                                                "Action output:\ngoto ({},{},{}) 超时——bot 在地下口袋里被挡住（Y={:.0}）。已自动转为 mine_above 向上挖出脱困，到地表后请用 goto 重试目标。",
                                                x, y, z, p.y
                                            )
                                        } else {
                                            // P69a：goto 超时（地表）自动清障——树冠/密林/山体地形下
                                            // pathfinder 找不到路（empty path），LLM 换坐标也白搭。
                                            // 挖开 bot 周围挡路的实心方块（树干/树叶/石头），每格让
                                            // pathfinder 多一条路。黑名单保护容器/工作台等设施不挖。
                                            // 借鉴 Mineflayer pathfinder 的 dig 模式。
                                            let mut cleared = 0u32;
                                            if let (Ok(bp), Ok(world)) =
                                                (bot.position(), bot.world())
                                            {
                                                let bx = bp.x.floor() as i32;
                                                let by = bp.y.floor() as i32;
                                                let bz = bp.z.floor() as i32;
                                                let no_dig = |bk: &BlockKind| {
                                                    matches!(
                                                        bk,
                                                        BlockKind::Chest
                                                            | BlockKind::CraftingTable
                                                            | BlockKind::Furnace
                                                            | BlockKind::BlastFurnace
                                                            | BlockKind::Smoker
                                                            | BlockKind::Barrel
                                                            | BlockKind::Anvil
                                                            | BlockKind::EnchantingTable
                                                            | BlockKind::BrewingStand
                                                            | BlockKind::Bedrock
                                                    )
                                                };
                                                for (dx, dz) in [
                                                    (0, 1),
                                                    (0, -1),
                                                    (1, 0),
                                                    (-1, 0),
                                                    (1, 1),
                                                    (1, -1),
                                                    (-1, 1),
                                                    (-1, -1),
                                                ] {
                                                    if cleared >= 3 {
                                                        break;
                                                    }
                                                    for dy in [0i32, 1] {
                                                        let pos =
                                                            BlockPos::new(bx + dx, by + dy, bz + dz);
                                                        let solid = world
                                                            .read()
                                                            .get_block_state(pos)
                                                            .map(|b| !b.is_air())
                                                            .unwrap_or(false);
                                                        if solid {
                                                            let bk: BlockKind = world
                                                                .read()
                                                                .get_block_state(pos)
                                                                .unwrap()
                                                                .into();
                                                            if no_dig(&bk) {
                                                                break;
                                                            }
                                                            bot.start_mining(pos);
                                                            cleared += 1;
                                                            break;
                                                        }
                                                    }
                                                }
                                            }
                                            if cleared > 0 {
                                                format!(
                                                    "Action output:\ngoto ({},{},{}) 超时——路径被阻（地表）。已自动挖开 {} 个挡路方块开道，稍后请重试 goto 同一目标。",
                                                    x, y, z, cleared
                                                )
                                            } else {
                                                format!(
                                                    "Action output:\ngoto ({},{},{}) 超时——路径被阻或目标不可达（地表）。perceive 确认位置后改用更近的中间点重试（已第 {} 次净不动，连 3 次将强制停止）。",
                                                    x, y, z, stall_count
                                                )
                                            }
                                        }
                                    } else {
                                        format!(
                                            "Action output:\ngoto ({},{},{}) 超时——路径被阻或目标不可达。",
                                            x, y, z
                                        )
                                    }
                                }
                                BotCommand::Mine { x, y, z } if done => {
                                    let (cx, cy, cz) = bot
                                        .position()
                                        .ok()
                                        .map(|p| (p.x, p.y, p.z))
                                        .unwrap_or((0.0, 0.0, 0.0));
                                    // P57：目标方块已是空气（可能是之前就挖掉了）→ 明确告知，
                                    // 避免 LLM 反复 mine 同一坐标（实测死循环：9 次连续 mine 同一格）。
                                    let target_is_air = bot
                                        .world()
                                        .ok()
                                        .map(|w| {
                                            w.read()
                                                .get_block_state(BlockPos::new(*x, *y, *z))
                                                .map(|b| b.is_air())
                                                .unwrap_or(true)
                                        })
                                        .unwrap_or(true);
                                    if target_is_air {
                                        // P71：拒绝时附上附近最近的实心方块建议坐标——LLM 经常盲猜
                                        // 坐标连续挖空气（实测 10 次连续 mine 空气死循环），
                                        // 直接给出可挖目标比让它自己 perceive 猜更高效。
                                        let mut suggestions: Vec<(i32, i32, i32)> = Vec::new();
                                        if let Ok(world) = bot.world() {
                                            'outer: for d in 1i32..=4 {
                                                for dx in -d..=d {
                                                    for dz in -d..=d {
                                                        for dy in -1..=2 {
                                                            if dx.abs() != d
                                                                && dz.abs() != d
                                                                && dy != -1
                                                                && dy != 2
                                                            {
                                                                continue;
                                                            }
                                                            let pos = BlockPos::new(
                                                                x + dx,
                                                                y + dy,
                                                                z + dz,
                                                            );
                                                            let bk: Option<BlockKind> = world
                                                                .read()
                                                                .get_block_state(pos)
                                                                .map(|b| b.into());
                                                            let solid = bk.map(|k| {
                                                                k != BlockKind::Air
                                                                    && k != BlockKind::Water
                                                                    && k != BlockKind::Lava
                                                            }).unwrap_or(false);
                                                            if solid {
                                                                suggestions.push((
                                                                    x + dx,
                                                                    y + dy,
                                                                    z + dz,
                                                                ));
                                                                if suggestions.len() >= 4 {
                                                                    break 'outer;
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                        let hint = if suggestions.is_empty() {
                                            "附近 4 格内无实心方块，请先 perceive 确认位置。"
                                                .to_string()
                                        } else {
                                            format!(
                                                "附近最近的实心方块（可挖）：{}。",
                                                suggestions
                                                    .iter()
                                                    .map(|(sx, sy, sz)| format!("({sx},{sy},{sz})"))
                                                    .collect::<Vec<_>>()
                                                    .join(", ")
                                            )
                                        };
                                        format!(
                                            "Action output:\nmine ({},{},{}): 该位置已是空气/方块不存在（可能之前已挖掉或坐标错误）。{hint}\
                                             直接 mine 上述坐标即可。",
                                            x, y, z
                                        )
                                    } else {
                                        format!(
                                            "Action output:\nMined block at ({},{},{}). Block removed. Bot still at ({:.0},{:.0},{:.0}) — 挖完不会自动掉进洞，无需 goto 刚挖的位置。",
                                            x, y, z, cx, cy, cz
                                        )
                                    }
                                }
                                BotCommand::Mine { x, y, z } => {
                                    format!(
                                        "Action output:\nmine ({},{},{}) 超时——可能方块太硬（需更高品质镐）或距离太远。建议 gather(item=..., count=...) 自动寻路挖掘。",
                                        x, y, z
                                    )
                                }
                                BotCommand::MineAbove if done => {
                                    let y = bot
                                        .position()
                                        .ok()
                                        .map(|position| position.y.floor() as i32)
                                        .unwrap_or_default();
                                    format!(
                                        "Action output:\nMineAbove progressed to Y={y}. Call mine_above again to continue toward the surface."
                                    )
                                }
                                BotCommand::MineAbove => {
                                    *state.mining_above.lock().unwrap() = false;
                                    *state.mining_above_start_y.lock().unwrap() = None;
                                    bot.force_stop_pathfinding();
                                    "Action output:\nmine_above failed: Y did not increase within 10 seconds. The ascent path is blocked; perceive and clear a horizontal staircase before retrying."
                                        .to_string()
                                }
                                BotCommand::Gather { item, .. } => {
                                    // P3：gather 超时时，采集 future 仍在后台运行（无法取消），
                                    // 实际可能已经/即将完成。让 LLM 用 perceive 确认背包，
                                    // 而不是直接重调 gather（会重复采集）。
                                    format!(
                                        "Action output:\ngather {item} 超时（ActionManager 120s 阈值）。\
                                    采集可能仍在后台进行——下一步请先 perceive 检查背包 {item} 数量，\
                                    若已满足需求就不要重调 gather；若确实不够，再 gather 补足差额。"
                                    )
                                }
                                BotCommand::Craft3x3 { item, .. } => {
                                    format!(
                                        "Action output:\ncraft_3x3 {item} 超时——可能工作台路径卡住或合成 UI 响应慢。\
                                    建议 perceive 确认背包是否有 {item}，若无再重试。"
                                    )
                                }
                                BotCommand::Smelt { output, .. } => {
                                    format!(
                                        "Action output:\nsmelt {output} 超时——熔炼本质慢。\
                                    建议 perceive 确认背包是否有 {output}，若无再重试。"
                                    )
                                }
                                _ if done => "Action output:\n命令完成".to_string(),
                                _ => "Action output:\n命令超时".to_string(),
                            };
                            if let Some(tx) = &qc.result_tx {
                                let _ = tx.send(result_msg);
                            }
                            if matches!(&qc.cmd, BotCommand::MineAbove) {
                                *state.mining_above_start_y.lock().unwrap() = None;
                            }
                            state.action_mgr.clear_pending();
                        }
                    }
                    // 取走 ActionManager 的快循环警告（若有则推到事件流供 Agent 注入）
                    if let Some(nudge) = state.action_mgr.take_loop_nudge() {
                        let _ = evt_tx.send(BotEvent::Chat { content: nudge });
                    }
                }
                // 取当前要执行的命令：pending 里的命令每 tick 都（重）执行其 start，
                // 非阻塞命令（Goto/Mine）重复 start 是幂等的（重设同一目标），由
                // cmd_finished 轮询完成；MineBelow 在 arm 内清空中途槽。
                // 异步命令（Craft/Gather 等）执行期间 busy=true，下一 tick 跳过避免重入。
                let to_run: Option<(BotCommand, Option<std::sync::mpsc::Sender<String>>)> = {
                    if state.action_mgr.is_busy() {
                        None
                    } else if let Some(qc) = state.action_mgr.peek_pending() {
                        let is_polling = matches!(
                            &qc.cmd,
                            BotCommand::Goto { .. }
                                | BotCommand::Mine { .. }
                                | BotCommand::MineBelow
                                | BotCommand::MineAbove
                        );
                        if !is_polling {
                            state.action_mgr.set_busy(true);
                        }
                        Some((qc.cmd.clone(), qc.result_tx.clone()))
                    } else {
                        None
                    }
                };
                if let Some((cmd, result_tx)) = to_run {
                    match cmd {
                        BotCommand::Goto { x, y, z } => {
                            *state.mining_below.lock().unwrap() = false;
                            // P66：冷却拦截。按 bot 当前格子检查冷却（而非目标坐标，
                            // 因为 LLM 会微调目标逃避同一坐标冷却）。在冷却期内任何 goto 直接拒绝，
                            // 强制 LLM/脚本换策略（挖开阻挡或上地表），打破 goto 洪泛。
                            {
                                if let Ok(p) = bot.position() {
                                    let cell = (
                                        p.x.floor() as i32,
                                        p.y.floor() as i32,
                                        p.z.floor() as i32,
                                    );
                                    let cd = state.goto_cooldown.lock().unwrap();
                                    if let Some(&until) = cd.get(&cell) {
                                        if until > bot.ticks_connected() as u64 {
                                            if let Some(tx) = &result_tx {
                                                let _ = tx.send(format!(
                                                    "Action output:\ngoto ({},{},{}) 被拒绝——你当前位置仍在导航冷却中（之前连续 goto 超时且没移动）。\
                                                     请改用 mine 挖开挡路方块，或 mine_above 上到地表开阔处，不要继续 goto 旁边区域。",
                                                    x, y, z
                                                ));
                                            }
                                            state.action_mgr.clear_pending();
                                            return bot;
                                        }
                                    }
                                }
                            }
                            // 距离限制：>32 格的 goto 拒绝执行，让 LLM 拆成多段。
                            // 原因：azalea pathfinder 的 A* 在长距离/复杂地形上计算量大，
                            // 每 tick 发 MovePlayerPos+PlayerInput 包会拖死 vanilla 服 TPS，
                            // 导致同服真实玩家 WASD 输入丢失（服务器来不及处理）。
                            let p = bot.position().ok();
                            if let Some(p) = p {
                                let dist = ((p.x - x as f64).powi(2)
                                    + (p.y - y as f64).powi(2)
                                    + (p.z - z as f64).powi(2))
                                .sqrt();
                                if dist > 32.0 {
                                    if let Some(tx) = &result_tx {
                                        let _ = tx.send(format!(
                                        "Action output:\ngoto ({},{},{}) 距离 {:.0}m 过远（>32m），\
                                         请拆成多段：先 goto 中间点（距当前 16-24m），到达后再 goto 目标。",
                                        x, y, z, dist
                                    ));
                                    }
                                    state.action_mgr.clear_pending();
                                    return bot;
                                }
                                // P65 修复：goto 目标是实心方块（脚下/身旁矿脉）时，bot 站旁边即被判
                                // "到达"(distance<1.5) 却永远挖不进/进不去 → 反复 goto 同一坐标死循环。
                                // 检测目标方块是否 solid：solid 则直接拒绝并（地下时）自动 mine_above 脱困。
                                let target_solid = if let Ok(world) = bot.world() {
                                    let world = world.read();
                                    world
                                        .get_block_state(BlockPos::new(x, y, z))
                                        .map(|b| !b.is_air())
                                        .unwrap_or(false)
                                } else {
                                    false
                                };
                                if target_solid {
                                    // P69b：目标实心（树干/树叶/山体/树冠）时不再直接拒绝——
                                    // LLM 在密林里看不见地面，经常选到树冠/树干坐标。
                                    // 自动向上找最近的可站立空气点，修正目标继续前往。
                                    // 若上方 8 格全是实心（如地下岩体）才走原拒绝逻辑。
                                    let mut fallback: Option<(i32, i32, i32)> = None;
                                    if let Ok(world) = bot.world() {
                                        for k in 1..=8 {
                                            let up = BlockPos::new(x, y + k, z);
                                            let is_air = world
                                                .read()
                                                .get_block_state(up)
                                                .map(|b| b.is_air())
                                                .unwrap_or(false);
                                            if is_air {
                                                fallback = Some((x, y + k, z));
                                                break;
                                            }
                                        }
                                    }
                                    if let Some((fx, fy, fz)) = fallback {
                                        if let Some(tx) = &result_tx {
                                            let _ = tx.send(format!(
                                                "Action output:\ngoto ({},{},{}) 目标方块是实心（树干/树叶/山体），已自动修正为上方可站立点 ({},{},{}) 继续前往。",
                                                x, y, z, fx, fy, fz
                                            ));
                                        }
                                        bot.start_goto(BlockPosGoal(BlockPos::new(fx, fy, fz)));
                                        state.action_mgr.clear_pending();
                                        return bot;
                                    }
                                    if let Some(tx) = &result_tx {
                                        let _ = tx.send(format!(
                                            "Action output:\ngoto ({},{},{}) 失败——目标方块是实心方块（不能站在里面）。请改用附近的空气方块坐标，或若在地下请用 mine_above 向上挖出脱困。",
                                            x, y, z
                                        ));
                                    }
                                    if (p.y as i32) < 62 {
                                        *state.mining_above.lock().unwrap() = true;
                                        *state.mining_above_start_y.lock().unwrap() =
                                            Some(p.y.floor() as i32);
                                        *state.mining_above_direction.lock().unwrap() = 0;
                                        *state.mine_above_tried_tp.lock().unwrap() = false;
                                        bot.force_stop_pathfinding();
                                    }
                                    state.action_mgr.clear_pending();
                                    return bot;
                                }
                            }
                            // P60: 自动挖回地表后再 goto——当 bot 在地下时，pathfinder 无法穿墙导航
                            // 先检测是否在地下，如果是，优先挖回地表再执行 goto
                            let mut needs_surface = false;
                            if let Ok(p) = bot.position() {
                                if (p.y as i32) < 62 {
                                    needs_surface = true;
                                } else {
                                    // 检查头上有无方块（可能在洞穴/室内）
                                    if let Ok(world) = bot.world() {
                                        let world = world.read();
                                        let head_pos = BlockPos::new(
                                            p.x.floor() as i32,
                                            p.y.floor() as i32 + 1,
                                            p.z.floor() as i32,
                                        );
                                        if let Some(head_block) = world.get_block_state(head_pos) {
                                            let bk: azalea_registry::builtin::BlockKind =
                                                head_block.into();
                                            if bk != azalea_registry::builtin::BlockKind::Air {
                                                needs_surface = true;
                                            }
                                        }
                                    }
                                }
                            }
                            if needs_surface {
                                // 启动自动挖回地表模式（handler 的 tick 循环会处理持续上挖）
                                *state.mining_above.lock().unwrap() = true;
                                // 快速返回：让 handler 在接下来的 tick 中执行 mine_above
                                // 同时启动一个延迟检查：当 Y>=62 时自动执行原始 goto
                                // 注：goto 目标坐标保存在 pending cmd 中，不会被清除
                                if let Some(tx) = &result_tx {
                                    let _ = tx.send(format!(
                                        "Action output:
goto ({},{},{}) ——bot 在地下，先自动挖回地表。mine_above 已启动。",
                                        x, y, z
                                    ));
                                }
                                state.action_mgr.clear_pending();
                                return bot;
                            }
                            // P59: 快速可达性检测——检查目标是否在同一 Y 层被实心方块包围
                            if let Ok(p) = bot.position() {
                                let dy = (y as f64 - p.y).abs();
                                let dxz =
                                    ((p.x - x as f64).powi(2) + (p.z - z as f64).powi(2)).sqrt();
                                if dxz < 5.0 && dy < 2.0 {
                                    if let Ok(world) = bot.world() {
                                        let world = world.read();
                                        let head_pos = BlockPos::new(
                                            p.x.floor() as i32,
                                            p.y.floor() as i32 + 1,
                                            p.z.floor() as i32,
                                        );
                                        if let Some(head_block) = world.get_block_state(head_pos) {
                                            let bk: azalea_registry::builtin::BlockKind =
                                                head_block.into();
                                            if bk != azalea_registry::builtin::BlockKind::Air
                                                || (p.y as i32) < 62
                                            {
                                                let reason = if (p.y as i32) < 62 {
                                                    format!(
                                                        "bot 当前 Y={} 在地下（Y<62）。",
                                                        p.y as i32
                                                    )
                                                } else {
                                                    "bot 头上有方块（可能在地下）。".to_string()
                                                };
                                                if let Some(tx) = &result_tx {
                                                    let _ = tx.send(format!(
                                                        "Action output:
goto ({},{},{}) 失败——{}
必须先用 mine_above() 挖回地表（Y>=62），才能用 goto 导航。",
                                                        x, y, z, reason
                                                    ));
                                                }
                                                state.action_mgr.clear_pending();
                                                return bot;
                                            }
                                        }
                                    }
                                }
                            }
                            // P59: 快速可达性检测——检查目标是否在同一 Y 层被实心方块包围
                            if let Ok(p) = bot.position() {
                                let dy = (y as f64 - p.y).abs();
                                let dxz =
                                    ((p.x - x as f64).powi(2) + (p.z - z as f64).powi(2)).sqrt();
                                if dxz < 5.0 && dy < 2.0 {
                                    if let Ok(world) = bot.world() {
                                        let world = world.read();
                                        let head_pos = BlockPos::new(
                                            p.x.floor() as i32,
                                            p.y.floor() as i32 + 1,
                                            p.z.floor() as i32,
                                        );
                                        if let Some(head_block) = world.get_block_state(head_pos) {
                                            let bk: azalea_registry::builtin::BlockKind =
                                                head_block.into();
                                            if bk != azalea_registry::builtin::BlockKind::Air {
                                                if let Some(tx) = &result_tx {
                                                    let _ = tx.send(format!(
                                                        "Action output:
goto ({},{},{}) 失败——bot 头上有方块（可能在地下）。
先用 perceive 确认位置，若 Y<62 说明在地下，需用 mine_above 挖回地表。",
                                                        x, y, z
                                                    ));
                                                }
                                                state.action_mgr.clear_pending();
                                                return bot;
                                            }
                                        }
                                    }
                                }
                            }
                            // P59: 快速可达性检测——检查目标是否在同一 Y 层被实心方块包围
                            if let Ok(p) = bot.position() {
                                let dy = (y as f64 - p.y).abs();
                                let dxz =
                                    ((p.x - x as f64).powi(2) + (p.z - z as f64).powi(2)).sqrt();
                                if dxz < 5.0 && dy < 2.0 {
                                    if let Ok(world) = bot.world() {
                                        let world = world.read();
                                        let head_pos = BlockPos::new(
                                            p.x.floor() as i32,
                                            p.y.floor() as i32 + 1,
                                            p.z.floor() as i32,
                                        );
                                        if let Some(head_block) = world.get_block_state(head_pos) {
                                            let bk: azalea_registry::builtin::BlockKind =
                                                head_block.into();
                                            if bk != azalea_registry::builtin::BlockKind::Air {
                                                if let Some(tx) = &result_tx {
                                                    let _ = tx.send(format!(
                                                        "Action output:
goto ({},{},{}) 失败——bot 头上有方块（可能在地下）。
先用 perceive 确认位置，若 Y<62 说明在地下，需用 mine_above 挖回地表。",
                                                        x, y, z
                                                    ));
                                                }
                                                state.action_mgr.clear_pending();
                                                return bot;
                                            }
                                        }
                                    }
                                }
                            }
                            bot.start_goto(BlockPosGoal(BlockPos::new(x, y, z)));
                        }
                        BotCommand::Mine { x, y, z } => {
                            *state.mining_below.lock().unwrap() = false;
                            // P5 修复：挖矿前自动装备最好的镐。否则 bot 拿面包挖石头
                            // 既慢又不掉落物，且 LLM 不会主动 equip（挖矿工具隐含前提）。
                            let _ = auto_equip_best_pickaxe(&bot).await;
                            bot.start_mining(BlockPos::new(x, y, z));
                        }
                        BotCommand::MineBelow => {
                            *state.mining_below.lock().unwrap() = true;
                            // 同 Mine：下挖也要装备镐
                            let _ = auto_equip_best_pickaxe(&bot).await;
                            if let Ok(p) = bot.position() {
                                let foot = BlockPos::new(
                                    p.x.floor() as i32,
                                    (p.y - 1.0).floor() as i32,
                                    p.z.floor() as i32,
                                );
                                bot.start_mining(foot);
                            }
                            if let Some(tx) = &result_tx {
                                let _ = tx.send("已开始向下挖掘".to_string());
                            }
                            state.action_mgr.clear_pending();
                        }
                        BotCommand::MakeObsidian { count } => {
                            // P67：初始化造黑曜石状态机 (remaining, phase, obsidian_pos)。
                            // 注意：tick handler 内严禁 await（会冻结整个事件循环导致 120s 超时）。
                            // 装备 bucket / 装水 / 找岩浆全部在状态机内每 tick 同步推进，不做任何 .await。
                            *state.make_obsidian.lock().unwrap() = Some((count.max(1), 0, None));
                            // 立即回报"已开始"，让工具层不阻塞等待（真正的完成由状态机结束帧回报）。
                            if let Some(tx) = &result_tx {
                                let _ = tx.send(format!(
                                    "已开始造黑曜石 x{}：状态机会自动装备水桶、找水源装水、再找岩浆造黑曜石。",
                                    count
                                ));
                            }
                            state.action_mgr.clear_pending();
                        }
                        BotCommand::MineAbove => {
                            // P5 新增：向上挖脱困。从 bot 头顶逐格挖到空气或达到 64 格上限。
                            // 持续触发模式（同 MineBelow）：mining_above 标志位驱动后续 tick 重复发起。
                            *state.mining_below.lock().unwrap() = false;
                            // P5 修复：原代码无脑要求"必须有镐"，但 dirt/grass/sand/gravel/sandstone
                            // 等软方块徒手就能挖。只有挖 stone/deepslate/ores 等硬方块才必须用镐。
                            // 现在改为：先看头顶方块类型，软方块直接挖；硬方块才检查镐。
                            let head_pos = bot.position().ok().map(|p| {
                                BlockPos::new(
                                    p.x.floor() as i32,
                                    (p.y + 1.0).floor() as i32,
                                    p.z.floor() as i32,
                                )
                            });
                            let head_state = head_pos.and_then(|pos| {
                                let world = bot.world().ok()?;
                                let world = world.read();
                                world.get_block_state(pos)
                            });
                            let head_is_air = head_state.is_some_and(|block| block.is_air());
                            // Surface pre-check: if already on surface (Y>=62 + air column),
                            // return immediately instead of starting 10s timeout.
                            if head_is_air {
                                if let Ok(p) = bot.position() {
                                    let y = p.y.floor() as i32;
                                    if y >= 62 {
                                        if let Ok(world) = bot.world() {
                                            let cx = p.x.floor() as i32;
                                            let cz = p.z.floor() as i32;
                                            let world = world.read();
                                            let mut five_air = true;
                                            for dy in 1..=5 {
                                                let check = BlockPos::new(cx, y + dy, cz);
                                                let is_air = world
                                                    .get_block_state(check)
                                                    .map(|s| s.is_air())
                                                    .unwrap_or(false);
                                                if !is_air {
                                                    five_air = false;
                                                    break;
                                                }
                                            }
                                            drop(world);
                                            if mine_above_reached_surface(y, true, five_air) {
                                                if let Some(tx) = &result_tx {
                                                    let _ = tx.send(format!(
                                                        "Action output:\nMineAbove done at Y={y} (已到地表，头顶是空气)。当前坐标 ({:.0},{y},{:.0})，可继续探索。",
                                                        p.x, p.z
                                                    ));
                                                }
                                                *state.mining_above.lock().unwrap() = false;
                                                state.action_mgr.clear_pending();
                                                return bot;
                                            }
                                        }
                                    }
                                }
                            }
                            let head_is_hard = head_state.map(is_hard_block).unwrap_or(true); // 不确定时按硬方块处理
                            if head_is_hard {
                                let has_pick = has_any_pickaxe_in_inventory(&bot).await;
                                if !has_pick {
                                    if let Some(tx) = &result_tx {
                                        let _ = tx.send(
                                        "Action output:\n❌ mine_above 失败：头顶是硬方块（石头/深板岩/矿石等）且背包里没有镐！\
                                         徒手挖硬方块极慢（~8秒/格）且不掉落。\
                                         建议：(1) chat(\"/tp @s ~ 70 ~\") 用命令传送到地表（需 cheats）；\
                                         (2) 横向 mine 看是否有 dirt/gravel 软方块通道；\
                                         (3) 先 craft 一个 wooden_pickaxe 再 mine_above。"
                                            .to_string(),
                                    );
                                    }
                                    *state.mining_above.lock().unwrap() = false;
                                    state.action_mgr.clear_pending();
                                    return bot;
                                }
                            }
                            let was_active = *state.mining_above.lock().unwrap();
                            *state.mining_above.lock().unwrap() = true;
                            if !was_active && let Ok(position) = bot.position() {
                                *state.mining_above_start_y.lock().unwrap() =
                                    Some(position.y.floor() as i32);
                                *state.mining_above_direction.lock().unwrap() = 0;
                            }
                            let _ = auto_equip_best_pickaxe(&bot).await;
                            if !head_is_air
                                && let Some(pos) = head_pos
                                && !bot.is_mining()
                            {
                                bot.start_mining(pos);
                            }
                        }
                        BotCommand::BlockInteract { x, y, z } => {
                            *state.mining_below.lock().unwrap() = false;
                            bot.block_interact(BlockPos::new(x, y, z));
                            if let Some(tx) = &result_tx {
                                let _ = tx.send(format!("已交互 ({},{},{})", x, y, z));
                            }
                        }
                        BotCommand::Chat { content } => {
                            bot.chat(&content);
                            if let Some(tx) = &result_tx {
                                let _ = tx.send(format!("Action output:\nSent chat: {content}"));
                            }
                        }
                        BotCommand::Attack { target } => {
                            if let Ok(entities) =
                            bot.nearest_entities::<bevy_ecs::query::Without<azalea::entity::metadata::Player>>()
                        {
                            let self_id = bot.entity().id();
                            let requested = normalize_entity_target(&target);
                            // 记录攻击前的生命，便于反馈损血
                            let health_before = bot.health().unwrap_or(20.0);
                            let mut hit_kind: Option<String> = None;
                            let mut nearest_match: Option<(i32, i32, i32, f64)> = None;
                            for e in entities.iter() {
                                if e.id() == self_id { continue; }
                                let Ok(kind) = e.kind() else { continue; };
                                let kind = entity_kind_name(kind);
                                if requested != "nearest" && requested != "chat" && kind != requested {
                                    continue;
                                }
                                if matches!(kind.as_str(), "item" | "experience_orb" | "item_frame" | "glow_item_frame") {
                                    continue;
                                }
                                let Ok(distance) = e.distance_to_client() else { continue; };
                                if nearest_match.is_none()
                                    && let Ok(position) = e.position()
                                {
                                    nearest_match = Some((
                                        position.x.floor() as i32,
                                        position.y.floor() as i32,
                                        position.z.floor() as i32,
                                        distance,
                                    ));
                                }
                                if distance > 4.5 {
                                    continue;
                                }
                                let indexed = bot
                                    .query_self::<&azalea::entity::indexing::EntityIdIndex, _>(|index| {
                                        index.contains_ecs_entity(e.id())
                                    })
                                    .unwrap_or(false);
                                if !indexed {
                                    continue;
                                }
                                e.attack();
                                hit_kind = Some(kind);
                                break;
                            }
                            let health_after = bot.health().unwrap_or(20.0);
                            let msg = match hit_kind {
                                Some(k) => {
                                    let dmg = (health_before - health_after).max(0.0);
                                    if dmg > 0.0 {
                                        format!("Action output:\nAttacked {k}. Took {dmg:.0} damage. Health: {health_after:.0}/20.")
                                    } else {
                                        format!("Action output:\nAttacked {k}. Health: {health_after:.0}/20.")
                                    }
                                }
                                None => match nearest_match {
                                    Some((x, y, z, distance)) => {
                                        // P76：远处实体攻击失败时按类型引导——LLM 曾连续 3+ 回合
                                        // 在树冠上追远处僵尸（11-22m），全 wasted（每回合 30-60s）。
                                        let is_hostile = matches!(
                                            requested.as_str(),
                                            "zombie"
                                                | "skeleton"
                                                | "creeper"
                                                | "spider"
                                                | "cave_spider"
                                                | "enderman"
                                                | "pillager"
                                                | "phantom"
                                                | "witch"
                                                | "drowned"
                                                | "husk"
                                                | "stray"
                                        );
                                        let guidance = if is_hostile {
                                            format!(
                                                "不要追击远处{requested}——追击引怪且浪费回合；远离它继续主线（如采集/合成/挖矿），它进入 4 格内时系统会自动反击。"
                                            )
                                        } else {
                                            format!(
                                                "动物在 {distance:.0}m 外：goto({x},{y},{z}) 靠近到 4 格内再 attack；动物会逃跑，靠近后立即攻击。"
                                            )
                                        };
                                        format!(
                                            "Action output:\nCould not attack {requested}: nearest match is {distance:.1} blocks away at ({x},{y},{z}). {guidance}"
                                        )
                                    }
                                    None => format!(
                                        "Action output:\nCould not find a valid {requested}. Use perceive to choose another action or flee if unsafe."
                                    ),
                                },
                            };
                            if let Some(tx) = &result_tx { let _ = tx.send(msg.clone()); }
                            let _ = evt_tx.send(BotEvent::Chat { content: msg });
                        }
                        }
                        BotCommand::Craft2x2 { item, count } => {
                            match crate::azalea::craft::do_craft_2x2(&bot, &item, count).await {
                                Ok(msg) => {
                                    let chat = format!("[合成] {msg}");
                                    let _ = evt_tx.send(BotEvent::Chat { content: chat });
                                    if let Some(tx) = &result_tx {
                                        let _ = tx.send(format!("Action output:\nSuccessfully crafted {item}, you now have it. ({msg})"));
                                    }
                                }
                                Err(e) => {
                                    let chat = format!("[合成失败] {e}");
                                    let _ = evt_tx.send(BotEvent::Chat { content: chat });
                                    if let Some(tx) = &result_tx {
                                        let _ = tx.send(format!(
                                            "Action output:\nFailed to craft {item}: {e}"
                                        ));
                                    }
                                }
                            }
                        }
                        BotCommand::Craft3x3 {
                            item,
                            count,
                            table_pos,
                        } => {
                            let hint = table_pos.map(|(x, y, z)| BlockPos::new(x, y, z));
                            // P1-4：自动放收桌流程（确保桌开 → 合成 → 关桌）
                            let table_open = crate::azalea::table_flow::ensure_table_open(
                                &bot,
                                "crafting_table",
                                hint,
                            )
                            .await;
                            let result = match table_open {
                                Ok(tp) => {
                                    let r = crate::azalea::craft::do_craft_3x3(
                                        &bot,
                                        &item,
                                        count,
                                        Some(tp),
                                    )
                                    .await;
                                    let _ =
                                        crate::azalea::table_flow::close_container_if_open(&bot);
                                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                                    r.map(|msg| {
                                        format!(
                                            "{msg}\n(桌位: ({},{},{}), 已自动关闭)",
                                            tp.x, tp.y, tp.z
                                        )
                                    })
                                }
                                Err(e) => Err(e),
                            };
                            match result {
                                Ok(msg) => {
                                    let chat = format!("[合成] {msg}");
                                    let _ = evt_tx.send(BotEvent::Chat { content: chat });
                                    if let Some(tx) = &result_tx {
                                        let _ = tx.send(format!("Action output:\nSuccessfully crafted {item}, you now have it. ({msg})"));
                                    }
                                }
                                Err(e) => {
                                    let chat = format!("[合成失败] {e}");
                                    let _ = evt_tx.send(BotEvent::Chat { content: chat });
                                    if let Some(tx) = &result_tx {
                                        let _ = tx.send(format!(
                                            "Action output:\nFailed to craft {item}: {e}"
                                        ));
                                    }
                                }
                            }
                        }
                        BotCommand::Smelt {
                            output,
                            fuel,
                            count,
                            table_pos,
                        } => {
                            let hint = table_pos.map(|(x, y, z)| BlockPos::new(x, y, z));
                            // P1-4：自动放收炉流程（确保炉开 → 熔炼 → 关炉）
                            let table_open =
                                crate::azalea::table_flow::ensure_table_open(&bot, "furnace", hint)
                                    .await;
                            let result = match table_open {
                                Ok(tp) => {
                                    let r =
                                        crate::azalea::craft::do_smelt(&bot, &output, &fuel, count)
                                            .await;
                                    let _ =
                                        crate::azalea::table_flow::close_container_if_open(&bot);
                                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                                    r.map(|msg| {
                                        format!(
                                            "{msg}\n(炉位: ({},{},{}), 已自动关闭)",
                                            tp.x, tp.y, tp.z
                                        )
                                    })
                                }
                                Err(e) => Err(e),
                            };
                            match result {
                                Ok(msg) => {
                                    let chat = format!("[熔炼] {msg}");
                                    let _ = evt_tx.send(BotEvent::Chat { content: chat });
                                    if let Some(tx) = &result_tx {
                                        let _ = tx.send(format!("Action output:\nSuccessfully smelted {output}, you now have it. ({msg})"));
                                    }
                                }
                                Err(e) => {
                                    let chat = format!("[熔炼失败] {e}");
                                    let _ = evt_tx.send(BotEvent::Chat { content: chat });
                                    if let Some(tx) = &result_tx {
                                        let _ = tx.send(format!(
                                            "Action output:\nFailed to smelt {output}: {e}"
                                        ));
                                    }
                                }
                            }
                        }
                        BotCommand::Gather { item, count } => {
                            // 用 smart_actions::collect_block_smart 替代 gather::do_gather：
                            // 支持别名展开（"oak_log" 匹配 9 种原木变体），多轮渐扩半径扫描。
                            match crate::azalea::smart_actions::collect_block_smart(
                                &bot, &item, count,
                            )
                            .await
                            {
                                Ok(msg) => {
                                    let chat = format!("[采集] {msg}");
                                    let _ = evt_tx.send(BotEvent::Chat { content: chat });
                                    if let Some(tx) = &result_tx {
                                        let _ = tx.send(format!(
                                            "Action output:\nSuccessfully gathered {item}, {msg}"
                                        ));
                                    }
                                }
                                Err(e) => {
                                    let chat = format!("[采集失败] {e}");
                                    let _ = evt_tx.send(BotEvent::Chat { content: chat });
                                    if let Some(tx) = &result_tx {
                                        let _ = tx.send(format!(
                                            "Action output:\nFailed to gather {item}: {e}"
                                        ));
                                    }
                                }
                            }
                        }
                        BotCommand::Place { item, x, y, z } => {
                            match crate::azalea::place::do_place(
                                &bot,
                                &item,
                                BlockPos::new(x, y, z),
                            )
                            .await
                            {
                                Ok(msg) => {
                                    let chat = format!("[放置] {msg}");
                                    let _ = evt_tx.send(BotEvent::Chat { content: chat });
                                    // P9 修复（2026-07-26）：do_place 返回的 msg 已包含实际放置坐标
                                    // （可能因自动重定位与 LLM 给的 x,y,z 不同）。原代码在外面包一层
                                    // "Placed {item} at ({x},{y},{z})" 用的是 LLM 原始坐标，导致 LLM
                                    // 记住错误坐标 → 后续 open(原始坐标) 必然失败。
                                    // 现在直接透传 msg，让 LLM 看到真实放置坐标。
                                    if let Some(tx) = &result_tx {
                                        let _ = tx.send(msg);
                                    }
                                }
                                Err(e) => {
                                    let chat = format!("[放置失败] {e}");
                                    let _ = evt_tx.send(BotEvent::Chat { content: chat });
                                    if let Some(tx) = &result_tx {
                                        let _ = tx.send(format!("Action output:\nFailed to place {item} at ({},{},{}): {e}", x, y, z));
                                    }
                                }
                            }
                        }
                        BotCommand::OpenContainer { x, y, z } => {
                            match crate::azalea::place::do_open_container(
                                &bot,
                                BlockPos::new(x, y, z),
                            )
                            .await
                            {
                                Ok(msg) => {
                                    let chat = format!("[开容器] {msg}");
                                    let _ = evt_tx.send(BotEvent::Chat { content: chat });
                                    if let Some(tx) = &result_tx {
                                        let _ = tx.send(format!("Action output:\nOpened container at ({},{},{}). ({msg})", x, y, z));
                                    }
                                }
                                Err(e) => {
                                    let chat = format!("[开容器失败] {e}");
                                    let _ = evt_tx.send(BotEvent::Chat { content: chat });
                                    if let Some(tx) = &result_tx {
                                        let _ = tx.send(format!("Action output:\nFailed to open container at ({},{},{}): {e}", x, y, z));
                                    }
                                }
                            }
                        }
                        BotCommand::AutoCraft { item, count } => {
                            match crate::azalea::auto_craft::do_auto_craft(&bot, &item, count).await
                            {
                                Ok(msg) => {
                                    let chat = format!("[自动合成] {msg}");
                                    let _ = evt_tx.send(BotEvent::Chat { content: chat });
                                    if let Some(tx) = &result_tx {
                                        let _ = tx.send(format!(
                                            "Action output:\nAuto-crafted {item}. ({msg})"
                                        ));
                                    }
                                }
                                Err(e) => {
                                    let chat = format!("[自动合成失败] {e}");
                                    let _ = evt_tx.send(BotEvent::Chat { content: chat });
                                    if let Some(tx) = &result_tx {
                                        let _ = tx.send(format!(
                                            "Action output:\nFailed to auto-craft {item}: {e}"
                                        ));
                                    }
                                }
                            }
                        }
                        BotCommand::Enchant { item, level } => {
                            match crate::azalea::craft::do_enchant(&bot, &item, level).await {
                                Ok(msg) => {
                                    let chat = format!("[附魔] {msg}");
                                    let _ = evt_tx.send(BotEvent::Chat { content: chat });
                                    if let Some(tx) = &result_tx {
                                        let _ = tx.send(format!("Action output:\nEnchanted {item} at level {level}. ({msg})"));
                                    }
                                }
                                Err(e) => {
                                    let chat = format!("[附魔失败] {e}");
                                    let _ = evt_tx.send(BotEvent::Chat { content: chat });
                                    if let Some(tx) = &result_tx {
                                        let _ = tx.send(format!(
                                            "Action output:\nFailed to enchant {item}: {e}"
                                        ));
                                    }
                                }
                            }
                        }
                        BotCommand::Trade { offer } => {
                            let ext = bot
                                .ecs
                                .read()
                                .resource::<crate::azalea::ext_state::BotExtResource>()
                                .0
                                .clone();
                            match crate::azalea::trade::do_trade(&bot, &ext, offer).await {
                                Ok(msg) => {
                                    let chat = format!("[交易] {msg}");
                                    let _ = evt_tx.send(BotEvent::Chat { content: chat });
                                    if let Some(tx) = &result_tx {
                                        let _ = tx.send(format!(
                                            "Action output:\nTrade offer {offer} completed. ({msg})"
                                        ));
                                    }
                                }
                                Err(e) => {
                                    let chat = format!("[交易失败] {e}");
                                    let _ = evt_tx.send(BotEvent::Chat { content: chat });
                                    if let Some(tx) = &result_tx {
                                        let _ = tx.send(format!(
                                            "Action output:\nFailed to trade offer {offer}: {e}"
                                        ));
                                    }
                                }
                            }
                        }
                        BotCommand::InteractEntity { kind } => {
                            let target = match kind.to_ascii_lowercase().as_str() {
                                "villager" => crate::azalea::trade::find_nearest_villager(&bot)
                                    .ok_or_else(|| "附近没有村民".to_string()),
                                other => {
                                    Err(format!("暂不支持的实体种类 {other}（目前仅 villager）"))
                                }
                            };
                            match target {
                                Ok(e) => {
                                    bot.entity_interact(e);
                                    let chat = format!("[交互] 已右键 {kind}");
                                    let _ = evt_tx.send(BotEvent::Chat { content: chat });
                                    if let Some(tx) = &result_tx {
                                        let _ = tx.send(format!(
                                            "Action output:\nInteracted with {kind}."
                                        ));
                                    }
                                }
                                Err(e) => {
                                    let chat = format!("[交互失败] {e}");
                                    let _ = evt_tx.send(BotEvent::Chat { content: chat });
                                    if let Some(tx) = &result_tx {
                                        let _ = tx.send(format!(
                                            "Action output:\nFailed to interact with {kind}: {e}"
                                        ));
                                    }
                                }
                            }
                        }
                        BotCommand::Pickup => {
                            match crate::azalea::smart_actions::pickup_nearby_items(&bot).await {
                                Ok(msg) => {
                                    let chat = format!("[捡物] {msg}");
                                    let _ = evt_tx.send(BotEvent::Chat { content: chat });
                                    if let Some(tx) = &result_tx {
                                        let _ = tx.send(format!("Action output:\n{msg}"));
                                    }
                                }
                                Err(e) => {
                                    if let Some(tx) = &result_tx {
                                        let _ =
                                            tx.send(format!("Action output:\nPickup failed: {e}"));
                                    }
                                }
                            }
                        }
                        BotCommand::Defend => {
                            match crate::azalea::smart_actions::defend_self(&bot).await {
                                Ok(msg) => {
                                    let chat = format!("[防御] {msg}");
                                    let _ = evt_tx.send(BotEvent::Chat { content: chat });
                                    if let Some(tx) = &result_tx {
                                        let _ = tx.send(format!("Action output:\n{msg}"));
                                    }
                                }
                                Err(e) => {
                                    if let Some(tx) = &result_tx {
                                        let _ =
                                            tx.send(format!("Action output:\nDefend failed: {e}"));
                                    }
                                }
                            }
                        }
                        BotCommand::Equip { item, slot } => {
                            let msg = do_equip(&bot, &item, &slot).await;
                            if let Some(tx) = &result_tx {
                                let _ = tx.send(format!("Action output:\n{msg}"));
                            }
                            let _ = evt_tx.send(BotEvent::Chat {
                                content: format!("[装备] {msg}"),
                            });
                        }
                        BotCommand::Discard { item, count } => {
                            let msg = do_discard(&bot, &item, count).await;
                            if let Some(tx) = &result_tx {
                                let _ = tx.send(format!("Action output:\n{msg}"));
                            }
                            let _ = evt_tx.send(BotEvent::Chat {
                                content: format!("[丢弃] {msg}"),
                            });
                        }
                        BotCommand::Consume { item } => {
                            let msg = do_consume(&bot, &item).await;
                            if let Some(tx) = &result_tx {
                                let _ = tx.send(format!("Action output:\n{msg}"));
                            }
                            let _ = evt_tx.send(BotEvent::Chat {
                                content: format!("[消耗] {msg}"),
                            });
                        }
                        BotCommand::ChestView { x, y, z } => {
                            match crate::azalea::chest::do_chest_view(&bot, BlockPos::new(x, y, z))
                                .await
                            {
                                Ok(msg) => {
                                    if let Some(tx) = &result_tx {
                                        let _ = tx.send(format!("Action output:\n{msg}"));
                                    }
                                    let _ = evt_tx.send(BotEvent::Chat {
                                        content: format!("[查看容器] {msg}"),
                                    });
                                }
                                Err(e) => {
                                    if let Some(tx) = &result_tx {
                                        let _ = tx.send(format!(
                                            "Action output:\nFailed to view chest: {e}"
                                        ));
                                    }
                                    let _ = evt_tx.send(BotEvent::Chat {
                                        content: format!("[查看容器失败] {e}"),
                                    });
                                }
                            }
                        }
                        BotCommand::ChestWithdraw {
                            x,
                            y,
                            z,
                            item,
                            count,
                        } => {
                            match crate::azalea::chest::do_chest_withdraw(
                                &bot,
                                BlockPos::new(x, y, z),
                                &item,
                                count,
                            )
                            .await
                            {
                                Ok(msg) => {
                                    if let Some(tx) = &result_tx {
                                        let _ = tx.send(format!("Action output:\n{msg}"));
                                    }
                                    let _ = evt_tx.send(BotEvent::Chat {
                                        content: format!("[取出] {msg}"),
                                    });
                                }
                                Err(e) => {
                                    if let Some(tx) = &result_tx {
                                        let _ = tx.send(format!(
                                            "Action output:\nFailed to withdraw: {e}"
                                        ));
                                    }
                                    let _ = evt_tx.send(BotEvent::Chat {
                                        content: format!("[取出失败] {e}"),
                                    });
                                }
                            }
                        }
                        BotCommand::ChestDeposit {
                            x,
                            y,
                            z,
                            item,
                            count,
                        } => {
                            match crate::azalea::chest::do_chest_deposit(
                                &bot,
                                BlockPos::new(x, y, z),
                                &item,
                                count,
                            )
                            .await
                            {
                                Ok(msg) => {
                                    if let Some(tx) = &result_tx {
                                        let _ = tx.send(format!("Action output:\n{msg}"));
                                    }
                                    let _ = evt_tx.send(BotEvent::Chat {
                                        content: format!("[存入] {msg}"),
                                    });
                                }
                                Err(e) => {
                                    if let Some(tx) = &result_tx {
                                        let _ = tx.send(format!(
                                            "Action output:\nFailed to deposit: {e}"
                                        ));
                                    }
                                    let _ = evt_tx.send(BotEvent::Chat {
                                        content: format!("[存入失败] {e}"),
                                    });
                                }
                            }
                        }
                        // P68：跟随玩家。设置 follow_target，handler 每 tick 读取目标坐标 goto。
                        BotCommand::Follow { target } => {
                            *state.follow_target.lock().unwrap() = Some(target.clone());
                            let who = target.clone().unwrap_or_else(|| "最近的玩家".to_string());
                            let msg = format!(
                                "已开始跟随 {who}（每 tick 自动走到其身边）。说 \"stop\" 或聊天 stop 可解除。"
                            );
                            if let Some(tx) = &result_tx {
                                let _ = tx.send(format!("Action output:\n{msg}"));
                            }
                            let _ = evt_tx.send(BotEvent::Chat {
                                content: format!("[跟随] {msg}"),
                            });
                            state.action_mgr.clear_pending();
                        }
                        // P68：停止跟随。
                        BotCommand::StopFollow => {
                            *state.follow_target.lock().unwrap() = None;
                            let msg = "已停止跟随。";
                            if let Some(tx) = &result_tx {
                                let _ = tx.send(format!("Action output:\n{msg}"));
                            }
                            let _ = evt_tx.send(BotEvent::Chat {
                                content: format!("[跟随] {msg}"),
                            });
                            state.action_mgr.clear_pending();
                        }
                        // P68：把物品丢在玩家脚边。基于 Discard 能力，但目标坐标改为玩家位置。
                        BotCommand::Give {
                            item,
                            count,
                            target,
                        } => {
                            let target_name = target.as_deref();
                            match nearby_player_position(&bot, target_name) {
                                Some(initial_target) => {
                                    let initial_distance = bot
                                        .position()
                                        .ok()
                                        .map(|position| {
                                            ((position.x - initial_target.x).powi(2)
                                                + (position.y - initial_target.y).powi(2)
                                                + (position.z - initial_target.z).powi(2))
                                            .sqrt()
                                        })
                                        .unwrap_or(f64::INFINITY);
                                    if initial_distance > 2.0 {
                                        let goal = RadiusGoal {
                                            pos: initial_target,
                                            radius: 1.5,
                                        };
                                        let navigation = bot.goto(goal);
                                        if tokio::time::timeout(Duration::from_secs(10), navigation)
                                            .await
                                            .is_err()
                                        {
                                            bot.force_stop_pathfinding();
                                        }
                                    }

                                    // The player may move during navigation. Re-read both
                                    // positions and refuse to drop at a stale destination.
                                    let final_target = nearby_player_position(&bot, target_name);
                                    let final_distance = bot.position().ok().zip(final_target).map(
                                        |(position, player)| {
                                            ((position.x - player.x).powi(2)
                                                + (position.y - player.y).powi(2)
                                                + (position.z - player.z).powi(2))
                                            .sqrt()
                                        },
                                    );
                                    if !final_distance.is_some_and(|distance| distance <= 2.0) {
                                        let distance = final_distance
                                            .map(|value| format!("{value:.1}m"))
                                            .unwrap_or_else(|| "未知".to_string());
                                        let msg = format!(
                                            "给予失败：导航后仍距玩家 {distance}，为避免把物品丢在远处未执行 discard。"
                                        );
                                        if let Some(tx) = &result_tx {
                                            let _ = tx.send(format!("Action output:\n{msg}"));
                                        }
                                        let _ = evt_tx.send(BotEvent::Chat {
                                            content: format!("[给予失败] {msg}"),
                                        });
                                    } else {
                                        let dmsg = do_discard(&bot, &item, count).await;
                                        let msg = format!(
                                            "已把 {item} x{count} 丢在玩家附近（距离确认 <=2m）：{dmsg}"
                                        );
                                        if let Some(tx) = &result_tx {
                                            let _ = tx.send(format!("Action output:\n{msg}"));
                                        }
                                        let _ = evt_tx.send(BotEvent::Chat {
                                            content: format!("[给予] {msg}"),
                                        });
                                    }
                                }
                                None => {
                                    let msg = "附近没有可给予的其他玩家（需同一世界且可见）。";
                                    if let Some(tx) = &result_tx {
                                        let _ = tx.send(format!("Action output:\n{msg}"));
                                    }
                                    let _ = evt_tx.send(BotEvent::Chat {
                                        content: format!("[给予失败] {msg}"),
                                    });
                                }
                            }
                            state.action_mgr.clear_pending();
                        }
                    }
                    // 非轮询命令（异步/即时）执行完即清空中途槽与 busy，让队列推进下一条。
                    {
                        if let Some(qc) = state.action_mgr.peek_pending() {
                            if !matches!(
                                &qc.cmd,
                                BotCommand::Goto { .. }
                                    | BotCommand::Mine { .. }
                                    | BotCommand::MineBelow
                                    | BotCommand::MineAbove
                            ) {
                                state.action_mgr.clear_pending();
                            }
                        }
                    }
                }
                // 持续下挖：只要标志为真且当前未在挖，就续挖（对齐 POC 逻辑，
                // 避免单次 start_mining 因中断失效导致 bot 停在原地不下降）。
                // **Y 下限保护**：Y<=-61 是深板岩+基岩层（1.18+ 基岩层 Y=-64~-59），
                // 继续下挖毫无意义且徒手挖深板岩极慢。到达后自动停止 mining_below 并提示。
                if *state.mining_below.lock().unwrap() && !bot.is_mining() {
                    if let Ok(p) = bot.position() {
                        let y = p.y.floor() as i32;
                        if y <= -61 {
                            // 到达深岩层，停止下挖
                            *state.mining_below.lock().unwrap() = false;
                            let _ = state.evt_tx.send(BotEvent::Chat {
                            content: format!(
                                "Action output:\nMineBelow stopped at Y={y} (深板岩/基岩层，继续下挖无意义)。\
                                 当前坐标 ({:.0},{y},{:.0})。建议改用 mine(x,y,z) 精确挖附近矿石，或 goto 上返回地面。",
                                p.x, p.z
                            ),
                        });
                        } else {
                            let foot = BlockPos::new(
                                p.x.floor() as i32,
                                (p.y - 1.0).floor() as i32,
                                p.z.floor() as i32,
                            );
                            bot.start_mining(foot);
                        }
                    }
                }
                // 持续上挖：mining_above 标志为真时，让 pathfinder 自动挖通头顶并 ascend。
                // **关键修复**：1x1 竖井里 bot 跳跃无法上升（物理限制），必须用 pathfinder
                //              的 ascend_move 让 bot 走到旁边一格的上方。pathfinder allow_mining=true
                //              会自动挖通 head + head+1 + 旁边方块让 bot ascend。
                // **YGoal**：用 YGoal(y+5) 而不是 BlockPosGoal，让 pathfinder 在水平方向自由选择
                //            最容易挖通的柱子，避免 1x1 竖井里 BlockPosGoal 算不出路径。
                // **Y 上限保护**：Y>=62（地表海平面）认为脱困，停止。
                // **大 timeout**：挖通深板岩需要计算长路径，默认 5s 不够，改为 30s。
                if *state.mining_above.lock().unwrap() {
                    if let Ok(p) = bot.position() {
                        let t = bot.ticks_connected();
                        let y = p.y.floor() as i32;
                        let cx = p.x.floor() as i32;
                        let cz = p.z.floor() as i32;
                        // Throttle surface detection to every 5 ticks to reduce per-tick
                        // world reads (6 block reads per check) and avoid GameTick lag.
                        if t % 5 == 0 {
                            // Air alone only proves that the bot entered a cave. Require a
                            // plausible overworld surface elevation before ending ascent.
                            let head_pos = BlockPos::new(cx, y + 1, cz);
                            let head_is_air = bot
                                .world()
                                .ok()
                                .and_then(|w| w.read().get_block_state(head_pos))
                                .map(|s| s.is_air())
                                .unwrap_or(false);
                            // Check an open column so a two-block tunnel at sea level does
                            // not get reported as the surface.
                            let mut five_air = true;
                            for dy in 1..=5 {
                                let check = BlockPos::new(cx, y + dy, cz);
                                let is_air = bot
                                    .world()
                                    .ok()
                                    .and_then(|w| w.read().get_block_state(check))
                                    .map(|s| s.is_air())
                                    .unwrap_or(false);
                                if !is_air {
                                    five_air = false;
                                    break;
                                }
                            }
                            if mine_above_reached_surface(y, head_is_air, five_air) {
                                *state.mining_above.lock().unwrap() = false;
                                let _ = state.evt_tx.send(BotEvent::Chat {
                                content: format!(
                                    "Action output:\nMineAbove done at Y={y} (已到地表，头顶是空气)。\
                                     当前坐标 ({:.0},{y},{:.0})，可继续探索。",
                                    p.x, p.z
                                ),
                            });
                            } else if y >= 320 {
                                *state.mining_above.lock().unwrap() = false;
                                let _ = state.evt_tx.send(BotEvent::Chat {
                                    content: format!(
                                        "Action output:\nMineAbove stopped at Y={y} (建筑高度上限)。\
                                     当前坐标 ({:.0},{y},{:.0})。",
                                        p.x, p.z
                                    ),
                                });
                            } else {
                                // Auto-tp rescue: when stuck in a cave air pocket below
                                // surface, try /tp once to unblock the workflow. If cheats
                                // are not enabled the staircase attempt below still runs.
                                if head_is_air
                                    && y < 62
                                    && !*state.mine_above_tried_tp.lock().unwrap()
                                {
                                    *state.mine_above_tried_tp.lock().unwrap() = true;
                                    bot.chat(&format!("/tp @s ~ {} ~", 70));
                                }
                            }
                        }
                        // auto_equip is expensive (inventory scan), throttle to every 20 ticks.
                        if t % 20 == 0 {
                            let _ = auto_equip_best_pickaxe(&bot).await;
                        }
                        // P60b: 强制楼梯脱困。当 bot 在 2 格高空气袋里（头顶是空气），
                        // pathfinder 用 YGoal 算出的路径"reached"却不会真正上升（因为
                        // 上方 y+2 是实心方块，bot 无法踏入）。这里每 4 tick 主动挖掉
                        // 头顶上方那格 (y+2)，打开竖井，让 bot 能站到 y+1；
                        // 同时发起一个 goto 到自身上方一格，触发真正的上升。
                        let p60b_head_air = bot
                            .world()
                            .ok()
                            .and_then(|w| w.read().get_block_state(BlockPos::new(cx, y + 1, cz)))
                            .map(|s| s.is_air())
                            .unwrap_or(false);
                        if p60b_head_air {
                            let above_head = BlockPos::new(cx, y + 2, cz);
                            let above_is_solid = bot
                                .world()
                                .ok()
                                .and_then(|w| w.read().get_block_state(above_head))
                                .map(|s| !s.is_air())
                                .unwrap_or(false);
                            if above_is_solid && !bot.is_mining() {
                                bot.start_mining(above_head);
                            } else if !above_is_solid && t % 4 == 0 {
                                // 头顶上方已空：强制走到上方一格，真正上升。
                                if !bot.is_calculating_path() && !bot.is_executing_path() {
                                    use azalea::pathfinder::PathfinderOpts;
                                    use std::time::Duration;
                                    let opts = PathfinderOpts::new()
                                        .allow_mining(true)
                                        .min_timeout(Duration::from_secs(1))
                                        .max_timeout(Duration::from_secs(10));
                                    bot.start_goto_with_opts(
                                        BlockPosGoal(BlockPos::new(cx, y + 1, cz)),
                                        opts,
                                    );
                                }
                            }
                        }
                        // An active goal with no calculation or execution can be
                        // permanent no-path retry. Reset it periodically instead of
                        // letting it suppress every future ascent attempt.
                        if !bot.is_calculating_path() && !bot.is_executing_path() && t % 40 == 0 {
                            use azalea::pathfinder::PathfinderOpts;
                            use std::time::Duration;
                            bot.force_stop_pathfinding();
                            let mut direction = state.mining_above_direction.lock().unwrap();
                            let directions = [(1, 0), (0, 1), (-1, 0), (0, -1)];
                            let (dx, dz) = directions[*direction % directions.len()];
                            *direction = (*direction + 1) % 4;
                            drop(direction);
                            // P60 关键修复：1x1 竖井里用 YGoal(y+5) 而不是 BlockPosGoal。
                            // BlockPosGoal 指向特定侧方方块，pathfinder 在 1x1 竖井里
                            // 算不出通往该固定坐标的路径（每根柱子都只有 1 格宽），
                            // 导致"reached end of path"却原地不动、永久卡死。
                            // YGoal 只要求到达 y+5 任意水平位置，pathfinder 可自由选择
                            // 最容易挖通的柱子上升，从而真正脱困。
                            let target = BlockPos::new(cx + dx, y + 5, cz + dz);
                            let opts = PathfinderOpts::new()
                                .allow_mining(true)
                                .min_timeout(Duration::from_secs(2))
                                .max_timeout(Duration::from_secs(30));
                            bot.start_goto_with_opts(YGoal::from(target), opts);
                        }
                    }
                }
                // P67：make_obsidian 状态机。每 tick 推进：
                //  phase 0：找附近（半径12）岩浆源；装备 water_bucket+diamond_pickaxe；
                //          在岩浆旁的空气块右键放水（block_interact 手持 water_bucket）→ 生成黑曜石。
                //  phase 1：等 ~4s（黑曜石生成）。
                //  phase 2：用 diamond_pickaxe 挖下黑曜石；remaining-1；回 phase 0。
                //  完成 remaining==0 或找不到岩浆/没水 → 结束并发结果。
                if let Some((remaining, phase, ob_pos)) = *state.make_obsidian.lock().unwrap() {
                    let t = bot.ticks_connected();
                    match phase {
                        0 => {
                            // P67c 同步装备水桶：tick handler 内严禁 await，这里用
                            // set_selected_hotbar_slot 同步把 bucket 切到主手（不等待服务端轮询）。
                            // 若 bucket 不在 hotbar，则同步 shift_click 到空 hotbar 槽。
                            if bot
                                .get_held_item()
                                .map(|s| {
                                    let k: azalea_registry::builtin::ItemKind = s.kind();
                                    k != azalea_registry::builtin::ItemKind::Bucket
                                        && k != azalea_registry::builtin::ItemKind::WaterBucket
                                })
                                .unwrap_or(true)
                            {
                                if let Ok(inv) = bot.get_inventory() {
                                    if let Some(h) = find_hotbar_slot_for(
                                        &inv,
                                        azalea_registry::builtin::ItemKind::Bucket,
                                    ) {
                                        bot.set_selected_hotbar_slot(h);
                                    } else if let Some(srcs) = Some(find_item_slots(
                                        &inv,
                                        azalea_registry::builtin::ItemKind::Bucket,
                                    )) && !srcs.is_empty()
                                    {
                                        let menu = inv.menu().ok().flatten();
                                        if let Some(menu) = menu {
                                            let hotbar_range = menu.hotbar_slots_range();
                                            if let Some(slots) = inv.slots() {
                                                let mut placed = false;
                                                for hb in hotbar_range {
                                                    if slots
                                                        .get(hb)
                                                        .map(|s| s.is_empty())
                                                        .unwrap_or(false)
                                                    {
                                                        inv.left_click(*srcs.first().unwrap());
                                                        inv.left_click(hb);
                                                        placed = true;
                                                        break;
                                                    }
                                                }
                                                let _ = placed;
                                            }
                                        }
                                    }
                                }
                            }
                            // 检查手持 water_bucket；没有则自动找水源装水（已装备 bucket）。
                            let held = bot
                                .get_held_item()
                                .map(|s| s.kind().to_string())
                                .unwrap_or_default();
                            if !held.contains("water_bucket") {
                                // 自动装水：扫描半径 16 内水源，对水块 block_interact（持 bucket 右键水→装水）
                                if let (Ok(p), Ok(world)) = (bot.position(), bot.world()) {
                                    let wp = p.x.floor() as i32;
                                    let wy = p.y.floor() as i32;
                                    let wz = p.z.floor() as i32;
                                    let world = world.read();
                                    let mut water: Option<(i32, i32, i32)> = None;
                                    'wscan: for r in 1..=16i32 {
                                        for dx in -r..=r {
                                            for dy in -3..=4i32 {
                                                for dz in -r..=r {
                                                    let wx = wp + dx;
                                                    let wy2 = wy + dy;
                                                    let wz2 = wz + dz;
                                                    if let Some(bs) = world.get_block_state(
                                                        BlockPos::new(wx, wy2, wz2),
                                                    ) {
                                                        let kind: azalea_registry::builtin::BlockKind =
                                                            bs.into();
                                                        if kind
                                                            == azalea_registry::builtin::BlockKind::Water
                                                        {
                                                            water = Some((wx, wy2, wz2));
                                                            break 'wscan;
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    drop(world);
                                    match water {
                                        Some((wx, wy2, wz2)) => {
                                            bot.block_interact(BlockPos::new(wx, wy2, wz2));
                                            // 装水后下一 tick 再检查手持，进入岩浆逻辑
                                            *state.make_obsidian.lock().unwrap() =
                                                Some((remaining, 0, None));
                                        }
                                        None => {
                                            let _ = state.evt_tx.send(BotEvent::Chat {
                                                content: "Action output:\nmake_obsidian 失败：附近（半径16）未找到水源。请先 goto 到河流/湖泊附近再调用。".to_string(),
                                            });
                                            *state.make_obsidian.lock().unwrap() = None;
                                        }
                                    }
                                } else {
                                    *state.make_obsidian.lock().unwrap() = None;
                                }
                            } else if let (Ok(p), Ok(world)) = (bot.position(), bot.world()) {
                                let wp = p.x.floor() as i32;
                                let wy = p.y.floor() as i32;
                                let wz = p.z.floor() as i32;
                                let world = world.read();
                                // 扫描半径 12 内岩浆方块（Lava）；视作岩浆源处理。
                                let mut found: Option<(i32, i32, i32)> = None;
                                'scan: for r in 1..=12i32 {
                                    for dx in -r..=r {
                                        for dy in -2..=4i32 {
                                            for dz in -r..=r {
                                                let lx = wp + dx;
                                                let ly = wy + dy;
                                                let lz = wz + dz;
                                                if let Some(bs) =
                                                    world.get_block_state(BlockPos::new(lx, ly, lz))
                                                {
                                                    let kind: azalea_registry::builtin::BlockKind =
                                                        bs.into();
                                                    if kind
                                                        == azalea_registry::builtin::BlockKind::Lava
                                                    {
                                                        // 找岩浆旁的空气邻居放水
                                                        for (nx, ny, nz) in [
                                                            (lx + 1, ly, lz),
                                                            (lx - 1, ly, lz),
                                                            (lx, ly, lz + 1),
                                                            (lx, ly, lz - 1),
                                                            (lx, ly + 1, lz),
                                                        ] {
                                                            if let Some(nb) = world.get_block_state(
                                                                BlockPos::new(nx, ny, nz),
                                                            ) {
                                                                if nb.is_air() {
                                                                    found = Some((nx, ny, nz));
                                                                    break 'scan;
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                drop(world);
                                match found {
                                    Some((nx, ny, nz)) => {
                                        // 右键该空气块放水→黑曜石（需手持 water_bucket，由 LLM 保证）
                                        bot.block_interact(BlockPos::new(nx, ny, nz));
                                        *state.make_obsidian.lock().unwrap() =
                                            Some((remaining, 1, Some((nx, ny, nz))));
                                    }
                                    None => {
                                        let _ = state.evt_tx.send(BotEvent::Chat {
                                            content: "Action output:\nmake_obsidian 失败：附近（半径12）未找到岩浆源。请先 goto 到岩浆湖附近再调用。".to_string(),
                                        });
                                        *state.make_obsidian.lock().unwrap() = None;
                                    }
                                }
                            }
                        }
                        1 => {
                            // 等 ~80 tick(4s) 让水与岩浆反应生成黑曜石。
                            // 用 ob_pos 记录起始 tick 比较麻烦，这里简单用 ticks%80==0 推进到挖阶段。
                            if t % 80 == 0 || ob_pos.is_none() {
                                if let Some((_nx, _ny, _nz)) = ob_pos {
                                    *state.make_obsidian.lock().unwrap() =
                                        Some((remaining, 2, ob_pos));
                                } else {
                                    *state.make_obsidian.lock().unwrap() =
                                        Some((remaining, 0, None));
                                }
                            }
                        }
                        2 => {
                            if let Some((nx, ny, nz)) = ob_pos {
                                // 黑曜石生成在岩浆源处（邻居的反方向）。尝试挖 (nx, ny-1, nz) 及 ob_pos 自身。
                                let targets = [(nx, ny - 1, nz), (nx, ny, nz)];
                                let mut mined = false;
                                if let Ok(world) = bot.world() {
                                    let world = world.read();
                                    for (tx, ty, tz) in targets {
                                        if let Some(bs) =
                                            world.get_block_state(BlockPos::new(tx, ty, tz))
                                        {
                                            let kind: azalea_registry::builtin::BlockKind =
                                                bs.into();
                                            if kind == azalea_registry::builtin::BlockKind::Obsidian
                                            {
                                                bot.start_mining(BlockPos::new(tx, ty, tz));
                                                mined = true;
                                                break;
                                            }
                                        }
                                    }
                                }
                                if mined {
                                    let _ = state.evt_tx.send(BotEvent::Chat {
                                        content: format!(
                                            "[造黑曜石] 已挖下 1 块黑曜石，剩余 {}",
                                            remaining.saturating_sub(1)
                                        ),
                                    });
                                    let left = remaining.saturating_sub(1);
                                    if left == 0 {
                                        let _ = state.evt_tx.send(BotEvent::Chat {
                                            content: "Action output:\nmake_obsidian 完成：已收集所需黑曜石。可用于搭建下界传送门框架。".to_string(),
                                        });
                                        *state.make_obsidian.lock().unwrap() = None;
                                    } else {
                                        *state.make_obsidian.lock().unwrap() =
                                            Some((left, 0, None));
                                    }
                                } else {
                                    // 没生成黑曜石（可能水没流到岩浆），重试
                                    *state.make_obsidian.lock().unwrap() =
                                        Some((remaining, 0, None));
                                }
                            } else {
                                *state.make_obsidian.lock().unwrap() = Some((remaining, 0, None));
                            }
                        }
                        _ => {
                            *state.make_obsidian.lock().unwrap() = None;
                        }
                    }
                }
                // P60c: 地下强制楼梯脱困（无条件运行，不依赖 LLM 是否调用 mine_above）。
                // 当 bot 在地下 (Y<62) 且头顶是空气（处于 2 格高空气袋），持续挖掉头顶上方
                // 那格并走到上方一格，保证 bot 真正上升——即使 LLM 反复下发无效的地下
                // goto/mine，bot 也能稳定爬出竖井，避免永久困死在 Y=12。
                if let Ok(p) = bot.position() {
                    let y = p.y.floor() as i32;
                    if y < 62 {
                        let cx = p.x.floor() as i32;
                        let cz = p.z.floor() as i32;
                        let head_air = bot
                            .world()
                            .ok()
                            .and_then(|w| w.read().get_block_state(BlockPos::new(cx, y + 1, cz)))
                            .map(|s| s.is_air())
                            .unwrap_or(false);
                        if head_air && !bot.is_executing_path() && !bot.is_calculating_path() {
                            let above_head = BlockPos::new(cx, y + 2, cz);
                            let above_is_solid = bot
                                .world()
                                .ok()
                                .and_then(|w| w.read().get_block_state(above_head))
                                .map(|s| !s.is_air())
                                .unwrap_or(false);
                            if above_is_solid && !bot.is_mining() {
                                bot.start_mining(above_head);
                            } else if !above_is_solid {
                                // 头顶上方已空：直接走上去一格，真正上升。
                                use azalea::pathfinder::PathfinderOpts;
                                use std::time::Duration;
                                let opts = PathfinderOpts::new()
                                    .allow_mining(true)
                                    .min_timeout(Duration::from_secs(1))
                                    .max_timeout(Duration::from_secs(10));
                                bot.start_goto_with_opts(
                                    BlockPosGoal(BlockPos::new(cx, y + 1, cz)),
                                    opts,
                                );
                            }
                        }
                        // 看门狗：完全卡死（头顶是实心、无法 ascent）时退回 mining_above 模式。
                        if !head_air
                            && !*state.mining_above.lock().unwrap()
                            && !bot.is_mining()
                            && bot.ticks_connected() % 20 == 0
                        {
                            *state.mining_above.lock().unwrap() = true;
                            *state.mining_above_start_y.lock().unwrap() = Some(y);
                            *state.mining_above_direction.lock().unwrap() = 0;
                            *state.mine_above_tried_tp.lock().unwrap() = false;
                        }
                    }
                }
                // 每 20 tick 推送状态快照。
                let t = bot.ticks_connected();
                if t % 20 == 0 {
                    if let Ok(p) = bot.position() {
                        // 全量背包：列出所有非空格，**按物品 ID 聚合后输出**（旧版每个槽位单独
                        // 输出，导致 `dirt:46, dirt:64, leaflitter:64, leaflitter:26` 这种重复条目，
                        // LLM 困惑且浪费 token）。聚合后输出 `dirt:110, leaflitter:90`。
                        let (inventory, armor_str) = match bot.get_inventory() {
                            Ok(inv) => match inv.slots() {
                                Some(slots) => {
                                    // P56：Player 菜单槽位布局（azalea declare_menus!）：
                                    // 0=craft_result, 1-4=craft, 5-8=armor(helmet/chestplate/
                                    // leggings/boots), 9-44=inventory, 45=offhand。
                                    // 原实现把 armor 槽混入"背包"聚合 → LLM 以为甲还在背包，
                                    // 反复 equip 又因 find_item_slots(9-44) 找不到而报"背包未持有"
                                    // → 死循环（实测甲已上身仍被反复驱赶）。现跳过 armor 槽并
                                    // 单独产出装备摘要行。仅 Player 菜单布局固定，容器菜单跳过。
                                    let is_player_menu = inv
                                        .menu()
                                        .ok()
                                        .flatten()
                                        .map(|m| matches!(m, azalea::inventory::Menu::Player(_)))
                                        .unwrap_or(false);
                                    let mut agg: std::collections::HashMap<String, u32> =
                                        std::collections::HashMap::new();
                                    let mut armor: [String; 4] = Default::default();
                                    for (idx, s) in slots.iter().enumerate() {
                                        if s.is_empty() {
                                            continue;
                                        }
                                        // P5 关键修复：用 to_str() 返回 minecraft id（如 "minecraft:crafting_table"），
                                        // 然后 strip "minecraft:" 前缀得到 "crafting_table"。
                                        // 原代码用 format!("{:?}", s.kind()).to_lowercase() 得到 enum Debug 名
                                        // （如 "CraftingTable".to_lowercase() = "craftingtable"，无下划线），
                                        // 与工具/craft 配方表期望的 snake_case id 不匹配 → LLM 看到 "craftingtable"
                                        // 却 craft("crafting_table") 报"无此物品" → 100% 卡死。
                                        let kind_full = s.kind().to_str();
                                        let kind = kind_full
                                            .strip_prefix("minecraft:")
                                            .unwrap_or(kind_full);
                                        let cnt = s.count() as u32;
                                        if is_player_menu && (5..=8).contains(&idx) {
                                            armor[idx - 5] = kind.to_string();
                                        } else {
                                            *agg.entry(kind.to_string()).or_insert(0) += cnt;
                                        }
                                    }
                                    let inv_str = if agg.is_empty() {
                                        "空背包".to_string()
                                    } else {
                                        // 按数量降序输出（多的在前，LLM 重点看前几个）
                                        let mut items: Vec<(String, u32)> =
                                            agg.into_iter().collect();
                                        items.sort_by(|a, b| b.1.cmp(&a.1));
                                        items
                                            .iter()
                                            .map(|(k, c)| format!("{k}:{c}"))
                                            .collect::<Vec<_>>()
                                            .join(", ")
                                    };
                                    let display = |s: &String| {
                                        if s.is_empty() {
                                            "无".to_string()
                                        } else {
                                            s.clone()
                                        }
                                    };
                                    let armor_summary = format!(
                                        "头盔: {}, 胸甲: {}, 护腿: {}, 靴子: {}",
                                        display(&armor[0]),
                                        display(&armor[1]),
                                        display(&armor[2]),
                                        display(&armor[3])
                                    );
                                    (inv_str, armor_summary)
                                }
                                None => (
                                    "slots=None".to_string(),
                                    "头盔: 无, 胸甲: 无, 护腿: 无, 靴子: 无".to_string(),
                                ),
                            },
                            Err(_) => (
                                "获取失败".to_string(),
                                "头盔: 无, 胸甲: 无, 护腿: 无, 靴子: 无".to_string(),
                            ),
                        };
                        let player_count = bot.nearby_players().map(|pp| pp.len()).unwrap_or(0);
                        // 朝向（yaw/pitch，度数）：从 LookDirection 的 Debug 输出解析（azalea 字段为私有，不改动库）。
                        let (yaw, pitch) = bot
                            .direction()
                            .map(|d| {
                                let s = format!("{d:?}");
                                let y = s
                                    .split("y_rot: ")
                                    .nth(1)
                                    .and_then(|x| x.split(',').next())
                                    .and_then(|x| x.trim().parse::<f64>().ok())
                                    .unwrap_or(0.0);
                                let pi = s
                                    .split("x_rot: ")
                                    .nth(1)
                                    .and_then(|x| x.split('}').next())
                                    .and_then(|x| x.trim().parse::<f64>().ok())
                                    .unwrap_or(0.0);
                                (y, pi)
                            })
                            .unwrap_or((0.0, 0.0));
                        // 脚下方块 + 前方 1 格方块（用于 bot 判断脚下是否悬空/面前是否墙）。
                        let block_name = |bp: BlockPos| -> String {
                            if let Ok(world) = bot.world() {
                                match world.read().get_block_state(bp) {
                                    Some(s) if !s.is_air() => {
                                        let bk: BlockKind = s.into();
                                        // P5 修复：用 to_str() 拿到 minecraft id（如 "minecraft:stone"）。
                                        // 原代码 format!("{bk:?}").to_lowercase() 得到 "stone"（无前缀），
                                        // 但对于多词方块如 "GrassBlock".to_lowercase() = "grassblock"（无下划线），
                                        // 与工具/mem 期望的 snake_case id 不匹配。
                                        let k = bk.to_str();
                                        k.strip_prefix("minecraft:").unwrap_or(k).to_string()
                                    }
                                    _ => "air".to_string(),
                                }
                            } else {
                                "?".to_string()
                            }
                        };
                        let foot_y = (p.y - 1.0).floor() as i32;
                        let block_under = block_name(BlockPos::new(
                            p.x.floor() as i32,
                            foot_y,
                            p.z.floor() as i32,
                        ));
                        // 前方方块：由 yaw/pitch 推算视线落点（水平 1 格 + 俯仰修正）。
                        let rad = yaw.to_radians();
                        let dx = (-rad.sin()) as f64; // 与 azalea 约定一致：yaw 0 朝 +Z
                        let dz = (rad.cos()) as f64;
                        let horiz = 1.0_f64.max((pitch.abs() / 90.0) * 2.0);
                        let ahead_x = (p.x + dx * horiz).floor() as i32;
                        let ahead_z = (p.z + dz * horiz).floor() as i32;
                        let ahead_y = (p.y + (pitch / 90.0) * -1.0).floor() as i32;
                        let block_ahead = block_name(BlockPos::new(ahead_x, ahead_y, ahead_z));
                        // 生命/饱食/主手/群系/附近方块
                        let health = bot.health().unwrap_or(20.0);
                        let hunger = bot.hunger().ok();
                        let food = hunger.as_ref().map(|h| h.food).unwrap_or(20);
                        let saturation = hunger.as_ref().map(|h| h.saturation).unwrap_or(5.0);
                        let held_item = match bot.get_held_item() {
                            Ok(item) if !item.is_empty() => {
                                // P5 修复：用 to_str() 拿到 minecraft id（同背包聚合逻辑）。
                                let k = item.kind().to_str();
                                k.strip_prefix("minecraft:").unwrap_or(k).to_string()
                            }
                            _ => "air".to_string(),
                        };
                        // biome 通过 registry 解析为可读 Identifier（如 "minecraft:dark_forest"）。
                        // 旧实现 `format!("{b:?}")` 会输出 "biome { id: 30 }" 这种调试串，LLM 看不懂。
                        let biome = bot
                            .world()
                            .ok()
                            .and_then(|w| {
                                w.read().get_biome(BlockPos::new(
                                    p.x.floor() as i32,
                                    p.y.floor() as i32,
                                    p.z.floor() as i32,
                                ))
                            })
                            .and_then(|b| bot.resolve_registry_key(&b).ok().flatten())
                            .map(|key| key.into_ident().to_string())
                            .map(|s| {
                                // "minecraft:dark_forest" → "dark_forest"
                                s.strip_prefix("minecraft:")
                                    .map(|x| x.to_string())
                                    .unwrap_or(s)
                            })
                            .unwrap_or_else(|| "unknown".to_string());
                        // 附近方块摘要：3x3 地面区域
                        let nearby = {
                            let foot_x = p.x.floor() as i32;
                            let foot_z = p.z.floor() as i32;
                            let mut counts: HashMap<String, u32> = HashMap::new();
                            let world = bot.world().ok();
                            for dx in -1..=1 {
                                for dz in -1..=1 {
                                    if let Some(ref w) = world {
                                        let bp = BlockPos::new(foot_x + dx, foot_y, foot_z + dz);
                                        let name = match w.read().get_block_state(bp) {
                                            Some(s) if !s.is_air() => {
                                                let bk: BlockKind = s.into();
                                                // P5 修复：用 to_str() 拿到 snake_case minecraft id
                                                let k = bk.to_str();
                                                k.strip_prefix("minecraft:")
                                                    .unwrap_or(k)
                                                    .to_string()
                                            }
                                            _ => "air".to_string(),
                                        };
                                        *counts.entry(name).or_insert(0) += 1;
                                    }
                                }
                            }
                            let parts: Vec<String> = counts
                                .into_iter()
                                .filter(|(k, _)| k != "air")
                                .map(|(k, v)| format!("{k}:{v}"))
                                .collect();
                            if parts.is_empty() {
                                "air".to_string()
                            } else {
                                parts.join(", ")
                            }
                        };
                        // 结构化游戏状态 JSON（前端面板可视化）
                        let game_state = {
                            let inv_slots: Vec<serde_json::Value> = match bot.get_inventory() {
                                Ok(inv) => match inv.slots() {
                                    Some(slots) => slots
                                        .iter()
                                        .enumerate()
                                        .map(|(i, s)| {
                                            let id = if s.is_empty() {
                                                "minecraft:air".to_string()
                                            } else {
                                                // P5 修复：to_str() 已返回 "minecraft:xxx"，不需要拼前缀
                                                s.kind().to_str().to_string()
                                            };
                                            let cnt = if s.is_empty() { 0 } else { s.count() };
                                            serde_json::json!({"slot": i, "id": id, "count": cnt})
                                        })
                                        .collect(),
                                    None => vec![],
                                },
                                Err(_) => vec![],
                            };
                            let xp = bot.experience().ok();
                            let dimension = bot
                                .world_name()
                                .map(|name| name.to_string())
                                .unwrap_or_else(|_| "unknown".to_string());
                            let portal_active = nearby_active_portal(
                                &bot,
                                BlockPos::new(
                                    p.x.floor() as i32,
                                    p.y.floor() as i32,
                                    p.z.floor() as i32,
                                ),
                            );
                            let kill_counts = bot
                                .ecs
                                .read()
                                .resource::<crate::azalea::ext_state::BotExtResource>()
                                .0
                                .lock()
                                .unwrap()
                                .kill_counts
                                .clone();
                            serde_json::json!({
                                "inventory": inv_slots,
                                // P56：盔甲槽位（Player 菜单 5-8）单独列出，与背包区分。
                                "armor": inv_slots
                                    .iter()
                                    .filter(|s| {
                                        s.get("slot")
                                            .and_then(|v| v.as_u64())
                                            .map(|i| (5..=8).contains(&i))
                                            .unwrap_or(false)
                                    })
                                    .cloned()
                                    .collect::<Vec<_>>(),
                                "experience_level": xp.as_ref().map(|e| e.level).unwrap_or(0),
                                "experience_progress": xp.as_ref().map(|e| e.progress).unwrap_or(0.0),
                                "held_item": held_item,
                                "selected_slot": bot.selected_hotbar_slot().unwrap_or(0),
                                "dimension": dimension,
                                "portal_active": portal_active,
                                "kill_counts": kill_counts,
                            })
                        };
                        // 回填世界记忆：更新当前位置锚点 + 扫描周边关键方块
                        if let Some(mem) = &state.memory {
                            let mp = MemoryPos::new(
                                p.x.floor() as i32,
                                p.y.floor() as i32,
                                p.z.floor() as i32,
                            );
                            mem.set_anchor("__self__", Some(mp), "当前位置");
                            record_surroundings(&bot, mem, &mp, &state.scanned);
                        }
                        // 10x10 范围方块扫描：列出所有非空气方块类型及计数
                        let nearby_blocks = {
                            let mut counts: HashMap<String, u32> = HashMap::new();
                            let world = bot.world().ok();
                            let cx = p.x.floor() as i32;
                            let cy = p.y.floor() as i32;
                            let cz = p.z.floor() as i32;
                            for dx in -5..=5 {
                                for dy in -5..=5 {
                                    for dz in -5..=5 {
                                        if let Some(ref w) = world {
                                            let bp = BlockPos::new(cx + dx, cy + dy, cz + dz);
                                            let name = match w.read().get_block_state(bp) {
                                                Some(s) if !s.is_air() => {
                                                    let bk: BlockKind = s.into();
                                                    // P5 修复：用 to_str() 拿到 snake_case minecraft id
                                                    let k = bk.to_str();
                                                    k.strip_prefix("minecraft:")
                                                        .unwrap_or(k)
                                                        .to_string()
                                                }
                                                _ => continue,
                                            };
                                            *counts.entry(name).or_insert(0) += 1;
                                        }
                                    }
                                }
                            }
                            let mut items: Vec<_> = counts.into_iter().collect();
                            items.sort_by(|a, b| b.1.cmp(&a.1));
                            items
                                .iter()
                                .map(|(k, v)| format!("{k}:{v}"))
                                .collect::<Vec<_>>()
                                .join(", ")
                        };
                        // 资源分类摘要：把 10x10 里的方块按 wood/stone/ore/other 分组，
                        // 让 WorldInfo 的 find_match_line 能为每类找到独立的 label 行，
                        // 避免【场景提示】里 Wood/Stone/Ore 三条都粘同一份 10x10 字符串。
                        let _resource_summary = {
                            let wood_kinds = [
                                "oaklog",
                                "darkoaklog",
                                "birchlog",
                                "sprucelog",
                                "acalog",
                                "junglelog",
                                "mangrovelog",
                                "cherrylog",
                                "oakplanks",
                                "darkoakplanks",
                            ];
                            let stone_kinds = [
                                "stone",
                                "cobblestone",
                                "dirt",
                                "grassblock",
                                "sand",
                                "gravel",
                                "andesite",
                                "granite",
                                "diorite",
                            ];
                            let ore_kinds = [
                                "coalore",
                                "ironore",
                                "copperore",
                                "goldore",
                                "diamondore",
                                "emeraldore",
                                "redstoneore",
                                "lapisore",
                                "netherquartzore",
                            ];
                            let mut wood = Vec::new();
                            let mut stone = Vec::new();
                            let mut ore = Vec::new();
                            for (k, v) in nearby_blocks.split(", ").map(|s| {
                                let mut it = s.split(':');
                                (
                                    it.next().unwrap_or("").to_string(),
                                    it.next().and_then(|x| x.parse::<u32>().ok()).unwrap_or(0),
                                )
                            }) {
                                if wood_kinds.iter().any(|x| *x == k) {
                                    wood.push(format!("{k}:{v}"));
                                } else if stone_kinds.iter().any(|x| *x == k) {
                                    stone.push(format!("{k}:{v}"));
                                } else if ore_kinds.iter().any(|x| *x == k) {
                                    ore.push(format!("{k}:{v}"));
                                }
                            }
                            let mut lines = Vec::new();
                            if !wood.is_empty() {
                                lines.push(format!("木材: {}", wood.join(", ")));
                            }
                            if !stone.is_empty() {
                                lines.push(format!("石头: {}", stone.join(", ")));
                            }
                            if !ore.is_empty() {
                                lines.push(format!("矿石: {}", ore.join(", ")));
                            }
                            lines.join("\n")
                        };
                        // 附近实体列表：按类型分组计数 + 最小距离（仅感知半径内，避免 LLM 追逐远处实体）
                        // P74：加最近实例坐标——LLM 想找动物狩猎/避开怪物时可直接 goto，
                        // 此前只有距离没有方向（实测 LLM 在树冠上找不到食物来源）。
                        let nearby_entities = {
                            const PERCEPTION_RADIUS: f64 = 24.0;
                            let mut kinds: HashMap<String, (u32, f64, (i32, i32, i32))> =
                                HashMap::new();
                            if let Ok(entities) = bot.nearest_entities::<bevy_ecs::query::Without<azalea::entity::metadata::Player>>() {
                            let self_id = bot.entity().id();
                            for e in entities.iter() {
                                if e.id() == self_id { continue; }
                                let Ok(distance) = e.distance_to_client() else { continue; };
                                if distance > PERCEPTION_RADIUS { continue; }
                                let name = entity_kind_name(e.kind().unwrap_or(EntityKind::Pig));
                                let pos = e.position().ok().map(|p| {
                                    (p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32)
                                });
                                let entry = kinds.entry(name).or_insert((0, distance, pos.unwrap_or((0,0,0))));
                                entry.0 += 1;
                                if distance < entry.1 {
                                    entry.1 = distance;
                                    if let Some(p) = pos { entry.2 = p; }
                                }
                            }
                        }
                            // 玩家分开计数
                            let player_count = bot.nearby_players().map(|pp| pp.len()).unwrap_or(0);
                            let mut parts: Vec<String> = Vec::new();
                            if player_count > 0 {
                                parts.push(format!("player:{}", player_count));
                            }
                            let mut items: Vec<_> = kinds.into_iter().collect();
                            items.sort_by(|a, b| b.1.0.cmp(&a.1.0));
                            for (k, (v, d, pos)) in items {
                                if v > 0 {
                                    parts.push(format!("{k}:{v}@{d:.0}m@{pos:?}"));
                                }
                            }
                            if parts.is_empty() {
                                "无".to_string()
                            } else {
                                parts.join(", ")
                            }
                        };
                        let _ = evt_tx.send(BotEvent::State {
                            position: p,
                            inventory,
                            armor: armor_str,
                            player_count,
                            yaw,
                            pitch,
                            block_under,
                            block_ahead,
                            health,
                            food,
                            saturation,
                            held_item,
                            biome,
                            nearby,
                            nearby_blocks,
                            nearby_entities,
                            game_state,
                        });
                    }
                }
                // ===== 反应式 modes（每 tick 检查，直接执行动作，不依赖 LLM）=====
                // self_preservation：检测火/岩浆，自动脱困
                // 使用 ActionManager 的 High 优先级抢占当前 pending（如正在合成时着火立即打断）
                if let Ok(p) = bot.position() {
                    let foot = BlockPos::new(
                        p.x.floor() as i32,
                        (p.y - 1.0).floor() as i32,
                        p.z.floor() as i32,
                    );
                    let head =
                        BlockPos::new(p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32);
                    if let Ok(world) = bot.world() {
                        let under = world.read().get_block_state(foot);
                        let at = world.read().get_block_state(head);
                        let is_danger = |s: Option<azalea::block::BlockState>| -> bool {
                            s.map(|s| {
                                let bk: BlockKind = s.into();
                                matches!(
                                    bk,
                                    BlockKind::Lava | BlockKind::Fire | BlockKind::MagmaBlock
                                )
                            })
                            .unwrap_or(false)
                        };
                        if is_danger(under) || is_danger(at) {
                            let escape_cmd = BotCommand::Goto {
                                x: p.x.floor() as i32 + 5,
                                y: p.y.floor() as i32 + 1,
                                z: p.z.floor() as i32 + 5,
                            };
                            // 高优先级提交：若当前 pending 是 Normal（合成/采集等）则抢占
                            let tick_now = bot.ticks_connected() as u64;
                            let outcome = state.action_mgr.submit(
                                escape_cmd,
                                Priority::High,
                                &cmd_queue,
                                tick_now,
                            );
                            let preempt_msg = match outcome {
                                SubmitOutcome::Preempted(old) => {
                                    format!(
                                        "[MODE] 检测到火/岩浆，抢占当前命令 ({:?}) 自动脱困",
                                        old
                                    )
                                }
                                _ => "[MODE] 检测到火/岩浆，自动脱困".to_string(),
                            };
                            let _ = evt_tx.send(BotEvent::Chat {
                                content: preempt_msg,
                            });
                        }
                    }
                }
                // auto_eat：饥饿 ≤14 且背包有安全食物 → 自动进食（每 80 tick ≈4s 检查一次）。
                // P58：借鉴 Mindcraft autoEat（startAt=14 + bannedFood）。此前靠 LLM 手动
                // consume（30-60s/回合延迟），且 LLM 吃过 rotten_flesh（食物中毒风险）。
                // 仅空闲时执行（不打断 LLM 的 goto/挖矿/合成），安全白名单排除毒物。
                {
                    let hunger_now = bot.hunger().ok().map(|h| h.food).unwrap_or(20);
                    let auto_eat_ok = hunger_now <= 14
                        && state.action_mgr.is_idle()
                        && bot.ticks_connected() % 80 == 0;
                    if auto_eat_ok {
                        if let Ok(inv) = bot.get_inventory() {
                            const SAFE_FOODS: [&str; 20] = [
                                "cooked_beef",
                                "cooked_porkchop",
                                "cooked_chicken",
                                "cooked_mutton",
                                "cooked_rabbit",
                                "cooked_cod",
                                "cooked_salmon",
                                "bread",
                                "apple",
                                "golden_apple",
                                "baked_potato",
                                "mushroom_stew",
                                "rabbit_stew",
                                "pumpkin_pie",
                                "cookie",
                                "melon_slice",
                                "sweet_berries",
                                "glow_berries",
                                "cake",
                                "dried_kelp",
                            ];
                            let found = SAFE_FOODS.iter().find_map(|name| {
                                ItemKind::from_str(name).ok().filter(|k| {
                                    find_item_slots(&inv, *k).first().is_some()
                                })
                            });
                            if let Some(k) = found {
                                let item_name = k
                                    .to_str()
                                    .strip_prefix("minecraft:")
                                    .unwrap_or_else(|| k.to_str());
                                let msg = do_consume(&bot, item_name).await;
                                if !msg.contains("失败") && !msg.contains("未持有") {
                                    let _ = evt_tx.send(BotEvent::Chat {
                                        content: format!(
                                            "[MODE:auto_eat] 饥饿 {hunger_now}/20，自动进食 {item_name}"
                                        ),
                                    });
                                }
                            }
                        }
                    }
                }
                // hunting：空闲时自动狩猎附近动物（P77，Mindcraft 移植）。
                // Mindcraft modes.js hunting: 8m 内 isHuntable 动物自动 attackEntity，
                // 掉落物靠 item_collecting 模式拾取。此前我们只有 LLM 决策层提示
                // （30-60s/回合），动物跑了/没 LLM 关注就没有食物来源。
                // 实现：100 tick 节流 + is_idle + hp≥10（濒死让位 cowardice）；
                // 攻击后 5s 拾取窗口内自动 pickup 掉落物。
                if bot.ticks_connected() % 100 == 0
                    && state.action_mgr.is_idle()
                    && !*state.mining_below.lock().unwrap()
                    && bot.health().unwrap_or(20.0) >= 10.0
                {
                    if let Ok(entities) = bot
                        .nearest_entities::<bevy_ecs::query::Without<azalea::entity::metadata::Player>>()
                    {
                        let self_id = bot.entity().id();
                        let self_pos = bot.position().ok();
                        'hunt: for e in entities.iter() {
                            if e.id() == self_id {
                                continue;
                            }
                            let Ok(kind) = e.kind() else { continue };
                            let huntable = matches!(
                                kind,
                                EntityKind::Cow
                                    | EntityKind::Pig
                                    | EntityKind::Chicken
                                    | EntityKind::Sheep
                                    | EntityKind::Rabbit
                                    | EntityKind::Mooshroom
                            );
                            if !huntable {
                                continue;
                            }
                            let (Some(sp), Ok(ep)) = (self_pos, e.position()) else {
                                continue;
                            };
                            let d = ((sp.x - ep.x).powi(2)
                                + (sp.y - ep.y).powi(2)
                                + (sp.z - ep.z).powi(2))
                                .sqrt();
                            if d <= 8.0 {
                                let indexed = bot
                                    .query_self::<&azalea::entity::indexing::EntityIdIndex, _>(
                                        |index| index.contains_ecs_entity(e.id()),
                                    )
                                    .unwrap_or(false);
                                if indexed
                                    && e.get_component::<azalea::entity::EntityKindComponent>()
                                        .is_some()
                                {
                                    e.attack();
                                    let tick = bot.ticks_connected() as u64;
                                    *state.hunt_pickup_until.lock().unwrap() = tick + 100;
                                    let _ = evt_tx.send(BotEvent::Chat {
                                        content: format!(
                                            "[MODE:hunting] 自动狩猎 {kind:?}（食物来源）"
                                        ),
                                    });
                                    break 'hunt;
                                }
                            }
                        }
                    }
                }
                // hunting 拾取窗口：攻击动物后自动捡掉落物（每 20 tick 一次，直到窗口结束）。
                {
                    let tick = bot.ticks_connected() as u64;
                    let until = *state.hunt_pickup_until.lock().unwrap();
                    if until > 0 && tick < until && tick % 20 == 0 && state.action_mgr.is_idle() {
                        let _ = crate::azalea::smart_actions::pickup_nearby_items(&bot).await;
                    }
                    if until > 0 && tick >= until {
                        *state.hunt_pickup_until.lock().unwrap() = 0;
                    }
                }
                // cowardice：hp 低 + 附近有敌对 → 自动逃离（Mindcraft 移植）。
                // self_defense 只攻击 4 格内敌人，而僵尸/骷髅 16m 外扑来时 LLM 回合
                // 30-60s 太慢（实测 hp=1 濒死时 LLM 想撤退但 goto 连续失败，被僵尸追死）。
                // P77：阈值 hp<6→hp<10（骷髅 2 箭 7-9 伤害就能破 6，之前的阈值太晚；
                // Mindcraft 是无条件 16m 逃，我们保留 hp 门槛避免 bot 见怪就放弃主线）。
                // 地下→自动向上挖洞逃生（僵尸不会挖方块）；地表→向远离敌人方向走 20 格。
                // 优先于 self_defense：hp<10 时 self_defense 的攻击会被跳过。
                if bot.ticks_connected() % 100 == 0 {
                    let health = bot.health().unwrap_or(20.0);
                    if health < 10.0 {
                        let mut flee_dir: Option<(f64, f64)> = None;
                        if let Ok(entities) = bot
                            .nearest_entities::<bevy_ecs::query::Without<azalea::entity::metadata::Player>>()
                        {
                            let self_id = bot.entity().id();
                            let self_pos = bot.position().ok();
                            for e in entities.iter() {
                                if e.id() == self_id {
                                    continue;
                                }
                                if flee_dir.is_some() {
                                    break;
                                }
                                if let Ok(kind) = e.kind() {
                                    let hostile = matches!(
                                        kind,
                                        EntityKind::Zombie
                                            | EntityKind::Skeleton
                                            | EntityKind::Creeper
                                            | EntityKind::Spider
                                            | EntityKind::CaveSpider
                                            | EntityKind::Enderman
                                            | EntityKind::Pillager
                                            | EntityKind::Phantom
                                            | EntityKind::Witch
                                            | EntityKind::Drowned
                                            | EntityKind::Husk
                                            | EntityKind::Stray
                                            // P77：下界/末地敌对（dragon 主线的自动防御保障）
                                            | EntityKind::Blaze
                                            | EntityKind::Ghast
                                            | EntityKind::Piglin
                                            | EntityKind::PiglinBrute
                                            | EntityKind::ZombifiedPiglin
                                            | EntityKind::Guardian
                                            | EntityKind::ElderGuardian
                                            | EntityKind::Shulker
                                            | EntityKind::Vex
                                            | EntityKind::Wither
                                            | EntityKind::WitherSkeleton
                                            | EntityKind::MagmaCube
                                    );
                                    if hostile {
                                        if let (Some(sp), Ok(ep)) = (self_pos, e.position()) {
                                            let dx = sp.x - ep.x;
                                            let dz = sp.z - ep.z;
                                            let d = (dx * dx + dz * dz).sqrt();
                                            // 20m 半径：僵尸 18m 外徘徊时也要提前逃
                                            // （实测 hp=1 时僵尸 18m 处 bot 原地等死，LLM 回合太慢）
                                            if d <= 20.0 && d > 0.01 {
                                                flee_dir = Some((dx / d, dz / d));
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        if let (Some((fx, fz)), Ok(p)) = (flee_dir, bot.position()) {
                            let head_blocked = bot
                                .world()
                                .ok()
                                .map(|w| {
                                    w.read()
                                        .get_block_state(BlockPos::new(
                                            p.x.floor() as i32,
                                            p.y.floor() as i32 + 1,
                                            p.z.floor() as i32,
                                        ))
                                        .map(|b| !b.is_air())
                                        .unwrap_or(false)
                                })
                                .unwrap_or(false);
                            if (p.y.floor() as i32) < 62 || head_blocked {
                                // 地下：向上挖逃生
                                if !*state.mining_above.lock().unwrap() {
                                    *state.mining_above.lock().unwrap() = true;
                                    *state.mining_above_start_y.lock().unwrap() =
                                        Some(p.y.floor() as i32);
                                    *state.mining_above_direction.lock().unwrap() = 0;
                                    *state.mine_above_tried_tp.lock().unwrap() = false;
                                    bot.force_stop_pathfinding();
                                    let _ = evt_tx.send(BotEvent::Chat {
                                        content: format!(
                                            "[MODE:cowardice] HP {health:.0}/20 过低且附近有敌对生物，自动向上挖洞逃生（mine_above）"
                                        ),
                                    });
                                }
                            } else {
                                // 地表：向远离方向走 20 格
                                let tx = (p.x + fx * 20.0).floor() as i32;
                                let ty = p.y.floor() as i32;
                                let tz = (p.z + fz * 20.0).floor() as i32;
                                let escape_cmd = BotCommand::Goto {
                                    x: tx,
                                    y: ty,
                                    z: tz,
                                };
                                let tick_now = bot.ticks_connected() as u64;
                                let _ = state.action_mgr.submit(
                                    escape_cmd,
                                    Priority::High,
                                    &cmd_queue,
                                    tick_now,
                                );
                                let _ = evt_tx.send(BotEvent::Chat {
                                    content: format!(
                                        "[MODE:cowardice] HP {health:.0}/20 过低且附近有敌对生物，自动向 ({tx},{ty},{tz}) 逃离"
                                    ),
                                });
                            }
                        }
                    }
                }
                // self_defense：空闲或寻路途中自动攻击附近敌对生物（每 100 tick ≈5s 检查一次）
                // 距离限制：只攻击 8 格内实体（P77：对齐 Mindcraft 的 8m；4m 太近——
                // 僵尸走到 4m 内往往已经开始扑击，8m 能提前两轮出手）。
                // 用 is_busy() 而非 is_idle()：Goto/Mine 等轮询命令执行期间 pending 非空但 busy=false，
                // 此时仍应自卫（否则 bot 寻路途中被僵尸攻击不还手——H3 bug）。
                // 只在异步命令（Craft/Gather/Smelt）执行中（busy=true）跳过，避免抢占。
                // hp<10 时不攻击（cowardice 逃跑优先，避免濒死还硬刚被补刀——P77 随 cowardice 同步 6→10）。
                // P77：主手非武器且背包有剑/斧 → 自动装备（Mindcraft pvp 插件默认行为），
                // 装备请求期间跳过攻击（等 5s 后下一轮装备好再打）。
                // P77：creeper ≤3m（爆炸半径）→ 撤离优先于攻击。
                if !state.action_mgr.is_busy()
                    && !*state.mining_below.lock().unwrap()
                    && bot.health().unwrap_or(20.0) >= 10.0
                    && bot.ticks_connected() % 100 == 0
                {
                    if let Ok(entities) = bot.nearest_entities::<bevy_ecs::query::Without<azalea::entity::metadata::Player>>() {
                    let self_id = bot.entity().id();
                    let self_pos = bot.position().ok();
                    let mut attacked = false;
                    let mut creeper_evaded = false;
                    for e in entities.iter() {
                        if e.id() == self_id { continue; }
                        if attacked { break; }
                        if let Ok(kind) = e.kind() {
                            let hostile = matches!(kind,
                                EntityKind::Zombie | EntityKind::Skeleton | EntityKind::Creeper
                                | EntityKind::Spider | EntityKind::CaveSpider | EntityKind::Enderman
                                | EntityKind::Pillager | EntityKind::Phantom | EntityKind::Witch
                                | EntityKind::Drowned | EntityKind::Husk | EntityKind::Stray
                                // P77：下界/末地敌对（dragon 主线的自动防御保障）
                                | EntityKind::Blaze | EntityKind::Ghast | EntityKind::Piglin
                                | EntityKind::PiglinBrute | EntityKind::ZombifiedPiglin
                                | EntityKind::Guardian | EntityKind::ElderGuardian
                                | EntityKind::Shulker | EntityKind::Vex | EntityKind::Wither
                                | EntityKind::WitherSkeleton | EntityKind::MagmaCube
                            );
                            if hostile {
                                // creeper 3m 内：爆炸半径内，撤离优先（High 优先级 goto 8m 外）
                                if kind == EntityKind::Creeper {
                                    if let (Some(sp), Ok(ep)) = (self_pos, e.position()) {
                                        let d = ((sp.x - ep.x).powi(2)
                                            + (sp.y - ep.y).powi(2)
                                            + (sp.z - ep.z).powi(2)).sqrt();
                                        if d <= 3.0 {
                                            let mut dx = sp.x - ep.x;
                                            let mut dz = sp.z - ep.z;
                                            let dl = (dx * dx + dz * dz).sqrt();
                                            if dl < 0.1 { dx = 1.0; dz = 0.0; } else { dx /= dl; dz /= dl; }
                                            let tx = (sp.x + dx * 8.0).floor() as i32;
                                            let ty = sp.y.floor() as i32;
                                            let tz = (sp.z + dz * 8.0).floor() as i32;
                                            let tick_now = bot.ticks_connected() as u64;
                                            let _ = state.action_mgr.submit(
                                                BotCommand::Goto { x: tx, y: ty, z: tz },
                                                Priority::High,
                                                &cmd_queue,
                                                tick_now,
                                            );
                                            let _ = evt_tx.send(BotEvent::Chat {
                                                content: format!("[MODE] creeper {d:.1}m 内即将爆炸，自动撤离 ({tx},{ty},{tz})"),
                                            });
                                            creeper_evaded = true;
                                            attacked = true;
                                            break;
                                        }
                                    }
                                }
                                // 距离检查：8 格内才攻击（远距离敌人由 LLM 决策是否拉近或撤退）
                                let in_range = if let Some(sp) = self_pos {
                                    if let Ok(ep) = e.position() {
                                        let d = ((sp.x - ep.x).powi(2)
                                            + (sp.y - ep.y).powi(2)
                                            + (sp.z - ep.z).powi(2)).sqrt();
                                        d <= 8.0
                                    } else {
                                        false
                                    }
                                } else {
                                    false
                                };
                                if !in_range { continue; }
                                // 自动换武器：主手非武器且背包有剑/斧 → Equip（防重复，本轮跳过攻击）
                                let held_is_weapon = bot
                                    .get_held_item()
                                    .ok()
                                    .map(|s| {
                                        let id = s.kind().to_str();
                                        id.ends_with("_sword") || id.ends_with("_axe")
                                    })
                                    .unwrap_or(false);
                                if !held_is_weapon {
                                    let mut pending = state.combat_equip_pending.lock().unwrap();
                                    if pending.is_none() {
                                        if let Ok(inv) = bot.get_inventory() {
                                            let best = [
                                                "diamond_sword", "iron_sword", "stone_sword",
                                                "wooden_sword", "diamond_axe", "iron_axe",
                                                "stone_axe", "wooden_axe",
                                            ]
                                            .iter()
                                            .find_map(|n| {
                                                ItemKind::from_str(n).ok().filter(|k| {
                                                    find_item_slots(&inv, *k).first().is_some()
                                                })
                                            });
                                            if let Some(k) = best {
                                                let name = k.to_str()
                                                    .strip_prefix("minecraft:")
                                                    .unwrap_or_else(|| k.to_str())
                                                    .to_string();
                                                *pending = Some(name.clone());
                                                let tick_now = bot.ticks_connected() as u64;
                                                let _ = state.action_mgr.submit(
                                                    BotCommand::Equip {
                                                        item: name.clone(),
                                                        slot: "hand".into(),
                                                    },
                                                    Priority::Normal,
                                                    &cmd_queue,
                                                    tick_now,
                                                );
                                                let _ = evt_tx.send(BotEvent::Chat {
                                                    content: format!("[MODE:self_defense] 主手无武器，自动装备 {name}"),
                                                });
                                            }
                                        }
                                    }
                                    continue;
                                } else {
                                    *state.combat_equip_pending.lock().unwrap() = None;
                                }
                                // 攻击前检查实体是否存活（get_component 失败说明已消失）
                                let indexed = bot
                                    .query_self::<&azalea::entity::indexing::EntityIdIndex, _>(|index| {
                                        index.contains_ecs_entity(e.id())
                                    })
                                    .unwrap_or(false);
                                if indexed && e.get_component::<azalea::entity::EntityKindComponent>().is_some() {
                                    e.attack();
                                    attacked = true;
                                    let _ = evt_tx.send(BotEvent::Chat {
                                        content: format!("[MODE] 攻击 {kind:?}"),
                                    });
                                }
                            }
                        }
                    }
                }
                }
            }
            _ => {}
        }
        bot
    }

    /// 攻击最近的生物（自卫/狩猎）。
    pub fn attack(&self, target: String) {
        self.push_cmd(BotCommand::Attack { target });
    }

    /// 2×2 背包合成（无需工作台）。item 为目标物品 id（如 "oak_planks"），count 为期望数量。
    pub fn craft_2x2(&self, item: String, count: u32) {
        self.push_cmd(BotCommand::Craft2x2 { item, count });
    }

    /// 3×3 工作台合成（P1-4：自动放收桌）。
    /// table_pos=Some 时使用该坐标的现有工作台；None 时 bot 自动放置+打开+关闭工作台。
    pub fn craft_3x3(&self, item: String, count: u32, table_pos: Option<(i32, i32, i32)>) {
        self.push_cmd(BotCommand::Craft3x3 {
            item,
            count,
            table_pos,
        });
    }

    /// 熔炼（P1-4：自动放收炉）。
    /// table_pos=Some 时使用该坐标的现有熔炉；None 时 bot 自动放置+打开+关闭熔炉。
    pub fn smelt(
        &self,
        output: String,
        fuel: String,
        count: u32,
        table_pos: Option<(i32, i32, i32)>,
    ) {
        self.push_cmd(BotCommand::Smelt {
            output,
            fuel,
            count,
            table_pos,
        });
    }

    /// 采集最近的指定方块（如 "oak_log"）并挖掘，直到背包有 count 个。
    pub fn gather(&self, item: String, count: u32) {
        self.push_cmd(BotCommand::Gather { item, count });
    }

    /// 把手持物品 item 放置到世界坐标 (x,y,z) 旁。
    pub fn place(&self, item: String, x: i32, y: i32, z: i32) {
        self.push_cmd(BotCommand::Place { item, x, y, z });
    }

    /// 打开世界坐标 (x,y,z) 处的容器（工作台/熔炉/箱子等）。
    pub fn open_container(&self, x: i32, y: i32, z: i32) {
        self.push_cmd(BotCommand::OpenContainer { x, y, z });
    }

    /// 高层自动合成（木链）：一句话造木制品（如 chest）。
    pub fn auto_craft(&self, item: String, count: u32) {
        self.push_cmd(BotCommand::AutoCraft { item, count });
    }

    /// 附魔：给背包中 item 附魔（需已打开附魔台且背包有 item 与青金石）。
    /// level 1/2/3 对应附魔台三个选项。
    pub fn enchant(&self, item: String, level: u32) {
        self.push_cmd(BotCommand::Enchant { item, level });
    }

    /// 村民交易：与最近的村民交易，选第 offer 个报价（0 起）。bot 自动打开村民。
    pub fn trade(&self, offer: u32) {
        self.push_cmd(BotCommand::Trade { offer });
    }

    /// 实体右键交互（打开村民/动物/展示框等）。kind 如 "villager"。
    pub fn interact_entity(&self, kind: String) {
        self.push_cmd(BotCommand::InteractEntity { kind });
    }

    /// 装备背包中的物品到指定槽位（hand/helmet/chestplate/leggings/boots）。
    pub fn equip(&self, item: String, slot: String) {
        self.push_cmd(BotCommand::Equip { item, slot });
    }

    /// 丢弃背包中的指定物品。count=0 全部，count>0 指定数量。
    pub fn discard(&self, item: String, count: u32) {
        self.push_cmd(BotCommand::Discard { item, count });
    }

    /// 消耗（吃/喝）背包中的指定物品。
    pub fn consume(&self, item: String) {
        self.push_cmd(BotCommand::Consume { item });
    }

    /// 查看世界坐标 (x,y,z) 处容器的物品列表。
    pub fn chest_view(&self, x: i32, y: i32, z: i32) {
        self.push_cmd(BotCommand::ChestView { x, y, z });
    }

    /// 从世界坐标 (x,y,z) 处容器取出 item（count 个）到 bot 背包。
    pub fn chest_withdraw(&self, x: i32, y: i32, z: i32, item: String, count: u32) {
        self.push_cmd(BotCommand::ChestWithdraw {
            x,
            y,
            z,
            item,
            count,
        });
    }

    /// 把背包中的 item（count 个）存入世界坐标 (x,y,z) 处容器。
    pub fn chest_deposit(&self, x: i32, y: i32, z: i32, item: String, count: u32) {
        self.push_cmd(BotCommand::ChestDeposit {
            x,
            y,
            z,
            item,
            count,
        });
    }

    /// 推送动作指令（fire-and-forget，handler tick 中执行）。
    fn push_cmd(&self, cmd: BotCommand) {
        self.cmd_queue.lock().unwrap().push(QueuedCommand {
            cmd,
            result_tx: None,
        });
    }

    /// 推送动作指令并等待执行结果（同步阻塞，超时默认 120s）。
    /// 返回命令执行后的结果描述字符串。
    pub fn push_cmd_and_wait(&self, cmd: BotCommand, timeout_ms: u64) -> anyhow::Result<String> {
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        self.cmd_queue.lock().unwrap().push(QueuedCommand {
            cmd,
            result_tx: Some(tx),
        });
        match rx.recv_timeout(Duration::from_millis(timeout_ms)) {
            Ok(msg) => Ok(msg),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                Err(anyhow::anyhow!("命令执行超时 ({}ms)", timeout_ms))
            }
            Err(e) => Err(anyhow::anyhow!("命令结果通道错误: {}", e)),
        }
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

    #[test]
    fn chat_parser_handles_give_count_and_target_forms() {
        assert!(matches!(
            parse_chat_command("give diamond 3 Steve"),
            Some(BotCommand::Give {
                item,
                count: 3,
                target: Some(target),
            }) if item == "diamond" && target == "Steve"
        ));
        assert!(matches!(
            parse_chat_command("give diamond Steve"),
            Some(BotCommand::Give {
                item,
                count: 0,
                target: Some(target),
            }) if item == "diamond" && target == "Steve"
        ));
        assert!(matches!(
            parse_chat_command("give diamond"),
            Some(BotCommand::Give { item, count: 0, target: None }) if item == "diamond"
        ));
    }

    #[test]
    fn chat_parser_rejects_malformed_coordinates_and_preserves_follow() {
        assert!(parse_chat_command("goto 1 2").is_none());
        assert!(matches!(
            parse_chat_command("follow Steve"),
            Some(BotCommand::Follow { target: Some(target) }) if target == "Steve"
        ));
        assert!(matches!(
            parse_chat_command("follow"),
            Some(BotCommand::Follow { target: None })
        ));
        assert!(matches!(
            parse_chat_command("stop"),
            Some(BotCommand::StopFollow)
        ));
    }
}

/// 便捷类型：共享的 bot 句柄。
pub type SharedBot = Arc<AzaleaBot>;

// ── 背包管理三件套：equip / discard / consume ──
//
// 学习自 Mindcraft library/skills.js 的 equip / discard / consumeItem。
// 这些是生存闭环的关键缺口：原系统只能挖/合成/放，无法切装备、丢垃圾、吃东西，
// 导致 bot 血量低时无法吃食物回血、背包满时无法丢垃圾腾空间。

use azalea::container::ContainerHandleRef;
use azalea::inventory::operations::ThrowClick;
use azalea_registry::builtin::ItemKind;
use std::str::FromStr;
use tokio::time::sleep;

/// 把 "oak_planks" / "minecraft:oak_planks" 统一为 "minecraft:oak_planks"。
fn normalize_item_id(item: &str) -> String {
    if item.starts_with("minecraft:") {
        item.to_string()
    } else {
        format!("minecraft:{item}")
    }
}

/// 在玩家背包范围（排除合成网格/盔甲槽）内找到所有持有指定物品种类的槽位。
fn find_item_slots(inv: &ContainerHandleRef, kind: ItemKind) -> Vec<usize> {
    let mut out = Vec::new();
    let Some(menu) = inv.menu().ok().flatten() else {
        return out;
    };
    let Some(slots) = inv.slots() else {
        return out;
    };
    let range = menu.player_slots_range();
    for s in range {
        if let Some(stack) = slots.get(s) {
            if !stack.is_empty() && stack.kind() == kind {
                out.push(s);
            }
        }
    }
    out
}

/// 找到背包里持有 item 的 hotbar 槽位（0..=8），无则 None。
fn find_hotbar_slot_for(inv: &ContainerHandleRef, kind: ItemKind) -> Option<u8> {
    let menu = inv.menu().ok().flatten()?;
    let slots = inv.slots()?;
    // P5 修复：原代码 idx 算反了（详见 place.rs 同名函数注释）。
    let hotbar_range = menu.hotbar_slots_range();
    let hotbar_start = *hotbar_range.start();
    for s in hotbar_range {
        if let Some(stack) = slots.get(s) {
            if !stack.is_empty() && stack.kind() == kind {
                let idx = (s - hotbar_start) as u8;
                debug_assert!(idx <= 8, "hotbar idx out of range: {idx}");
                return Some(idx);
            }
        }
    }
    None
}

/// 自动装备背包里最好的镐到主手（挖矿前调用）。
///
/// 优先级：netherite > diamond > iron > golden > stone > wooden。
/// 若主手已是任意镐则不切换（避免每 tick 切换造成闪烁）；
/// 若背包无镐则保持当前手持物品（徒手挖）。
/// 返回 Some(描述) 表示发生了切换；None 表示无需切换。
pub async fn auto_equip_best_pickaxe(bot: &Client) -> Option<String> {
    use azalea_registry::builtin::ItemKind as IK;
    // 镐品质优先级（越大越好）
    fn pickaxe_rank(k: IK) -> Option<u8> {
        match k {
            IK::NetheritePickaxe => Some(6),
            IK::DiamondPickaxe => Some(5),
            IK::IronPickaxe => Some(4),
            IK::GoldenPickaxe => Some(3),
            IK::StonePickaxe => Some(2),
            IK::WoodenPickaxe => Some(1),
            _ => None,
        }
    }
    // 主手已经是镐？不切换。
    if let Ok(st) = bot.get_held_item()
        && !st.is_empty()
        && pickaxe_rank(st.kind()).is_some()
    {
        return None;
    }
    let inv = bot.get_inventory().ok()?;
    // 找背包里最好的镐
    let menu = inv.menu().ok().flatten()?;
    let slots = inv.slots()?;
    let range = menu.player_slots_range();
    let mut best_kind: Option<IK> = None;
    let mut best_rank: u8 = 0;
    for s in range.clone() {
        if let Some(st) = slots.get(s)
            && !st.is_empty()
            && let Some(r) = pickaxe_rank(st.kind())
            && r > best_rank
        {
            best_rank = r;
            best_kind = Some(st.kind());
        }
    }
    let best_kind = best_kind?;
    drop(inv);
    // 用 do_equip 切到主手
    // P5 修复：用 to_str() 拿到 snake_case minecraft id（如 "wooden_pickaxe"），
    // 原 format!("{best_kind:?}").to_lowercase() 得到 "woodenpickaxe"（无下划线），
    // do_equip 用 ItemKind::from_str("woodenpickaxe") 解析失败 → 自动装备镐 100% 失败。
    let full = best_kind.to_str();
    let name = full.strip_prefix("minecraft:").unwrap_or(full);
    let msg = do_equip(bot, name, "hand").await;
    Some(msg)
}

/// 自动装备背包里最好的斧到主手（砍树/砍木头前调用）。
///
/// 优先级：netherite > diamond > iron > golden > stone > wooden。
/// 若主手已是任意斧则不切换；若背包无斧则保持当前手持物（徒手砍）。
/// 返回 Some(描述) 表示发生了切换；None 表示无需切换或无斧可切。
pub async fn auto_equip_best_axe(bot: &Client) -> Option<String> {
    use azalea_registry::builtin::ItemKind as IK;
    fn axe_rank(k: IK) -> Option<u8> {
        match k {
            IK::NetheriteAxe => Some(6),
            IK::DiamondAxe => Some(5),
            IK::IronAxe => Some(4),
            IK::GoldenAxe => Some(3),
            IK::StoneAxe => Some(2),
            IK::WoodenAxe => Some(1),
            _ => None,
        }
    }
    if let Ok(st) = bot.get_held_item()
        && !st.is_empty()
        && axe_rank(st.kind()).is_some()
    {
        return None;
    }
    let inv = bot.get_inventory().ok()?;
    let menu = inv.menu().ok().flatten()?;
    let slots = inv.slots()?;
    let range = menu.player_slots_range();
    let mut best_kind: Option<IK> = None;
    let mut best_rank: u8 = 0;
    for s in range.clone() {
        if let Some(st) = slots.get(s)
            && !st.is_empty()
            && let Some(r) = axe_rank(st.kind())
            && r > best_rank
        {
            best_rank = r;
            best_kind = Some(st.kind());
        }
    }
    let best_kind = best_kind?;
    drop(inv);
    let full = best_kind.to_str();
    let name = full.strip_prefix("minecraft:").unwrap_or(full);
    let msg = do_equip(bot, name, "hand").await;
    Some(msg)
}

/// 检查 bot 背包里是否有任意一种斧。
pub(crate) async fn has_any_axe_in_inventory(bot: &Client) -> bool {
    use azalea_registry::builtin::ItemKind as IK;
    let axes = [
        IK::WoodenAxe,
        IK::StoneAxe,
        IK::GoldenAxe,
        IK::IronAxe,
        IK::DiamondAxe,
        IK::NetheriteAxe,
    ];
    let Ok(inv) = bot.get_inventory() else {
        return false;
    };
    let Some(menu) = inv.menu().ok().flatten() else {
        return false;
    };
    let Some(slots) = inv.slots() else {
        return false;
    };
    let range = menu.player_slots_range();
    for s in range {
        if let Some(st) = slots.get(s)
            && !st.is_empty()
            && axes.contains(&st.kind())
        {
            return true;
        }
    }
    false
}

/// 判断方块是否是原木/木头类（砍这类方块适合用斧）。
/// 用 BlockKind::to_str() 字符串判断，避免不同版本的枚举缺项。
pub(crate) fn is_log_block(state: azalea::block::BlockState) -> bool {
    let kind: BlockKind = state.into();
    let s = kind.to_str();
    let bare = s.strip_prefix("minecraft:").unwrap_or(s);
    bare.ends_with("_log") || bare.ends_with("_wood") || bare.starts_with("stripped_")
}

/// 检查 bot 背包里是否有任意一种镐（用于 mine_above 前置校验）。
pub(crate) async fn has_any_pickaxe_in_inventory(bot: &Client) -> bool {
    use azalea_registry::builtin::ItemKind as IK;
    let pickaxes = [
        IK::WoodenPickaxe,
        IK::StonePickaxe,
        IK::GoldenPickaxe,
        IK::IronPickaxe,
        IK::DiamondPickaxe,
        IK::NetheritePickaxe,
    ];
    let Ok(inv) = bot.get_inventory() else {
        return false;
    };
    let Some(menu) = inv.menu().ok().flatten() else {
        return false;
    };
    let Some(slots) = inv.slots() else {
        return false;
    };
    let range = menu.player_slots_range();
    for s in range {
        if let Some(st) = slots.get(s)
            && !st.is_empty()
            && pickaxes.contains(&st.kind())
        {
            return true;
        }
    }
    false
}

/// 判断一个方块状态是否"硬方块"（需要镐才能有效挖掘）。
/// 软方块（dirt/grass/sand/gravel/snow/etc.）徒手可挖，返回 false。
/// 硬方块（stone/deepslate/ores/bricks/etc.）需要镐，返回 true。
/// 用于 mine_above / gather：决定是否在没有镐时直接放弃。
pub(crate) fn is_hard_block(state: azalea::block::BlockState) -> bool {
    use azalea_registry::builtin::BlockKind as B;
    let kind: BlockKind = state.into();
    matches!(
        kind,
        B::Stone
            | B::Granite
            | B::Diorite
            | B::Andesite
            | B::Deepslate
            | B::CobbledDeepslate
            | B::Tuff
            | B::CoalOre
            | B::DeepslateCoalOre
            | B::IronOre
            | B::DeepslateIronOre
            | B::CopperOre
            | B::DeepslateCopperOre
            | B::GoldOre
            | B::DeepslateGoldOre
            | B::RedstoneOre
            | B::DeepslateRedstoneOre
            | B::LapisOre
            | B::DeepslateLapisOre
            | B::DiamondOre
            | B::DeepslateDiamondOre
            | B::EmeraldOre
            | B::DeepslateEmeraldOre
            | B::NetherGoldOre
            | B::NetherQuartzOre
            | B::AncientDebris
            | B::Cobblestone
            | B::MossyCobblestone
            | B::Bedrock
            | B::Obsidian
            | B::CryingObsidian
            | B::Netherrack
            | B::Basalt
            | B::Blackstone
            | B::EndStone
            | B::Sandstone
            | B::RedSandstone
            | B::Bricks
            | B::StoneBricks
            | B::DeepslateBricks
            | B::NetherBricks
            | B::IronBlock
            | B::GoldBlock
            | B::DiamondBlock
            | B::EmeraldBlock
            | B::LapisBlock
            | B::RedstoneBlock
            | B::CoalBlock
            | B::NetheriteBlock
            | B::SmoothStone
            | B::SmoothStoneSlab
            | B::StoneSlab
    )
}

fn mine_above_reached_surface(y: i32, head_is_air: bool, five_air: bool) -> bool {
    y >= 62 && head_is_air && five_air
}

/// 返回镐的品质等级（用于判断能否挖某种方块）。
///
/// Minecraft 工具等级规则（vanilla 26.2）：
/// - 0 = 无镐（徒手挖硬方块不掉落物品）
/// - 1 = wooden/golden（可挖 stone/coal_ore/granite/diorite/andesite 等基础石类）
/// - 2 = stone（可挖 iron_ore/copper_ore/lapis_ore 等中级矿）
/// - 3 = iron（可挖 diamond_ore/gold_ore/redstone_ore/emerald_ore 等高级矿）
/// - 4 = diamond/netherite（可挖 ancient_debris/obsidian 等顶级方块）
///
/// 注意：golden_pickaxe 在 vanilla 中等同于 wooden（tier 1），尽管其挖掘速度更快。
pub(crate) fn pickaxe_tier(k: ItemKind) -> u8 {
    use azalea_registry::builtin::ItemKind as IK;
    match k {
        IK::WoodenPickaxe | IK::GoldenPickaxe => 1,
        IK::StonePickaxe => 2,
        IK::IronPickaxe => 3,
        IK::DiamondPickaxe | IK::NetheritePickaxe => 4,
        _ => 0,
    }
}

/// P39 本质修复（2026-07-27）：返回挖掘指定方块时**实际掉落的物品**。
///
/// 这是 gather 0/8 死循环的根本原因修复。原 gather 用 LLM 传入的方块名（如 "iron_ore"）
/// 作为 ItemKind 去 `count_item` 统计背包数量，但 vanilla 1.18+ 中：
/// - 挖 iron_ore / deepslate_iron_ore 方块掉落的是 **raw_iron** 物品（不是 iron_ore！）
/// - 挖 gold_ore / deepslate_gold_ore 方块掉落的是 **raw_gold** 物品
/// - 挖 copper_ore / deepslate_copper_ore 方块掉落的是 **raw_copper** 物品
/// - 挖 coal_ore / deepslate_coal_ore 方块掉落的是 **coal** 物品（不是 coal_ore）
/// - 挖 diamond_ore 方块掉落的是 **diamond** 物品
/// - 挖 redstone_ore 方块掉落的是 **redstone** 物品
/// - 挖 lapis_ore 方块掉落的是 **lapis_lazuli** 物品
/// - 挖 nether_quartz_ore 方块掉落的是 **quartz** 物品
/// - 挖 stone 方块掉落的是 **cobblestone** 物品
///
/// 所以 `count_item(inv, ItemKind::IronOre)` 永远返回 0，gather 永远 0/N 失败。
/// 之前的 P11/P15/P35 修复都只处理症状（错误信息），没修这个根因。
///
/// 本函数返回实际掉落物 ItemKind，让 gather 统计正确的物品。
/// 对于「方块本身即是掉落物」的情况（如 cobblestone, dirt, oak_log）返回 None，
/// 调用方回退到「LLM 传入的 item 名」即可。
pub(crate) fn block_drops_item(kind: BlockKind) -> Option<ItemKind> {
    use azalea_registry::builtin::BlockKind as B;
    use azalea_registry::builtin::ItemKind as IK;
    // vanilla 1.18+ raw ore 机制：挖矿石方块掉落 raw 形态
    let drop_item: ItemKind = match kind {
        // 铁矿 → raw_iron（最常见错误，LLM 经常 gather("iron_ore")）
        B::IronOre | B::DeepslateIronOre => IK::RawIron,
        // 金矿 → raw_gold
        B::GoldOre | B::DeepslateGoldOre => IK::RawGold,
        // 铜矿 → raw_copper
        B::CopperOre | B::DeepslateCopperOre => IK::RawCopper,
        // 煤矿 → coal（不是 coal_ore 物品，根本不存在 coal_ore 物品）
        B::CoalOre | B::DeepslateCoalOre => IK::Coal,
        // 钻石矿 → diamond
        B::DiamondOre | B::DeepslateDiamondOre => IK::Diamond,
        // 绿宝石矿 → emerald
        B::EmeraldOre | B::DeepslateEmeraldOre => IK::Emerald,
        // 红石矿 → redstone
        B::RedstoneOre | B::DeepslateRedstoneOre => IK::Redstone,
        // 青金石矿 → lapis_lazuli（注意是 dye 不是 ore）
        B::LapisOre | B::DeepslateLapisOre => IK::LapisLazuli,
        // 下界石英矿 → quartz
        B::NetherQuartzOre => IK::Quartz,
        // 下界金矿 → gold_nugget（多个）+ raw_gold（少量），主要掉落是 nugget，但 vanilla
        // 实际是 2-6 gold_nugget。这里用 raw_gold 是错的，应该用 gold_nugget。
        // 但因为 gold_nugget 不容易再处理，先返回 None 让 gather 走默认逻辑。
        // 实际上 nether_gold_ore 很少被 LLM 主动 gather，先不处理。
        // 石头 → 圆石（精准采集除外，bot 没有 silk touch）
        B::Stone => IK::Cobblestone,
        // 其他矿石/方块：方块本身即是掉落物（如 cobblestone→cobblestone, dirt→dirt）
        // 返回 None 让调用方回退到「LLM 传入的 item 名」
        _ => return None,
    };
    Some(drop_item)
}

/// 返回挖掘指定方块所需的最低镐品质等级。
///
/// 0 = 不需要镐（软方块如 dirt/sand/gravel，徒手可挖且掉落）
/// 1 = wooden/golden 起步（stone/coal_ore/granite 等基础石类）
/// 2 = stone 起步（iron_ore/copper_ore/lapis_ore 等中级矿）
/// 3 = iron 起步（diamond_ore/gold_ore/redstone_ore/emerald_ore 等高级矿）
/// 4 = diamond 起步（ancient_debris/obsidian 等顶级方块）
///
/// 这是 vanilla 26.2 的「工具要求」规则：等级不足的镐挖该方块时方块会消失但**不掉落物品**，
/// 这是 gather 工具「方块消失但背包数量不增」误报的根因。
pub(crate) fn block_required_pickaxe_tier(kind: BlockKind) -> u8 {
    use azalea_registry::builtin::BlockKind as B;
    // tier 4：仅 diamond/netherite 镐可挖出物品
    if matches!(kind, B::AncientDebris | B::Obsidian | B::CryingObsidian) {
        return 4;
    }
    // tier 3：iron 镐起步（diamond_ore/gold_ore/redstone_ore/emerald_ore）
    if matches!(
        kind,
        B::DiamondOre
            | B::DeepslateDiamondOre
            | B::GoldOre
            | B::DeepslateGoldOre
            | B::RedstoneOre
            | B::DeepslateRedstoneOre
            | B::EmeraldOre
            | B::DeepslateEmeraldOre
    ) {
        return 3;
    }
    // tier 2：stone 镐起步（iron_ore/copper_ore/lapis_ore）
    if matches!(
        kind,
        B::IronOre
            | B::DeepslateIronOre
            | B::CopperOre
            | B::DeepslateCopperOre
            | B::LapisOre
            | B::DeepslateLapisOre
    ) {
        return 2;
    }
    // tier 1：wooden 镐起步（stone/coal_ore/granite/diorite/andesite 等基础石类）
    if matches!(
        kind,
        B::Stone
            | B::Granite
            | B::Diorite
            | B::Andesite
            | B::Deepslate
            | B::CobbledDeepslate
            | B::Tuff
            | B::CoalOre
            | B::DeepslateCoalOre
            | B::Cobblestone
            | B::MossyCobblestone
            | B::Netherrack
            | B::Basalt
            | B::Blackstone
            | B::EndStone
            | B::Sandstone
            | B::RedSandstone
            | B::Bricks
            | B::StoneBricks
            | B::DeepslateBricks
            | B::NetherBricks
            | B::NetherGoldOre
            | B::NetherQuartzOre
    ) {
        return 1;
    }
    0
}

/// 返回 bot 背包中最高等级的镐的 tier（无镐返回 0）。
///
/// 用于 gather/smart_gather 预检：判断当前背包最好的镐能否挖目标方块。
/// 若不能，立即返回错误让 LLM 先合成更高 tier 的镐，避免「方块消失但无掉落」的死循环。
pub(crate) async fn best_pickaxe_tier_in_inventory(bot: &Client) -> u8 {
    use azalea_registry::builtin::ItemKind as IK;
    let pickaxes = [
        IK::WoodenPickaxe,
        IK::StonePickaxe,
        IK::GoldenPickaxe,
        IK::IronPickaxe,
        IK::DiamondPickaxe,
        IK::NetheritePickaxe,
    ];
    let Ok(inv) = bot.get_inventory() else {
        return 0;
    };
    let Some(menu) = inv.menu().ok().flatten() else {
        return 0;
    };
    let Some(slots) = inv.slots() else {
        return 0;
    };
    let range = menu.player_slots_range();
    let mut best = 0u8;
    for s in range {
        if let Some(st) = slots.get(s)
            && !st.is_empty()
            && pickaxes.contains(&st.kind())
        {
            best = best.max(pickaxe_tier(st.kind()));
        }
    }
    best
}

/// 返回 tier 对应的中文名（用于错误提示）。
pub(crate) fn pickaxe_tier_name(tier: u8) -> &'static str {
    match tier {
        0 => "无镐",
        1 => "木/金镐",
        2 => "石镐",
        3 => "铁镐",
        4 => "钻石/下界合金镐",
        _ => "未知",
    }
}

/// 返回需要合成哪个镐才能达到指定 tier（用于错误提示中的合成建议）。
pub(crate) fn pickaxe_to_craft_for_tier(tier: u8) -> &'static str {
    match tier {
        1 => "wooden_pickaxe（需要 oak_planks×3 + stick×2）",
        2 => "stone_pickaxe（需要 cobblestone×3 + stick×2）",
        3 => "iron_pickaxe（需要 iron_ingot×3 + stick×2；iron_ingot 需熔炼 iron_ore）",
        4 => "diamond_pickaxe（需要 diamond×3 + stick×2；diamond 需在 Y<-59 挖 diamond_ore）",
        _ => "未知镐",
    }
}

/// 装备物品到指定槽位。
///
/// - slot="hand"：把 item 移到 hotbar 并选中（武器/工具/方块都走这条路径）
/// - slot="helmet"/"chestplate"/"leggings"/"boots"：shift_click 让服务端自动归位
///   （仅对相应盔甲物品有效，服务端会拒绝非盔甲物品）
pub async fn do_equip(bot: &Client, item: &str, slot: &str) -> String {
    let kind =
        match ItemKind::from_str(&normalize_item_id(item)).or_else(|_| ItemKind::from_str(item)) {
            Ok(k) => k,
            Err(_) => return format!("未知物品 {item}"),
        };

    let slot_norm = slot.to_lowercase();
    match slot_norm.as_str() {
        "hand" | "main_hand" | "mainhand" => {
            // P11 修复（2026-07-26）：原代码一次性读 inv 检查 hotbar + 主背包，
            // 但 craft_3x3/equip 等操作刚完成后服务端可能还在同步背包状态
            // （ContainerSetContent 包可能在路上），导致 find_hotbar_slot_for 返回 None
            // 即使物品确实在 hotbar。现象：刚 craft 完 iron_pickaxe 后立即 equip 报"背包未持有"。
            // 修复：最多重试 3 次（每次间隔 200ms），覆盖服务端同步延迟场景。
            for attempt in 0..3u8 {
                let inv = match bot.get_inventory() {
                    Ok(i) => i,
                    Err(e) => return format!("获取背包失败: {e:?}"),
                };

                // 已在 hotbar？
                if let Some(h) = find_hotbar_slot_for(&inv, kind) {
                    bot.set_selected_hotbar_slot(h);
                    // P24 修复：用轮询替代单次 sleep+verify，覆盖服务端同步延迟场景。
                    // 原 sleep(80ms) + verify_held_item 只查一次，200ms 内同步没完成就误报失败。
                    // 现在轮询最多 1.5s，主手一就绪立即返回。
                    if wait_for_held_item(bot, kind, 1500).await {
                        return format!("已装备 {item} 到主手（hotbar 槽 {h}）");
                    }
                    return format!(
                        "装备 {item} 失败：set_selected_hotbar_slot({h}) 后主手仍未持有 {item}\
                         （已轮询 1.5s，可能服务端同步延迟或 hotbar 内容被覆盖，建议稍后重试）"
                    );
                }

                // 不在 hotbar，从主背包 shift_click 到 hotbar（服务端找第一个空槽）
                let srcs = find_item_slots(&inv, kind);
                if let Some(src) = srcs.first() {
                    // P8 修复：hotbar 满时 shift_click 无法移动物品。
                    // 先检查 hotbar 是否已满，若是则把第一个 hotbar 物品移到主背包腾出空位。
                    if let Some(menu) = inv.menu().ok().flatten() {
                        let hotbar_range = menu.hotbar_slots_range();
                        if let Some(slots) = inv.slots() {
                            let hotbar_full = hotbar_range
                                .clone()
                                .all(|s| slots.get(s).map(|st| !st.is_empty()).unwrap_or(false));
                            if hotbar_full {
                                // 把第一个 hotbar 物品移到主背包腾空位
                                let player_range = menu.player_slots_range();
                                for hs in hotbar_range.clone() {
                                    if let Some(st) = slots.get(hs) {
                                        if !st.is_empty() {
                                            inv.left_click(hs);
                                            sleep(Duration::from_millis(80)).await;
                                            // 找一个空的主背包槽位放下
                                            for ps in player_range.clone() {
                                                let is_empty = slots
                                                    .get(ps)
                                                    .map(|st| st.is_empty())
                                                    .unwrap_or(false);
                                                if is_empty || ps >= slots.len() {
                                                    inv.left_click(ps);
                                                    sleep(Duration::from_millis(80)).await;
                                                    break;
                                                }
                                            }
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    inv.shift_click(*src);
                    sleep(Duration::from_millis(200)).await;
                    // 重新读 backpack 拿到新 hotbar 槽
                    drop(inv);
                    let inv2 = match bot.get_inventory() {
                        Ok(i) => i,
                        Err(e) => return format!("装备后获取背包失败: {e:?}"),
                    };
                    if let Some(h) = find_hotbar_slot_for(&inv2, kind) {
                        bot.set_selected_hotbar_slot(h);
                        sleep(Duration::from_millis(80)).await;
                        // P5 修复：验证主手实际持有物
                        return match verify_held_item(bot, kind).await {
                            true => {
                                format!("已装备 {item} 到主手（从槽 {src} 移到 hotbar 槽 {h}）")
                            }
                            false => format!(
                                "装备 {item} 失败：shift_click 后主手仍未持有 {item}\
                                 （可能 hotbar 满，或服务端拒绝移动）"
                            ),
                        };
                    }
                    // P5 修复：原代码返回"已 shift_click"暗示成功——实际未装备。
                    // 改为明确报错，让 LLM 知道装备未完成。
                    return format!(
                        "装备 {item} 失败：shift_click 槽 {src} 后未在 hotbar 找到该物品。\
                         可能原因：1) hotbar 已满（9 格全非空）；2) 服务端同步延迟。\
                         建议：先 discard 一些 hotbar 里的无用物品腾出空位，再重试 equip。"
                    );
                }

                // P11 修复：背包在本次读取中没找到 item。可能是服务端同步延迟
                // （刚 craft 完物品还没出现在背包中）。等 200ms 后重试。
                drop(inv);
                if attempt < 2 {
                    eprintln!(
                        "[equip] {item} not found in inventory (attempt {}), retrying after 200ms",
                        attempt + 1
                    );
                    sleep(Duration::from_millis(200)).await;
                }
            }

            // 3 次重试后仍未找到——给出可操作的诊断
            let final_inv = match bot.get_inventory() {
                Ok(i) => i,
                Err(e) => return format!("背包未持有 {item}（获取背包失败: {e:?}）"),
            };
            let mut diag_items = Vec::new();
            if let (Some(menu), Some(slots)) = (final_inv.menu().ok().flatten(), final_inv.slots())
            {
                let range = menu.player_slots_range();
                for s in range {
                    if let Some(st) = slots.get(s) {
                        if !st.is_empty() {
                            diag_items.push(format!(
                                "slot{}={}x{}",
                                s,
                                st.kind().to_str(),
                                st.count()
                            ));
                        }
                    }
                }
            }
            // P12 修复（2026-07-26）：针对工具类（pickaxe/axe/sword/hoe）的失败
            // 增加合成建议，避免 LLM 反复 equip 不存在的物品。
            let is_tool = matches!(
                item,
                "wooden_pickaxe"
                    | "stone_pickaxe"
                    | "iron_pickaxe"
                    | "diamond_pickaxe"
                    | "netherite_pickaxe"
                    | "wooden_axe"
                    | "stone_axe"
                    | "iron_axe"
                    | "diamond_axe"
                    | "netherite_axe"
                    | "wooden_sword"
                    | "stone_sword"
                    | "iron_sword"
                    | "diamond_sword"
                    | "netherite_sword"
                    | "wooden_hoe"
                    | "stone_hoe"
                    | "iron_hoe"
                    | "diamond_hoe"
                    | "netherite_hoe"
                    | "wooden_shovel"
                    | "stone_shovel"
                    | "iron_shovel"
                    | "diamond_shovel"
                    | "netherite_shovel"
            );
            let craft_hint = if is_tool {
                let (tool_base, tier) = if item.contains("pickaxe") {
                    ("pickaxe", item.split('_').next().unwrap_or(""))
                } else if item.contains("axe") {
                    ("axe", item.split('_').next().unwrap_or(""))
                } else if item.contains("sword") {
                    ("sword", item.split('_').next().unwrap_or(""))
                } else if item.contains("hoe") {
                    ("hoe", item.split('_').next().unwrap_or(""))
                } else if item.contains("shovel") {
                    ("shovel", item.split('_').next().unwrap_or(""))
                } else {
                    ("", "")
                };
                let recipe_hint = match (tool_base, tier) {
                    ("pickaxe", "wooden") => {
                        "wooden_pickaxe = oak_planks×3 + stick×2（craft 2×2 即可，无需工作台）"
                    }
                    ("pickaxe", "stone") => {
                        "stone_pickaxe = cobblestone×3 + stick×2（需 craft_3x3 工作台）"
                    }
                    ("pickaxe", "iron") => {
                        "iron_pickaxe = iron_ingot×3 + stick×2（需 craft_3x3 工作台 + 熔炼 iron_ore→iron_ingot）"
                    }
                    ("axe", "wooden") => "wooden_axe = oak_planks×3 + stick×2（craft 2×2）",
                    ("axe", "stone") => "stone_axe = cobblestone×3 + stick×2（需 craft_3x3）",
                    ("sword", "wooden") => "wooden_sword = oak_planks×2 + stick×1（craft 2×2）",
                    ("sword", "stone") => "stone_sword = cobblestone×2 + stick×1（需 craft_3x3）",
                    _ => "",
                };
                if !recipe_hint.is_empty() {
                    format!(
                        "\n该物品是工具，请先合成：{recipe_hint}\nstick 由 2 个 planks 合成 4 个。合成后再 equip。"
                    )
                } else {
                    String::new()
                }
            } else {
                String::new()
            };
            format!(
                "背包未持有 {item}（重试 3 次后仍找不到）。\
                 可能原因：1) 物品名称错误（用 perceive 查看背包实际物品名）；\
                 2) 物品已被使用/丢弃；3) 服务端长时间未同步背包。\
                 当前背包: {}{craft_hint}",
                diag_items
                    .iter()
                    .take(15)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
        "helmet" | "chestplate" | "leggings" | "boots" => {
            // 盔甲槽位（Player 菜单）：5=helmet, 6=chestplate, 7=leggings, 8=boots
            // shift_click 让服务端自动归位（仅对正确种类的盔甲有效）
            let armor_slot_idx = match slot_norm.as_str() {
                "helmet" => 5usize,
                "chestplate" => 6,
                "leggings" => 7,
                "boots" => 8,
                _ => unreachable!(),
            };
            // P11 修复：原代码 inv 在函数顶部声明，重构后移入 for 循环——
            // 此处需独立读取一次 inv。
            // P55 修复：armor 分支原只读一次背包，刚 craft/移动后服务端同步延迟
            // 会导致"背包未持有"误报（实测 perceive 有铁甲但 equip 报未持有）。
            // 与 hand 分支的 P11 一致：最多重试 3 次（每次间隔 200ms）。
            // P56 修复：目标 armor 槽已装备同款甲 → 直接幂等返回成功。
            // 根因：find_item_slots 只搜 player_slots_range（menu 槽 9-44），
            // 而 azalea Player 菜单 armor 槽是 5-8——甲已穿上后重试 equip 必报
            // "背包未持有"，LLM 反复重穿死循环（实测 11:32 甲已上身仍被反复驱赶）。
            if verify_armor_slot(bot, armor_slot_idx, kind).await {
                return format!("已装备 {item} 到 {slot_norm}（目标槽已是该物品，无需重穿）");
            }
            let mut found_src: Option<usize> = None;
            for attempt in 0..3u8 {
                let inv = match bot.get_inventory() {
                    Ok(i) => i,
                    Err(e) => return format!("获取背包失败: {e:?}"),
                };
                let srcs = find_item_slots(&inv, kind);
                if let Some(src) = srcs.first() {
                    found_src = Some(*src);
                    drop(inv);
                    break;
                }
                drop(inv);
                if attempt < 2 {
                    sleep(Duration::from_millis(200)).await;
                }
            }
            if let Some(src) = found_src {
                // P54 修复：不用 shift_click 穿甲。azalea quick_move_stack 对 Player 菜单
                // 的 armor 处理是 TODO（只模拟 hotbar/inventory 互移），本地状态与服务端
                // QuickMove 行为不一致，且服务端可能拒绝（实测 2s 轮询仍穿不上）。
                // 改用最基础的 Pickup 点击：left_click 拿起 → left_click 盔甲槽放下。
                // P55 追加：放下前先 left_click 目标盔甲槽清空旧盔甲（若已有其他盔甲，
                // 直接 left_click 放置会与光标物品交换/失败）。
                let mut placed = false;
                for click_round in 0..3u8 {
                    if verify_armor_slot(bot, armor_slot_idx, kind).await {
                        placed = true;
                        break;
                    }
                    let inv = match bot.get_inventory() {
                        Ok(i) => i,
                        Err(e) => return format!("获取背包失败: {e:?}"),
                    };
                    // 目标槽已有其他物品 → 先拿起（放到光标）
                    let target_occupied = match inv.slots() {
                        Some(s) => s.get(armor_slot_idx).map(|st| !st.is_empty()).unwrap_or(false),
                        None => false,
                    };
                    if target_occupied {
                        inv.left_click(armor_slot_idx);
                        sleep(Duration::from_millis(120)).await;
                    }
                    let inv2 = match bot.get_inventory() {
                        Ok(i) => i,
                        Err(e) => return format!("获取背包失败: {e:?}"),
                    };
                    let srcs = find_item_slots(&inv2, kind);
                    let src = match srcs.first() {
                        Some(s) => *s,
                        None => {
                            // 背包里也没有（可能刚放上去了）→ 验证一次再决定
                            drop(inv2);
                            sleep(Duration::from_millis(100)).await;
                            if verify_armor_slot(bot, armor_slot_idx, kind).await {
                                placed = true;
                            }
                            break;
                        }
                    };
                    inv2.left_click(src);
                    sleep(Duration::from_millis(120)).await;
                    inv2.left_click(armor_slot_idx);
                    drop(inv2);
                    // 轮询验证（每次点击轮 2s），覆盖服务端同步延迟。
                    for _ in 0..20u8 {
                        sleep(Duration::from_millis(100)).await;
                        if verify_armor_slot(bot, armor_slot_idx, kind).await {
                            placed = true;
                            break;
                        }
                    }
                    if placed {
                        break;
                    }
                    eprintln!(
                        "[equip] armor click_round {} failed for {item}, retrying",
                        click_round + 1
                    );
                }
                if placed {
                    return format!("已装备 {item} 到 {slot_norm}（left_click 槽 {src}→{armor_slot_idx}）");
                }
                return format!(
                    "装备 {item} 到 {slot_norm} 失败：left_click 后轮询 2s×3，盔甲槽仍未持有 {item}。\
                     可能原因：1) {item} 不是 {slot_norm} 类型的盔甲（如 leggings 放 helmet 槽）；\
                     2) 服务端同步持续延迟。建议：用 perceive 查看当前盔甲槽状态。"
                );
            }
            format!(
                "背包未持有 {item}（重试 3 次后仍找不到）。\
                 可能原因：1) 物品名称错误（用 perceive 查看背包实际物品名）；\
                 2) 服务端长时间未同步背包。"
            )
        }
        other => format!("不支持的槽位 {other}（可选：hand/helmet/chestplate/leggings/boots）"),
    }
}

/// 验证 bot 主手是否持有指定 ItemKind（用于 do_equip 后置校验）。
async fn verify_held_item(bot: &Client, expected: ItemKind) -> bool {
    match bot.get_held_item() {
        Ok(st) if !st.is_empty() => st.kind() == expected,
        _ => false,
    }
}

/// P24 新增（2026-07-27）：轮询等待主手变为指定物品。
///
/// 背景：`set_selected_hotbar_slot` 只是在本地 ECS 触发一个事件，真正发包
/// （`ServerboundSetCarriedItem`）由 `ensure_has_sent_carried_item` 系统在
/// 下一个 tick 发送，服务端处理后才会更新 bot 主手。原代码 `sleep(200ms)`
/// 后直接 `block_interact`，但：
/// 1. 200ms（4 tick）可能不够——如果 bevy Update 被其他系统占用，发包会延迟。
/// 2. 即使服务端收到切换包，hotbar[slot] 的内容可能还没同步（shift_click
///    是 QuickMove，服务端异步处理），导致服务端认为 bot 主手是空手/旧物品。
/// 3. `block_interact` 用 `force_block` 绕过 crosshair 检查，但服务端仍会
///    校验 bot 主手物品——空手不会放下方块。
///
/// 修复：轮询最多 `timeout_ms`（默认 1500ms），每 100ms 查一次主手物品。
/// 一旦主手变为 expected 立即返回 true；超时返回 false。
///
/// 学习自 mindcraft equipItem：mindcraft 也只是 sleep(200) 后检查一次，
/// 但 mineflayer 的 inventory 同步比 azalea 更即时（mineflayer 是纯 JS，
/// 没有 bevy ECS 的 tick 延迟）。azalea 需要更保守的同步策略。
pub async fn wait_for_held_item(bot: &Client, expected: ItemKind, timeout_ms: u64) -> bool {
    let rounds = (timeout_ms / 100).max(1) as usize;
    for _ in 0..rounds {
        sleep(Duration::from_millis(100)).await;
        match bot.get_held_item() {
            Ok(st) if !st.is_empty() && st.kind() == expected => return true,
            _ => continue,
        }
    }
    false
}

/// 验证指定盔甲槽（5=helmet/6=chestplate/7=leggings/8=boots）是否持有指定 ItemKind。
async fn verify_armor_slot(bot: &Client, armor_slot: usize, expected: ItemKind) -> bool {
    let inv = match bot.get_inventory() {
        Ok(i) => i,
        Err(_) => return false,
    };
    let slots = match inv.slots() {
        Some(s) => s,
        None => return false,
    };
    // 盔甲槽是 Player 菜单的固定槽位 5/6/7/8（不在 player_slots_range 内）
    match slots.get(armor_slot) {
        Some(st) if !st.is_empty() => st.kind() == expected,
        _ => false,
    }
}

/// 丢弃背包中的指定物品。
///
/// count=0 表示丢弃全部；count>0 表示丢弃指定数量（按堆丢，最后不足一堆用 Single 丢）。
/// 丢弃后物品以掉落物形式存在于 bot 脚边，可重新捡起。
pub async fn do_discard(bot: &Client, item: &str, count: u32) -> String {
    let kind =
        match ItemKind::from_str(&normalize_item_id(item)).or_else(|_| ItemKind::from_str(item)) {
            Ok(k) => k,
            Err(_) => return format!("未知物品 {item}"),
        };
    let inv = match bot.get_inventory() {
        Ok(i) => i,
        Err(e) => return format!("获取背包失败: {e:?}"),
    };

    let slots = find_item_slots(&inv, kind);
    if slots.is_empty() {
        return format!("背包未持有 {item}（无需丢弃）");
    }

    let mut dropped: u32 = 0;
    let mut remaining = count; // 0 表示全丢
    // 取一份槽位 → 堆叠数快照（避免每次 click 后 re-read 引用失效）
    let stack_counts: Vec<(usize, u32)> = slots
        .iter()
        .filter_map(|&s| {
            let sc = inv
                .slots()?
                .get(s)
                .filter(|st| !st.is_empty())
                .map(|st| st.count() as u32)?;
            Some((s, sc))
        })
        .collect();
    for (s, stack_count) in stack_counts {
        if count != 0 && remaining == 0 {
            break;
        }
        if stack_count == 0 {
            continue;
        }
        if count == 0 || remaining >= stack_count {
            // 丢整堆
            inv.click(ThrowClick::All { slot: s as u16 });
            sleep(Duration::from_millis(60)).await;
            dropped += stack_count;
            remaining = remaining.saturating_sub(stack_count);
        } else {
            // 丢指定数量（单个丢）
            for _ in 0..remaining {
                inv.click(ThrowClick::Single { slot: s as u16 });
                sleep(Duration::from_millis(40)).await;
            }
            dropped += remaining;
            remaining = 0;
        }
    }
    if count == 0 {
        // 扔出后有 ~2s pickup delay，随后 1.5m 内会被服务端自动拾回。
        // 必须立即走开 3+ 格，否则刚丢的掉落物会被吸回背包（LLM 看到"丢不掉"死循环）。
        // 依次尝试 4 个方向各走 4 格，覆盖 2s 延迟窗口。
        let start_pos = bot.position().ok();
        let dirs = [(1.0, 0.0), (0.0, 1.0), (-1.0, 0.0), (0.0, -1.0)];
        for (dx, dz) in dirs {
            if let Ok(p) = bot.position() {
                bot.start_goto(BlockPosGoal(BlockPos::new(
                    (p.x + dx * 4.0).floor() as i32,
                    p.y.floor() as i32,
                    (p.z + dz * 4.0).floor() as i32,
                )));
                sleep(Duration::from_millis(1300)).await;
            }
        }
        bot.stop_pathfinding();
        // 验证是否真的走开（若原地打转则物品会被吸回）
        let moved_away = match (start_pos, bot.position().ok()) {
            (Some(s), Some(p)) => {
                (p.x - s.x).abs() > 2.0 || (p.z - s.z).abs() > 2.0
            }
            _ => true,
        };
        // 验证物品是否还在背包（吸回检测）
        let still_held = bot
            .get_inventory()
            .ok()
            .and_then(|inv| {
                let slots = inv.slots()?;
                Some(
                    slots
                        .iter()
                        .filter(|st| !st.is_empty() && st.kind() == kind)
                        .map(|st| st.count() as u32)
                        .sum::<u32>(),
                )
            })
            .unwrap_or(0);
        if still_held > 0 {
            format!(
                "已丢弃 {item}（共 {dropped} 个），但扔出的掉落物被 1.5m 自动拾取吸回，背包仍剩 {still_held} 个。\
                 可能原因：走开失败（被卡住/路径不可达）。请先换个开阔平坦的位置，再重试 discard。"
            )
        } else if moved_away {
            format!("已丢弃全部 {item}（共 {dropped} 个），已走开避免吸回")
        } else {
            format!("已丢弃全部 {item}（共 {dropped} 个）")
        }
    } else {
        format!("已丢弃 {dropped} 个 {item}")
    }
}

/// 消耗（吃/喝）背包中的指定物品。
///
/// 把 item 移到主手并按住右键使用。食物 32 tick（1.6s）吃完一个，这里等待 2s 兜底。
/// 药水 32 tick 喝完。返回时物品已被服务端消耗。
pub async fn do_consume(bot: &Client, item: &str) -> String {
    let kind =
        match ItemKind::from_str(&normalize_item_id(item)).or_else(|_| ItemKind::from_str(item)) {
            Ok(k) => k,
            Err(_) => return format!("未知物品 {item}"),
        };
    let inv = match bot.get_inventory() {
        Ok(i) => i,
        Err(e) => return format!("获取背包失败: {e:?}"),
    };

    // 已在 hotbar？
    let mut hotbar_slot = find_hotbar_slot_for(&inv, kind);
    if hotbar_slot.is_none() {
        // 从主背包 shift_click 到 hotbar
        let srcs = find_item_slots(&inv, kind);
        if let Some(src) = srcs.first() {
            inv.shift_click(*src);
            sleep(Duration::from_millis(150)).await;
            drop(inv);
            let inv2 = match bot.get_inventory() {
                Ok(i) => i,
                Err(e) => return format!("消耗前获取背包失败: {e:?}"),
            };
            hotbar_slot = find_hotbar_slot_for(&inv2, kind);
        }
    }
    let Some(h) = hotbar_slot else {
        // 诊断：找出 player_slots_range、所有非空槽位、匹配 kind 的槽位
        // 注意：inv 可能已被上面的 drop(inv) 释放，重新获取一份
        let mut diag = String::new();
        if let Ok(inv3) = bot.get_inventory() {
            if let Some(menu) = inv3.menu().ok().flatten() {
                let range = menu.player_slots_range();
                diag.push_str(&format!("player_slots_range={range:?}; "));
                if let Some(slots) = inv3.slots() {
                    let nonempty: Vec<String> = slots
                        .iter()
                        .enumerate()
                        .filter(|(_, s)| !s.is_empty())
                        .map(|(i, s)| format!("slot{i}:{:?}", s.kind()))
                        .collect();
                    diag.push_str(&format!(
                        "非空槽位({}): {}",
                        nonempty.len(),
                        nonempty.join(", ")
                    ));
                }
            }
        }
        return format!("背包未持有 {item}（无法消耗）[diag: {diag}]");
    };

    bot.set_selected_hotbar_slot(h);
    sleep(Duration::from_millis(100)).await;
    // 记录消耗前的数量，便于回报
    let before = count_item(bot, kind);

    // P8 修复（2026-07-26）：右键命中方块会被当作 ServerboundUseItemOn（右键方块），
    // 服务端不会消耗食物，而是尝试放置/交互方块——表现为"数量未减少(2→2)"。
    // 修复：先把视角朝向正上方（x_rot=-90），让 hit_result.miss=true，
    // 这样 azalea 才会发 ServerboundUseItem（右键空气），服务端才会执行食用逻辑。
    // 保存原方向，吃完后恢复，避免影响后续寻路/挖矿。
    let orig_direction = bot.direction().ok();
    let _ = bot.set_direction(0.0, -89.0); // 朝天（接近 -90 但留 1 度避免边界问题）
    sleep(Duration::from_millis(150)).await; // 等方向同步到服务端

    // Minecraft 吃食物需要「持续按住右键 32 tick (1.6s)」服务端才完成消耗。
    // azalea 的 start_use_item() 只发一次 ServerboundUseItem 包（单次点击），
    // 不足以完成进食——单次使用包发完服务端会等持续按住，若没有后续 use 信号
    // 就不会减少物品数量，表现为「可能不是可消耗物品」。
    // 修复：循环调用 start_use_item() 每 50ms 一次，持续 2.5s，模拟持续按住右键。
    // （每隔 ~1 tick 重发一次 use 包，让服务端累计使用时长到 32 tick 完成消耗）
    let hold_total_ms = 2500u64;
    let step_ms = 50u64;
    let mut steps = 0u64;
    while steps * step_ms < hold_total_ms {
        bot.start_use_item();
        sleep(Duration::from_millis(step_ms)).await;
        steps += 1;
        // 提前检测：数量已减少说明消耗成功，无需继续按住
        if count_item(bot, kind) < before {
            // 再等一小会让动画完成
            sleep(Duration::from_millis(200)).await;
            break;
        }
    }
    // 恢复原方向
    if let Some(orig) = orig_direction {
        let _ = bot.set_direction(orig.y_rot(), orig.x_rot());
        sleep(Duration::from_millis(80)).await;
    }
    let after = count_item(bot, kind);
    if after < before {
        format!(
            "已消耗 {}（{} → {}，-{}）",
            item,
            before,
            after,
            before - after
        )
    } else {
        // P8 改进：根据饥饿值给更精准的提示
        let hint = if let Ok(h) = bot.hunger() {
            if h.food >= 20 {
                format!(
                    "饥饿值已满 ({}/20)，无法进食——先消耗体力或受伤降低饱食度",
                    h.food
                )
            } else {
                format!(
                    "饥饿值 {}/20（应该可进食但未生效），可能服务端拒绝或物品不可食用",
                    h.food
                )
            }
        } else {
            "可能不是可消耗物品或饥饿值已满".to_string()
        };
        format!("尝试消耗 {item}，但数量未减少（{before} → {after}，{hint}）")
    }
}

/// 统计背包中指定物品的总数。
fn count_item(bot: &Client, kind: ItemKind) -> u32 {
    let Some(slots) = bot.get_inventory().ok().and_then(|i| i.slots()) else {
        return 0;
    };
    slots
        .iter()
        .filter(|s| !s.is_empty() && s.kind() == kind)
        .map(|s| s.count() as u32)
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use azalea_registry::builtin::{BlockKind as B, ItemKind as IK};

    /// 验证 pickaxe_tier 返回正确的等级。
    /// vanilla 规则：wooden/golden=1, stone=2, iron=3, diamond/netherite=4, 其他=0。
    #[test]
    fn pickaxe_tier_returns_correct_levels() {
        assert_eq!(pickaxe_tier(IK::WoodenPickaxe), 1);
        assert_eq!(pickaxe_tier(IK::GoldenPickaxe), 1);
        assert_eq!(pickaxe_tier(IK::StonePickaxe), 2);
        assert_eq!(pickaxe_tier(IK::IronPickaxe), 3);
        assert_eq!(pickaxe_tier(IK::DiamondPickaxe), 4);
        assert_eq!(pickaxe_tier(IK::NetheritePickaxe), 4);
        // 非镐物品
        assert_eq!(pickaxe_tier(IK::WoodenAxe), 0);
        assert_eq!(pickaxe_tier(IK::Stick), 0);
        assert_eq!(pickaxe_tier(IK::Air), 0);
    }

    #[test]
    fn mine_above_does_not_treat_deep_cave_as_surface() {
        assert!(!mine_above_reached_surface(-16, true, true));
        assert!(!mine_above_reached_surface(62, true, false));
        assert!(mine_above_reached_surface(62, true, true));
    }

    /// 验证 block_required_pickaxe_tier 返回正确的需求等级。
    /// 这是 gather 工具判断「镐等级是否足够」的关键依据。
    #[test]
    fn block_required_pickaxe_tier_correct_for_each_category() {
        // tier 0：软方块（不需要镐）
        assert_eq!(block_required_pickaxe_tier(B::Dirt), 0);
        assert_eq!(block_required_pickaxe_tier(B::GrassBlock), 0);
        assert_eq!(block_required_pickaxe_tier(B::Sand), 0);
        assert_eq!(block_required_pickaxe_tier(B::Gravel), 0);

        // tier 1：基础石类（wooden/golden 起步）
        assert_eq!(block_required_pickaxe_tier(B::Stone), 1);
        assert_eq!(block_required_pickaxe_tier(B::Cobblestone), 1);
        assert_eq!(block_required_pickaxe_tier(B::CoalOre), 1);
        assert_eq!(block_required_pickaxe_tier(B::DeepslateCoalOre), 1);
        assert_eq!(block_required_pickaxe_tier(B::Granite), 1);
        assert_eq!(block_required_pickaxe_tier(B::Deepslate), 1);
        assert_eq!(block_required_pickaxe_tier(B::Netherrack), 1);

        // tier 2：中级矿（stone 起步）—— 关键测试用例
        // 这是 P11 修复的核心：stone_pickaxe 应能挖 iron_ore，wooden_pickaxe 不行
        assert_eq!(block_required_pickaxe_tier(B::IronOre), 2);
        assert_eq!(block_required_pickaxe_tier(B::DeepslateIronOre), 2);
        assert_eq!(block_required_pickaxe_tier(B::CopperOre), 2);
        assert_eq!(block_required_pickaxe_tier(B::DeepslateCopperOre), 2);
        assert_eq!(block_required_pickaxe_tier(B::LapisOre), 2);
        assert_eq!(block_required_pickaxe_tier(B::DeepslateLapisOre), 2);

        // tier 3：高级矿（iron 起步）
        assert_eq!(block_required_pickaxe_tier(B::DiamondOre), 3);
        assert_eq!(block_required_pickaxe_tier(B::DeepslateDiamondOre), 3);
        assert_eq!(block_required_pickaxe_tier(B::GoldOre), 3);
        assert_eq!(block_required_pickaxe_tier(B::DeepslateGoldOre), 3);
        assert_eq!(block_required_pickaxe_tier(B::RedstoneOre), 3);
        assert_eq!(block_required_pickaxe_tier(B::EmeraldOre), 3);

        // tier 4：顶级方块（diamond/netherite 起步）
        assert_eq!(block_required_pickaxe_tier(B::AncientDebris), 4);
        assert_eq!(block_required_pickaxe_tier(B::Obsidian), 4);
        assert_eq!(block_required_pickaxe_tier(B::CryingObsidian), 4);
    }

    /// 关键回归测试：stone_pickaxe (tier 2) 能挖 iron_ore (需要 tier 2)。
    /// 这是 P11 修复的目标场景：之前 gather 工具会因「方块消失但无掉落」死循环，
    /// 实际根因是没检查镐 tier，可能 bot 主手是 wooden_pickaxe (tier 1)。
    #[test]
    fn stone_pickaxe_can_mine_iron_ore() {
        let pickaxe_tier = pickaxe_tier(IK::StonePickaxe);
        let required_tier = block_required_pickaxe_tier(B::IronOre);
        assert!(
            pickaxe_tier >= required_tier,
            "stone_pickaxe (tier {pickaxe_tier}) 应能挖 iron_ore (需要 tier {required_tier})"
        );
    }

    /// 回归测试：wooden_pickaxe (tier 1) 不能挖 iron_ore (需要 tier 2)。
    /// vanilla 规则：wooden_pickaxe 挖 iron_ore 时方块会消失但**不掉落物品**。
    #[test]
    fn wooden_pickaxe_cannot_mine_iron_ore() {
        let pickaxe_tier = pickaxe_tier(IK::WoodenPickaxe);
        let required_tier = block_required_pickaxe_tier(B::IronOre);
        assert!(
            pickaxe_tier < required_tier,
            "wooden_pickaxe (tier {pickaxe_tier}) 不应能挖 iron_ore (需要 tier {required_tier})"
        );
    }

    /// 回归测试：iron_pickaxe (tier 3) 能挖 diamond_ore (需要 tier 3)。
    #[test]
    fn iron_pickaxe_can_mine_diamond_ore() {
        let pickaxe_tier = pickaxe_tier(IK::IronPickaxe);
        let required_tier = block_required_pickaxe_tier(B::DiamondOre);
        assert!(
            pickaxe_tier >= required_tier,
            "iron_pickaxe (tier {pickaxe_tier}) 应能挖 diamond_ore (需要 tier {required_tier})"
        );
    }

    /// 回归测试：stone_pickaxe (tier 2) 不能挖 diamond_ore (需要 tier 3)。
    #[test]
    fn stone_pickaxe_cannot_mine_diamond_ore() {
        let pickaxe_tier = pickaxe_tier(IK::StonePickaxe);
        let required_tier = block_required_pickaxe_tier(B::DiamondOre);
        assert!(
            pickaxe_tier < required_tier,
            "stone_pickaxe (tier {pickaxe_tier}) 不应能挖 diamond_ore (需要 tier {required_tier})"
        );
    }

    /// 验证 pickaxe_tier_name 返回正确的中文名。
    #[test]
    fn pickaxe_tier_name_returns_correct_names() {
        assert_eq!(pickaxe_tier_name(0), "无镐");
        assert_eq!(pickaxe_tier_name(1), "木/金镐");
        assert_eq!(pickaxe_tier_name(2), "石镐");
        assert_eq!(pickaxe_tier_name(3), "铁镐");
        assert_eq!(pickaxe_tier_name(4), "钻石/下界合金镐");
    }

    /// 验证 pickaxe_to_craft_for_tier 返回正确的合成建议。
    #[test]
    fn pickaxe_to_craft_for_tier_returns_correct_recipe_hints() {
        assert!(pickaxe_to_craft_for_tier(1).contains("wooden_pickaxe"));
        assert!(pickaxe_to_craft_for_tier(2).contains("stone_pickaxe"));
        assert!(pickaxe_to_craft_for_tier(3).contains("iron_pickaxe"));
        assert!(pickaxe_to_craft_for_tier(4).contains("diamond_pickaxe"));
    }

    /// P39 关键回归测试：block_drops_item 必须返回 vanilla 1.18+ 的实际掉落物。
    ///
    /// 这是 gather 0/8 死循环的根本原因修复。原 gather 用 LLM 传入的方块名（如 "iron_ore"）
    /// 作为 ItemKind 去 count_item 统计背包数量，但 vanilla 1.18+ 中挖 iron_ore 方块
    /// 掉落的是 raw_iron 物品，不是 iron_ore——导致 count_item 永远返回 0，gather 永远失败。
    ///
    /// 此测试确保 block_drops_item 返回正确的物品类型，防止以后改回旧 bug。
    #[test]
    fn regression_block_drops_item_returns_vanilla_correct_drops() {
        // 铁矿 → raw_iron（最常见的 LLM gather 调用，也是 0/8 死循环的根因）
        assert_eq!(block_drops_item(B::IronOre), Some(IK::RawIron));
        assert_eq!(block_drops_item(B::DeepslateIronOre), Some(IK::RawIron));

        // 金矿 → raw_gold
        assert_eq!(block_drops_item(B::GoldOre), Some(IK::RawGold));
        assert_eq!(block_drops_item(B::DeepslateGoldOre), Some(IK::RawGold));

        // 铜矿 → raw_copper
        assert_eq!(block_drops_item(B::CopperOre), Some(IK::RawCopper));
        assert_eq!(block_drops_item(B::DeepslateCopperOre), Some(IK::RawCopper));

        // 煤矿 → coal（vanilla 中根本不存在 coal_ore 物品）
        assert_eq!(block_drops_item(B::CoalOre), Some(IK::Coal));
        assert_eq!(block_drops_item(B::DeepslateCoalOre), Some(IK::Coal));

        // 钻石矿 → diamond
        assert_eq!(block_drops_item(B::DiamondOre), Some(IK::Diamond));
        assert_eq!(block_drops_item(B::DeepslateDiamondOre), Some(IK::Diamond));

        // 绿宝石矿 → emerald
        assert_eq!(block_drops_item(B::EmeraldOre), Some(IK::Emerald));
        assert_eq!(block_drops_item(B::DeepslateEmeraldOre), Some(IK::Emerald));

        // 红石矿 → redstone
        assert_eq!(block_drops_item(B::RedstoneOre), Some(IK::Redstone));
        assert_eq!(
            block_drops_item(B::DeepslateRedstoneOre),
            Some(IK::Redstone)
        );

        // 青金石矿 → lapis_lazuli（注意是 dye 不是 ore）
        assert_eq!(block_drops_item(B::LapisOre), Some(IK::LapisLazuli));
        assert_eq!(
            block_drops_item(B::DeepslateLapisOre),
            Some(IK::LapisLazuli)
        );

        // 下界石英矿 → quartz
        assert_eq!(block_drops_item(B::NetherQuartzOre), Some(IK::Quartz));

        // 石头 → 圆石（精准采集除外，bot 没有 silk touch）
        assert_eq!(block_drops_item(B::Stone), Some(IK::Cobblestone));

        // 方块本身即是掉落物 → 返回 None（让 gather 回退到 LLM 传入的 item 名）
        assert_eq!(block_drops_item(B::Dirt), None);
        assert_eq!(block_drops_item(B::Cobblestone), None);
        assert_eq!(block_drops_item(B::OakLog), None);
        assert_eq!(block_drops_item(B::Sand), None);
        assert_eq!(block_drops_item(B::Gravel), None);
    }

    /// P39 关键回归测试：gather iron_ore 必须统计 raw_iron 数量，不是 iron_ore。
    ///
    /// 这是 P39 修复的核心验证：block_drops_item(IronOre) 返回 RawIron，
    /// 而不是 None 或 IronOre。如果未来有人改回旧逻辑（统计 iron_ore 数量），
    /// 这个测试会立刻失败。
    #[test]
    fn regression_gather_iron_ore_must_count_raw_iron() {
        // 模拟 gather 内部的 drop_item 计算逻辑
        let block_kind = B::IronOre;
        let target = IK::IronOre; // LLM 传入 "iron_ore" 解析得到的 ItemKind
        let drop_item = block_drops_item(block_kind).unwrap_or(target);

        assert_eq!(
            drop_item,
            IK::RawIron,
            "挖 iron_ore 方块必须统计 raw_iron 数量，否则 gather 永远 0/N 失败"
        );
        assert_ne!(
            drop_item, target,
            "drop_item 不能等于 target（iron_ore），否则就是 P39 修复前的 bug"
        );

        // 同理验证 deepslate_iron_ore
        let block_kind = B::DeepslateIronOre;
        let drop_item = block_drops_item(block_kind).unwrap_or(IK::DeepslateIronOre);
        assert_eq!(
            drop_item,
            IK::RawIron,
            "挖 deepslate_iron_ore 方块也必须统计 raw_iron 数量"
        );
    }
}
