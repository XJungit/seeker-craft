//! Azalea bot 句柄（连接/动作封装/取消）：从 mod.rs 纯移动拆出，行为一致。
//!
//! `AzaleaBot::connect` 构造 `handler::BotState` 并 spawn 专用 current-thread runtime
//! 跑 azalea 客户端循环；动作方法全是 `push_cmd` 薄封装，感知经事件通道。

use super::BotEvent;
use super::action_manager::ActionManager;
use super::commands::{BotCommand, QueuedCommand};
use super::handler::BotState;
use azalea::prelude::*;
use azalea_client::account::Account;
use craft_agent::core::memory::WorldMemory;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;

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
    /// P95：取消请求标志（与 handler 内 BotState.cancel_flag 同一实例）。
    pub cancel_flag: Arc<AtomicBool>,
}

impl AzaleaBot {
    /// 异步接收下一个 bot 事件（供 harness 主循环消费）。
    pub async fn next_event(&self) -> Option<BotEvent> {
        let mut rx = self.events.lock().await;
        rx.recv().await
    }
}

/// 测试构造：离线 bot（不连接服务器），仅用于纯逻辑/schema 测试。
/// 被 azalea 内部 cancel_tests 与 adapter_azalea 的 `MinecraftAzaleaAdapter::default()`
/// 复用（字段私有，只有本模块可构造）。
#[cfg(test)]
impl AzaleaBot {
    pub(crate) fn offline_for_test() -> AzaleaBot {
        let (_, evt_rx) = mpsc::unbounded_channel::<BotEvent>();
        AzaleaBot {
            cmd_queue: Arc::new(Mutex::new(Vec::new())),
            events: Arc::new(tokio::sync::Mutex::new(evt_rx)),
            last_position: Arc::new(Mutex::new(None)),
            ext: Arc::new(Mutex::new(crate::azalea::ext_state::BotExtState::default())),
            memory: None,
            cancel_flag: Arc::new(AtomicBool::new(false)),
        }
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
        let cancel_flag: Arc<AtomicBool> = Arc::new(AtomicBool::new(false));
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
            action_mgr: ActionManager::new(),
            memory,
            scanned: Arc::new(Mutex::new(HashMap::new())),
            hunt_pickup_until: Arc::new(Mutex::new(0)),
            combat_equip_pending: Arc::new(Mutex::new(None)),
            combat_strafe_cd: Arc::new(Mutex::new(0)),
            goto_watchdog: Arc::new(Mutex::new((0, 0, 0, 0))),
            goto_cooldown: Arc::new(Mutex::new(HashMap::new())),
            goto_stuck: Arc::new(Mutex::new((None, 0))),
            no_move_ticks: Arc::new(Mutex::new(0)),
            last_seen_pos: Arc::new(Mutex::new((0, 0, 0))),
            make_obsidian: Arc::new(Mutex::new(None)),
            make_obsidian_start_tick: Arc::new(Mutex::new(None)),
            interact_hold_until: Arc::new(Mutex::new(None)),
            escape_up_cooldown: Arc::new(Mutex::new(None)),
            cancel_flag: cancel_flag.clone(),
            last_mine_eff: Arc::new(Mutex::new(None)),
            mode_switches: Arc::new(Mutex::new(std::collections::HashSet::new())),
            mine_approach_watchdog: Arc::new(Mutex::new(None)),
            mining_above_no_pick_warned: Arc::new(Mutex::new(false)),
            mining_above_soft_column: Arc::new(Mutex::new(None)),
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
            cancel_flag,
        })
    }

    /// 攻击最近的生物（自卫/狩猎）。
    pub fn attack(&self, target: String) {
        self.push_cmd(BotCommand::Attack { target });
    }

    /// P119：拉弓射箭（龙战远程必需）。target 为实体名（None=朝当前视角方向射）。
    pub fn shoot(&self, target: Option<String>) {
        self.push_cmd(BotCommand::Shoot { target });
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

    /// P95：取消所有排队命令 + 请求中断当前执行中的命令。
    ///
    /// - 队列中未执行的命令全部丢弃，其 `result_tx` 收到「已取消」文本（若存在）。
    /// - 置位 `cancel_flag`，由 handler 下一 tick 执行真正的中止：
    ///   轮询命令（Goto/Mine）强停寻路并清槽；异步命令（Craft/Gather 等）无法
    ///   中断执行体，等其自然完成后因队列已空而停止。
    /// - 返回被取消的排队命令数。
    pub fn cancel_commands(&self) -> usize {
        let drained: Vec<QueuedCommand> = {
            let mut q = self.cmd_queue.lock().unwrap();
            q.drain(..).collect()
        };
        for qc in &drained {
            if let Some(tx) = &qc.result_tx {
                let _ = tx.send("已取消（cancel_commands）".to_string());
            }
        }
        self.cancel_flag.store(true, Ordering::SeqCst);
        drained.len()
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
mod cancel_tests {
    use super::*;

    fn offline_bot() -> AzaleaBot {
        let (_, evt_rx) = mpsc::unbounded_channel::<BotEvent>();
        AzaleaBot {
            cmd_queue: Arc::new(Mutex::new(Vec::new())),
            events: Arc::new(tokio::sync::Mutex::new(evt_rx)),
            last_position: Arc::new(Mutex::new(None)),
            ext: Arc::new(Mutex::new(crate::azalea::ext_state::BotExtState::default())),
            memory: None,
            cancel_flag: Arc::new(AtomicBool::new(false)),
        }
    }

    #[test]
    fn cancel_commands_drains_queue_and_notifies_waiters() {
        let bot = offline_bot();
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        bot.cmd_queue.lock().unwrap().push(QueuedCommand {
            cmd: BotCommand::Goto { x: 1, y: 2, z: 3 },
            result_tx: Some(tx),
        });
        bot.cmd_queue.lock().unwrap().push(QueuedCommand {
            cmd: BotCommand::Gather {
                item: "oak_log".into(),
                count: 4,
            },
            result_tx: None,
        });
        let cancelled = bot.cancel_commands();
        assert_eq!(cancelled, 2, "应返回被取消的排队命令数");
        assert!(bot.cmd_queue.lock().unwrap().is_empty(), "队列应清空");
        assert!(
            bot.cancel_flag.load(Ordering::SeqCst),
            "cancel_flag 应置位供 handler 取走"
        );
        let msg = rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(msg.contains("已取消"), "等待者应收到取消文本, got: {msg}");
    }

    #[test]
    fn cancel_flag_is_taken_by_handler_semantics() {
        // 模拟 handler tick 的 swap：第二次检查应为 false（只处理一次）。
        let flag = Arc::new(AtomicBool::new(false));
        assert!(!flag.swap(false, Ordering::SeqCst));
        flag.store(true, Ordering::SeqCst);
        assert!(flag.swap(false, Ordering::SeqCst), "第一次应取到取消请求");
        assert!(!flag.swap(false, Ordering::SeqCst), "取走后不再重复处理");
    }
}
