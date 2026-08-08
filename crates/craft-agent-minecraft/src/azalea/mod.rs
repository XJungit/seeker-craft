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
pub mod chest;
pub mod commands;
pub mod craft;
pub mod ext_state;
pub mod gather;
pub mod handler;
pub mod harvest;
pub mod place;
pub mod recipe_book;
pub mod recipes;
pub mod sleep;
pub mod smart_actions;
pub mod table_flow;
pub mod till;
pub mod trade;

pub use action_manager::{ActionManager, Priority, SubmitOutcome, cmd_signature, timeout_ticks};
pub use commands::{BotCommand, QueuedCommand, parse_chat_command};
pub use handler::BotState;

use azalea::BlockPos;
use azalea::pathfinder::goals::BlockPosGoal;
use azalea::prelude::*;
use azalea_registry::builtin::{BlockKind, EntityKind};
use craft_agent::core::memory::WorldMemory;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;

/// 整数方块坐标（watchdog / 冷却表用）。
pub type ChunkPos = (i32, i32, i32);
/// make_obsidian 状态机：(剩余数, 阶段, 黑曜石坐标)。
pub type ObsidianTask = Option<(u32, u8, Option<ChunkPos>)>;
/// 感知聚合：(实体名, (数量, 最近距离, 坐标))。
pub type EntityAgg = HashMap<String, (u32, f64, ChunkPos)>;

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
            no_move_ticks: Arc::new(Mutex::new(0)),
            last_seen_pos: Arc::new(Mutex::new((0, 0, 0))),
            make_obsidian: Arc::new(Mutex::new(None)),
            cancel_flag: cancel_flag.clone(),
            last_mine_eff: Arc::new(Mutex::new(None)),
            mode_switches: Arc::new(Mutex::new(std::collections::HashSet::new())),
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

#[cfg(test)]
mod normalize_item_tests {
    use super::*;

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
/// 归一化物品 id：补 `minecraft:` 前缀，并对常见单复数笔误容错
/// （Mindcraft commands/index.js 同款规则）：`oak_plank` → `oak_planks`、
/// `wheat_seed` → `wheat_seeds`。已带前缀/已复数的 id 不受影响
/// （`planks` 不以 `plank` 结尾，`seeds` 不以 `seed` 结尾）。
pub(crate) fn normalize_item_id(item: &str) -> String {
    let bare = item.strip_prefix("minecraft:").unwrap_or(item);
    let mut id = bare.to_string();
    // 单数结尾（plank/seed）→ 补 s 变复数；已复数（planks/seeds）不以单数结尾，不受影响。
    for suffix in ["plank", "seed"] {
        if id.ends_with(suffix) {
            id.push('s');
            break;
        }
    }
    format!("minecraft:{id}")
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
        if let Some(stack) = slots.get(s)
            && !stack.is_empty()
            && stack.kind() == kind
        {
            out.push(s);
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
        if let Some(stack) = slots.get(s)
            && !stack.is_empty()
            && stack.kind() == kind
        {
            let idx = (s - hotbar_start) as u8;
            debug_assert!(idx <= 8, "hotbar idx out of range: {idx}");
            return Some(idx);
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

/// P83：从 (x, head_y, z) 的上一格起向上数连续实心方块（非空气/水/岩浆），
/// 遇空气或未加载（None）即停，上限 64。0 = 头顶即空气（洞穴/地表）；
/// N 越大埋得越深（需 mine_above 挖出）。
fn count_overhead_solid(
    get_state: impl Fn(BlockPos) -> Option<azalea::block::BlockState>,
    x: i32,
    head_y: i32,
    z: i32,
) -> u32 {
    let mut n = 0u32;
    for dy in 1..=64 {
        let state = get_state(BlockPos::new(x, head_y + dy, z));
        let solid = state
            .map(|s| {
                let bk: BlockKind = s.into();
                bk != BlockKind::Air && !matches!(bk, BlockKind::Water | BlockKind::Lava)
            })
            .unwrap_or(false);
        if solid {
            n += 1;
        } else {
            break;
        }
    }
    n
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
                    // 缓存过期兜底：find_hotbar_slot_for 命中但切槽后主手不对
                    // （本地 slots 缓存滞后于服务端），强制 shift_click 归位重试。
                    drop(inv);
                    match force_hold_in_hotbar(bot, kind).await {
                        Ok(h2) => {
                            return format!(
                                "已装备 {item} 到主手（缓存过期，归位后 hotbar 槽 {h2}）"
                            );
                        }
                        Err(e) => {
                            return format!(
                                "装备 {item} 失败：set_selected_hotbar_slot({h}) 后主手仍未持有 {item}\
                                 （已轮询 1.5s，兜底归位也失败: {e}，建议稍后重试）"
                            );
                        }
                    }
                }

                // 不在 hotbar，从主背包 shift_click 到 hotbar（服务端找第一个空槽）
                let srcs = find_item_slots(&inv, kind);
                if !srcs.is_empty() {
                    // P8/P134 修复：hotbar 满时 shift_click 无法移动物品。
                    // 两级腾槽：① QuickMoveClick（服务端自动合并同类堆/找空槽，不产生
                    // 光标悬挂——旧实现 left_click 手动拾放在主背包满时把物品卡在光标上，
                    // 后续 click 全部错乱）；② QuickMove 仍失败（主背包 36 格全满且无
                    // 同类堆可合并，实测 cobblestone 448 = 7 整堆）→ 直接丢弃 hotbar 中
                    // 堆叠最大的非目标物品腾出一格（装备在 2s 吸回前完成，扔出的掉落物
                    // 只能回主背包，不影响 hotbar 结果）。
                    if let Some(menu) = inv.menu().ok().flatten() {
                        let hotbar_range = menu.hotbar_slots_range();
                        if let Some(slots) = inv.slots() {
                            let hotbar_full = hotbar_range
                                .clone()
                                .all(|s| slots.get(s).map(|st| !st.is_empty()).unwrap_or(false));
                            if hotbar_full {
                                for hs in hotbar_range.clone() {
                                    if let Some(st) = slots.get(hs)
                                        && !st.is_empty()
                                    {
                                        inv.shift_click(hs);
                                        sleep(Duration::from_millis(200)).await;
                                        break;
                                    }
                                }
                            }
                        }
                    }
                    // slots 是快照——重读验证腾挪结果，仍满则丢弃兜底
                    drop(inv);
                    let inv = match bot.get_inventory() {
                        Ok(i) => i,
                        Err(e) => return format!("获取背包失败: {e:?}"),
                    };
                    if let (Some(menu), Some(slots)) = (inv.menu().ok().flatten(), inv.slots()) {
                        let hotbar_range = menu.hotbar_slots_range();
                        let still_full = hotbar_range
                            .clone()
                            .all(|s| slots.get(s).map(|st| !st.is_empty()).unwrap_or(false));
                        if still_full
                            && let Some(victim) = hotbar_range
                                .clone()
                                .filter(|&s| {
                                    // P134c：兜底丢弃只允许"多件堆叠"（count>1）。工具/武器/镐/盔甲
                                    // 恒为单件——旧实现取"堆叠最大非目标"，hotbar 全是单件工具时
                                    // 会把刚装备的镐或关键工具当最大堆丢出去（实测 iron_pickaxe /
                                    // stone_pickaxe 两次神秘消失都发生在换装/挖掘后，主背包同时满）。
                                    slots
                                        .get(s)
                                        .map(|st| {
                                            !st.is_empty()
                                                && st.count() > 1
                                                && st.kind() != kind
                                        })
                                        .unwrap_or(false)
                                })
                                .max_by_key(|&s| slots.get(s).map(|st| st.count()).unwrap_or(0))
                        {
                            inv.click(ThrowClick::All {
                                slot: victim as u16,
                            });
                            sleep(Duration::from_millis(200)).await;
                        }
                    }
                    // 重定位槽位（丢弃/移动可能改变布局）
                    let src = match find_item_slots(&inv, kind).first() {
                        Some(s) => *s,
                        None => return format!("背包未持有 {item}（腾槽后重新定位失败）"),
                    };
                    inv.shift_click(src);
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
                         可能原因：1) hotbar 已满（9 格全非空）且主背包也无空槽；2) 服务端同步延迟。\
                         建议：先 discard 一些主背包的无用物品（丢后走开 2+ 格防吸回），再重试 equip。"
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
                    if let Some(st) = slots.get(s)
                        && !st.is_empty()
                    {
                        diag_items.push(format!("slot{}={}x{}", s, st.kind().to_str(), st.count()));
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
                        Some(s) => s
                            .get(armor_slot_idx)
                            .map(|st| !st.is_empty())
                            .unwrap_or(false),
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
                    return format!(
                        "已装备 {item} 到 {slot_norm}（left_click 槽 {src}→{armor_slot_idx}）"
                    );
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

/// 强制把 item 归位到 hotbar 并选中（兜底 hotbar 缓存过期场景）。
///
/// 背景：`get_inventory().slots()` 是本地缓存，可能滞后于服务端实际内容。
/// 现象：`find_hotbar_slot_for` 命中槽 h（缓存里槽 h = item），但
/// `set_selected_hotbar_slot(h)` 后主手是别的物品（服务端槽 h 实际内容不同）。
/// 此时原逻辑直接报错——LLM 重试同样失败，形成死循环。
///
/// 兜底：把主背包里的 item shift_click 到 hotbar（服务端自动找落点），
/// 重新选中并轮询验证。成功返回 hotbar 槽号；失败返回 Err（含原因）。
pub async fn force_hold_in_hotbar(bot: &Client, kind: ItemKind) -> Result<u8, String> {
    // 尝试 1：当前缓存命中槽位 + 轮询验证（缓存可能是准的）
    let inv = bot
        .get_inventory()
        .map_err(|e| format!("获取背包失败: {e:?}"))?;
    if let Some(h) = find_hotbar_slot_for(&inv, kind) {
        bot.set_selected_hotbar_slot(h);
        if wait_for_held_item(bot, kind, 1200).await {
            return Ok(h);
        }
    }
    // 尝试 2：从主背包 shift_click 归位（排除 hotbar 槽，避免把 item 移出 hotbar）
    let is_hotbar = |s: usize| {
        inv.menu()
            .ok()
            .flatten()
            .map(|m| m.hotbar_slots_range().contains(&s))
            .unwrap_or(false)
    };
    let src = find_item_slots(&inv, kind)
        .into_iter()
        .find(|&s| !is_hotbar(s));
    let Some(src) = src else {
        return Err(format!("背包未持有 {}（无法归位到 hotbar）", kind.to_str()));
    };
    inv.shift_click(src);
    sleep(Duration::from_millis(250)).await;
    drop(inv);
    let inv2 = bot
        .get_inventory()
        .map_err(|e| format!("归位后重新获取背包失败: {e:?}"))?;
    if let Some(h) = find_hotbar_slot_for(&inv2, kind) {
        bot.set_selected_hotbar_slot(h);
        if wait_for_held_item(bot, kind, 1200).await {
            return Ok(h);
        }
    }
    Err(format!(
        "强制归位失败：shift_click 后 hotbar 仍未持有 {}",
        kind.to_str()
    ))
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
        // P134：1x1 竖井/窄洞等水平 4 方向全堵场景，向上走是唯一脱身方向
        // （上方是挖出的空气柱）。竖直距离 4 格 > 1.5m 吸回半径，物品不会被吸回。
        // 逐格上移：4 格一跳 pathfinder 在竖井中经常判不可达，单格跳跃每次都能过
        // （每格 700ms，4 格共 2.8s > 2s pickup delay 窗口）。
        let mut last_y = start_pos.map(|p| p.y);
        for _ in 0..4 {
            if let Ok(p) = bot.position() {
                bot.start_goto(BlockPosGoal(BlockPos::new(
                    p.x.floor() as i32,
                    (p.y + 1.0).floor() as i32,
                    p.z.floor() as i32,
                )));
                sleep(Duration::from_millis(700)).await;
            }
            if let Ok(p) = bot.position() {
                // 已脱离吸回半径，尽早停止
                if start_pos.is_some_and(|s| (p.y - s.y).abs() > 2.0) {
                    break;
                }
                // y 未上升说明上方也堵（头顶实心），放弃向上
                if last_y.is_some_and(|prev| (p.y - prev).abs() < 0.5) {
                    break;
                }
                last_y = Some(p.y);
            }
        }
        bot.stop_pathfinding();
        // 验证是否真的走开（若原地打转则物品会被吸回）
        let moved_away = match (start_pos, bot.position().ok()) {
            (Some(s), Some(p)) => {
                (p.x - s.x).abs() > 2.0 || (p.y - s.y).abs() > 2.0 || (p.z - s.z).abs() > 2.0
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
        // P127：移植 do_equip 的 P8 修复——hotbar 满时 shift_click 无法移动物品
        // （服务端没有空 hotbar 槽可接收），表现为"背包未持有 X"但 perceive 明明
        // 显示 X 在主背包（实测 red_mushroom 就在 slot 17 却无法消耗）。
        // 先腾一个空 hotbar 槽：把第一个 hotbar 物品 QuickMove 到主背包
        // （P134 修复：旧 left_click 拿放逻辑在主背包无空槽时物品卡在光标上，
        // 后续 click 全乱——QuickMoveClick 由服务端自动合并同类堆/找空槽）。
        if let Some(menu) = inv.menu().ok().flatten() {
            let hotbar_range = menu.hotbar_slots_range();
            if let Some(slots) = inv.slots() {
                let hotbar_full = hotbar_range
                    .clone()
                    .all(|s| slots.get(s).map(|st| !st.is_empty()).unwrap_or(false));
                if hotbar_full {
                    for hs in hotbar_range.clone() {
                        if let Some(st) = slots.get(hs)
                            && !st.is_empty()
                        {
                            inv.shift_click(hs);
                            sleep(Duration::from_millis(200)).await;
                            break;
                        }
                    }
                }
            }
        }
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
        if let Ok(inv3) = bot.get_inventory()
            && let Some(menu) = inv3.menu().ok().flatten()
        {
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
        // P128：Java 版蘑菇不能生吃（无食物组件），但 1 蘑菇 + 1 碗可合成
        // 蘑菇煲（+6 饥饿）。实测 LLM 会误以为"蘑菇不可食用"转去找其它食物，
        // 忽略背包里现成的 red_mushroom + bowl——给明确合成指引。
        let stew_hint = if item.contains("mushroom") && !item.contains("stew") {
            "。蘑菇不能生吃（Java 无食物组件）——用 craft('mushroom_stew') 合成蘑菇煲（1 蘑菇 + 1 碗，2x2）后再 consume('mushroom_stew')"
        } else {
            ""
        };
        format!("尝试消耗 {item}，但数量未减少（{before} → {after}，{hint}{stew_hint}）")
    }
}

/// 统计背包中指定物品的总数。
pub(crate) fn count_item(bot: &Client, kind: ItemKind) -> u32 {
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

    /// P83：头顶实心计数——地表/洞穴头顶空气 n=0；
    /// 深埋时数到空气为止；未加载视为空气停止；水/岩浆不算实心。
    #[test]
    fn count_overhead_solid_basic() {
        // 头顶全空气 → 0
        let air = |_bp: BlockPos| None;
        assert_eq!(count_overhead_solid(air, 0, 94, 0), 0);
        // 从 +1 起 3 格石头，+4 是空气 → 3
        let stone3 = |bp: BlockPos| {
            if (1..=3).contains(&bp.y) {
                Some(azalea::block::BlockState::from(B::Stone))
            } else {
                None
            }
        };
        assert_eq!(count_overhead_solid(stone3, 0, 0, 0), 3);
        // 5 格实心（stone）后空气 → 5
        let stone5 = |bp: BlockPos| {
            if (1..=5).contains(&bp.y) {
                Some(azalea::block::BlockState::from(B::Stone))
            } else {
                None
            }
        };
        assert_eq!(count_overhead_solid(stone5, 0, 0, 0), 5);
    }

    /// P83：水/岩浆不算实心（头顶积水洞穴不算埋住），计数应停在它们下方。
    #[test]
    fn count_overhead_solid_ignores_liquid() {
        // +1 石头、+2 水、+3 石头 → 只数到 +1，水即停止 → 1
        let blocks = |bp: BlockPos| {
            let b = match bp.y {
                1 => B::Stone,
                2 => B::Water,
                3 => B::Stone,
                _ => B::Air,
            };
            Some(azalea::block::BlockState::from(b))
        };
        assert_eq!(count_overhead_solid(blocks, 0, 0, 0), 1);
    }

    /// P83：64 格全实心（极限深埋）封顶计数 64。
    #[test]
    fn count_overhead_solid_caps_at_64() {
        let all_solid = |bp: BlockPos| {
            if bp.y >= 1 {
                Some(azalea::block::BlockState::from(B::Stone))
            } else {
                None
            }
        };
        assert_eq!(count_overhead_solid(all_solid, 0, 0, 0), 64);
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

    /// P105 回归测试：mine_above P60b 分支的镐检查依赖 is_hard_block 正确判定
    /// 上方 y+2 方块——头顶是空气时入口的 head_is_hard 检查被跳过，石头/深板岩等
    /// 硬方块必须被判为 hard（否则无镐也会继续徒手挖，空转 10s 后误导报错）。
    #[test]
    fn regression_is_hard_block_above_head_requires_pickaxe() {
        use azalea::block::BlockState;
        // P60b 场景：头顶(y+1)是空气、y+2 是石头/深板岩——必须判定为硬块触发镐检查
        assert!(is_hard_block(BlockState::from(B::Stone)));
        assert!(is_hard_block(BlockState::from(B::Deepslate)));
        assert!(is_hard_block(BlockState::from(B::CobbledDeepslate)));
        assert!(is_hard_block(BlockState::from(B::Granite)));
        // 软方块不应误判为硬块（否则有镐检查被无谓触发——但行为无害，仅防逻辑反转）
        assert!(!is_hard_block(BlockState::from(B::Dirt)));
        assert!(!is_hard_block(BlockState::from(B::OakLog)));
        // 空气也不应判定为硬块（P60b 分支 only 在 above_is_solid 时检查）
        assert!(!is_hard_block(BlockState::from(B::Air)));
    }
}
