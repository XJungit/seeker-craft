//! ActionManager 层（学习自 Mindcraft action_manager.js 的设计）。
//!
//! 取代原硬编码 60-tick 超时，提供：
//! 1. **单槽串行执行**：保留原 pending 槽语义，一次只跑一条命令
//! 2. **抢占中断**：高优先级命令（self_preservation）可中断当前 pending
//! 3. **按命令类型超时**：Goto 60 tick / Mine 100 / Craft 200 / Smelt 600 / Gather 800 ...
//!    避免长任务（合成/采集/熔炼）被 3s 超时误杀
//! 4. **快循环检测**：记录最近 10 条命令签名，相同签名 3+ 次注入打断提示
//!
//! 与 agent 层 `recent_calls`（LLM tool_call 级别）互补：
//! - agent 层检测 LLM 重复调用同一 tool_call
//! - ActionManager 检测底层 BotCommand 重复（即使 LLM 用不同 tool 仍可能落到同 BotCommand）

use super::{BotCommand, QueuedCommand};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// 命令优先级。高优先级可抢占低优先级 pending。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Priority {
    /// 普通命令（LLM 工具调用产生）。FIFO 排队。
    #[default]
    Normal,
    /// 高优先级（self_preservation / self_defense 等 mode 产生）。
    /// 若 pending 是 Normal，抢占之；若 pending 也是 High，则排队等。
    High,
}

/// 快循环检测阈值：最近 10 条命令中相同签名出现 N 次即视为循环。
const LOOP_DETECT_THRESHOLD: usize = 3;
const RECENT_CMD_CAP: usize = 10;

/// ActionManager：封装 pending 槽 + 超时 + 抢占 + 循环检测。
///
/// 设计要点：
/// - 所有字段 Arc<Mutex> 共享，handler 与外部 API 均可访问
/// - 不持有 bot 引用，只做状态管理；执行仍由 handler 内 bot 完成
/// - 提交命令时立即检查循环，超阈值返回 Nudge 提示给调用方
#[derive(Clone)]
pub struct ActionManager {
    /// 当前正在执行的单条命令（串行槽）。
    pub pending: Arc<Mutex<Option<QueuedCommand>>>,
    /// pending 命令开始的 tick（ticks_connected）。
    pub pending_since: Arc<Mutex<Option<u64>>>,
    /// 异步命令 await 期间的中途锁（防止重入）。
    pub busy: Arc<Mutex<bool>>,
    /// 最近命令签名（归一化后），用于快循环检测。
    recent_cmds: Arc<Mutex<VecDeque<String>>>,
    /// 最近一次循环警告文本（供 handler 取走注入 LLM）。
    pub loop_nudge: Arc<Mutex<Option<String>>>,
}

impl Default for ActionManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ActionManager {
    pub fn new() -> Self {
        Self {
            pending: Arc::new(Mutex::new(None)),
            pending_since: Arc::new(Mutex::new(None)),
            busy: Arc::new(Mutex::new(false)),
            recent_cmds: Arc::new(Mutex::new(VecDeque::with_capacity(RECENT_CMD_CAP))),
            loop_nudge: Arc::new(Mutex::new(None)),
        }
    }

    /// 提交一条命令。返回值传达额外事件（如循环警告）。
    ///
    /// - 若 priority=High 且当前 pending=Normal：抢占当前 pending（清空），
    ///   把新命令设为 pending，返回 `SubmitOutcome::Preempted(old_cmd)`。
    /// - 若 pending 为空：直接设为 pending。
    /// - 若 pending 占用且不能抢占：push 到 cmd_queue（调用方负责）。
    ///
    /// 同时记录命令签名，超阈值时把循环警告写入 `loop_nudge`。
    pub fn submit(
        &self,
        cmd: BotCommand,
        priority: Priority,
        cmd_queue: &Arc<Mutex<Vec<QueuedCommand>>>,
        current_tick: u64,
    ) -> SubmitOutcome {
        // 1. 记录签名 + 循环检测
        let sig = cmd_signature(&cmd);
        let loop_warn = {
            let mut recent = self.recent_cmds.lock().unwrap();
            recent.push_back(sig.clone());
            if recent.len() > RECENT_CMD_CAP {
                recent.pop_front();
            }
            let count = recent.iter().filter(|c| **c == sig).count();
            if count >= LOOP_DETECT_THRESHOLD {
                Some(format!(
                    "【动作循环警告】底层命令 {} 在最近 {} 次中重复 {} 次。\
                     可能 bot 卡在同一动作（如反复 goto 同坐标/反复 mine 同方块）。\
                     请：1) perceive 确认当前实际状态 2) 换一种完全不同的策略 3) 若目标已达成则停止。",
                    sig, RECENT_CMD_CAP, count
                ))
            } else {
                None
            }
        };
        if let Some(warn) = loop_warn {
            *self.loop_nudge.lock().unwrap() = Some(warn.clone());
            return SubmitOutcome::LoopDetected(warn);
        }

        // 2. 抢占检查
        let qc = QueuedCommand {
            cmd: cmd.clone(),
            result_tx: None,
        };
        let mut pending = self.pending.lock().unwrap();
        if let Some(old) = pending.take() {
            // pending 占用
            if priority == Priority::High {
                // 抢占：旧命令回退到队列头部（高优先级先于其它 Normal）
                let mut q = cmd_queue.lock().unwrap();
                q.insert(0, old.clone());
                *pending = Some(qc);
                *self.pending_since.lock().unwrap() = Some(current_tick);
                *self.busy.lock().unwrap() = false; // 重置 busy 让新命令能进入
                SubmitOutcome::Preempted(old.cmd)
            } else {
                // Normal 命令排队
                let mut q = cmd_queue.lock().unwrap();
                q.push(qc);
                *pending = Some(old);
                SubmitOutcome::Queued
            }
        } else {
            // pending 空，直接占槽
            *pending = Some(qc);
            *self.pending_since.lock().unwrap() = Some(current_tick);
            SubmitOutcome::Running
        }
    }

    /// 检查当前 pending 是否超时。超时则清空 pending + busy，返回 true。
    ///
    /// 超时阈值按命令类型不同（见 `timeout_ticks`）。
    pub fn check_timeout(&self, current_tick: u64) -> Option<BotCommand> {
        let pending = self.pending.lock().unwrap();
        let qc = pending.as_ref()?;
        let since = self.pending_since.lock().unwrap().unwrap_or(current_tick);
        let elapsed = current_tick.saturating_sub(since);
        let limit = timeout_ticks(&qc.cmd);
        if elapsed > limit {
            let cmd = qc.cmd.clone();
            drop(pending);
            self.clear_pending();
            return Some(cmd);
        }
        None
    }

    /// 清空 pending 槽 + busy 标志（命令完成或超时调用）。
    pub fn clear_pending(&self) {
        *self.pending.lock().unwrap() = None;
        *self.pending_since.lock().unwrap() = None;
        *self.busy.lock().unwrap() = false;
    }

    /// 取走并清空 loop_nudge（handler 每 tick 调用，有则注入 LLM）。
    pub fn take_loop_nudge(&self) -> Option<String> {
        self.loop_nudge.lock().unwrap().take()
    }

    /// 当前 pending 是否空闲（可推进下一条）。
    pub fn is_idle(&self) -> bool {
        self.pending.lock().unwrap().is_none()
    }

    /// 是否处于 busy（异步命令执行中，不可重入）。
    pub fn is_busy(&self) -> bool {
        *self.busy.lock().unwrap()
    }

    /// 标记 busy（异步命令开始执行）。
    pub fn set_busy(&self, v: bool) {
        *self.busy.lock().unwrap() = v;
    }

    /// 占用 pending 槽（从队列 pop 出来时调用）。
    pub fn occupy(&self, qc: QueuedCommand, current_tick: u64) {
        *self.pending.lock().unwrap() = Some(qc);
        *self.pending_since.lock().unwrap() = Some(current_tick);
    }

    /// 从 pending 槽取出当前命令的克隆（不改状态）。
    pub fn peek_pending(&self) -> Option<QueuedCommand> {
        self.pending.lock().unwrap().as_ref().cloned()
    }
}

/// submit 返回的事件。
#[derive(Debug)]
pub enum SubmitOutcome {
    /// 命令直接占槽开始执行
    Running,
    /// 命令入队等待
    Queued,
    /// 高优先级抢占当前 pending，旧的被回退到队列头部
    Preempted(BotCommand),
    /// 检测到动作循环，拒绝提交（调用方应改换策略）
    LoopDetected(String),
}

/// 按命令类型返回超时阈值（tick，20 tick = 1 秒）。
///
/// 学习自 Mindcraft action_manager 的 per-action timeout 设计：
/// 短动作（chat/attack）快速释放，长动作（auto_craft/smelt/gather）给足时间。
/// 避免一刀切 60 tick 把合成/采集误杀。
pub fn timeout_ticks(cmd: &BotCommand) -> u64 {
    match cmd {
        // 即时命令
        BotCommand::Chat { .. } => 20,          // 1s
        BotCommand::Attack { .. } => 60,        // 3s
        BotCommand::BlockInteract { .. } => 60, // 3s
        BotCommand::TillAndSow { .. } => 200,   // 10s（犁地+播种+两次验证）
        BotCommand::Sleep => 600,               // 30s（找床+走过去+入睡+睡到醒）
        BotCommand::Harvest => 600,             // 30s（走到作物+挖掘+等待拾取，最多 24 棵）
        // 寻路/挖掘
        BotCommand::Goto { .. } => 30, // 1.5s（长距离由 32m 限制拦截；无路径时快速失败）
        // P110：锚点导航解析后转 Goto，沿用 Goto 超时。
        BotCommand::GotoAnchor { .. } => 30,
        // P110b：probe 侧 memory 操作即时完成。
        BotCommand::Memory { .. } => 20,
        BotCommand::Mine { .. } => 200, // 10s（深板岩/黑曜石等硬方块可能慢；wooden_pickaxe 挖 deepslate ~4.5s）
        BotCommand::MineBelow => 200,   // 10s（持续下挖，由 Y≤-61 停止）
        BotCommand::MineAbove => 600, // 30s（持续上挖，由头顶空气/Y≥320 停止；P120 徒手挖硬方块 ~8s/格，10s 不够挖穿+爬升）
        // 合成
        BotCommand::Craft2x2 { .. } => 200, // 10s
        BotCommand::Craft3x3 { .. } => 500, // 25s（含放桌+开桌+合成+收桌，P1-4）
        BotCommand::Smelt { .. } => 2400,   // 120s（含放炉+开炉+熔炼+收炉；熔炼 10 个铁锭需 ~100s）
        // 采集（多轮渐扩半径，最慢；24 轮 × 10s/轮 = 240s 理论上限，给 120s 余量）
        BotCommand::Gather { .. } => 2400,       // 120s
        BotCommand::MakeObsidian { .. } => 1600, // 80s（含多次放水+等待+挖取）
        // 放置/开容器
        BotCommand::Place { .. } => 100,         // 5s
        BotCommand::OpenContainer { .. } => 100, // 5s
        // 高层自动合成（采集→合成→放置链）
        BotCommand::AutoCraft { .. } => 1000, // 50s
        // 附魔/交易
        BotCommand::Enchant { .. } => 400,        // 20s
        BotCommand::Trade { .. } => 200,          // 10s
        BotCommand::InteractEntity { .. } => 100, // 5s
        // 智能技能
        BotCommand::Pickup => 200, // 10s
        BotCommand::Defend => 300, // 15s
        // 背包管理
        BotCommand::Equip { .. } => 100, // 5s（shift_click + 选槽）
        BotCommand::Discard { .. } => 200, // 10s（可能多次 ThrowClick）
        BotCommand::Consume { .. } => 600, // 30s（吃饭 1.6s + 余量）
        // 容器交互（开→操作→关，含 shift_click 多次）
        BotCommand::ChestView { .. } => 200,     // 10s
        BotCommand::ChestWithdraw { .. } => 400, // 20s
        BotCommand::ChestDeposit { .. } => 400,  // 20s
        // P68：跟随/停止跟随是持续模式（handler 每 tick 自行推进），不需要长超时；
        // give 基于 discard，给 20s 余量。
        BotCommand::Follow { .. } => 20,
        // P111：按玩家名单次导航——先解析玩家坐标再走 Goto，给 30s（同 GotoAnchor）。
        BotCommand::GotoPlayer { .. } => 30,
        // P112：搜块返回坐标是读扫描，20s 足够。
        BotCommand::SearchBlock { .. } => 20,
        // P113：远离=解析实体 + goto 反向，给 30s（同 GotoAnchor）。
        BotCommand::MoveAway { .. } => 30,
        BotCommand::StopFollow => 20,
        // P116：模式开关即时生效。
        BotCommand::SetMode { .. } => 20,
        // P118：右键使用/投掷物品即时生效。
        BotCommand::UseItem { .. } => 20,
        // P119：拉弓射箭（装备+瞄准+拉弦 1s+放箭）。
        BotCommand::Shoot { .. } => 60,
        BotCommand::Give { .. } => 400,
        // P88：raw dump 是调试通道，即时完成。
        BotCommand::RawState => 20,
    }
}

/// 命令签名：归一化数字到 #，便于检测"goto 同坐标"循环。
///
/// 例：`Goto(10,64,10)` → `goto(#,#,#)`，无论具体坐标，所有 goto 都是同一签名。
/// 这样能检测"反复 goto 不同坐标但都是 goto"的死循环（bot 卡住只会反复 goto）。
///
/// 但若想检测"goto 同一坐标"，需保留坐标——这里选保守策略：
/// 动作名 + 关键参数（item/kind）归一化，坐标归一化为 #。
pub fn cmd_signature(cmd: &BotCommand) -> String {
    match cmd {
        BotCommand::Goto { .. } => "goto(#,#,#)".to_string(),
        // P110：锚点导航签名含锚点名（不同锚点不算重复循环）。
        BotCommand::GotoAnchor { name } => format!("goto_anchor({name})"),
        // P110b：memory 操作签名含动作名。
        BotCommand::Memory { action, .. } => format!("memory({action})"),
        BotCommand::Mine { .. } => "mine(#,#,#)".to_string(),
        BotCommand::MineBelow => "mine_below".to_string(),
        BotCommand::MineAbove => "mine_above".to_string(),
        BotCommand::BlockInteract { .. } => "block_interact(#,#,#)".to_string(),
        BotCommand::TillAndSow { seed, .. } => format!("till_and_sow(#,#,#,{seed})"),
        BotCommand::Sleep => "sleep".to_string(),
        BotCommand::Harvest => "harvest".to_string(),
        BotCommand::Chat { content } => {
            format!("chat({})", content.chars().take(20).collect::<String>())
        }
        BotCommand::Attack { .. } => "attack".to_string(),
        BotCommand::Craft2x2 { item, count } => format!("craft_2x2({item},{count})"),
        BotCommand::Craft3x3 { item, count, .. } => format!("craft_3x3({item},{count})"),
        BotCommand::Smelt {
            output,
            fuel,
            count,
            ..
        } => format!("smelt({output},{fuel},{count})"),
        BotCommand::Gather { item, count } => format!("gather({item},{count})"),
        BotCommand::MakeObsidian { count } => format!("make_obsidian({count})"),
        BotCommand::Place { item, .. } => format!("place({item},#,#,#)"),
        BotCommand::OpenContainer { .. } => "open_container(#,#,#)".to_string(),
        BotCommand::AutoCraft { item, count } => format!("auto_craft({item},{count})"),
        BotCommand::Enchant { item, level } => format!("enchant({item},{level})"),
        BotCommand::Trade { offer } => format!("trade({offer})"),
        BotCommand::InteractEntity { kind } => format!("interact_entity({kind})"),
        BotCommand::Pickup => "pickup".to_string(),
        BotCommand::Defend => "defend".to_string(),
        BotCommand::Equip { item, slot } => format!("equip({item},{slot})"),
        BotCommand::Discard { item, count } => format!("discard({item},{count})"),
        BotCommand::Consume { item } => format!("consume({item})"),
        BotCommand::ChestView { .. } => "chest_view(#,#,#)".to_string(),
        BotCommand::ChestWithdraw { item, count, .. } => format!("chest_withdraw({item},{count})"),
        BotCommand::ChestDeposit { item, count, .. } => format!("chest_deposit({item},{count})"),
        BotCommand::Follow { .. } => "follow".to_string(),
        // P111：签名含玩家名（不同玩家不算重复导航循环）。
        BotCommand::GotoPlayer { name } => match name {
            Some(n) => format!("goto_player({n})"),
            None => "goto_player(any)".to_string(),
        },
        // P112：签名含方块名（不同方块不算重复搜索）。
        BotCommand::SearchBlock { item, .. } => format!("search_block({item})"),
        // P113：签名含目标名（不同目标不算重复远离）。
        BotCommand::MoveAway { target, .. } => match target {
            Some(t) => format!("move_away({t})"),
            None => "move_away(any)".to_string(),
        },
        BotCommand::StopFollow => "stop_follow".to_string(),
        // P116：签名含模式名与开关（不同模式/开关不算重复操作）。
        BotCommand::SetMode { mode, enabled } => {
            format!("set_mode({mode},{})", if *enabled { "on" } else { "off" })
        }
        // P118：签名含物品（不同物品不算重复使用）。
        BotCommand::UseItem { item, yaw, pitch } => format!("use_item({item},{yaw:?},{pitch:?})"),
        // P119：签名含目标（不同目标不算重复射击）。
        BotCommand::Shoot { target } => format!("shoot({target:?})"),
        BotCommand::Give { item, count, .. } => format!("give({item},{count})"),
        BotCommand::RawState => "raw_state".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_timeout_short_for_chat() {
        assert_eq!(
            timeout_ticks(&BotCommand::Chat {
                content: "hi".into()
            }),
            20
        );
    }

    #[test]
    fn test_timeout_long_for_gather() {
        assert_eq!(
            timeout_ticks(&BotCommand::Gather {
                item: "oak_log".into(),
                count: 8
            }),
            2400
        );
    }

    #[test]
    fn test_timeout_long_for_autocraft() {
        assert_eq!(
            timeout_ticks(&BotCommand::AutoCraft {
                item: "chest".into(),
                count: 1
            }),
            1000
        );
    }

    #[test]
    fn test_signature_normalizes_coords() {
        let a = cmd_signature(&BotCommand::Goto { x: 1, y: 2, z: 3 });
        let b = cmd_signature(&BotCommand::Goto {
            x: 100,
            y: 64,
            z: -50,
        });
        assert_eq!(a, b, "不同坐标应归一化为同一签名");
    }

    #[test]
    fn test_signature_preserves_item() {
        let a = cmd_signature(&BotCommand::Gather {
            item: "oak_log".into(),
            count: 4,
        });
        let b = cmd_signature(&BotCommand::Gather {
            item: "stone".into(),
            count: 4,
        });
        assert_ne!(a, b, "不同 item 应有不同签名");
    }

    /// P110：锚点导航签名含锚点名；不同锚点不算重复循环。
    #[test]
    fn test_signature_goto_anchor_keeps_name() {
        let home = cmd_signature(&BotCommand::GotoAnchor {
            name: "home".into(),
        });
        let portal = cmd_signature(&BotCommand::GotoAnchor {
            name: "nether_portal".into(),
        });
        assert_ne!(home, portal, "不同锚点应有不同签名");
        assert!(home.contains("home"), "签名应含锚点名");
    }

    /// P110：锚点导航超时与 Goto 一致。
    #[test]
    fn test_timeout_goto_anchor_matches_goto() {
        assert_eq!(
            timeout_ticks(&BotCommand::GotoAnchor {
                name: "home".into()
            }),
            timeout_ticks(&BotCommand::Goto { x: 0, y: 64, z: 0 })
        );
    }

    #[test]
    fn test_submit_runs_when_idle() {
        let am = ActionManager::new();
        let q = Arc::new(Mutex::new(Vec::new()));
        let outcome = am.submit(
            BotCommand::Chat {
                content: "hi".into(),
            },
            Priority::Normal,
            &q,
            0,
        );
        assert!(matches!(outcome, SubmitOutcome::Running));
        assert!(am.peek_pending().is_some());
    }

    #[test]
    fn test_submit_queues_when_busy() {
        let am = ActionManager::new();
        let q = Arc::new(Mutex::new(Vec::new()));
        am.submit(
            BotCommand::Chat {
                content: "first".into(),
            },
            Priority::Normal,
            &q,
            0,
        );
        let outcome = am.submit(
            BotCommand::Chat {
                content: "second".into(),
            },
            Priority::Normal,
            &q,
            0,
        );
        assert!(matches!(outcome, SubmitOutcome::Queued));
        assert_eq!(q.lock().unwrap().len(), 1);
    }

    #[test]
    fn test_high_priority_preempts_normal() {
        let am = ActionManager::new();
        let q = Arc::new(Mutex::new(Vec::new()));
        am.submit(
            BotCommand::Chat {
                content: "normal".into(),
            },
            Priority::Normal,
            &q,
            0,
        );
        let outcome = am.submit(
            BotCommand::Goto { x: 5, y: 64, z: 5 },
            Priority::High,
            &q,
            10,
        );
        assert!(matches!(outcome, SubmitOutcome::Preempted(_)));
        // 旧命令应回退到队列头部
        assert_eq!(q.lock().unwrap().len(), 1);
        // pending 应是新命令
        let pending = am.peek_pending().unwrap();
        assert!(matches!(pending.cmd, BotCommand::Goto { .. }));
    }

    #[test]
    fn test_loop_detection_after_3_repeats() {
        let am = ActionManager::new();
        let q = Arc::new(Mutex::new(Vec::new()));
        // 前两次正常提交（占用 pending，后续都因 pending 占用而 Queued）
        am.submit(
            BotCommand::Goto { x: 1, y: 2, z: 3 },
            Priority::Normal,
            &q,
            0,
        );
        am.submit(
            BotCommand::Goto { x: 4, y: 5, z: 6 },
            Priority::Normal,
            &q,
            0,
        );
        // 第三次相同签名（goto 归一化）应触发循环检测
        let outcome = am.submit(
            BotCommand::Goto { x: 7, y: 8, z: 9 },
            Priority::Normal,
            &q,
            0,
        );
        assert!(
            matches!(outcome, SubmitOutcome::LoopDetected(_)),
            "第三次相同签名应触发循环检测"
        );
    }

    #[test]
    fn test_check_timeout_uses_per_command_threshold() {
        let am = ActionManager::new();
        let q = Arc::new(Mutex::new(Vec::new()));
        am.submit(
            BotCommand::Chat {
                content: "hi".into(),
            },
            Priority::Normal,
            &q,
            0,
        );
        // Chat 超时 20 tick，21 tick 应超时
        assert!(am.check_timeout(21).is_some());
        // 超时后 pending 应清空
        assert!(am.is_idle());
    }

    #[test]
    fn test_check_timeout_not_triggered_for_long_command() {
        let am = ActionManager::new();
        let q = Arc::new(Mutex::new(Vec::new()));
        am.submit(
            BotCommand::Smelt {
                output: "iron_ingot".into(),
                fuel: "coal".into(),
                count: 8,
                table_pos: None,
            },
            Priority::Normal,
            &q,
            0,
        );
        // Smelt 超时 800 tick，100 tick 不应超时
        assert!(am.check_timeout(100).is_none());
    }

    #[test]
    fn test_timeout_follow_give_defined() {
        // P68：Follow/StopFollow/Give 必须有超时阈值（不应 panic / 缺臂）。
        assert_eq!(timeout_ticks(&BotCommand::Follow { target: None }), 20);
        assert_eq!(timeout_ticks(&BotCommand::StopFollow), 20);
        assert_eq!(
            timeout_ticks(&BotCommand::Give {
                item: "dirt".into(),
                count: 0,
                target: None
            }),
            400
        );
    }

    #[test]
    fn test_signature_follow_give() {
        // P68：新增命令的签名应唯一且不 panic。
        assert_eq!(
            cmd_signature(&BotCommand::Follow {
                target: Some("steve".into())
            }),
            "follow"
        );
        assert_eq!(cmd_signature(&BotCommand::StopFollow), "stop_follow");
        assert_eq!(
            cmd_signature(&BotCommand::Give {
                item: "cooked_beef".into(),
                count: 3,
                target: None
            }),
            "give(cooked_beef,3)"
        );
    }
}
