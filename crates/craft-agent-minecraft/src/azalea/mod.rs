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
pub mod client;
pub mod perception;

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
            // 消费命令队列（非阻塞）。
            let mut q = cmd_queue.lock().unwrap();
            let cmds: Vec<BotCommand> = q.drain(..).collect();
            drop(q);
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

    /// 推送动作指令（fire-and-forget，handler tick 中执行）。
    fn push_cmd(&self, cmd: BotCommand) {
        self.cmd_queue.lock().unwrap().push(cmd);
    }
}

/// 便捷类型：共享的 bot 句柄。
pub type SharedBot = Arc<AzaleaBot>;
