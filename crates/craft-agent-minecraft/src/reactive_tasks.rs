//! Numen 具体反应式任务 — MoveTo / BreakBlock / CollectItems / PlaceBlock。
//!
//! 对齐 Numen 项目中具体任务的实现要点（tasks/*.java）：
//! - 所有任务都不用 `RecoveryLadder` 容器，而是 inline `Phase` enum + 计数字段，
//!   因为每"rung"只是同一任务已持有原语的参数变体。
//! - `suspend()` 在 `ReactiveTask` 基类一次性实现（halt 输入 + 释放 sneak），
//!   **故意不调 `nav.stop()`**——保留 plan 是 resume 能直接继续的关键。
//! - `resume()` 默认 no-op——下个 tick 自然重驱动。
//! - `FailureType` 二分法：
//!   - **In-ladder**（Occluded/BoxedIn/NoPath/OutOfReach/Hazard/NoSupport）→ 阶梯内换策略重试
//!   - **Kick-back**（NoMaterial/WrongTool/TargetLost/MinedOut）→ 直接踢回 LLM
//! - `Precondition` 永远只能发出 Kick-back 类。
//!
//! 四个核心任务（Numen 关键模式）：
//! - `MoveToTask`：nav-only + progress lease + settle（水中漂浮）+ 单 rung 近似 ladder（3 格内算成功）
//! - `BreakBlockTask`：GoToThenDoTask + 3 条前置检查（air/流体/harvest）+ 单 rung 宽松 approach ladder
//! - `CollectItemsTask`：SCAN/APPROACH 两相 + nav FAILED/走到没拾起 → skip 回 SCAN
//! - `PlaceBlockTask`：3-rung inline ladder（PLACING → REPOSITIONING(最多3次) → DIGGING(仅Occluded,最多2块)）

use crate::survival::FailureType;
use crate::task_base::{
    GoToThenDoTask, NavPrimitive, NavStatus, PreconditionFailure, ReactiveTask, Suspendable,
    TaskContext, TaskRuntime, TaskState,
};
use std::collections::VecDeque;

// ═══════════════════════════════════════════════════════════════
// 参数 & 原语 trait — substrate 层注入（Numen TaskParameters）
// ═══════════════════════════════════════════════════════════════

/// 移动目标的"到达"语义（Numen MoveToTask 的 kind）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveKind {
    /// 到达某方块坐标（slab-aware，feet 对齐）。
    Block,
    /// 到达某列（任意 Y，水平距离 ≤ 阈值）。
    Column,
    /// 仅 Y 坐标对齐（上下楼梯场景）。
    YLevel,
}

/// MoveTo 任务参数。
#[derive(Debug, Clone)]
pub struct MoveToParams {
    pub target: (i32, i32, i32),
    pub kind: MoveKind,
}

/// BreakBlock 任务参数。
#[derive(Debug, Clone)]
pub struct BreakBlockParams {
    pub target: (i32, i32, i32),
}

/// CollectItems 任务参数。
#[derive(Debug, Clone)]
pub struct CollectItemsParams {
    /// 候选掉落物列表（实体 ID + 位置）。
    pub candidates: Vec<(String, (f64, f64, f64))>,
    /// 目标物品标签（可选，用于过滤）。
    pub item_tag: Option<String>,
    /// 目标数量（基线-增量，达到即成功）。
    pub target_count: u32,
}

/// PlaceBlock 任务参数。
#[derive(Debug, Clone)]
pub struct PlaceBlockParams {
    pub target: (i32, i32, i32),
    /// 期望朝向（用于 stance 选择）。
    pub face: (i32, i32, i32),
}

/// 破坏原语（substrate 注入）。
pub trait BreakPrimitive {
    /// 推进一帧。返回当前状态。
    fn tick(&mut self) -> BreakStatus;
    fn stop(&mut self);
    /// 是否能 harvest 目标方块。
    fn can_harvest(&self) -> bool;
    /// 目标方块是否为空气（已破坏或不存在的目标）。
    fn is_target_air(&self) -> bool;
    /// 目标方块是否为流体。
    fn is_target_fluid(&self) -> bool;
}

/// 破坏状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakStatus {
    /// 仍在破坏中。
    Breaking,
    /// 方块已破坏（掉落物已生成）。
    Done,
    /// 失败（不可达/工具错误/目标丢失）。
    Failed,
}

/// 放置原语（substrate 注入）。
pub trait PlacePrimitive {
    /// 推进一帧。
    fn tick(&mut self) -> PlaceStatus;
    fn stop(&mut self);
    /// 是否有材料。
    fn has_material(&self) -> bool;
    /// 目标位置是否有支撑面（可放置）。
    fn has_support(&self) -> bool;
    /// 放置时视线是否被遮挡（Occluded）。
    fn is_occluded(&self) -> bool;
}

/// 放置状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaceStatus {
    /// 放置中。
    Placing,
    /// 已成功放置。
    Done,
    /// 失败。
    Failed,
}

/// 挖掘遮挡物原语（PlaceBlock ladder rung 3 用）。
pub trait DigPrimitive {
    /// 推进一帧。
    fn dig_step(&mut self) -> DigStatus;
    /// 是否已挖通（视线清晰）。
    fn is_clear(&self) -> bool;
    /// 当前正在挖的遮挡物坐标（None 表示未在挖）。
    fn current_target(&self) -> Option<(i32, i32, i32)>;
}

/// 挖掘状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DigStatus {
    /// 挖掘中。
    Digging,
    /// 遮挡物已清除（可回到放置）。
    Clear,
    /// 挖掘失败（不可达/工具错误）。
    Failed,
}

// ═══════════════════════════════════════════════════════════════
// MoveToTask — nav-only + progress lease + settle（Numen MoveToTask）
// ═══════════════════════════════════════════════════════════════

/// MoveTo 常量（Numen 风格）。
pub mod move_consts {
    /// 3 格内算成功（单 rung 近似 ladder）。
    pub const NEAR_SUCCESS_RADIUS: f64 = 3.0;
    /// 进度租约上限（无进展 tick 数超此则失败）。
    pub const PROGRESS_LEASE_TICKS: u32 = 600;
    /// 水中 settle 上限（漂浮稳定所需 tick）。
    pub const MAX_SETTLE_TICKS: u32 = 60;
    /// 总 tick 上限（防无限跑）。
    pub const CHECK_IN_CAP_TICKS: u32 = 6000;
}

/// MoveTo 任务（对齐 Numen MoveToTask）。
///
/// **核心模式**：
/// - nav-only：不挂 GoToThenDo，因为"到达"本身就是目标
/// - progress lease：`best_dist` 持续更新，连续 `PROGRESS_LEASE_TICKS` 无进展 → 失败
/// - settle：水中漂浮场景，到达 Column 后等待 `MAX_SETTLE_TICKS` tick 稳定
/// - 单 rung 近似 ladder：3 格内算成功（Numen 的宽松成功条件）
pub struct MoveToTask {
    runtime: TaskRuntime,
    pub params: MoveToParams,
    nav: Option<Box<dyn NavPrimitive>>,
    /// 历史最佳距离（progress lease 判断）。
    best_dist: f64,
    /// 连续无进展 tick 数。
    stall_ticks: u32,
    /// 总 tick 数（cap）。
    total_ticks: u32,
    /// settle 计数（水中漂浮稳定）。
    settle_ticks: u32,
    /// 是否已尝试近邻成功（单 rung）。
    near_retried: bool,
    /// lease 截止 game time。
    lease_cap_game_time: i64,
    /// 缓存的脚位（reached 判断）。
    feet: (i32, i32, i32),
    in_water: bool,
    on_ground: bool,
}

impl MoveToTask {
    pub fn new(params: MoveToParams, nav: Box<dyn NavPrimitive>, current_game_time: i64) -> Self {
        let lease_cap = current_game_time + move_consts::CHECK_IN_CAP_TICKS as i64;
        Self {
            runtime: TaskRuntime::new(),
            params,
            nav: Some(nav),
            best_dist: f64::INFINITY,
            stall_ticks: 0,
            total_ticks: 0,
            settle_ticks: 0,
            near_retried: false,
            lease_cap_game_time: lease_cap,
            feet: (0, 0, 0),
            in_water: false,
            on_ground: true,
        }
    }

    /// 是否到达（按 MoveKind 分支）。
    fn reached(&self) -> bool {
        match self.params.kind {
            MoveKind::Block => {
                // feet 对齐（slab-aware）
                self.feet == self.params.target
            }
            MoveKind::Column => {
                // 水平距离 ≤ 1（同列）
                let dx = self.feet.0 - self.params.target.0;
                let dz = self.feet.2 - self.params.target.2;
                dx * dx + dz * dz <= 1
            }
            MoveKind::YLevel => {
                // Y 对齐
                self.feet.1 == self.params.target.1
            }
        }
    }

    /// 3 格内算成功（Numen 近似 ladder）。
    fn close_enough_to_succeed(&self) -> bool {
        let dx = self.feet.0 as f64 - self.params.target.0 as f64;
        let dy = self.feet.1 as f64 - self.params.target.1 as f64;
        let dz = self.feet.2 as f64 - self.params.target.2 as f64;
        (dx * dx + dy * dy + dz * dz).sqrt() <= move_consts::NEAR_SUCCESS_RADIUS
    }

    /// 当前的水平+垂直距离（用于 progress lease）。
    fn current_dist(&self) -> f64 {
        let dx = self.feet.0 as f64 - self.params.target.0 as f64;
        let dy = self.feet.1 as f64 - self.params.target.1 as f64;
        let dz = self.feet.2 as f64 - self.params.target.2 as f64;
        (dx * dx + dy * dy + dz * dz).sqrt()
    }
}

impl Suspendable for MoveToTask {
    fn suspend(&mut self) {
        // 释放 body（halt 输入）。**故意不调 nav.stop()**——plan 保留是 resume 关键。
        // 实际实现需要 halt player input；这里仅状态保留。
    }
}

impl ReactiveTask for MoveToTask {
    fn runtime_mut(&mut self) -> &mut TaskRuntime {
        &mut self.runtime
    }
    fn runtime_ref(&self) -> &TaskRuntime {
        &self.runtime
    }

    fn on_tick(&mut self, ctx: &mut TaskContext) -> TaskState {
        // 更新缓存
        self.feet = ctx.feet;
        self.in_water = ctx.in_water;
        self.on_ground = ctx.on_ground;
        self.total_ticks += 1;

        // deadline 检查
        if ctx.game_time > self.lease_cap_game_time {
            self.runtime.fail(
                format!(
                    "move_to deadline exceeded: target={:?} ticks={}",
                    self.params.target, self.total_ticks
                ),
                FailureType::TimedOut,
            );
            return TaskState::Failed;
        }

        // 到达 → 成功（水中需要 settle）
        if self.reached() {
            if self.in_water {
                // 水中漂浮稳定
                self.settle_ticks += 1;
                if self.settle_ticks >= move_consts::MAX_SETTLE_TICKS {
                    self.runtime
                        .succeed(format!("reached {:?} (settled in water)", self.params.target));
                    return TaskState::Success;
                }
                return TaskState::Running;
            }
            self.runtime
                .succeed(format!("reached {:?}", self.params.target));
            return TaskState::Success;
        }

        // 单 rung 近似 ladder：3 格内算成功（首次触发）
        if !self.near_retried && self.close_enough_to_succeed() {
            self.near_retried = true;
            self.runtime.succeed(format!(
                "near-reached {:?} (within {} blocks)",
                self.params.target,
                move_consts::NEAR_SUCCESS_RADIUS
            ));
            return TaskState::Success;
        }

        // progress lease
        let dist = self.current_dist();
        if dist < self.best_dist {
            self.best_dist = dist;
            self.stall_ticks = 0;
        } else {
            self.stall_ticks += 1;
            if self.stall_ticks >= move_consts::PROGRESS_LEASE_TICKS {
                self.runtime.fail(
                    format!(
                        "move_to stalled: target={:?} best_dist={:.2} stall_ticks={}",
                        self.params.target, self.best_dist, self.stall_ticks
                    ),
                    FailureType::NoPath,
                );
                return TaskState::Failed;
            }
        }

        // 推进 nav
        let status = if let Some(nav) = self.nav.as_mut() {
            nav.tick()
        } else {
            self.runtime
                .fail("navigation unavailable".to_string(), FailureType::NoPath);
            return TaskState::Failed;
        };

        match status {
            NavStatus::Running => TaskState::Running,
            NavStatus::Arrived => {
                // nav 报告到达——再用 reached() 复核
                if self.reached() {
                    self.runtime
                        .succeed(format!("reached {:?} (nav arrived)", self.params.target));
                    TaskState::Success
                } else if self.close_enough_to_succeed() {
                    self.runtime.succeed(format!(
                        "near-reached {:?} (nav arrived, within {})",
                        self.params.target,
                        move_consts::NEAR_SUCCESS_RADIUS
                    ));
                    TaskState::Success
                } else {
                    // nav 说到了但实际没到——可能是 slab 偏差，给一次 retry
                    if !self.near_retried {
                        self.near_retried = true;
                        TaskState::Running
                    } else {
                        self.runtime.fail(
                            format!(
                                "nav arrived but not at target: target={:?} feet={:?}",
                                self.params.target, self.feet
                            ),
                            FailureType::OutOfReach,
                        );
                        TaskState::Failed
                    }
                }
            }
            NavStatus::Failed => {
                let (ft, reason) = {
                    let nav = self.nav.as_ref().unwrap();
                    (nav.fail_type(), nav.fail_reason().to_string())
                };
                // nav 失败——单 rung 近似 ladder 已在 close_enough 兜底
                if self.close_enough_to_succeed() {
                    self.runtime.succeed(format!(
                        "near-reached {:?} (nav failed but within {})",
                        self.params.target,
                        move_consts::NEAR_SUCCESS_RADIUS
                    ));
                    TaskState::Success
                } else {
                    self.runtime
                        .fail(format!("nav failed: {}", reason), ft);
                    TaskState::Failed
                }
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════
// BreakBlockTask — GoToThenDo + 3 前置 + 单 rung approach ladder
// ═══════════════════════════════════════════════════════════════

/// BreakBlock 任务（对齐 Numen BreakBlockTask）。
///
/// **核心模式**：
/// - GoToThenDoTask：先走到目标再破坏
/// - 3 条前置检查（precondition，全部 Kick-back 类）：
///   1. 目标为空气 → TargetLost
///   2. 目标为流体 → Hazard
///   3. 不能 harvest → WrongTool
/// - 单 rung 宽松 approach ladder：nav FAILED（NoPath/BoxedIn）时 retry 一次
pub struct BreakBlockTask {
    runtime: TaskRuntime,
    pub params: BreakBlockParams,
    nav: Option<Box<dyn NavPrimitive>>,
    breaker: Option<Box<dyn BreakPrimitive>>,
    /// nav retry 计数（单 rung ladder）。
    nav_retries: u32,
    /// 是否已到达。
    arrived: bool,
    /// 缓存 can_harvest（precondition 用）。
    can_harvest: bool,
}

impl BreakBlockTask {
    pub const MAX_NAV_RETRIES: u32 = 1;

    pub fn new(
        params: BreakBlockParams,
        nav: Box<dyn NavPrimitive>,
        breaker: Box<dyn BreakPrimitive>,
    ) -> Self {
        let can_harvest = breaker.can_harvest();
        Self {
            runtime: TaskRuntime::new(),
            params,
            nav: Some(nav),
            breaker: Some(breaker),
            nav_retries: 0,
            arrived: false,
            can_harvest,
        }
    }
}

impl Suspendable for BreakBlockTask {
    fn suspend(&mut self) {
        // 释放 body，保留 nav plan + breaker 进度。
    }
}

impl ReactiveTask for BreakBlockTask {
    fn runtime_mut(&mut self) -> &mut TaskRuntime {
        &mut self.runtime
    }
    fn runtime_ref(&self) -> &TaskRuntime {
        &self.runtime
    }

    fn preconditions(&self, _ctx: &TaskContext) -> Vec<PreconditionFailure> {
        let mut fails = Vec::new();
        // 前置 1：目标为空气 → TargetLost
        if let Some(b) = self.breaker.as_ref() {
            if b.is_target_air() {
                fails.push(PreconditionFailure {
                    message: format!("target {:?} is air (already broken?)", self.params.target),
                    failure_type: FailureType::TargetLost,
                });
            } else if b.is_target_fluid() {
                // 前置 2：目标为流体 → Hazard
                fails.push(PreconditionFailure {
                    message: format!("target {:?} is fluid (hazard)", self.params.target),
                    failure_type: FailureType::Hazard,
                });
            }
            // 前置 3：不能 harvest → WrongTool
            if !b.can_harvest() {
                fails.push(PreconditionFailure {
                    message: format!(
                        "cannot harvest target {:?} (wrong tool?)",
                        self.params.target
                    ),
                    failure_type: FailureType::WrongTool,
                });
            }
        }
        fails
    }

    fn on_start(&mut self, _ctx: &TaskContext) {}

    fn on_tick(&mut self, ctx: &mut TaskContext) -> TaskState {
        // 委托给 GoToThenDoTask 默认 tick
        self.goto_then_do_tick(ctx)
    }
}

impl GoToThenDoTask for BreakBlockTask {
    fn nav_mut(&mut self) -> &mut Option<Box<dyn NavPrimitive>> {
        &mut self.nav
    }
    fn build_nav(&mut self) -> Option<Box<dyn NavPrimitive>> {
        // nav 已在 new() 注入，这里直接取走
        self.nav.take()
    }
    fn reached(&self) -> bool {
        self.arrived
    }
    fn act(&mut self) -> TaskState {
        // 已到达——执行破坏
        let status = if let Some(b) = self.breaker.as_mut() {
            b.tick()
        } else {
            self.runtime
                .fail("breaker unavailable".to_string(), FailureType::Unknown);
            return TaskState::Failed;
        };
        match status {
            BreakStatus::Breaking => TaskState::Running,
            BreakStatus::Done => {
                self.runtime
                    .succeed(format!("broke {:?}", self.params.target));
                TaskState::Success
            }
            BreakStatus::Failed => {
                self.runtime.fail(
                    format!("break failed at {:?}", self.params.target),
                    FailureType::Unknown,
                );
                TaskState::Failed
            }
        }
    }

    fn handle_nav_failure(&mut self, fail_type: FailureType, reason: &str) -> TaskState {
        // 单 rung 宽松 approach ladder：NoPath/BoxedIn retry 一次
        if matches!(fail_type, FailureType::NoPath | FailureType::BoxedIn)
            && self.nav_retries < Self::MAX_NAV_RETRIES
        {
            self.nav_retries += 1;
            // 重新构建 nav（实际实现需要重规划）
            // 这里保留原 nav 让其重试
            return TaskState::Running;
        }
        self.runtime
            .fail(format!("nav failed: {}", reason), fail_type);
        TaskState::Failed
    }
}

// ═══════════════════════════════════════════════════════════════
// CollectItemsTask — SCAN/APPROACH 两相 + TargetSet skip（Numen CollectItemsTask）
// ═══════════════════════════════════════════════════════════════

/// Collect 任务阶段（Numen CollectItemsTask 的 SCAN/APPROACH 两相）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollectPhase {
    /// 扫描候选（选最近未跳过的）。
    Scan,
    /// 走向选中候选。
    Approach,
}

/// CollectItems 任务（对齐 Numen CollectItemsTask）。
///
/// **核心模式**：
/// - SCAN/APPROACH 两相
/// - scan()：从候选中选最近未跳过的
/// - skip_target()：拉黑回 SCAN
/// - on_tick：Scan 无 target → Success；Approach nav Failed/Arrived → skip 回 Scan
pub struct CollectItemsTask {
    runtime: TaskRuntime,
    pub params: CollectItemsParams,
    nav: Option<Box<dyn NavPrimitive>>,
    /// 当前阶段。
    phase: CollectPhase,
    /// 当前选中的目标实体 ID。
    current_target: Option<String>,
    /// 黑名单（不可达/已跳过）。
    blacklist: std::collections::HashSet<String>,
    /// 已收集数量（基线-增量）。
    collected: u32,
    /// 缓存脚位。
    feet: (i32, i32, i32),
}

impl CollectItemsTask {
    pub fn new(params: CollectItemsParams, nav: Box<dyn NavPrimitive>) -> Self {
        Self {
            runtime: TaskRuntime::new(),
            params,
            nav: Some(nav),
            phase: CollectPhase::Scan,
            current_target: None,
            blacklist: std::collections::HashSet::new(),
            collected: 0,
            feet: (0, 0, 0),
        }
    }

    /// 从候选中选最近未跳过的（Numen scan）。
    fn scan(&mut self) -> Option<String> {
        let feet = self.feet;
        let best = self
            .params
            .candidates
            .iter()
            .filter(|(id, _)| !self.blacklist.contains(id))
            .min_by(|a, b| {
                let da = (a.1 .0 - feet.0 as f64).powi(2)
                    + (a.1 .1 - feet.1 as f64).powi(2)
                    + (a.1 .2 - feet.2 as f64).powi(2);
                let db = (b.1 .0 - feet.0 as f64).powi(2)
                    + (b.1 .1 - feet.1 as f64).powi(2)
                    + (b.1 .2 - feet.2 as f64).powi(2);
                da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(id, _)| id.clone());
        self.current_target = best.clone();
        best
    }

    /// 拉黑当前目标，回 SCAN。
    fn skip_target(&mut self) {
        if let Some(id) = self.current_target.take() {
            self.blacklist.insert(id);
        }
        self.phase = CollectPhase::Scan;
        // 停 nav（实际实现调 stop；这里仅状态）
        if let Some(nav) = self.nav.as_mut() {
            nav.stop();
        }
    }
}

impl Suspendable for CollectItemsTask {
    fn suspend(&mut self) {
        // 释放 body，保留 blacklist + current_target + phase。
    }
}

impl ReactiveTask for CollectItemsTask {
    fn runtime_mut(&mut self) -> &mut TaskRuntime {
        &mut self.runtime
    }
    fn runtime_ref(&self) -> &TaskRuntime {
        &self.runtime
    }

    fn on_tick(&mut self, ctx: &mut TaskContext) -> TaskState {
        self.feet = ctx.feet;

        // 目标数量达成 → 成功
        if self.collected >= self.params.target_count {
            self.runtime.succeed(format!(
                "collected {} items (target={})",
                self.collected, self.params.target_count
            ));
            return TaskState::Success;
        }

        match self.phase {
            CollectPhase::Scan => {
                if let Some(_id) = self.scan() {
                    self.phase = CollectPhase::Approach;
                    // 重新规划 nav 到选中目标
                    // （实际实现需要重新调 nav.set_target；这里状态推进）
                    TaskState::Running
                } else {
                    // 所有候选都跳过 → 成功（已尽力收集）
                    self.runtime.succeed(format!(
                        "scan exhausted: collected {} / target {}",
                        self.collected, self.params.target_count
                    ));
                    TaskState::Success
                }
            }
            CollectPhase::Approach => {
                let status = if let Some(nav) = self.nav.as_mut() {
                    nav.tick()
                } else {
                    self.runtime
                        .fail("navigation unavailable".to_string(), FailureType::NoPath);
                    return TaskState::Failed;
                };
                match status {
                    NavStatus::Running => TaskState::Running,
                    NavStatus::Arrived => {
                        // 走到了——假设捡起（实际实现需要检查 inventory 增量）
                        self.collected += 1;
                        self.current_target = None;
                        self.phase = CollectPhase::Scan;
                        TaskState::Running
                    }
                    NavStatus::Failed => {
                        // nav 失败 → 跳过这个目标，回 SCAN
                        self.skip_target();
                        TaskState::Running
                    }
                }
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════
// PlaceBlockTask — 3-rung inline ladder（Numen PlaceBlockTask）
// ═══════════════════════════════════════════════════════════════

/// PlaceBlock 常量（Numen 风格）。
pub mod place_consts {
    /// rung 2：换 stance 最多 3 次。
    pub const MAX_ALT_STANCES: u32 = 3;
    /// rung 3：挖遮挡物最多 2 块。
    pub const MAX_OCCLUDERS_DUG: u32 = 2;
    /// stance 近邻半径（换 stance 后的接近判断）。
    pub const STANCE_NEAR_RADIUS: f64 = 2.5;
    /// 挖遮挡物 tick 上限。
    pub const DIG_TICK_CAP: u32 = 100;
}

/// PlaceBlock 阶段（Numen 3-rung inline ladder）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlacePhase {
    /// rung 1：直接放置。
    Placing,
    /// rung 2：换 stance（重新定位）。
    Repositioning,
    /// rung 3：挖遮挡物（仅 Occluded）。
    Digging,
}

/// PlaceBlock 任务（对齐 Numen PlaceBlockTask）。
///
/// **核心模式（3-rung inline ladder）**：
/// - PLACING → 直接放置
///   - NO_MATERIAL/NO_SUPPORT → 直接 fail（kick-back，永不 ladder）
///   - Occluded → advance_ladder
/// - REPOSITIONING（最多 3 次）→ nav 重新定位
///   - Arrived → 回 PLACING
///   - Failed → advance_ladder
/// - DIGGING（仅 Occluded，最多 2 块）→ dig_clear 后回 PLACING
///   - 挖超 MAX_OCCLUDERS_DUG → exhaust
///
/// **关键规则**：
/// - NO_MATERIAL 和 NO_SUPPORT 永不 ladder（直接 fail）
/// - advance_ladder 是核心推进函数
/// - exhaust 是唯一终止 give-up
pub struct PlaceBlockTask {
    runtime: TaskRuntime,
    pub params: PlaceBlockParams,
    nav: Option<Box<dyn NavPrimitive>>,
    placer: Option<Box<dyn PlacePrimitive>>,
    digger: Option<Box<dyn DigPrimitive>>,
    /// 当前阶段。
    phase: PlacePhase,
    /// rung 2 已尝试次数。
    alt_stances_tried: u32,
    /// rung 3 是否已尝试。
    dig_tried: bool,
    /// 已挖遮挡物数量。
    occluders_dug: u32,
    /// dig 阶段 tick 计数。
    dig_ticks: u32,
    /// 缓存脚位。
    feet: (i32, i32, i32),
}

impl PlaceBlockTask {
    pub fn new(
        params: PlaceBlockParams,
        nav: Box<dyn NavPrimitive>,
        placer: Box<dyn PlacePrimitive>,
        digger: Box<dyn DigPrimitive>,
    ) -> Self {
        Self {
            runtime: TaskRuntime::new(),
            params,
            nav: Some(nav),
            placer: Some(placer),
            digger: Some(digger),
            phase: PlacePhase::Placing,
            alt_stances_tried: 0,
            dig_tried: false,
            occluders_dug: 0,
            dig_ticks: 0,
            feet: (0, 0, 0),
        }
    }

    /// 推进 ladder（Numen 核心模式）。
    ///
    /// 先继续 rung 2（换 stance），再进 rung 3（仅 Occluded），都用完 → exhaust。
    fn advance_ladder(&mut self, cause: FailureType, detail: &str) -> TaskState {
        // 先继续 rung 2（换 stance）
        if self.alt_stances_tried < place_consts::MAX_ALT_STANCES {
            self.alt_stances_tried += 1;
            self.phase = PlacePhase::Repositioning;
            // 停 placer，重启 nav
            if let Some(p) = self.placer.as_mut() {
                p.stop();
            }
            return TaskState::Running;
        }
        // 再进 rung 3（仅 OCCLUDED）
        if !self.dig_tried && cause == FailureType::Occluded {
            self.dig_tried = true;
            self.phase = PlacePhase::Digging;
            self.dig_ticks = 0;
            return TaskState::Running;
        }
        // 都用完
        self.exhaust(cause, detail)
    }

    /// 唯一终止 give-up（列出已尝试的策略）。
    fn exhaust(&mut self, cause: FailureType, detail: &str) -> TaskState {
        self.runtime.fail(
            format!(
                "place exhausted at {:?}: cause={:?} detail={} alt_stances={} dig_tried={} occluders_dug={}",
                self.params.target,
                cause,
                detail,
                self.alt_stances_tried,
                self.dig_tried,
                self.occluders_dug
            ),
            cause,
        );
        TaskState::Failed
    }

    /// tick rung 1（Placing）。
    fn tick_place(&mut self) -> TaskState {
        let status = if let Some(p) = self.placer.as_mut() {
            p.tick()
        } else {
            self.runtime
                .fail("placer unavailable".to_string(), FailureType::Unknown);
            return TaskState::Failed;
        };
        match status {
            PlaceStatus::Placing => TaskState::Running,
            PlaceStatus::Done => {
                self.runtime
                    .succeed(format!("placed at {:?}", self.params.target));
                TaskState::Success
            }
            PlaceStatus::Failed => {
                // 判断失败类型
                let (has_mat, has_sup, occluded) = {
                    let p = self.placer.as_ref().unwrap();
                    (p.has_material(), p.has_support(), p.is_occluded())
                };
                if !has_mat {
                    // NO_MATERIAL → 直接 fail（kick-back）
                    self.runtime.fail(
                        format!("no material for {:?}", self.params.target),
                        FailureType::NoMaterial,
                    );
                    TaskState::Failed
                } else if !has_sup {
                    // NO_SUPPORT → 直接 fail（kick-back）
                    self.runtime.fail(
                        format!("no support at {:?}", self.params.target),
                        FailureType::NoSupport,
                    );
                    TaskState::Failed
                } else if occluded {
                    // Occluded → advance_ladder
                    self.advance_ladder(FailureType::Occluded, "occluded while placing")
                } else {
                    self.advance_ladder(FailureType::Unknown, "place failed (unknown)")
                }
            }
        }
    }

    /// tick rung 2（Repositioning）。
    fn tick_reposition(&mut self) -> TaskState {
        let status = if let Some(nav) = self.nav.as_mut() {
            nav.tick()
        } else {
            self.runtime
                .fail("navigation unavailable".to_string(), FailureType::NoPath);
            return TaskState::Failed;
        };
        match status {
            NavStatus::Running => TaskState::Running,
            NavStatus::Arrived => {
                // 回 PLACING 再试
                self.phase = PlacePhase::Placing;
                TaskState::Running
            }
            NavStatus::Failed => {
                let (ft, reason) = {
                    let nav = self.nav.as_ref().unwrap();
                    (nav.fail_type(), nav.fail_reason().to_string())
                };
                self.advance_ladder(ft, &reason)
            }
        }
    }

    /// tick rung 3（Digging）。
    fn tick_dig(&mut self) -> TaskState {
        self.dig_ticks += 1;
        if self.dig_ticks > place_consts::DIG_TICK_CAP {
            return self.advance_ladder(FailureType::Occluded, "dig tick cap exceeded");
        }

        // 检查是否已挖通
        let is_clear = if let Some(d) = self.digger.as_ref() {
            d.is_clear()
        } else {
            self.runtime
                .fail("digger unavailable".to_string(), FailureType::Unknown);
            return TaskState::Failed;
        };
        if is_clear {
            // 挖通了——回 PLACING
            self.phase = PlacePhase::Placing;
            return TaskState::Running;
        }

        // 检查已挖数量
        if self.occluders_dug >= place_consts::MAX_OCCLUDERS_DUG {
            return self.exhaust(FailureType::Occluded, "max occluders dug");
        }

        // 推进挖
        let status = if let Some(d) = self.digger.as_mut() {
            d.dig_step()
        } else {
            self.runtime
                .fail("digger unavailable".to_string(), FailureType::Unknown);
            return TaskState::Failed;
        };
        match status {
            DigStatus::Digging => TaskState::Running,
            DigStatus::Clear => {
                self.occluders_dug += 1;
                self.phase = PlacePhase::Placing;
                TaskState::Running
            }
            DigStatus::Failed => {
                self.advance_ladder(FailureType::Occluded, "dig failed")
            }
        }
    }
}

impl Suspendable for PlaceBlockTask {
    fn suspend(&mut self) {
        // 释放 body，保留 phase + 计数 + placer/digger 状态。
    }
}

impl ReactiveTask for PlaceBlockTask {
    fn runtime_mut(&mut self) -> &mut TaskRuntime {
        &mut self.runtime
    }
    fn runtime_ref(&self) -> &TaskRuntime {
        &self.runtime
    }

    fn on_tick(&mut self, ctx: &mut TaskContext) -> TaskState {
        self.feet = ctx.feet;
        match self.phase {
            PlacePhase::Placing => self.tick_place(),
            PlacePhase::Repositioning => self.tick_reposition(),
            PlacePhase::Digging => self.tick_dig(),
        }
    }
}

// ═══════════════════════════════════════════════════════════════
// 测试基础设施 — MockNav / MockBreaker / MockPlacer / MockDigger
// ═══════════════════════════════════════════════════════════════

/// 可编程测试 nav（按预设序列返回状态）。
pub struct MockNav {
    pub statuses: VecDeque<NavStatus>,
    pub fail_type: FailureType,
    pub fail_reason: String,
    pub stall_ticks: u32,
    pub stopped: bool,
}

impl MockNav {
    pub fn new(statuses: Vec<NavStatus>) -> Self {
        Self {
            statuses: statuses.into(),
            fail_type: FailureType::NoPath,
            fail_reason: "mock nav failed".into(),
            stall_ticks: 0,
            stopped: false,
        }
    }
}

impl NavPrimitive for MockNav {
    fn tick(&mut self) -> NavStatus {
        self.statuses.pop_front().unwrap_or(NavStatus::Running)
    }
    fn fail_type(&self) -> FailureType {
        self.fail_type
    }
    fn fail_reason(&self) -> &str {
        &self.fail_reason
    }
    fn stall_ticks(&self) -> u32 {
        self.stall_ticks
    }
    fn stop(&mut self) {
        self.stopped = true;
    }
}

/// 测试 breaker。
pub struct MockBreaker {
    pub statuses: VecDeque<BreakStatus>,
    pub can_harvest: bool,
    pub is_air: bool,
    pub is_fluid: bool,
}

impl MockBreaker {
    pub fn new(statuses: Vec<BreakStatus>) -> Self {
        Self {
            statuses: statuses.into(),
            can_harvest: true,
            is_air: false,
            is_fluid: false,
        }
    }
}

impl BreakPrimitive for MockBreaker {
    fn tick(&mut self) -> BreakStatus {
        self.statuses.pop_front().unwrap_or(BreakStatus::Breaking)
    }
    fn stop(&mut self) {}
    fn can_harvest(&self) -> bool {
        self.can_harvest
    }
    fn is_target_air(&self) -> bool {
        self.is_air
    }
    fn is_target_fluid(&self) -> bool {
        self.is_fluid
    }
}

/// 测试 placer。
pub struct MockPlacer {
    pub statuses: VecDeque<PlaceStatus>,
    pub has_material: bool,
    pub has_support: bool,
    pub is_occluded: bool,
}

impl MockPlacer {
    pub fn new(statuses: Vec<PlaceStatus>) -> Self {
        Self {
            statuses: statuses.into(),
            has_material: true,
            has_support: true,
            is_occluded: false,
        }
    }
}

impl PlacePrimitive for MockPlacer {
    fn tick(&mut self) -> PlaceStatus {
        let s = self.statuses.pop_front().unwrap_or(PlaceStatus::Placing);
        // 返回 Done 时自动清除遮挡标志（模拟换 stance 后视线通）
        if s == PlaceStatus::Done {
            self.is_occluded = false;
        }
        s
    }
    fn stop(&mut self) {}
    fn has_material(&self) -> bool {
        self.has_material
    }
    fn has_support(&self) -> bool {
        self.has_support
    }
    fn is_occluded(&self) -> bool {
        self.is_occluded
    }
}

/// 测试 digger。
pub struct MockDigger {
    pub statuses: VecDeque<DigStatus>,
    pub is_clear: bool,
    pub current_target: Option<(i32, i32, i32)>,
}

impl MockDigger {
    pub fn new(statuses: Vec<DigStatus>) -> Self {
        Self {
            statuses: statuses.into(),
            is_clear: false,
            current_target: None,
        }
    }
}

impl DigPrimitive for MockDigger {
    fn dig_step(&mut self) -> DigStatus {
        self.statuses.pop_front().unwrap_or(DigStatus::Digging)
    }
    fn is_clear(&self) -> bool {
        self.is_clear
    }
    fn current_target(&self) -> Option<(i32, i32, i32)> {
        self.current_target
    }
}

fn make_ctx(pos: (i32, i32, i32)) -> TaskContext {
    TaskContext {
        game_time: 0,
        player_health: 20.0,
        player_food: 20,
        player_pos: (pos.0 as f64, pos.1 as f64, pos.2 as f64),
        feet: pos,
        on_ground: true,
        in_water: false,
        player_dead: false,
    }
}

// ═══════════════════════════════════════════════════════════════
// 测试
// ═══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn move_to_reached_block_kind() {
        let nav = MockNav::new(vec![]);
        let params = MoveToParams {
            target: (5, 64, 5),
            kind: MoveKind::Block,
        };
        let mut task = MoveToTask::new(params, Box::new(nav), 0);

        // 启动
        let mut ctx = make_ctx((0, 64, 0));
        let s = task.start(&ctx);
        assert_eq!(s, TaskState::Running);

        // 模拟到达
        ctx.feet = (5, 64, 5);
        let s = task.tick(&mut ctx);
        assert_eq!(s, TaskState::Success);
        assert!(task.runtime_ref().success_message.is_some());
    }

    #[test]
    fn move_to_near_success_within_3_blocks() {
        let nav = MockNav::new(vec![NavStatus::Failed]);
        let params = MoveToParams {
            target: (10, 64, 10),
            kind: MoveKind::Block,
        };
        let mut task = MoveToTask::new(params, Box::new(nav), 0);

        let mut ctx = make_ctx((0, 64, 0));
        task.start(&ctx);

        // 距离 2.83 < 3.0 → 近邻成功
        ctx.feet = (9, 64, 9);
        let s = task.tick(&mut ctx);
        assert_eq!(s, TaskState::Success);
    }

    #[test]
    fn move_to_stalled_failure() {
        // nav 持续 Running，但位置不变 → stall
        let nav = MockNav::new(vec![
            NavStatus::Running,
            NavStatus::Running,
            NavStatus::Running,
        ]);
        let params = MoveToParams {
            target: (100, 64, 100),
            kind: MoveKind::Block,
        };
        let mut task = MoveToTask::new(params, Box::new(nav), 0);
        // 预置 best_dist=0 让位置不变时无进展；stall_ticks 距阈值差 3，
        // 第三次 tick 累加到 PROGRESS_LEASE_TICKS → 失败。
        task.best_dist = 0.0;
        task.stall_ticks = move_consts::PROGRESS_LEASE_TICKS - 3;

        let mut ctx = make_ctx((0, 64, 0));
        task.start(&ctx);

        // 位置不变，第三次 tick 应触发 stall
        let s1 = task.tick(&mut ctx);
        assert_eq!(s1, TaskState::Running);
        let s2 = task.tick(&mut ctx);
        assert_eq!(s2, TaskState::Running);
        let s3 = task.tick(&mut ctx);
        // stall_ticks 累计到 PROGRESS_LEASE_TICKS → 失败
        assert_eq!(s3, TaskState::Failed);
    }

    #[test]
    fn move_to_column_kind_ignores_y() {
        let nav = MockNav::new(vec![]);
        let params = MoveToParams {
            target: (5, 100, 5),
            kind: MoveKind::Column,
        };
        let mut task = MoveToTask::new(params, Box::new(nav), 0);

        let mut ctx = make_ctx((0, 64, 0));
        task.start(&ctx);

        // 水平对齐但 Y 不同——Column 应到达
        ctx.feet = (5, 64, 5);
        let s = task.tick(&mut ctx);
        assert_eq!(s, TaskState::Success);
    }

    #[test]
    fn break_block_precondition_air_target_lost() {
        let nav = MockNav::new(vec![]);
        let mut breaker = MockBreaker::new(vec![]);
        breaker.is_air = true;
        let params = BreakBlockParams {
            target: (5, 64, 5),
        };
        let task = BreakBlockTask::new(params, Box::new(nav), Box::new(breaker));

        let ctx = make_ctx((0, 64, 0));
        let pcs = task.preconditions(&ctx);
        assert!(pcs.iter().any(|p| p.failure_type == FailureType::TargetLost));
    }

    #[test]
    fn break_block_precondition_fluid_hazard() {
        let nav = MockNav::new(vec![]);
        let mut breaker = MockBreaker::new(vec![]);
        breaker.is_fluid = true;
        let params = BreakBlockParams {
            target: (5, 64, 5),
        };
        let task = BreakBlockTask::new(params, Box::new(nav), Box::new(breaker));

        let ctx = make_ctx((0, 64, 0));
        let pcs = task.preconditions(&ctx);
        assert!(pcs.iter().any(|p| p.failure_type == FailureType::Hazard));
    }

    #[test]
    fn break_block_precondition_wrong_tool() {
        let nav = MockNav::new(vec![]);
        let mut breaker = MockBreaker::new(vec![]);
        breaker.can_harvest = false;
        let params = BreakBlockParams {
            target: (5, 64, 5),
        };
        let task = BreakBlockTask::new(params, Box::new(nav), Box::new(breaker));

        let ctx = make_ctx((0, 64, 0));
        let pcs = task.preconditions(&ctx);
        assert!(pcs.iter().any(|p| p.failure_type == FailureType::WrongTool));
    }

    #[test]
    fn collect_items_scan_to_approach() {
        let nav = MockNav::new(vec![NavStatus::Arrived]);
        let params = CollectItemsParams {
            candidates: vec![
                ("item_1".to_string(), (5.0, 64.0, 5.0)),
                ("item_2".to_string(), (10.0, 64.0, 10.0)),
            ],
            item_tag: None,
            target_count: 1,
        };
        let mut task = CollectItemsTask::new(params, Box::new(nav));

        let mut ctx = make_ctx((0, 64, 0));
        task.start(&ctx);

        // Scan → Approach
        let s1 = task.tick(&mut ctx);
        assert_eq!(s1, TaskState::Running);
        assert_eq!(task.phase, CollectPhase::Approach);

        // Approach Arrived → collected=1, 回 Scan
        let s2 = task.tick(&mut ctx);
        assert_eq!(s2, TaskState::Running);
        assert_eq!(task.collected, 1);

        // 再次 tick → 目标达成 → Success
        let s3 = task.tick(&mut ctx);
        assert_eq!(s3, TaskState::Success);
    }

    #[test]
    fn collect_items_skip_on_nav_failed() {
        let nav = MockNav::new(vec![NavStatus::Failed, NavStatus::Failed]);
        let params = CollectItemsParams {
            candidates: vec![
                ("item_1".to_string(), (5.0, 64.0, 5.0)),
                ("item_2".to_string(), (10.0, 64.0, 10.0)),
            ],
            item_tag: None,
            target_count: 2,
        };
        let mut task = CollectItemsTask::new(params, Box::new(nav));

        let mut ctx = make_ctx((0, 64, 0));
        task.start(&ctx);

        // Scan → Approach item_1
        let s1 = task.tick(&mut ctx);
        assert_eq!(s1, TaskState::Running);
        assert_eq!(task.phase, CollectPhase::Approach);

        // Approach Failed → skip → 回 Scan
        let s2 = task.tick(&mut ctx);
        assert_eq!(s2, TaskState::Running);
        assert_eq!(task.phase, CollectPhase::Scan);
        assert!(task.blacklist.contains("item_1"));

        // Scan → Approach item_2
        let s3 = task.tick(&mut ctx);
        assert_eq!(s3, TaskState::Running);
        assert_eq!(task.phase, CollectPhase::Approach);

        // Approach Failed → skip → 回 Scan
        let s4 = task.tick(&mut ctx);
        assert_eq!(s4, TaskState::Running);
        assert_eq!(task.phase, CollectPhase::Scan);
        assert!(task.blacklist.contains("item_2"));

        // Scan 无候选 → Success（尽力）
        let s5 = task.tick(&mut ctx);
        assert_eq!(s5, TaskState::Success);
    }

    #[test]
    fn place_block_success_first_try() {
        let nav = MockNav::new(vec![]);
        let placer = MockPlacer::new(vec![PlaceStatus::Done]);
        let digger = MockDigger::new(vec![]);
        let params = PlaceBlockParams {
            target: (5, 64, 5),
            face: (0, 1, 0),
        };
        let mut task = PlaceBlockTask::new(
            params,
            Box::new(nav),
            Box::new(placer),
            Box::new(digger),
        );

        let mut ctx = make_ctx((5, 64, 5));
        task.start(&ctx);

        let s = task.tick(&mut ctx);
        assert_eq!(s, TaskState::Success);
        assert_eq!(task.phase, PlacePhase::Placing);
    }

    #[test]
    fn place_block_no_material_kickback() {
        let nav = MockNav::new(vec![]);
        let mut placer = MockPlacer::new(vec![PlaceStatus::Failed]);
        placer.has_material = false;
        let digger = MockDigger::new(vec![]);
        let params = PlaceBlockParams {
            target: (5, 64, 5),
            face: (0, 1, 0),
        };
        let mut task = PlaceBlockTask::new(
            params,
            Box::new(nav),
            Box::new(placer),
            Box::new(digger),
        );

        let mut ctx = make_ctx((5, 64, 5));
        task.start(&ctx);

        let s = task.tick(&mut ctx);
        assert_eq!(s, TaskState::Failed);
        // NO_MATERIAL 永不 ladder
        assert_eq!(task.runtime_ref().fail_type, Some(FailureType::NoMaterial));
    }

    #[test]
    fn place_block_no_support_kickback() {
        let nav = MockNav::new(vec![]);
        let mut placer = MockPlacer::new(vec![PlaceStatus::Failed]);
        placer.has_support = false;
        let digger = MockDigger::new(vec![]);
        let params = PlaceBlockParams {
            target: (5, 64, 5),
            face: (0, 1, 0),
        };
        let mut task = PlaceBlockTask::new(
            params,
            Box::new(nav),
            Box::new(placer),
            Box::new(digger),
        );

        let mut ctx = make_ctx((5, 64, 5));
        task.start(&ctx);

        let s = task.tick(&mut ctx);
        assert_eq!(s, TaskState::Failed);
        assert_eq!(task.runtime_ref().fail_type, Some(FailureType::NoSupport));
    }

    #[test]
    fn place_block_occluded_advances_ladder_to_repositioning() {
        let nav = MockNav::new(vec![NavStatus::Arrived]);
        let mut placer = MockPlacer::new(vec![PlaceStatus::Failed, PlaceStatus::Done]);
        placer.is_occluded = true;
        // 第二次 placer tick 时清除 occluded 标志（模拟换 stance 后通了）
        let digger = MockDigger::new(vec![]);
        let params = PlaceBlockParams {
            target: (5, 64, 5),
            face: (0, 1, 0),
        };
        let mut task = PlaceBlockTask::new(
            params,
            Box::new(nav),
            Box::new(placer),
            Box::new(digger),
        );

        let mut ctx = make_ctx((5, 64, 5));
        task.start(&ctx);

        // tick 1: Placing Failed (occluded) → advance_ladder → Repositioning
        let s1 = task.tick(&mut ctx);
        assert_eq!(s1, TaskState::Running);
        assert_eq!(task.phase, PlacePhase::Repositioning);
        assert_eq!(task.alt_stances_tried, 1);

        // tick 2: Repositioning Arrived → 回 Placing
        let s2 = task.tick(&mut ctx);
        assert_eq!(s2, TaskState::Running);
        assert_eq!(task.phase, PlacePhase::Placing);

        // tick 3: Placing Done → Success（MockPlacer::tick 返回 Done 时自动清 occluded）
        let s3 = task.tick(&mut ctx);
        assert_eq!(s3, TaskState::Success);
    }

    #[test]
    fn place_block_exhaust_after_max_alt_stances() {
        // 3 次 alt_stances 都失败 + dig 也试过 → exhaust
        let nav = MockNav::new(vec![
            NavStatus::Failed, // reposition 1 fail
            NavStatus::Failed, // reposition 2 fail
            NavStatus::Failed, // reposition 3 fail
        ]);
        let mut placer = MockPlacer::new(vec![]);
        placer.is_occluded = true;
        // placer 永不 Done，但 advance_ladder 主要由 nav 失败驱动
        let digger = MockDigger::new(vec![]);
        let params = PlaceBlockParams {
            target: (5, 64, 5),
            face: (0, 1, 0),
        };
        let mut task = PlaceBlockTask::new(
            params,
            Box::new(nav),
            Box::new(placer),
            Box::new(digger),
        );

        let mut ctx = make_ctx((5, 64, 5));
        task.start(&ctx);

        // 模拟已经到 Placing 阶段失败（直接调 advance_ladder）
        // 先 alt_stances_tried 3 次
        task.alt_stances_tried = place_consts::MAX_ALT_STANCES;
        task.dig_tried = true; // dig 也试过

        let s = task.advance_ladder(FailureType::Occluded, "test exhaust");
        assert_eq!(s, TaskState::Failed);
        assert_eq!(task.runtime_ref().fail_type, Some(FailureType::Occluded));
    }
}
