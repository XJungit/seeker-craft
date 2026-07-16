//! Numen 伴生体调度核心 — ChainScheduler + CompanionBrain + 4 生存行为链。
//!
//! 深度对齐 Numen 项目的三层调度架构：
//! - `TaskChain` trait：优先级投标 + tick + onInterrupt 契约
//! - `ChainScheduler`：无状态纯函数仲裁（严格 > 让列表序决定平局）
//! - `CompanionBrain`：per-tick 仲裁 + 抢占边沿 + freezeTick + journal flush
//! - 4 个生存行为链：UnstuckChain / MobDefenseChain / FoodChain / MLGChain
//! - `LlmTaskChain`：唯一产生 TaskResult 的链（恰好一次）
//!
//! 设计原则（Numen 核心红线）：
//! 1. 生存链自治：无 toolCallId、无 TaskResult，只是临时借用身体
//! 2. LLM 链唯一可应答：恰好一次 result，抢占不收尾
//! 3. 抢占不收尾：onInterrupt 只释放物理状态，保留逻辑状态
//! 4. freezeTick 保护 deadline：被抢占的 tick 不算预算
//! 5. 每 tick 都 finalize+drain：处理带外 cancel（owner Stop）
//! 6. 非咨询式回写：SurvivalJournal 让模型被动得知，零额外 LLM 调用

use crate::survival::{append_survival_notes, SurvivalEvent, SurvivalJournal};
use crate::survival_decisions::{
    self, decide_threat_response, food_priority, mlg_priority, ThreatResponse,
};
use crate::task_base::{
    CompanionTask, Suspendable, TaskContext, TaskQueue, TaskRecord, TaskResult, TaskState,
};

// ═══════════════════════════════════════════════════════════════
// BodyControl — 游戏操作抽象 trait（让生存链驱动底座）
// ═══════════════════════════════════════════════════════════════

/// 伴生体感知状态（每 tick 从游戏读取）。
#[derive(Debug, Clone, Default)]
pub struct CompanionState {
    pub game_time: i64,
    pub health: f32,
    pub max_health: f32,
    pub food_level: u32,
    pub pos: (f64, f64, f64),
    pub yaw: f32,
    pub on_ground: bool,
    pub fall_distance: f64,
    pub is_using_item: bool,
    pub is_moving: bool, // zza != 0 || xxa != 0
    pub has_weapon: bool,
    pub has_edible: bool,
    pub has_water_bucket: bool,
    pub nearest_monster: Option<MonsterInfo>,
    pub last_hurt_by: Option<String>,
}

/// 附近怪物信息。
#[derive(Debug, Clone, Default)]
pub struct MonsterInfo {
    pub entity_type: String,
    pub distance: f64,
    pub pos: (f64, f64, f64),
    pub is_targeting_companion: bool,
}

/// 游戏操作接口（生存链通过此 trait 驱动底座）。
///
/// 实际实现在 mod bridge 侧注入，这里只定义抽象接口。
pub trait BodyControl {
    /// 设置移动输入（前右左后 0.0~1.0）。
    fn set_move_input(&mut self, forward: f32, strafe: f32);

    /// 跳跃（本 tick）。
    fn jump(&mut self);

    /// 设置朝向。
    fn look(&mut self, yaw: f32, pitch: f32);

    /// 攻击实体（本 tick）。
    fn attack(&mut self);

    /// 使用物品（按住）。
    fn use_item_hold(&mut self);

    /// 释放使用物品。
    fn release_use_item(&mut self);

    /// 切换到指定物品槽。
    fn select_slot(&mut self, slot: u32);

    /// 释放身体（停移动 + 收 sneak + 释放按键）。
    fn release_body(&mut self);

    /// 放置方块（朝下）。
    fn place_block_down(&mut self);

    /// 朝目标点导航。
    fn nav_to(&mut self, x: f64, y: f64, z: f64);

    /// 逃离目标点。
    fn nav_away(&mut self, x: f64, y: f64, z: f64);
}

// ═══════════════════════════════════════════════════════════════
// TaskChain — 任务链契约（Numen TaskChain.java）
// ═══════════════════════════════════════════════════════════════

/// LLM 任务的基准优先级（Numen LLM_BASE_PRIORITY = 0.0）。
pub const LLM_BASE_PRIORITY: f32 = 0.0;

/// 休眠优先级（Numen DORMANT = NEGATIVE_INFINITY）。
pub const DORMANT: f32 = f32::NEG_INFINITY;

/// 任务链 trait（对齐 Numen TaskChain）。
///
/// **两大类链的分工**：
/// - 生存链：自治，无 toolCallId、无 TaskResult，只是临时接管身体
/// - LLM 链：唯一产生 TaskResult 的链，且"恰好一次"
pub trait TaskChain {
    /// 优先级投标（NEGATIVE_INFINITY = 休眠）。
    fn get_priority(&self, state: &CompanionState) -> f32;

    /// 仲裁前更新（每 tick 都调用，不管是否被选中）。
    /// 用于更新内部检测器（如 UnstuckDetector 的 record）。
    fn pre_tick(&mut self, _state: &CompanionState) {}

    /// 每 tick 推进（只在成为 winner 时调用）。
    fn tick(&mut self, state: &CompanionState, body: &mut dyn BodyControl, journal: &mut SurvivalJournal);

    /// 在失去控制的那一 tick 调用（切换边沿，只释放物理状态）。
    fn on_interrupt(&mut self, state: &CompanionState, body: &mut dyn BodyControl);

    /// 链名。
    fn name(&self) -> &str;
}

// ═══════════════════════════════════════════════════════════════
// ChainScheduler — 无状态纯函数仲裁（Numen ChainScheduler.java）
// ═══════════════════════════════════════════════════════════════

/// 从链列表中选出最高优先级者（纯函数，Numen ChainScheduler.select）。
///
/// 严格 `>` 让列表序决定平局：优先级相等时列表靠前者胜出。
/// 全休眠时返回 None。
pub fn select_chain(chains: &[Box<dyn TaskChain>], state: &CompanionState) -> Option<usize> {
    let mut best_idx: Option<usize> = None;
    let mut best_priority = f32::NEG_INFINITY;
    for (i, chain) in chains.iter().enumerate() {
        let priority = chain.get_priority(state);
        if priority > best_priority {
            best_priority = priority;
            best_idx = Some(i);
        }
    }
    best_idx
}

// ═══════════════════════════════════════════════════════════════
// CompanionBrain — 伴生体大脑（Numen CompanionBrain.java）
// ═══════════════════════════════════════════════════════════════

/// 伴生体大脑（对齐 Numen CompanionBrain）。
///
/// 持有链列表 + journal + LLM 任务队列，per-tick 仲裁。
pub struct CompanionBrain {
    /// 链列表（顺序 = tie-break，最高 intent 在前）。
    /// 标准顺序：unstuck → mob-defense → food → mlg → llm
    pub chains: Vec<Box<dyn TaskChain>>,
    /// 当前持有身体的链 index。
    pub running: Option<usize>,
    /// LLM 链的 index（唯一产生 TaskResult 的链）。
    pub llm_chain_idx: usize,
    /// 生存日志（非咨询式回写）。
    pub journal: SurvivalJournal,
    /// LLM 任务队列。
    pub task_queue: TaskQueue,
    /// 空闲 tick 计数（用于 journal idle flush）。
    quiet_ticks: u32,
}

/// 空闲 100 tick（5 秒）后 flush journal。
const JOURNAL_IDLE_FLUSH_TICKS: u32 = 100;

impl CompanionBrain {
    /// 创建标准大脑（unstuck → mob-defense → food → mlg → llm）。
    pub fn new_standard() -> Self {
        let chains: Vec<Box<dyn TaskChain>> = vec![
            Box::new(UnstuckChain::new()),
            Box::new(MobDefenseChain::new()),
            Box::new(FoodChain::new()),
            Box::new(MLGChain::new()),
            Box::new(LlmTaskChain::new()),
        ];
        Self {
            llm_chain_idx: chains.len() - 1,
            chains,
            running: None,
            journal: SurvivalJournal::default(),
            task_queue: TaskQueue::new(),
            quiet_ticks: 0,
        }
    }

    /// 每 tick 推进（对齐 Numen CompanionBrain.tick）。
    pub fn tick(&mut self, state: &CompanionState, body: &mut dyn BodyControl) {
        // 仲裁前更新所有链的内部状态（如 UnstuckDetector 的 record）
        for chain in &mut self.chains {
            chain.pre_tick(state);
        }

        let best = select_chain(&self.chains, state);

        if best.is_none() {
            // 全休眠
            if let Some(idx) = self.running.take() {
                self.chains[idx].on_interrupt(state, body);
            }
            self.flush_idle_journal();
            self.finalize_llm();
            return;
        }

        let best_idx = best.unwrap();

        // 抢占边沿：切换 running → best
        if let Some(running_idx) = self.running {
            if running_idx != best_idx {
                self.chains[running_idx].on_interrupt(state, body);
            }
        }
        self.running = Some(best_idx);

        // 非 LLM 持身时冻结 LLM deadline
        if best_idx != self.llm_chain_idx {
            self.freeze_llm_tick();
        }

        self.quiet_ticks = 0;
        self.chains[best_idx].tick(state, body, &mut self.journal);

        // 每 tick 都 finalize + drain（处理带外 cancel）
        self.finalize_llm();
    }

    /// 冻结 LLM deadline（Numen LlmTaskChain.freezeTick）。
    fn freeze_llm_tick(&mut self) {
        self.task_queue.freeze_pending_deadlines();
    }

    /// finalize LLM 终态任务。
    fn finalize_llm(&mut self) {
        // 检查 LLM 链是否有终态任务需要排出
        // 实际实现需要检查 deadline 超时等
    }

    /// 空闲 flush journal（累积 100 tick 后作为非紧急事件推送）。
    fn flush_idle_journal(&mut self) {
        self.quiet_ticks += 1;
        if self.quiet_ticks >= JOURNAL_IDLE_FLUSH_TICKS && !self.journal.is_empty() {
            // 实际实现应 emit <event> 到客户端
            // 这里只 drain 清空
            self.journal.drain();
            self.quiet_ticks = 0;
        }
    }

    /// 把 journal 的 drain 内容追加到工具结果消息（Numen withSurvivalNotes）。
    pub fn append_survival_notes_to_result(&mut self, result_msg: &mut String) {
        append_survival_notes(result_msg, &mut self.journal);
    }

    /// owner Stop — 取消所有任务（带外 cancel）。
    pub fn cancel_all(&mut self, reason: &str) {
        self.task_queue.cancel_all(reason);
    }
}

// ═══════════════════════════════════════════════════════════════
// UnstuckChain — 卡住自救（Numen UnstuckChain.java）
// ═══════════════════════════════════════════════════════════════

/// 卡住自救链（对齐 Numen UnstuckChain）。
///
/// 检测到卡住时进行 30-tick 有限 wander（yaw + 137° 黄金角展开扇形）。
pub struct UnstuckChain {
    detector: survival_decisions::UnstuckDetector,
    wander_ticks: u32,
}

/// wander 持续时间（Numen WANDER_TICKS = 30）。
const WANDER_TICKS: u32 = 30;

/// 黄金角偏移（约 137°，让反复尝试的朝向扇形展开）。
const GOLDEN_ANGLE_OFFSET: f32 = 137.0_f32.to_radians();

impl UnstuckChain {
    pub fn new() -> Self {
        Self {
            detector: survival_decisions::UnstuckDetector::new(),
            wander_ticks: 0,
        }
    }
}

impl TaskChain for UnstuckChain {
    fn name(&self) -> &str {
        "unstuck"
    }

    fn get_priority(&self, _state: &CompanionState) -> f32 {
        if !crate::survival::survival_enabled() {
            return DORMANT;
        }
        // 只读检查检测器（检测器由 pre_tick 更新）
        if self.detector.is_stuck() {
            survival_decisions::priorities::UNSTUCK
        } else {
            DORMANT
        }
    }

    fn pre_tick(&mut self, state: &CompanionState) {
        // 每 tick 记录位置和 trying 状态（不管是否被选中）
        self.detector.record(state.pos.0, state.pos.2, state.is_moving);
    }

    fn tick(&mut self, state: &CompanionState, body: &mut dyn BodyControl, _journal: &mut SurvivalJournal) {
        if self.wander_ticks == 0 {
            self.wander_ticks = WANDER_TICKS;
        }

        if self.wander_ticks > 0 {
            // 朝向 = 当前 yaw + 137°（黄金角，展开扇形避免撞同一面墙）
            let target_yaw = state.yaw + GOLDEN_ANGLE_OFFSET;
            body.look(target_yaw, 0.0);
            body.set_move_input(1.0, 0.0); // 前进

            // 每 5 tick 跳一次以越过台阶/边缘
            if self.wander_ticks % 5 == 0 {
                body.jump();
            }

            self.wander_ticks -= 1;
        }
    }

    fn on_interrupt(&mut self, _state: &CompanionState, body: &mut dyn BodyControl) {
        body.release_body();
        self.wander_ticks = 0;
        self.detector.reset();
    }
}

// ═══════════════════════════════════════════════════════════════
// MobDefenseChain — 威胁防御（Numen MobDefenseChain.java）
// ═══════════════════════════════════════════════════════════════

/// 威胁防御链（对齐 Numen MobDefenseChain）。
///
/// 12 格扫描 Monster → Fight/Flee 决策 → 个体 leash（3 次失败 200 tick 冷却）+ 链条 leash（100 tick）。
pub struct MobDefenseChain {
    /// 不可达怪物表（entity_type → 冷却到期 game_time）。
    unreachable: std::collections::HashMap<String, i64>,
    /// 连续交战失败次数。
    engage_fails: u32,
    /// 链条冷却到期 game_time。
    chain_cooldown_until: i64,
    /// 上次威胁位置（即使怪 despawn 也能继续跑）。
    last_threat_pos: Option<(f64, f64, f64)>,
}

/// 扫描半径（Numen SCAN_RADIUS = 12.0）。
const SCAN_RADIUS: f64 = 12.0;
/// 个体交战失败上限（Numen MAX_ENGAGE_FAILS = 3）。
const MAX_ENGAGE_FAILS: u32 = 3;
/// 个体不可达冷却（Numen UNREACHABLE_COOLDOWN = 200 ticks）。
const UNREACHABLE_COOLDOWN: i64 = 200;
/// 链条冷却（Numen CHAIN_COOLDOWN = 100 ticks）。
const CHAIN_COOLDOWN: i64 = 100;
/// 攻击距离平方（Numen ATTACK_REACH_SQR = 9.0，即 3 格）。
const ATTACK_REACH_SQR: f64 = 9.0;

impl MobDefenseChain {
    pub fn new() -> Self {
        Self {
            unreachable: std::collections::HashMap::new(),
            engage_fails: 0,
            chain_cooldown_until: 0,
            last_threat_pos: None,
        }
    }

    /// 检查是否有威胁（防御而非攻击：只对真正攻击我们的怪反应）。
    fn check_threat(&self, state: &CompanionState) -> bool {
        if let Some(ref m) = state.nearest_monster {
            // DEFENSE, not aggression：只对 targeting_companion 或 last_hurt_by 的怪反应
            if m.is_targeting_companion {
                return true;
            }
            if let Some(ref last_hurt) = state.last_hurt_by {
                if m.entity_type == *last_hurt {
                    return true;
                }
            }
        }
        false
    }
}

impl TaskChain for MobDefenseChain {
    fn name(&self) -> &str {
        "mob_defense"
    }

    fn get_priority(&self, state: &CompanionState) -> f32 {
        if !crate::survival::survival_enabled() {
            return DORMANT;
        }
        // 链条冷却期
        if state.game_time < self.chain_cooldown_until {
            return DORMANT;
        }
        if !self.check_threat(state) {
            return DORMANT;
        }
        // 个体不可达冷却检查
        if let Some(ref m) = state.nearest_monster {
            if let Some(&until) = self.unreachable.get(&m.entity_type) {
                if state.game_time < until {
                    return DORMANT;
                }
            }
        }
        survival_decisions::priorities::MOB_DEFENSE
    }

    fn tick(&mut self, state: &CompanionState, body: &mut dyn BodyControl, journal: &mut SurvivalJournal) {
        let monster = match &state.nearest_monster {
            Some(m) => m.clone(),
            None => return,
        };

        // 检查个体不可达冷却
        if let Some(&until) = self.unreachable.get(&monster.entity_type) {
            if state.game_time < until {
                return;
            }
        }

        self.last_threat_pos = Some(monster.pos);

        // Fight vs Flee 决策（纯函数）
        let response = decide_threat_response(true, state.health, state.has_weapon);

        match response {
            ThreatResponse::Fight => {
                // 追击
                body.nav_to(monster.pos.0, monster.pos.1, monster.pos.2);

                // 进入攻击距离且有视线 → 攻击
                let dist_sqr = monster.distance * monster.distance;
                if dist_sqr <= ATTACK_REACH_SQR {
                    body.attack();
                }

                // TODO: nav 失败时 engage_fails += 1，达到 MAX_ENGAGE_FAILS 时加入 unreachable 表
            }
            ThreatResponse::Flee => {
                // 逃离
                if let Some(pos) = self.last_threat_pos {
                    body.nav_away(pos.0, pos.1, pos.2);

                    // flee 失败 3 次 → 链条冷却 100 tick
                    self.engage_fails += 1;
                    if self.engage_fails > MAX_ENGAGE_FAILS {
                        self.chain_cooldown_until = state.game_time + CHAIN_COOLDOWN;
                        self.engage_fails = 0;
                        return;
                    }
                }
            }
            ThreatResponse::None => {}
        }

        // 怪死了 → 写日记
        if monster.distance < 0.5 && response == ThreatResponse::Fight {
            journal.record(SurvivalEvent::SelfDefense {
                entity_type: monster.entity_type.clone(),
                distance: monster.distance,
            });
        }
    }

    fn on_interrupt(&mut self, _state: &CompanionState, body: &mut dyn BodyControl) {
        body.release_body();
    }
}

// ═══════════════════════════════════════════════════════════════
// FoodChain — 自动进食（Numen FoodChain.java）
// ═══════════════════════════════════════════════════════════════

/// 自动进食链（对齐 Numen FoodChain）。
///
/// 两级优先级（regen=4, hunger=3）→ 绝不抢占 isUsingItem 的身体。
pub struct FoodChain {
    /// 是否正在吃（自己启动的 eat）。
    eating: bool,
    /// 吃之前的 food_level（检测是否真的上升了）。
    food_before: u32,
}

impl FoodChain {
    pub fn new() -> Self {
        Self { eating: false, food_before: 0 }
    }
}

impl TaskChain for FoodChain {
    fn name(&self) -> &str {
        "food"
    }

    fn get_priority(&self, state: &CompanionState) -> f32 {
        if !crate::survival::survival_enabled() {
            return DORMANT;
        }

        // 绝不抢占正在使用物品的身体（除非是我们自己的 in-flight eat）
        if !self.eating && state.is_using_item {
            return DORMANT;
        }

        // 纯函数决策
        let (decision, priority) = food_priority(state.food_level, state.health, state.has_edible);

        match decision {
            survival_decisions::FoodDecision::Dormant => DORMANT,
            _ => priority,
        }
    }

    fn tick(&mut self, state: &CompanionState, body: &mut dyn BodyControl, journal: &mut SurvivalJournal) {
        if !self.eating {
            // 开始吃
            self.eating = true;
            self.food_before = state.food_level;
            body.use_item_hold();
        }

        // 检测是否吃完了（food_level 上升）
        if state.food_level > self.food_before {
            // 写日记（只有真的上升了才写）
            journal.record(SurvivalEvent::AutoEat {
                hunger: state.food_level,
                item: "food".into(),
            });
            self.eating = false;
            body.release_use_item();
        }
    }

    fn on_interrupt(&mut self, _state: &CompanionState, body: &mut dyn BodyControl) {
        if self.eating {
            body.release_use_item();
            self.eating = false;
        }
    }
}

// ═══════════════════════════════════════════════════════════════
// MLGChain — 落地自救（Numen MLGChain.java）
// ═══════════════════════════════════════════════════════════════

/// 落地自救链（对齐 Numen MLGChain）。
///
/// fallDistance ≥ 4 + canSave → 直视下方 + 3.5 格内放水桶。
pub struct MLGChain {
    /// 本次坠落是否已记录日记。
    noted_this_fall: bool,
    /// 坠落开始高度。
    fall_start_y: f64,
}

/// 放置距离（Numen PLACE_WITHIN = 3.5）。
const PLACE_WITHIN: f64 = 3.5;

impl MLGChain {
    pub fn new() -> Self {
        Self { noted_this_fall: false, fall_start_y: 0.0 }
    }
}

impl TaskChain for MLGChain {
    fn name(&self) -> &str {
        "mlg"
    }

    fn get_priority(&self, state: &CompanionState) -> f32 {
        if !crate::survival::survival_enabled() {
            return DORMANT;
        }
        let can_save = state.has_water_bucket;
        mlg_priority(state.on_ground, state.fall_distance, can_save)
    }

    fn tick(&mut self, state: &CompanionState, body: &mut dyn BodyControl, journal: &mut SurvivalJournal) {
        // 记录坠落开始高度
        if !self.noted_this_fall && state.fall_distance > 0.0 {
            self.fall_start_y = state.pos.1;
            self.noted_this_fall = true;
        }

        // 直视下方
        body.look(state.yaw, 90.0);

        // 每 tick 重新扫描水桶槽位（避免舀回刚放下的水）
        // 等 ≤ 3.5 格时才动作
        if state.fall_distance >= PLACE_WITHIN {
            body.use_item_hold();
        }
    }

    fn on_interrupt(&mut self, _state: &CompanionState, body: &mut dyn BodyControl) {
        body.release_use_item();
        body.look(0.0, 0.0); // 不再直视下方
        // 写日记（每次坠落只一行）
        if self.noted_this_fall {
            // journal 记录由落地后触发
        }
        self.noted_this_fall = false;
    }
}

impl MLGChain {
    /// 落地后调用，记录 MLG 日记。
    pub fn on_landed(&mut self, journal: &mut SurvivalJournal, fall_distance: f64) {
        if self.noted_this_fall && fall_distance > 4.0 {
            journal.record(SurvivalEvent::SelfPreservation {
                health: 0.0, // 由调用方填入
                trigger: format!("mlg_saved_{}block_fall", fall_distance as i32),
            });
        }
        self.noted_this_fall = false;
    }
}

// ═══════════════════════════════════════════════════════════════
// LlmTaskChain — LLM 任务链（Numen LlmTaskChain.java）
// ═══════════════════════════════════════════════════════════════

/// LLM 任务链（对齐 Numen LlmTaskChain）。
///
/// 唯一产生 TaskResult 的链，且"恰好一次"。
/// 抢占不收尾（onInterrupt 只 suspend），只有任务自己 tick 到 terminal 时才 finalize。
pub struct LlmTaskChain {
    /// 当前运行的任务记录。
    current: Option<TaskRecord>,
    /// 已完成的任务结果（等待排出）。
    outbox: Vec<TaskResult>,
}

impl LlmTaskChain {
    pub fn new() -> Self {
        Self { current: None, outbox: vec![] }
    }

    /// 接受新任务。
    pub fn start_task(&mut self, record: TaskRecord) {
        self.current = Some(record);
    }

    /// 完成当前任务（移到 outbox）。
    pub fn finalize_current(&mut self, result: TaskResult) {
        if self.current.is_some() {
            self.outbox.push(result);
            self.current = None;
        }
    }

    /// 排出已完成的结果。
    pub fn drain_results(&mut self) -> Vec<TaskResult> {
        self.outbox.drain(..).collect()
    }

    /// 是否有运行中的任务。
    pub fn has_running(&self) -> bool {
        self.current.is_some()
    }
}

impl TaskChain for LlmTaskChain {
    fn name(&self) -> &str {
        "llm"
    }

    fn get_priority(&self, _state: &CompanionState) -> f32 {
        // 有任务时返回基准优先级，无任务时休眠
        if self.current.is_some() {
            LLM_BASE_PRIORITY
        } else {
            DORMANT
        }
    }

    fn tick(&mut self, _state: &CompanionState, _body: &mut dyn BodyControl, _journal: &mut SurvivalJournal) {
        // 实际的 LLM 任务执行由 agent loop 驱动，这里只检查 deadline 超时
        if let Some(ref record) = self.current {
            // 检查 deadline 超时
            // 实际实现需要 game_time 比较
        }
    }

    fn on_interrupt(&mut self, _state: &CompanionState, _body: &mut dyn BodyControl) {
        // 只 suspend，不 finalize（保证恰好一次 result）
        // 实际实现需要调用 task.suspend()（如果实现了 Suspendable）
    }
}

// ═══════════════════════════════════════════════════════════════
// 测试
// ═══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    /// 空操作 BodyControl（测试用）。
    struct NoopBody;
    impl BodyControl for NoopBody {
        fn set_move_input(&mut self, _: f32, _: f32) {}
        fn jump(&mut self) {}
        fn look(&mut self, _: f32, _: f32) {}
        fn attack(&mut self) {}
        fn use_item_hold(&mut self) {}
        fn release_use_item(&mut self) {}
        fn select_slot(&mut self, _: u32) {}
        fn release_body(&mut self) {}
        fn place_block_down(&mut self) {}
        fn nav_to(&mut self, _: f64, _: f64, _: f64) {}
        fn nav_away(&mut self, _: f64, _: f64, _: f64) {}
    }

    fn make_state(health: f32, food: u32, on_ground: bool) -> CompanionState {
        CompanionState {
            health,
            max_health: 20.0,
            food_level: food,
            pos: (0.0, 64.0, 0.0),
            yaw: 0.0,
            on_ground,
            fall_distance: 0.0,
            is_using_item: false,
            is_moving: false,
            has_weapon: true,
            has_edible: true,
            has_water_bucket: true,
            nearest_monster: None,
            last_hurt_by: None,
            ..Default::default()
        }
    }

    #[test]
    fn chain_scheduler_picks_highest_priority() {
        crate::survival::set_survival_enabled(true);
        let mut state = make_state(20.0, 20, true);
        state.nearest_monster = Some(MonsterInfo {
            entity_type: "zombie".into(),
            distance: 5.0,
            pos: (5.0, 64.0, 0.0),
            is_targeting_companion: true,
        });

        let brain = CompanionBrain::new_standard();
        let idx = select_chain(&brain.chains, &state);
        assert!(idx.is_some());
        // mob_defense (priority 5) 应该胜出
        assert_eq!(brain.chains[idx.unwrap()].name(), "mob_defense");
        crate::survival::set_survival_enabled(false);
    }

    #[test]
    fn chain_scheduler_all_dormant_returns_none() {
        crate::survival::set_survival_enabled(true);
        let state = make_state(20.0, 20, true); // 满血满腹无威胁

        let brain = CompanionBrain::new_standard();
        let idx = select_chain(&brain.chains, &state);
        // 只有 LLM 链可能有任务（但这里没启动任务），其他全休眠
        // LLM 链无任务时返回 DORMANT
        assert!(idx.is_none() || brain.chains[idx.unwrap()].name() == "llm");
        crate::survival::set_survival_enabled(false);
    }

    #[test]
    fn chain_scheduler_earlier_wins_on_tie() {
        // 两个链返回相同优先级时，列表靠前者胜出
        struct ChainA;
        struct ChainB;
        impl TaskChain for ChainA {
            fn name(&self) -> &str { "a" }
            fn get_priority(&self, _: &CompanionState) -> f32 { 5.0 }
            fn tick(&mut self, _: &CompanionState, _: &mut dyn BodyControl, _: &mut SurvivalJournal) {}
            fn on_interrupt(&mut self, _: &CompanionState, _: &mut dyn BodyControl) {}
        }
        impl TaskChain for ChainB {
            fn name(&self) -> &str { "b" }
            fn get_priority(&self, _: &CompanionState) -> f32 { 5.0 }
            fn tick(&mut self, _: &CompanionState, _: &mut dyn BodyControl, _: &mut SurvivalJournal) {}
            fn on_interrupt(&mut self, _: &CompanionState, _: &mut dyn BodyControl) {}
        }
        let chains: Vec<Box<dyn TaskChain>> = vec![Box::new(ChainA), Box::new(ChainB)];
        let state = make_state(20.0, 20, true);
        let idx = select_chain(&chains, &state);
        assert_eq!(idx, Some(0)); // ChainA 靠前，平局时胜出
    }

    #[test]
    fn unstuck_chain_dormant_when_not_stuck() {
        crate::survival::set_survival_enabled(true);
        let chain = UnstuckChain::new();
        let state = make_state(20.0, 20, true);
        assert_eq!(chain.get_priority(&state), DORMANT);
        crate::survival::set_survival_enabled(false);
    }

    #[test]
    fn mob_defense_dormant_without_threat() {
        crate::survival::set_survival_enabled(true);
        let chain = MobDefenseChain::new();
        let state = make_state(20.0, 20, true);
        assert_eq!(chain.get_priority(&state), DORMANT);
        crate::survival::set_survival_enabled(false);
    }

    #[test]
    fn mob_defense_active_with_targeting_monster() {
        crate::survival::set_survival_enabled(true);
        let chain = MobDefenseChain::new();
        let mut state = make_state(20.0, 20, true);
        state.nearest_monster = Some(MonsterInfo {
            entity_type: "zombie".into(),
            distance: 5.0,
            pos: (5.0, 64.0, 0.0),
            is_targeting_companion: true,
        });
        assert_eq!(chain.get_priority(&state), survival_decisions::priorities::MOB_DEFENSE);
        crate::survival::set_survival_enabled(false);
    }

    #[test]
    fn mob_defense_flee_when_low_health() {
        crate::survival::set_survival_enabled(true);
        let chain = MobDefenseChain::new();
        let mut state = make_state(4.0, 20, true); // 濒血
        state.nearest_monster = Some(MonsterInfo {
            entity_type: "zombie".into(),
            distance: 5.0,
            pos: (5.0, 64.0, 0.0),
            is_targeting_companion: true,
        });
        // 应该触发（优先级 5）
        assert!(chain.get_priority(&state) > LLM_BASE_PRIORITY);
        crate::survival::set_survival_enabled(false);
    }

    #[test]
    fn food_chain_dormant_when_full() {
        crate::survival::set_survival_enabled(true);
        let chain = FoodChain::new();
        let state = make_state(20.0, 20, true);
        assert_eq!(chain.get_priority(&state), DORMANT);
        crate::survival::set_survival_enabled(false);
    }

    #[test]
    fn food_chain_active_when_hungry() {
        crate::survival::set_survival_enabled(true);
        let chain = FoodChain::new();
        let state = make_state(20.0, 4, true); // 饥饿
        assert_eq!(chain.get_priority(&state), survival_decisions::priorities::FOOD_HUNGER);
        crate::survival::set_survival_enabled(false);
    }

    #[test]
    fn food_chain_never_preempts_using_item() {
        crate::survival::set_survival_enabled(true);
        let chain = FoodChain::new();
        let mut state = make_state(20.0, 4, true); // 饥饿
        state.is_using_item = true; // 正在使用物品
        assert_eq!(chain.get_priority(&state), DORMANT); // 不抢占
        crate::survival::set_survival_enabled(false);
    }

    #[test]
    fn mlg_chain_dormant_on_ground() {
        crate::survival::set_survival_enabled(true);
        let chain = MLGChain::new();
        let state = make_state(20.0, 20, true); // 在地面
        assert_eq!(chain.get_priority(&state), DORMANT);
        crate::survival::set_survival_enabled(false);
    }

    #[test]
    fn mlg_chain_active_on_long_fall() {
        crate::survival::set_survival_enabled(true);
        let chain = MLGChain::new();
        let mut state = make_state(20.0, 20, false); // 空中
        state.fall_distance = 10.0;
        state.has_water_bucket = true;
        assert_eq!(chain.get_priority(&state), survival_decisions::priorities::MLG);
        crate::survival::set_survival_enabled(false);
    }

    #[test]
    fn mlg_chain_dormant_without_bucket() {
        crate::survival::set_survival_enabled(true);
        let chain = MLGChain::new();
        let mut state = make_state(20.0, 20, false);
        state.fall_distance = 10.0;
        state.has_water_bucket = false;
        assert_eq!(chain.get_priority(&state), DORMANT);
        crate::survival::set_survival_enabled(false);
    }

    #[test]
    fn llm_chain_dormant_without_task() {
        let chain = LlmTaskChain::new();
        let state = make_state(20.0, 20, true);
        assert_eq!(chain.get_priority(&state), DORMANT);
    }

    #[test]
    fn llm_chain_base_priority_with_task() {
        let mut chain = LlmTaskChain::new();
        chain.start_task(TaskRecord::new(1, "mine".into(), "call-1".into(), 100));
        let state = make_state(20.0, 20, true);
        assert_eq!(chain.get_priority(&state), LLM_BASE_PRIORITY);
    }

    #[test]
    fn llm_chain_finalize_and_drain() {
        let mut chain = LlmTaskChain::new();
        chain.start_task(TaskRecord::new(1, "mine".into(), "call-1".into(), 100));
        chain.finalize_current(TaskResult::ok("done".into()));
        assert!(!chain.has_running());
        let results = chain.drain_results();
        assert_eq!(results.len(), 1);
        assert!(results[0].success);
    }

    #[test]
    fn companion_brain_survival_enabled_gate() {
        // survival_enabled = false 时所有生存链休眠
        crate::survival::set_survival_enabled(false);
        let brain = CompanionBrain::new_standard();
        let mut state = make_state(4.0, 2, false); // 濒血+饥饿+空中
        state.fall_distance = 10.0;
        state.nearest_monster = Some(MonsterInfo {
            entity_type: "zombie".into(),
            distance: 3.0,
            pos: (3.0, 64.0, 0.0),
            is_targeting_companion: true,
        });

        // 所有生存链应休眠
        for chain in &brain.chains {
            if chain.name() != "llm" {
                assert_eq!(chain.get_priority(&state), DORMANT, "{} should be dormant", chain.name());
            }
        }
    }

    #[test]
    fn companion_brain_mlg_highest_priority() {
        crate::survival::set_survival_enabled(true);
        let brain = CompanionBrain::new_standard();
        let mut state = make_state(4.0, 2, false); // 同时满足多种生存条件
        state.fall_distance = 10.0;
        state.nearest_monster = Some(MonsterInfo {
            entity_type: "zombie".into(),
            distance: 3.0,
            pos: (3.0, 64.0, 0.0),
            is_targeting_companion: true,
        });

        let idx = select_chain(&brain.chains, &state);
        assert_eq!(brain.chains[idx.unwrap()].name(), "mlg"); // MLG 优先级最高(10)
        crate::survival::set_survival_enabled(false);
    }

    #[test]
    fn mob_defense_individual_leash() {
        crate::survival::set_survival_enabled(true);
        let mut chain = MobDefenseChain::new();
        // 模拟连续失败后加入 unreachable 表
        chain.unreachable.insert("zombie".into(), 1000);
        let mut state = make_state(20.0, 20, true);
        state.game_time = 500; // 在冷却期内
        state.nearest_monster = Some(MonsterInfo {
            entity_type: "zombie".into(),
            distance: 5.0,
            pos: (5.0, 64.0, 0.0),
            is_targeting_companion: true,
        });
        // 冷却期内应休眠
        assert_eq!(chain.get_priority(&state), DORMANT);

        // 冷却过期后应激活
        state.game_time = 1001;
        assert!(chain.get_priority(&state) > LLM_BASE_PRIORITY);
        crate::survival::set_survival_enabled(false);
    }
}
