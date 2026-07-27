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
use azalea::pathfinder::goals::BlockPosGoal;
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
    /// 周期性状态快照（位置 + 背包 + 生命/饱食 + 主手 + 群系 + 附近方块）。
    State {
        position: azalea::Vec3,
        /// 全量非空格：格式 `oak_log:3, cobblestone:64, wooden_pickaxe:1`
        inventory: String,
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
    /// ActionManager：封装 pending 槽 + 按命令类型超时 + 抢占 + 快循环检测。
    /// 取代原硬编码 60-tick 超时（合成/采集/熔炼等长任务被误杀）。
    /// 字段保留 pending/pending_since/busy 的 Arc 引用，供旧代码兼容访问。
    pub action_mgr: ActionManager,
    /// 共享世界记忆库（适配器/工具/Agent 共用；handler 内扫描回填）。
    pub memory: Option<WorldMemory>,
    /// 已扫描记录的坐标 → 上次扫描时间戳（TTL 去重 + 重验世界变化）。
    pub scanned: Arc<Mutex<HashMap<MemoryPos, u64>>>,
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
            mining_below: Arc::new(Mutex::new(false)),
            mining_above: Arc::new(Mutex::new(false)),
            action_mgr: ActionManager::new(),
            memory: None,
            scanned: Arc::new(Mutex::new(HashMap::new())),
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
            mining_below: Arc::new(Mutex::new(false)),
            mining_above: Arc::new(Mutex::new(false)),
            action_mgr: ActionManager::new(),
            memory: memory,
            scanned: Arc::new(Mutex::new(HashMap::new())),
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
                {
                    let mut q = cmd_queue.lock().unwrap();
                    let mut push = |cmd: BotCommand| {
                        q.push(QueuedCommand {
                            cmd,
                            result_tx: None,
                        });
                    };
                    if let Some(rest) = content.strip_prefix("autocraft ") {
                        let mut parts = rest.split_whitespace();
                        if let Some(item) = parts.next() {
                            let count = parts
                                .next()
                                .and_then(|s| s.parse::<u32>().ok())
                                .unwrap_or(1);
                            push(BotCommand::AutoCraft {
                                item: item.to_string(),
                                count,
                            });
                        }
                    } else if let Some(rest) = content.strip_prefix("open ") {
                        let c: Vec<i32> = rest
                            .split_whitespace()
                            .filter_map(|s| s.parse().ok())
                            .collect();
                        if c.len() == 3 {
                            push(BotCommand::OpenContainer {
                                x: c[0],
                                y: c[1],
                                z: c[2],
                            });
                        }
                    } else if let Some(rest) = content.strip_prefix("place ") {
                        let mut parts = rest.split_whitespace();
                        if let Some(item) = parts.next() {
                            let c: Vec<i32> = parts.filter_map(|s| s.parse().ok()).collect();
                            if c.len() == 3 {
                                push(BotCommand::Place {
                                    item: item.to_string(),
                                    x: c[0],
                                    y: c[1],
                                    z: c[2],
                                });
                            }
                        }
                    } else if let Some(rest) = content.strip_prefix("gather ") {
                        let mut parts = rest.split_whitespace();
                        if let Some(item) = parts.next() {
                            let count = parts
                                .next()
                                .and_then(|s| s.parse::<u32>().ok())
                                .unwrap_or(1);
                            push(BotCommand::Gather {
                                item: item.to_string(),
                                count,
                            });
                        }
                    } else if let Some(rest) = content.strip_prefix("craft3 ") {
                        let mut parts = rest.split_whitespace();
                        if let Some(item) = parts.next() {
                            let count = parts
                                .next()
                                .and_then(|s| s.parse::<u32>().ok())
                                .unwrap_or(1);
                            push(BotCommand::Craft3x3 {
                                item: item.to_string(),
                                count,
                                table_pos: None,
                            });
                        }
                    } else if let Some(rest) = content.strip_prefix("smelt ") {
                        let mut parts = rest.split_whitespace();
                        if let Some(output) = parts.next() {
                            let fuel = parts.next().unwrap_or("coal").to_string();
                            let count = parts
                                .next()
                                .and_then(|s| s.parse::<u32>().ok())
                                .unwrap_or(1);
                            push(BotCommand::Smelt {
                                output: output.to_string(),
                                fuel,
                                count,
                                table_pos: None,
                            });
                        }
                    } else if let Some(rest) = content.strip_prefix("craft ") {
                        let mut parts = rest.split_whitespace();
                        if let Some(item) = parts.next() {
                            let count = parts
                                .next()
                                .and_then(|s| s.parse::<u32>().ok())
                                .unwrap_or(1);
                            push(BotCommand::Craft2x2 {
                                item: item.to_string(),
                                count,
                            });
                        }
                    } else if let Some(rest) = content.strip_prefix("goto ") {
                        let c: Vec<i32> = rest
                            .split_whitespace()
                            .filter_map(|s| s.parse().ok())
                            .collect();
                        if c.len() == 3 {
                            push(BotCommand::Goto {
                                x: c[0],
                                y: c[1],
                                z: c[2],
                            });
                        }
                    } else if let Some(rest) = content.strip_prefix("mine ") {
                        let c: Vec<i32> = rest
                            .split_whitespace()
                            .filter_map(|s| s.parse().ok())
                            .collect();
                        if c.len() == 3 {
                            push(BotCommand::Mine {
                                x: c[0],
                                y: c[1],
                                z: c[2],
                            });
                        }
                    } else if content == "minebelow" {
                        push(BotCommand::MineBelow);
                    } else if content == "attack" {
                        push(BotCommand::Attack {
                            target: "chat".into(),
                        });
                    } else if let Some(rest) = content.strip_prefix("enchant ") {
                        let mut parts = rest.split_whitespace();
                        if let Some(item) = parts.next() {
                            let level = parts
                                .next()
                                .and_then(|s| s.parse::<u32>().ok())
                                .unwrap_or(1);
                            push(BotCommand::Enchant {
                                item: item.to_string(),
                                level,
                            });
                        }
                    } else if let Some(rest) = content.strip_prefix("trade ") {
                        if let Ok(offer) = rest.trim().parse::<u32>() {
                            push(BotCommand::Trade { offer });
                        }
                    } else if let Some(rest) = content.strip_prefix("interact ") {
                        let kind = rest.trim().to_string();
                        if !kind.is_empty() {
                            push(BotCommand::InteractEntity { kind });
                        }
                    }
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
                        let done = match &qc.cmd {
                            BotCommand::Mine { x, y, z } => {
                                if let Ok(world) = bot.world() {
                                    let s = world.read().get_block_state(BlockPos::new(*x, *y, *z));
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
                                    d < 1.5
                                } else {
                                    false
                                }
                            }
                            BotCommand::MineBelow => false,
                            BotCommand::MineAbove => false,
                            // 非轮询命令（Equip/Craft/Gather/Place/...）由下方执行块处理，
                            // 这里不能标记 done=true——否则会在执行前就清空 pending，
                            // 导致 do_equip/do_craft 等从未运行（bug 表现：equip 返回"命令完成"但主手没变）。
                            _ => false,
                        };
                        // 按命令类型超时（取代原硬编码 60 tick）
                        let timed_out_cmd = state.action_mgr.check_timeout(tick_now);
                        let timed_out = timed_out_cmd.is_some();
                        if done || timed_out {
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
                                    format!(
                                        "Action output:\ngoto ({},{},{}) 超时——可能路径被阻或目标不可达。建议改 goto 附近 3 格外空地脱困。",
                                        x, y, z
                                    )
                                }
                                BotCommand::Mine { x, y, z } if done => {
                                    let (cx, cy, cz) = bot
                                        .position()
                                        .ok()
                                        .map(|p| (p.x, p.y, p.z))
                                        .unwrap_or((0.0, 0.0, 0.0));
                                    format!(
                                        "Action output:\nMined block at ({},{},{}). Block removed. Bot still at ({:.0},{:.0},{:.0}) — 挖完不会自动掉进洞，无需 goto 刚挖的位置。",
                                        x, y, z, cx, cy, cz
                                    )
                                }
                                BotCommand::Mine { x, y, z } => {
                                    format!(
                                        "Action output:\nmine ({},{},{}) 超时——可能方块太硬（需更高品质镐）或距离太远。建议 gather(item=..., count=...) 自动寻路挖掘。",
                                        x, y, z
                                    )
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
                            // 距离限制：>32 格的 goto 拒绝执行，让 LLM 拆成多段。
                            // 原因：azalea pathfinder 的 A* 在长距离/复杂地形上计算量大，
                            // 每 tick 发 MovePlayerPos+PlayerInput 包会拖死 vanilla 服 TPS，
                            // 导致同服真实玩家 WASD 输入丢失（服务器来不及处理）。
                            if let Ok(p) = bot.position() {
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
                            let head_is_hard = head_pos
                                .and_then(|pos| {
                                    let world = bot.world().ok()?;
                                    let world = world.read();
                                    let state = world.get_block_state(pos)?;
                                    Some(is_hard_block(state))
                                })
                                .unwrap_or(true); // 不确定时按硬方块处理
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
                            *state.mining_above.lock().unwrap() = true;
                            let _ = auto_equip_best_pickaxe(&bot).await;
                            if let Some(pos) = head_pos {
                                bot.start_mining(pos);
                            }
                            if let Some(tx) = &result_tx {
                                let note = if head_is_hard {
                                    ""
                                } else {
                                    "（软方块，徒手可挖）"
                                };
                                let _ = tx.send(format!("已开始向上挖掘{note}"));
                            }
                            state.action_mgr.clear_pending();
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
                        BotCommand::Attack { target: _target } => {
                            if let Ok(entities) =
                            bot.nearest_entities::<bevy_ecs::query::Without<azalea::entity::metadata::Player>>()
                        {
                            let self_id = bot.entity().id();
                            // 记录攻击前的生命，便于反馈损血
                            let health_before = bot.health().unwrap_or(20.0);
                            let mut hit_kind: Option<String> = None;
                            for e in entities.iter() {
                                if e.id() == self_id { continue; }
                                let kind = e.kind().map(|k| {
                                    // P5 修复：用 to_str() 拿到 snake_case id（如 "zombie"），
                                    // 原 format!("{k:?}").to_lowercase() 得到 "zombie"（巧合一致），
                                    // 但对 "Allay".to_lowercase() = "allay" 仍正确，
                                    // 对 "Mooshroom".to_lowercase() = "mooshroom" 也对——
                                    // 但对 "ItemFrame".to_lowercase() = "itemframe"（无下划线）错误。
                                    // 统一用 to_str() 保证 snake_case。
                                    let s = k.to_str();
                                    s.strip_prefix("minecraft:").unwrap_or(s).to_string()
                                }).unwrap_or_else(|_| "entity".to_string());
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
                                None => "Action output:\nCould not find any non-player entity nearby to attack.".to_string(),
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
                        let y = p.y.floor() as i32;
                        let cx = p.x.floor() as i32;
                        let cz = p.z.floor() as i32;
                        // P8 修复：用「头顶是空气 + 足够高」判断是否到地表，而非固定 Y≥62。
                        // 原代码 Y≥62 就停止，但在洞穴中 bot 可能 Y=62 仍在地下（地表 Y=70+）。
                        // 在洞穴中必须继续向上挖到 Y≥70 才能保证到地表。
                        let head_pos = BlockPos::new(cx, y + 1, cz);
                        let head_is_air = bot
                            .world()
                            .ok()
                            .and_then(|w| w.read().get_block_state(head_pos))
                            .map(|s| s.is_air())
                            .unwrap_or(false);
                        // 检查上方 5 格是否都是空气（确认是开放空间而非洞穴小气室）
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
                        // 到达地表条件：头顶是空气 AND (Y≥70 OR 上方5格都是空气)
                        if head_is_air && (y >= 70 || five_air) {
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
                            // 让 pathfinder 自动挖通并 ascend
                            // YGoal(y+5)：目标到达 y+5 高度（任意 x/z），给 pathfinder 水平自由度
                            // pathfinder allow_mining=true 会挖通 head + head+1 + 旁边方块让 bot ascend
                            let target_y = y + 5;
                            // 装备镐（如果有的话）加速挖掘
                            let _ = auto_equip_best_pickaxe(&bot).await;
                            // 只在 pathfinder 空闲时启动新 goto，用大 timeout 让 pathfinder 有时间计算
                            if bot.is_goto_target_reached() && !bot.is_calculating_path() {
                                use azalea::pathfinder::PathfinderOpts;
                                use azalea::pathfinder::goals::YGoal;
                                use std::time::Duration;
                                let opts = PathfinderOpts::new()
                                    .allow_mining(true)
                                    .min_timeout(Duration::from_secs(2))
                                    .max_timeout(Duration::from_secs(30));
                                bot.start_goto_with_opts(YGoal { y: target_y }, opts);
                                let _ = (cx, cz); // 调试用坐标
                            }
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
                        let inventory = match bot.get_inventory() {
                            Ok(inv) => match inv.slots() {
                                Some(slots) => {
                                    let mut agg: std::collections::HashMap<String, u32> =
                                        std::collections::HashMap::new();
                                    for s in slots.iter() {
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
                                        *agg.entry(kind.to_string()).or_insert(0) += cnt;
                                    }
                                    if agg.is_empty() {
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
                                    }
                                }
                                None => "slots=None".to_string(),
                            },
                            Err(_) => "获取失败".to_string(),
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
                            serde_json::json!({
                                "inventory": inv_slots,
                                "experience_level": xp.as_ref().map(|e| e.level).unwrap_or(0),
                                "experience_progress": xp.as_ref().map(|e| e.progress).unwrap_or(0.0),
                                "held_item": held_item,
                                "selected_slot": bot.selected_hotbar_slot().unwrap_or(0),
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
                        // 附近实体列表：按类型分组计数
                        let nearby_entities = {
                            let mut kinds: HashMap<String, u32> = HashMap::new();
                            if let Ok(entities) = bot.nearest_entities::<bevy_ecs::query::Without<azalea::entity::metadata::Player>>() {
                            let self_id = bot.entity().id();
                            for e in entities.iter() {
                                if e.id() == self_id { continue; }
                                let name = format!("{:?}", e.kind().unwrap_or(EntityKind::Pig)).to_lowercase();
                                *kinds.entry(name).or_insert(0) += 1;
                            }
                        }
                            // 玩家分开计数
                            let player_count = bot.nearby_players().map(|pp| pp.len()).unwrap_or(0);
                            let mut parts: Vec<String> = Vec::new();
                            if player_count > 0 {
                                parts.push(format!("player:{}", player_count));
                            }
                            let mut items: Vec<_> = kinds.into_iter().collect();
                            items.sort_by(|a, b| b.1.cmp(&a.1));
                            for (k, v) in items {
                                if v > 0 {
                                    parts.push(format!("{k}:{v}"));
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
                // self_defense：空闲或寻路途中自动攻击附近敌对生物（每 100 tick ≈5s 检查一次）
                // 距离限制：只攻击 4 格内实体，避免对远距离敌人（如持弩掠夺者）对着空气挥拳。
                // 用 is_busy() 而非 is_idle()：Goto/Mine 等轮询命令执行期间 pending 非空但 busy=false，
                // 此时仍应自卫（否则 bot 寻路途中被僵尸攻击不还手——H3 bug）。
                // 只在异步命令（Craft/Gather/Smelt）执行中（busy=true）跳过，避免抢占。
                if !state.action_mgr.is_busy()
                    && !*state.mining_below.lock().unwrap()
                    && bot.ticks_connected() % 100 == 0
                {
                    if let Ok(entities) = bot.nearest_entities::<bevy_ecs::query::Without<azalea::entity::metadata::Player>>() {
                    let self_id = bot.entity().id();
                    let self_pos = bot.position().ok();
                    let mut attacked = false;
                    for e in entities.iter() {
                        if e.id() == self_id { continue; }
                        if attacked { break; }
                        if let Ok(kind) = e.kind() {
                            let hostile = matches!(kind,
                                EntityKind::Zombie | EntityKind::Skeleton | EntityKind::Creeper
                                | EntityKind::Spider | EntityKind::CaveSpider | EntityKind::Enderman
                                | EntityKind::Pillager | EntityKind::Phantom | EntityKind::Witch
                                | EntityKind::Drowned | EntityKind::Husk | EntityKind::Stray
                            );
                            if hostile {
                                // 距离检查：只在 4 格内才攻击，否则会反复对空气挥拳
                                // （远距离敌人由 LLM 决策是否 goto 拉近或撤退）
                                let in_range = if let Some(sp) = self_pos {
                                    if let Ok(ep) = e.position() {
                                        let d = ((sp.x - ep.x).powi(2)
                                            + (sp.y - ep.y).powi(2)
                                            + (sp.z - ep.z).powi(2)).sqrt();
                                        d <= 4.0
                                    } else {
                                        false
                                    }
                                } else {
                                    false
                                };
                                if !in_range { continue; }
                                // 攻击前检查实体是否存活（get_component 失败说明已消失）
                                if e.get_component::<azalea::entity::EntityKindComponent>().is_some() {
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
            let inv = match bot.get_inventory() {
                Ok(i) => i,
                Err(e) => return format!("获取背包失败: {e:?}"),
            };
            let srcs = find_item_slots(&inv, kind);
            if let Some(src) = srcs.first() {
                inv.shift_click(*src);
                sleep(Duration::from_millis(150)).await;
                // P5 修复：验证盔甲槽是否真的装上了——shift_click 可能被服务端拒绝
                // （如该槽已有其他盔甲，或物品不是对应种类的盔甲）。
                drop(inv);
                return match verify_armor_slot(bot, armor_slot_idx, kind).await {
                    true => format!("已装备 {item} 到 {slot_norm}（shift_click 槽 {src}）"),
                    false => format!(
                        "装备 {item} 到 {slot_norm} 失败：shift_click 后该盔甲槽未持有 {item}。\
                         可能原因：1) 该槽已有其他盔甲（需先 discard 旧盔甲）；\
                         2) {item} 不是 {slot_norm} 类型的盔甲；3) 服务端同步延迟。\
                         建议：用 perceive 查看当前盔甲槽状态，或换一个空槽位。"
                    ),
                };
            }
            format!("背包未持有 {item}")
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
        format!("已丢弃全部 {item}（共 {dropped} 个）")
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
