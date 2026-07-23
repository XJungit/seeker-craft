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
pub mod gather;
pub mod perception;
pub mod place;

use azalea::prelude::*;
use azalea::pathfinder::goals::BlockPosGoal;
use azalea::BlockPos;
use azalea_client::client_chat::ChatPacket;
use bevy_ecs::component::Component;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

/// 转发给外部的 bot 事件（供 harness / LLM 消费）。
#[derive(Debug, Clone)]
pub enum BotEvent {
    /// 连入世界成功。
    Spawn { position: azalea::Vec3 },
    /// 收到游戏聊天（LLM 指令入口）。
    Chat { content: String },
    /// 与服务端断开。
    Disconnect { reason: String },
    /// 周期性状态快照（位置 + 背包概要 + 附近玩家数）。
    State {
        position: azalea::Vec3,
        inventory: Vec<String>,
        player_count: usize,
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
}

/// handler 状态：持有命令队列、事件发送端与最近坐标（跨事件持久，Arc 共享）。
#[derive(Component, Clone)]
pub struct BotState {
    pub cmd_queue: Arc<Mutex<Vec<BotCommand>>>,
    pub evt_tx: Arc<mpsc::UnboundedSender<BotEvent>>,
    pub last_position: Arc<Mutex<Option<azalea::Vec3>>>,
    /// 持续下挖标志：收到 MineBelow 后置 true，Tick 内只要未在挖就重复触发，
    /// 对齐 POC 的持续挖矿逻辑（azalea 单次 start_mining 可能因中断失效）。
    pub mining_below: Arc<Mutex<bool>>,
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
        }
    }
}

/// Azalea bot 句柄：连入后持有命令队列与事件通道，提供动作与感知 API。
pub struct AzaleaBot {
    cmd_queue: Arc<Mutex<Vec<BotCommand>>>,
    events: Arc<tokio::sync::Mutex<mpsc::UnboundedReceiver<BotEvent>>>,
    /// 最近一次已知坐标（由 handler Tick 更新，供同步读取）。
    pub last_position: Arc<Mutex<Option<azalea::Vec3>>>,
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
    pub async fn connect(address: &str, username: &str) -> anyhow::Result<AzaleaBot> {
        let account = Account::offline(username);
        let (evt_tx, evt_rx) = mpsc::unbounded_channel::<BotEvent>();
        let evt_tx = Arc::new(evt_tx);
        let cmd_queue: Arc<Mutex<Vec<BotCommand>>> = Arc::new(Mutex::new(Vec::new()));
        let last_position: Arc<Mutex<Option<azalea::Vec3>>> = Arc::new(Mutex::new(None));

        let state = BotState {
            cmd_queue: cmd_queue.clone(),
            evt_tx: evt_tx.clone(),
            last_position: last_position.clone(),
            mining_below: Arc::new(Mutex::new(false)),
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
            //   goto <x> <y> <z> / mine <x> <y> <z> / minebelow / attack
            {
                let mut q = cmd_queue.lock().unwrap();
                if let Some(rest) = content.strip_prefix("autocraft ") {
                    let mut parts = rest.split_whitespace();
                    if let Some(item) = parts.next() {
                        let count =
                            parts.next().and_then(|s| s.parse::<u32>().ok()).unwrap_or(1);
                        q.push(BotCommand::AutoCraft {
                            item: item.to_string(),
                            count,
                        });
                    }
                } else if let Some(rest) = content.strip_prefix("open ") {
                    let c: Vec<i32> =
                        rest.split_whitespace().filter_map(|s| s.parse().ok()).collect();
                    if c.len() == 3 {
                        q.push(BotCommand::OpenContainer { x: c[0], y: c[1], z: c[2] });
                    }
                } else if let Some(rest) = content.strip_prefix("place ") {
                    let mut parts = rest.split_whitespace();
                    if let Some(item) = parts.next() {
                        let c: Vec<i32> =
                            parts.filter_map(|s| s.parse().ok()).collect();
                        if c.len() == 3 {
                            q.push(BotCommand::Place {
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
                        let count =
                            parts.next().and_then(|s| s.parse::<u32>().ok()).unwrap_or(1);
                        q.push(BotCommand::Gather {
                            item: item.to_string(),
                            count,
                        });
                    }
                } else if let Some(rest) = content.strip_prefix("craft3 ") {
                    let mut parts = rest.split_whitespace();
                    if let Some(item) = parts.next() {
                        let count =
                            parts.next().and_then(|s| s.parse::<u32>().ok()).unwrap_or(1);
                        q.push(BotCommand::Craft3x3 {
                            item: item.to_string(),
                            count,
                        });
                    }
                } else if let Some(rest) = content.strip_prefix("smelt ") {
                    let mut parts = rest.split_whitespace();
                    if let Some(output) = parts.next() {
                        let fuel = parts.next().unwrap_or("coal").to_string();
                        let count =
                            parts.next().and_then(|s| s.parse::<u32>().ok()).unwrap_or(1);
                        q.push(BotCommand::Smelt {
                            output: output.to_string(),
                            fuel,
                            count,
                        });
                    }
                } else if let Some(rest) = content.strip_prefix("craft ") {
                    let mut parts = rest.split_whitespace();
                    if let Some(item) = parts.next() {
                        let count =
                            parts.next().and_then(|s| s.parse::<u32>().ok()).unwrap_or(1);
                        q.push(BotCommand::Craft2x2 {
                            item: item.to_string(),
                            count,
                        });
                    }
                } else if let Some(rest) = content.strip_prefix("goto ") {
                    let c: Vec<i32> =
                        rest.split_whitespace().filter_map(|s| s.parse().ok()).collect();
                    if c.len() == 3 {
                        q.push(BotCommand::Goto { x: c[0], y: c[1], z: c[2] });
                    }
                } else if let Some(rest) = content.strip_prefix("mine ") {
                    let c: Vec<i32> =
                        rest.split_whitespace().filter_map(|s| s.parse().ok()).collect();
                    if c.len() == 3 {
                        q.push(BotCommand::Mine { x: c[0], y: c[1], z: c[2] });
                    }
                } else if content == "minebelow" {
                    q.push(BotCommand::MineBelow);
                } else if content == "attack" {
                    q.push(BotCommand::Attack { target: "chat".into() });
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
            // 消费命令队列（非阻塞）。把 drain 包在独立作用域，确保 MutexGuard
            // 在 await 前彻底离开作用域（否则 future 因持有 !Send 的 guard 而不满足 Send）。
            let cmds: Vec<BotCommand> = {
                let mut q = cmd_queue.lock().unwrap();
                q.drain(..).collect()
            };
            for cmd in cmds {
                match cmd {
                    BotCommand::Goto { x, y, z } => {
                        *state.mining_below.lock().unwrap() = false;
                        bot.start_goto(BlockPosGoal(BlockPos::new(x, y, z)));
                    }
                    BotCommand::Mine { x, y, z } => {
                        *state.mining_below.lock().unwrap() = false;
                        bot.start_mining(BlockPos::new(x, y, z));
                    }
                    BotCommand::MineBelow => {
                        // 进入持续下挖模式：置标志，Tick 内自动续挖。
                        *state.mining_below.lock().unwrap() = true;
                        if let Ok(p) = bot.position() {
                            let foot = BlockPos::new(
                                p.x.floor() as i32,
                                (p.y - 1.0).floor() as i32,
                                p.z.floor() as i32,
                            );
                            bot.start_mining(foot);
                        }
                    }
                    BotCommand::BlockInteract { x, y, z } => {
                        *state.mining_below.lock().unwrap() = false;
                        bot.block_interact(BlockPos::new(x, y, z));
                    }
                    BotCommand::Chat { content } => {
                        bot.chat(&content);
                    }
                    BotCommand::Attack { target: _target } => {
                        // 攻击最近的「非玩家」实体（自卫/狩猎）。
                        // nearest_entities 返回按距离排序的 EntityRef；用 Without<Player>
                        // 过滤掉玩家，再跳过本地 bot 自身。找不到则无操作。
                        if let Ok(entities) =
                            bot.nearest_entities::<bevy_ecs::query::Without<azalea::entity::metadata::Player>>()
                        {
                            let self_id = bot.entity().id();
                            for e in entities.iter() {
                                if e.id() == self_id {
                                    continue;
                                }
                                e.attack();
                                break;
                            }
                        }
                    }
                    BotCommand::Craft2x2 { item, count } => {
                        // 2×2 背包合成（异步：需等服务端回填网格与结果槽）。
                        // 在 handler 内 await 是 azalea 既有模式（其示例也在事件
                        // handler 中 await 阻塞式 bot 操作）；current_thread runtime
                        // 在 await 期间仍可处理容器更新包。
                        match crate::azalea::craft::do_craft_2x2(&bot, &item, count).await {
                            Ok(msg) => {
                                let _ = evt_tx.send(BotEvent::Chat { content: format!("[合成] {msg}") });
                            }
                            Err(e) => {
                                let _ = evt_tx.send(BotEvent::Chat {
                                    content: format!("[合成失败] {e}"),
                                });
                            }
                        }
                    }
                    BotCommand::Craft3x3 { item, count } => {
                        match crate::azalea::craft::do_craft_3x3(&bot, &item, count).await {
                            Ok(msg) => {
                                let _ = evt_tx.send(BotEvent::Chat { content: format!("[合成] {msg}") });
                            }
                            Err(e) => {
                                let _ = evt_tx.send(BotEvent::Chat {
                                    content: format!("[合成失败] {e}"),
                                });
                            }
                        }
                    }
                    BotCommand::Smelt { output, fuel, count } => {
                        match crate::azalea::craft::do_smelt(&bot, &output, &fuel, count).await {
                            Ok(msg) => {
                                let _ = evt_tx.send(BotEvent::Chat { content: format!("[熔炼] {msg}") });
                            }
                            Err(e) => {
                                let _ = evt_tx.send(BotEvent::Chat {
                                    content: format!("[熔炼失败] {e}"),
                                });
                            }
                        }
                    }
                    BotCommand::Gather { item, count } => {
                        match crate::azalea::gather::do_gather(&bot, &item, count).await {
                            Ok(msg) => {
                                let _ = evt_tx.send(BotEvent::Chat { content: format!("[采集] {msg}") });
                            }
                            Err(e) => {
                                let _ = evt_tx.send(BotEvent::Chat {
                                    content: format!("[采集失败] {e}"),
                                });
                            }
                        }
                    }
                    BotCommand::Place { item, x, y, z } => {
                        match crate::azalea::place::do_place(&bot, &item, BlockPos::new(x, y, z)).await {
                            Ok(msg) => {
                                let _ = evt_tx.send(BotEvent::Chat { content: format!("[放置] {msg}") });
                            }
                            Err(e) => {
                                let _ = evt_tx.send(BotEvent::Chat {
                                    content: format!("[放置失败] {e}"),
                                });
                            }
                        }
                    }
                    BotCommand::OpenContainer { x, y, z } => {
                        match crate::azalea::place::do_open_container(&bot, BlockPos::new(x, y, z)).await {
                            Ok(msg) => {
                                let _ = evt_tx.send(BotEvent::Chat { content: format!("[开容器] {msg}") });
                            }
                            Err(e) => {
                                let _ = evt_tx.send(BotEvent::Chat {
                                    content: format!("[开容器失败] {e}"),
                                });
                            }
                        }
                    }
                    BotCommand::AutoCraft { item, count } => {
                        match crate::azalea::auto_craft::do_auto_craft(&bot, &item, count).await {
                            Ok(msg) => {
                                let _ = evt_tx.send(BotEvent::Chat { content: format!("[自动合成] {msg}") });
                            }
                            Err(e) => {
                                let _ = evt_tx.send(BotEvent::Chat {
                                    content: format!("[自动合成失败] {e}"),
                                });
                            }
                        }
                    }
                }
            }
            // 持续下挖：只要标志为真且当前未在挖，就续挖（对齐 POC 逻辑，
            // 避免单次 start_mining 因中断失效导致 bot 停在原地不下降）。
            if *state.mining_below.lock().unwrap() && !bot.is_mining() {
                if let Ok(p) = bot.position() {
                    let foot = BlockPos::new(
                        p.x.floor() as i32,
                        (p.y - 1.0).floor() as i32,
                        p.z.floor() as i32,
                    );
                    bot.start_mining(foot);
                }
            }
            // 每 20 tick 推送状态快照。
            let t = bot.ticks_connected();
            if t % 20 == 0 {
                if let Ok(p) = bot.position() {
                    let inventory = match bot.get_inventory() {
                        Ok(inv) => match inv.slots() {
                            Some(slots) => slots
                                .iter()
                                .take(5)
                                .map(|s| {
                                    if s.is_empty() {
                                        "空".to_string()
                                    } else {
                                        format!("{s:?}")
                                    }
                                })
                                .collect(),
                            None => vec!["slots=None".to_string()],
                        },
                        Err(_) => vec!["获取失败".to_string()],
                    };
                    let player_count = bot.nearby_players().map(|p| p.len()).unwrap_or(0);
                    let _ = evt_tx.send(BotEvent::State {
                        position: p,
                        inventory,
                        player_count,
                    });
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

    /// 推送动作指令（fire-and-forget，handler tick 中执行）。
    fn push_cmd(&self, cmd: BotCommand) {
        self.cmd_queue.lock().unwrap().push(cmd);
    }
}

/// 便捷类型：共享的 bot 句柄。
pub type SharedBot = Arc<AzaleaBot>;
