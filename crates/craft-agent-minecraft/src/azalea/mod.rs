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

pub mod actions;
pub mod auto_craft;
pub mod client;
pub mod craft;
pub mod ext_state;
pub mod gather;
pub mod perception;
pub mod place;
pub mod recipe_book;
pub mod recipes;
pub mod trade;

use azalea::prelude::*;
use azalea::pathfinder::goals::BlockPosGoal;
use azalea::BlockPos;
use azalea_registry::builtin::{BlockKind, EntityKind};
use azalea_registry::DataRegistryKey;
use azalea_client::client_chat::ChatPacket;
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
    let name = format!("{bk:?}");
    // 原木类
    if name.ends_with("Log") || name.ends_with("Stem") {
        return Some((name.to_lowercase(), "树木/原木", MemoryKind::Resource));
    }
    // 矿石类
    if name.ends_with("Ore") || name == "AncientDebris" {
        return Some((name.to_lowercase(), "矿石", MemoryKind::Resource));
    }
    match bk {
        BlockKind::CraftingTable => Some(("crafting_table".into(), "工作台", MemoryKind::Structure)),
        BlockKind::Furnace => Some(("furnace".into(), "熔炉", MemoryKind::Structure)),
        BlockKind::Chest => Some(("chest".into(), "箱子", MemoryKind::Container)),
        BlockKind::SmithingTable => Some(("smithing_table".into(), "锻造台", MemoryKind::Structure)),
        BlockKind::EnchantingTable => Some(("enchanting_table".into(), "附魔台", MemoryKind::Structure)),
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
    Goto { x: i32, y: i32, z: i32 },
    Mine { x: i32, y: i32, z: i32 },
    MineBelow,
    BlockInteract { x: i32, y: i32, z: i32 },
    Chat { content: String },
    Attack { target: String },
    /// 2×2 背包合成（无需工作台）：item 为目标物品 id（如 "oak_planks"），count 为期望数量。
    Craft2x2 { item: String, count: u32 },
    /// 3×3 工作台合成：要求已打开工作台（Crafting 菜单）。item 为目标物品 id，count 为期望数量。
    Craft3x3 { item: String, count: u32 },
    /// 熔炼：要求已打开熔炉/高炉/烟熏炉（Furnace 类菜单）。
    /// output 为目标物品 id（如 "iron_ingot"），fuel 为燃料物品 id（如 "coal"），count 为期望数量。
    Smelt { output: String, fuel: String, count: u32 },
    /// 采集：走到最近的指定方块（如 "oak_log" / "stone" / "coal_ore"）并挖掘，直到背包有 count 个。
    Gather { item: String, count: u32 },
    /// 放置：把手持物品 item 放到世界坐标 (x,y,z) 旁（右键放置）。
    Place { item: String, x: i32, y: i32, z: i32 },
    /// 打开容器：打开世界坐标 (x,y,z) 处的容器（工作台/熔炉/箱子等）。
    OpenContainer { x: i32, y: i32, z: i32 },
    /// 高层自动合成（木链）：采集→2×2→放置工作台→开→3×3，一键造木制品。
    AutoCraft { item: String, count: u32 },
    /// 附魔：在已打开的附魔台中，给 item 附魔（需背包有 item 与青金石 lapis_lazuli）。
    /// level 为 1/2/3，对应附魔台三个选项槽。
    Enchant { item: String, level: u32 },
    /// 村民交易：与最近的村民交易，选第 offer 个报价（0 起）。bot 自动打开村民。
    Trade { offer: u32 },
    /// 实体右键交互（打开村民/动物/展示框等）：与最近的指定种类实体交互。
    /// kind 为实体种类关键词，如 "villager"。
    InteractEntity { kind: String },
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
    /// 当前正在执行的单条命令（串行槽）。每 tick 只推进一条，完成后才取队列下一条。
    /// 修复：原实现每 tick drain 全部命令一起执行，多个 start_mining 互相覆盖只剩
    /// 最后一个生效——导致"一轮多动作"实际只执行了最后一个动作。
    pub pending: Arc<Mutex<Option<QueuedCommand>>>,
    /// pending 命令开始的 tick（ticks_connected），用于超时释放：若命令长时间
    /// 未完成（如 goto 被卡住到不了），强制清空 pending 放行队列下一条，避免死锁。
    pub pending_since: Arc<Mutex<Option<u64>>>,
    /// 异步命令（Craft/Gather/Place 等）在 tick 内 await 期间的中途锁：防止下一 tick
    /// 重复进入同一异步命令（handle 每 tick 都会触发）。非阻塞命令（Goto/Mine）不用。
    pub busy: Arc<Mutex<bool>>,
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
            pending: Arc::new(Mutex::new(None)),
            pending_since: Arc::new(Mutex::new(None)),
            busy: Arc::new(Mutex::new(false)),
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
            pending: Arc::new(Mutex::new(None)),
            pending_since: Arc::new(Mutex::new(None)),
            busy: Arc::new(Mutex::new(false)),
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
            let content = match &packet {
                ChatPacket::System(p) => format!("{:?}", p.content),
                _ => format!("{packet:?}"),
            };
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
                    q.push(QueuedCommand { cmd, result_tx: None });
                };
                if let Some(rest) = content.strip_prefix("autocraft ") {
                    let mut parts = rest.split_whitespace();
                    if let Some(item) = parts.next() {
                        let count = parts.next().and_then(|s| s.parse::<u32>().ok()).unwrap_or(1);
                        push(BotCommand::AutoCraft { item: item.to_string(), count });
                    }
                } else if let Some(rest) = content.strip_prefix("open ") {
                    let c: Vec<i32> = rest.split_whitespace().filter_map(|s| s.parse().ok()).collect();
                    if c.len() == 3 {
                        push(BotCommand::OpenContainer { x: c[0], y: c[1], z: c[2] });
                    }
                } else if let Some(rest) = content.strip_prefix("place ") {
                    let mut parts = rest.split_whitespace();
                    if let Some(item) = parts.next() {
                        let c: Vec<i32> = parts.filter_map(|s| s.parse().ok()).collect();
                        if c.len() == 3 {
                            push(BotCommand::Place { item: item.to_string(), x: c[0], y: c[1], z: c[2] });
                        }
                    }
                } else if let Some(rest) = content.strip_prefix("gather ") {
                    let mut parts = rest.split_whitespace();
                    if let Some(item) = parts.next() {
                        let count = parts.next().and_then(|s| s.parse::<u32>().ok()).unwrap_or(1);
                        push(BotCommand::Gather { item: item.to_string(), count });
                    }
                } else if let Some(rest) = content.strip_prefix("craft3 ") {
                    let mut parts = rest.split_whitespace();
                    if let Some(item) = parts.next() {
                        let count = parts.next().and_then(|s| s.parse::<u32>().ok()).unwrap_or(1);
                        push(BotCommand::Craft3x3 { item: item.to_string(), count });
                    }
                } else if let Some(rest) = content.strip_prefix("smelt ") {
                    let mut parts = rest.split_whitespace();
                    if let Some(output) = parts.next() {
                        let fuel = parts.next().unwrap_or("coal").to_string();
                        let count = parts.next().and_then(|s| s.parse::<u32>().ok()).unwrap_or(1);
                        push(BotCommand::Smelt { output: output.to_string(), fuel, count });
                    }
                } else if let Some(rest) = content.strip_prefix("craft ") {
                    let mut parts = rest.split_whitespace();
                    if let Some(item) = parts.next() {
                        let count = parts.next().and_then(|s| s.parse::<u32>().ok()).unwrap_or(1);
                        push(BotCommand::Craft2x2 { item: item.to_string(), count });
                    }
                } else if let Some(rest) = content.strip_prefix("goto ") {
                    let c: Vec<i32> = rest.split_whitespace().filter_map(|s| s.parse().ok()).collect();
                    if c.len() == 3 {
                        push(BotCommand::Goto { x: c[0], y: c[1], z: c[2] });
                    }
                } else if let Some(rest) = content.strip_prefix("mine ") {
                    let c: Vec<i32> = rest.split_whitespace().filter_map(|s| s.parse().ok()).collect();
                    if c.len() == 3 {
                        push(BotCommand::Mine { x: c[0], y: c[1], z: c[2] });
                    }
                } else if content == "minebelow" {
                    push(BotCommand::MineBelow);
                } else if content == "attack" {
                    push(BotCommand::Attack { target: "chat".into() });
                } else if let Some(rest) = content.strip_prefix("enchant ") {
                    let mut parts = rest.split_whitespace();
                    if let Some(item) = parts.next() {
                        let level = parts.next().and_then(|s| s.parse::<u32>().ok()).unwrap_or(1);
                        push(BotCommand::Enchant { item: item.to_string(), level });
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
            {
                let mut pending = state.pending.lock().unwrap();
                if pending.is_none() {
                    let next = {
                        let mut q = cmd_queue.lock().unwrap();
                        q.pop() // FIFO：取最早入队的命令
                    };
                    if let Some(qc) = next {
                        *pending = Some(qc);
                        *state.pending_since.lock().unwrap() = Some(bot.ticks_connected() as u64);
                    }
                }
                // 轮询非阻塞命令（Goto/Mine）完成状态，超时（60 tick≈3s）强制释放。
                // 旧值 200 tick(10s) 太长——bot 卡在 goto 10 秒会拖死 vanilla 服 TPS，
                // 导致同服玩家 WASD 输入丢失。3 秒足够短距离 goto 完成，超时让 LLM 改策略。
                if let Some(qc) = pending.as_ref() {
                    let done = match &qc.cmd {
                        BotCommand::Mine { x, y, z } => {
                            if let Ok(world) = bot.world() {
                                let s = world.read().get_block_state(BlockPos::new(*x, *y, *z));
                                s.is_none() || s.map(|b| b.is_air()).unwrap_or(false)
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
                        _ => true,
                    };
                    let timed_out = {
                        let since = *state.pending_since.lock().unwrap();
                        match since {
                            Some(t0) => (bot.ticks_connected() as u64).saturating_sub(t0) > 60,
                            None => false,
                        }
                    };
                    if done || timed_out {
                        // 统一用 Mindcraft 风格 "Action output:\n..." 让 LLM 看到一致的反馈。
                        // 加副作用信息：goto 带距离 + 当前坐标，mine 带方块类型 + 挖后背包获得数。
                        let result_msg = match &qc.cmd {
                            BotCommand::Goto { x, y, z } if done => {
                                let (cx, cy, cz) = bot.position().ok()
                                    .map(|p| (p.x, p.y, p.z))
                                    .unwrap_or((0.0, 0.0, 0.0));
                                let dist = ((cx - *x as f64).powi(2)
                                    + (cy - *y as f64).powi(2)
                                    + (cz - *z as f64).powi(2)).sqrt();
                                format!("Action output:\nArrived at ({},{},{}). Distance traveled: {:.1}m. Current pos: ({:.0},{:.0},{:.0}).",
                                    x, y, z, dist, cx, cy, cz)
                            }
                            BotCommand::Goto { x, y, z } => {
                                format!("Action output:\ngoto ({},{},{}) 超时——可能路径被阻或目标不可达。建议改 goto 附近 3 格外空地脱困。", x, y, z)
                            }
                            BotCommand::Mine { x, y, z } if done => {
                                // 挖完后方块已是 air，无法得知类型；但可读背包获得数变化（需调用方对比）。
                                // 返回 bot 当前坐标——LLM 写 plan 时常误以为挖完会自动掉进洞里
                                // (e.g. mine→goto 同坐标)，告知实际位置避免无意义 goto 超时。
                                let (cx, cy, cz) = bot.position().ok()
                                    .map(|p| (p.x, p.y, p.z))
                                    .unwrap_or((0.0, 0.0, 0.0));
                                format!("Action output:\nMined block at ({},{},{}). Block removed. Bot still at ({:.0},{:.0},{:.0}) — 挖完不会自动掉进洞，无需 goto 刚挖的位置。",
                                    x, y, z, cx, cy, cz)
                            }
                            BotCommand::Mine { x, y, z } => {
                                format!("Action output:\nmine ({},{},{}) 超时——可能方块太硬（需更高品质镐）或距离太远。建议 gather(item=..., count=...) 自动寻路挖掘。", x, y, z)
                            }
                            _ if done => "Action output:\n命令完成".to_string(),
                            _ => "Action output:\n命令超时".to_string(),
                        };
                        if let Some(tx) = &qc.result_tx {
                            let _ = tx.send(result_msg);
                        }
                        *pending = None;
                        *state.pending_since.lock().unwrap() = None;
                    }
                }
            }
            // 取当前要执行的命令：pending 里的命令每 tick 都（重）执行其 start，
            // 非阻塞命令（Goto/Mine）重复 start 是幂等的（重设同一目标），由
            // cmd_finished 轮询完成；MineBelow 在 arm 内清空中途槽。
            // 异步命令（Craft/Gather 等）执行期间 busy=true，下一 tick 跳过避免重入。
            let to_run: Option<(BotCommand, Option<std::sync::mpsc::Sender<String>>)> = {
                if *state.busy.lock().unwrap() {
                    None
                } else if let Some(qc) = state.pending.lock().unwrap().as_ref() {
                    let is_polling = matches!(
                        &qc.cmd,
                        BotCommand::Goto { .. } | BotCommand::Mine { .. } | BotCommand::MineBelow
                    );
                    if !is_polling {
                        *state.busy.lock().unwrap() = true;
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
                                + (p.z - z as f64).powi(2)).sqrt();
                            if dist > 32.0 {
                                if let Some(tx) = &result_tx {
                                    let _ = tx.send(format!(
                                        "Action output:\ngoto ({},{},{}) 距离 {:.0}m 过远（>32m），\
                                         请拆成多段：先 goto 中间点（距当前 16-24m），到达后再 goto 目标。",
                                        x, y, z, dist
                                    ));
                                }
                                *state.pending.lock().unwrap() = None;
                                *state.pending_since.lock().unwrap() = None;
                                return bot;
                            }
                        }
                        bot.start_goto(BlockPosGoal(BlockPos::new(x, y, z)));
                    }
                    BotCommand::Mine { x, y, z } => {
                        *state.mining_below.lock().unwrap() = false;
                        bot.start_mining(BlockPos::new(x, y, z));
                    }
                    BotCommand::MineBelow => {
                        *state.mining_below.lock().unwrap() = true;
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
                        *state.pending.lock().unwrap() = None;
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
                                let kind = e.kind().map(|k| format!("{k:?}").to_lowercase()).unwrap_or_else(|_| "entity".to_string());
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
                                if let Some(tx) = &result_tx { let _ = tx.send(format!("Action output:\nSuccessfully crafted {item}, you now have it. ({msg})")); }
                            }
                            Err(e) => {
                                let chat = format!("[合成失败] {e}");
                                let _ = evt_tx.send(BotEvent::Chat { content: chat });
                                if let Some(tx) = &result_tx { let _ = tx.send(format!("Action output:\nFailed to craft {item}: {e}")); }
                            }
                        }
                    }
                    BotCommand::Craft3x3 { item, count } => {
                        match crate::azalea::craft::do_craft_3x3(&bot, &item, count).await {
                            Ok(msg) => {
                                let chat = format!("[合成] {msg}");
                                let _ = evt_tx.send(BotEvent::Chat { content: chat });
                                if let Some(tx) = &result_tx { let _ = tx.send(format!("Action output:\nSuccessfully crafted {item}, you now have it. ({msg})")); }
                            }
                            Err(e) => {
                                let chat = format!("[合成失败] {e}");
                                let _ = evt_tx.send(BotEvent::Chat { content: chat });
                                if let Some(tx) = &result_tx { let _ = tx.send(format!("Action output:\nFailed to craft {item}: {e}")); }
                            }
                        }
                    }
                    BotCommand::Smelt { output, fuel, count } => {
                        match crate::azalea::craft::do_smelt(&bot, &output, &fuel, count).await {
                            Ok(msg) => {
                                let chat = format!("[熔炼] {msg}");
                                let _ = evt_tx.send(BotEvent::Chat { content: chat });
                                if let Some(tx) = &result_tx { let _ = tx.send(format!("Action output:\nSuccessfully smelted {output}, you now have it. ({msg})")); }
                            }
                            Err(e) => {
                                let chat = format!("[熔炼失败] {e}");
                                let _ = evt_tx.send(BotEvent::Chat { content: chat });
                                if let Some(tx) = &result_tx { let _ = tx.send(format!("Action output:\nFailed to smelt {output}: {e}")); }
                            }
                        }
                    }
                    BotCommand::Gather { item, count } => {
                        match crate::azalea::gather::do_gather(&bot, &item, count).await {
                            Ok(msg) => {
                                let chat = format!("[采集] {msg}");
                                let _ = evt_tx.send(BotEvent::Chat { content: chat });
                                if let Some(tx) = &result_tx { let _ = tx.send(format!("Action output:\nSuccessfully gathered {item}, {msg}")); }
                            }
                            Err(e) => {
                                let chat = format!("[采集失败] {e}");
                                let _ = evt_tx.send(BotEvent::Chat { content: chat });
                                if let Some(tx) = &result_tx { let _ = tx.send(format!("Action output:\nFailed to gather {item}: {e}")); }
                            }
                        }
                    }
                    BotCommand::Place { item, x, y, z } => {
                        match crate::azalea::place::do_place(&bot, &item, BlockPos::new(x, y, z)).await {
                            Ok(msg) => {
                                let chat = format!("[放置] {msg}");
                                let _ = evt_tx.send(BotEvent::Chat { content: chat });
                                if let Some(tx) = &result_tx { let _ = tx.send(format!("Action output:\nPlaced {item} at ({},{},{}). ({msg})", x, y, z)); }
                            }
                            Err(e) => {
                                let chat = format!("[放置失败] {e}");
                                let _ = evt_tx.send(BotEvent::Chat { content: chat });
                                if let Some(tx) = &result_tx { let _ = tx.send(format!("Action output:\nFailed to place {item} at ({},{},{}): {e}", x, y, z)); }
                            }
                        }
                    }
                    BotCommand::OpenContainer { x, y, z } => {
                        match crate::azalea::place::do_open_container(&bot, BlockPos::new(x, y, z)).await {
                            Ok(msg) => {
                                let chat = format!("[开容器] {msg}");
                                let _ = evt_tx.send(BotEvent::Chat { content: chat });
                                if let Some(tx) = &result_tx { let _ = tx.send(format!("Action output:\nOpened container at ({},{},{}). ({msg})", x, y, z)); }
                            }
                            Err(e) => {
                                let chat = format!("[开容器失败] {e}");
                                let _ = evt_tx.send(BotEvent::Chat { content: chat });
                                if let Some(tx) = &result_tx { let _ = tx.send(format!("Action output:\nFailed to open container at ({},{},{}): {e}", x, y, z)); }
                            }
                        }
                    }
                    BotCommand::AutoCraft { item, count } => {
                        match crate::azalea::auto_craft::do_auto_craft(&bot, &item, count).await {
                            Ok(msg) => {
                                let chat = format!("[自动合成] {msg}");
                                let _ = evt_tx.send(BotEvent::Chat { content: chat });
                                if let Some(tx) = &result_tx { let _ = tx.send(format!("Action output:\nAuto-crafted {item}. ({msg})")); }
                            }
                            Err(e) => {
                                let chat = format!("[自动合成失败] {e}");
                                let _ = evt_tx.send(BotEvent::Chat { content: chat });
                                if let Some(tx) = &result_tx { let _ = tx.send(format!("Action output:\nFailed to auto-craft {item}: {e}")); }
                            }
                        }
                    }
                    BotCommand::Enchant { item, level } => {
                        match crate::azalea::craft::do_enchant(&bot, &item, level).await {
                            Ok(msg) => {
                                let chat = format!("[附魔] {msg}");
                                let _ = evt_tx.send(BotEvent::Chat { content: chat });
                                if let Some(tx) = &result_tx { let _ = tx.send(format!("Action output:\nEnchanted {item} at level {level}. ({msg})")); }
                            }
                            Err(e) => {
                                let chat = format!("[附魔失败] {e}");
                                let _ = evt_tx.send(BotEvent::Chat { content: chat });
                                if let Some(tx) = &result_tx { let _ = tx.send(format!("Action output:\nFailed to enchant {item}: {e}")); }
                            }
                        }
                    }
                    BotCommand::Trade { offer } => {
                        let ext = bot.ecs.read().resource::<crate::azalea::ext_state::BotExtResource>().0.clone();
                        match crate::azalea::trade::do_trade(&bot, &ext, offer).await {
                            Ok(msg) => {
                                let chat = format!("[交易] {msg}");
                                let _ = evt_tx.send(BotEvent::Chat { content: chat });
                                if let Some(tx) = &result_tx { let _ = tx.send(format!("Action output:\nTrade offer {offer} completed. ({msg})")); }
                            }
                            Err(e) => {
                                let chat = format!("[交易失败] {e}");
                                let _ = evt_tx.send(BotEvent::Chat { content: chat });
                                if let Some(tx) = &result_tx { let _ = tx.send(format!("Action output:\nFailed to trade offer {offer}: {e}")); }
                            }
                        }
                    }
                    BotCommand::InteractEntity { kind } => {
                        let target = match kind.to_ascii_lowercase().as_str() {
                            "villager" => {
                                crate::azalea::trade::find_nearest_villager(&bot)
                                    .ok_or_else(|| "附近没有村民".to_string())
                            }
                            other => Err(format!("暂不支持的实体种类 {other}（目前仅 villager）")),
                        };
                        match target {
                            Ok(e) => {
                                bot.entity_interact(e);
                                let chat = format!("[交互] 已右键 {kind}");
                                let _ = evt_tx.send(BotEvent::Chat { content: chat });
                                if let Some(tx) = &result_tx { let _ = tx.send(format!("Action output:\nInteracted with {kind}.")); }
                            }
                            Err(e) => {
                                let chat = format!("[交互失败] {e}");
                                let _ = evt_tx.send(BotEvent::Chat { content: chat });
                                if let Some(tx) = &result_tx { let _ = tx.send(format!("Action output:\nFailed to interact with {kind}: {e}")); }
                            }
                        }
                    }
                }
                // 非轮询命令（异步/即时）执行完即清空中途槽与 busy，让队列推进下一条。
                {
                    let mut pending = state.pending.lock().unwrap();
                    if let Some(qc) = pending.as_ref() {
                        if !matches!(
                            &qc.cmd,
                            BotCommand::Goto { .. } | BotCommand::Mine { .. } | BotCommand::MineBelow
                        ) {
                            *pending = None;
                            *state.pending_since.lock().unwrap() = None;
                            *state.busy.lock().unwrap() = false;
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
                                    if s.is_empty() { continue; }
                                    let kind = format!("{:?}", s.kind()).to_lowercase();
                                    let cnt = s.count() as u32;
                                    *agg.entry(kind).or_insert(0) += cnt;
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
                                    format!("{bk:?}").to_lowercase()
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
                            format!("{:?}", item.kind()).to_lowercase()
                        }
                        _ => "air".to_string(),
                    };
                    // biome 通过 registry 解析为可读 Identifier（如 "minecraft:dark_forest"）。
                    // 旧实现 `format!("{b:?}")` 会输出 "biome { id: 30 }" 这种调试串，LLM 看不懂。
                    let biome = bot
                        .world()
                        .ok()
                        .and_then(|w| {
                            w.read()
                                .get_biome(BlockPos::new(
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
                                            format!("{bk:?}").to_lowercase()
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
                                            let kind = format!("{:?}", s.kind()).to_lowercase();
                                            format!("minecraft:{kind}")
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
                            "selected_slot": 0,
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
                                                format!("{bk:?}").to_lowercase()
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
                        items.iter().map(|(k, v)| format!("{k}:{v}")).collect::<Vec<_>>().join(", ")
                    };
                    // 资源分类摘要：把 10x10 里的方块按 wood/stone/ore/other 分组，
                    // 让 WorldInfo 的 find_match_line 能为每类找到独立的 label 行，
                    // 避免【场景提示】里 Wood/Stone/Ore 三条都粘同一份 10x10 字符串。
                    let resource_summary = {
                        let wood_kinds = ["oaklog", "darkoaklog", "birchlog", "sprucelog", "acalog", "junglelog", "mangrovelog", "cherrylog", "oakplanks", "darkoakplanks"];
                        let stone_kinds = ["stone", "cobblestone", "dirt", "grassblock", "sand", "gravel", "andesite", "granite", "diorite"];
                        let ore_kinds = ["coalore", "ironore", "copperore", "goldore", "diamondore", "emeraldore", "redstoneore", "lapisore", "netherquartzore"];
                        let mut wood = Vec::new();
                        let mut stone = Vec::new();
                        let mut ore = Vec::new();
                        for (k, v) in nearby_blocks.split(", ").map(|s| {
                            let mut it = s.split(':');
                            (it.next().unwrap_or("").to_string(), it.next().and_then(|x| x.parse::<u32>().ok()).unwrap_or(0))
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
                        if !wood.is_empty() { lines.push(format!("木材: {}", wood.join(", "))); }
                        if !stone.is_empty() { lines.push(format!("石头: {}", stone.join(", "))); }
                        if !ore.is_empty() { lines.push(format!("矿石: {}", ore.join(", "))); }
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
                            if v > 0 { parts.push(format!("{k}:{v}")); }
                        }
                        if parts.is_empty() { "无".to_string() } else { parts.join(", ") }
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
            if let Ok(p) = bot.position() {
                let foot = BlockPos::new(p.x.floor() as i32, (p.y - 1.0).floor() as i32, p.z.floor() as i32);
                let head = BlockPos::new(p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32);
                if let Ok(world) = bot.world() {
                    let under = world.read().get_block_state(foot);
                    let at = world.read().get_block_state(head);
                    let is_danger = |s: Option<azalea::block::BlockState>| -> bool {
                        s.map(|s| {
                            let bk: BlockKind = s.into();
                            matches!(bk, BlockKind::Lava | BlockKind::Fire | BlockKind::MagmaBlock)
                        }).unwrap_or(false)
                    };
                    if is_danger(under) || is_danger(at) {
                        let mut q = cmd_queue.lock().unwrap();
                        q.push(QueuedCommand {
                            cmd: BotCommand::Goto {
                                x: p.x.floor() as i32 + 5,
                                y: p.y.floor() as i32 + 1,
                                z: p.z.floor() as i32 + 5,
                            },
                            result_tx: None,
                        });
                        let _ = evt_tx.send(BotEvent::Chat {
                            content: "[MODE] 检测到火/岩浆，自动脱困".to_string(),
                        });
                    }
                }
            }
            // self_defense：空闲时自动攻击附近敌对生物（每 100 tick ≈5s 检查一次）
            if state.pending.lock().unwrap().is_none()
                && !*state.mining_below.lock().unwrap()
                && bot.ticks_connected() % 100 == 0
            {
                if let Ok(entities) = bot.nearest_entities::<bevy_ecs::query::Without<azalea::entity::metadata::Player>>() {
                    let self_id = bot.entity().id();
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

    /// 3×3 工作台合成（要求已打开工作台）。item 为目标物品 id，count 为期望数量。
    pub fn craft_3x3(&self, item: String, count: u32) {
        self.push_cmd(BotCommand::Craft3x3 { item, count });
    }

    /// 熔炼（要求已打开熔炉/高炉/烟熏炉）。output 目标物品 id，fuel 燃料 id，count 数量。
    pub fn smelt(&self, output: String, fuel: String, count: u32) {
        self.push_cmd(BotCommand::Smelt {
            output,
            fuel,
            count,
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

    /// 推送动作指令（fire-and-forget，handler tick 中执行）。
    fn push_cmd(&self, cmd: BotCommand) {
        self.cmd_queue.lock().unwrap().push(QueuedCommand { cmd, result_tx: None });
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
