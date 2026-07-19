//! Numen 生存决策核心 — 纯函数 + 阈值常量（对齐 SurvivalDecisions.java + UnstuckDetector.java）。
//!
//! 设计原则（Numen "pure function split for headless test"）：
//! 所有可纯化的决策逻辑都从 Minecraft 依赖中抽出来，用合成数据流单测。
//! 零 MC 依赖，零 I/O，纯算术。

// ═══════════════════════════════════════════════════════════════
// SurvivalDecisions — 纯决策函数 + 阈值常量（Numen SurvivalDecisions.java）
// ═══════════════════════════════════════════════════════════════

/// 生存链优先级常量（Numen 风格，越大越优先）。
pub mod priorities {
    /// MLG 落地自救（最致命的死亡，最高优先级）
    pub const MLG: f32 = 10.0;
    /// 怪物防御
    pub const MOB_DEFENSE: f32 = 5.0;
    /// 进食回血（health≤12 且 food<18）
    pub const FOOD_REGEN: f32 = 4.0;
    /// 进食充饥（food≤6）
    pub const FOOD_HUNGER: f32 = 3.0;
    /// 卡住自救
    pub const UNSTUCK: f32 = 2.0;
    /// LLM 基线（无生存链激活时的默认优先级）
    pub const LLM_BASE: f32 = 0.0;
    /// 休眠（链不激活）
    pub const DORMANT: f32 = f32::NEG_INFINITY;
}

/// 生存决策阈值常量。
pub mod thresholds {
    /// 进食回血触发：health ≤ 此值 且 food < 18
    pub const LOW_HEALTH: f32 = 12.0;
    /// 撤退触发：health ≤ 此值
    pub const FLEE_HEALTH: f32 = 8.0;
    /// 进食充饥触发：food ≤ 此值
    pub const HUNGRY_LEVEL: u32 = 6;
    /// 进食回血触发：food < 此值
    pub const REGEN_FOOD_LEVEL: u32 = 18;
    /// MLG 落地自救触发：fallDistance > 此值
    pub const MLG_FALL_TRIGGER: f64 = 4.0;
}

/// 威胁响应决策（Numen decideThreatResponse）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreatResponse {
    /// 战斗（health > FLEE_HEALTH 且有武器）
    Fight,
    /// 逃跑（health ≤ FLEE_HEALTH 或无武器）
    Flee,
    /// 无威胁
    None,
}

/// 进食决策（Numen foodPriority）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FoodDecision {
    /// 回血进食（health ≤ 12 且 food < 18，优先级 4）
    Regen,
    /// 充饥进食（food ≤ 6，优先级 3）
    Hunger,
    /// 不需要进食
    Dormant,
}

/// 决定威胁响应（纯函数，Numen SurvivalDecisions.decideThreatResponse）。
///
/// - `threat_present`: 附近是否有敌对怪物
/// - `health`: 当前血量
/// - `armed`: 是否有武器
pub fn decide_threat_response(threat_present: bool, health: f32, armed: bool) -> ThreatResponse {
    if !threat_present {
        return ThreatResponse::None;
    }
    if health <= thresholds::FLEE_HEALTH || !armed {
        ThreatResponse::Flee
    } else {
        ThreatResponse::Fight
    }
}

/// 决定进食优先级（纯函数，Numen SurvivalDecisions.foodPriority）。
///
/// - `food_level`: 0-20 饥饿值
/// - `health`: 当前血量
/// - `has_edible`: 背包是否有食物
pub fn food_priority(food_level: u32, health: f32, has_edible: bool) -> (FoodDecision, f32) {
    if !has_edible {
        return (FoodDecision::Dormant, priorities::DORMANT);
    }
    if health <= thresholds::LOW_HEALTH && food_level < thresholds::REGEN_FOOD_LEVEL {
        return (FoodDecision::Regen, priorities::FOOD_REGEN);
    }
    if food_level <= thresholds::HUNGRY_LEVEL {
        return (FoodDecision::Hunger, priorities::FOOD_HUNGER);
    }
    (FoodDecision::Dormant, priorities::DORMANT)
}

/// 决定 MLG 落地自救优先级（纯函数，Numen SurvivalDecisions.mlgPriority）。
///
/// - `on_ground`: 是否在地面上
/// - `fall_distance`: 当前坠落距离
/// - `can_save`: 是否有水桶/软方块可救
pub fn mlg_priority(on_ground: bool, fall_distance: f64, can_save: bool) -> f32 {
    if on_ground || !can_save || fall_distance <= thresholds::MLG_FALL_TRIGGER {
        return priorities::DORMANT;
    }
    priorities::MLG
}

// ═══════════════════════════════════════════════════════════════
// UnstuckDetector — 卡住检测器（Numen UnstuckDetector.java）
// ═══════════════════════════════════════════════════════════════

/// 卡住检测参数。
pub mod unstuck_params {
    /// 滚动窗口大小（tick）
    pub const WINDOW: usize = 40;
    /// 位移阈值（米²，最大位移平方 < 此值则判卡住）
    pub const MOVE_THRESHOLD: f64 = 0.75;
    /// trying 分数阈值（窗口中 trying tick 占比 ≥ 此值才判卡住）
    pub const TRYING_FRACTION: f64 = 0.8;
}

/// 卡住检测器（Numen UnstuckDetector 风格）。
///
/// 滚动窗口记录 (x, z, trying)，仅当：
/// 1. 窗口已满
/// 2. trying 占比 ≥ 80%
/// 3. 最大位移平方 < 0.75 才判定为卡住。
///
/// **关键性质**：idle body（无 locomotion 输入）永远不会被判为 stuck——
/// trying 为 false 的 tick 不计入分子。
#[derive(Debug, Clone)]
pub struct UnstuckDetector {
    xs: Vec<f64>,
    zs: Vec<f64>,
    tryings: Vec<bool>,
    /// 窗口内 trying=true 的 tick 数
    trying_count: usize,
}

impl Default for UnstuckDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl UnstuckDetector {
    pub fn new() -> Self {
        Self {
            xs: Vec::with_capacity(unstuck_params::WINDOW),
            zs: Vec::with_capacity(unstuck_params::WINDOW),
            tryings: Vec::with_capacity(unstuck_params::WINDOW),
            trying_count: 0,
        }
    }

    /// 记录一个 tick 的位置和 trying 状态。
    ///
    /// - `x`, `z`: 玩家水平坐标
    /// - `trying_to_move`: 是否有移动输入（zz/xx != 0）
    pub fn record(&mut self, x: f64, z: f64, trying_to_move: bool) {
        if self.xs.len() >= unstuck_params::WINDOW {
            // 弹出最旧
            self.xs.remove(0);
            self.zs.remove(0);
            let old_trying = self.tryings.remove(0);
            if old_trying {
                self.trying_count -= 1;
            }
        }
        self.xs.push(x);
        self.zs.push(z);
        self.tryings.push(trying_to_move);
        if trying_to_move {
            self.trying_count += 1;
        }
    }

    /// 是否判定为卡住。
    pub fn is_stuck(&self) -> bool {
        if self.xs.len() < unstuck_params::WINDOW {
            return false;
        }
        let required_trying =
            (unstuck_params::WINDOW as f64 * unstuck_params::TRYING_FRACTION).ceil() as usize;
        if self.trying_count < required_trying {
            return false;
        }
        // 计算窗口内最大位移
        let min_x = self.xs.iter().cloned().fold(f64::INFINITY, f64::min);
        let max_x = self.xs.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let min_z = self.zs.iter().cloned().fold(f64::INFINITY, f64::min);
        let max_z = self.zs.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let dx = max_x - min_x;
        let dz = max_z - min_z;
        let max_disp_sq = dx * dx + dz * dz;
        max_disp_sq < unstuck_params::MOVE_THRESHOLD
    }

    /// 重置检测器。
    pub fn reset(&mut self) {
        self.xs.clear();
        self.zs.clear();
        self.tryings.clear();
        self.trying_count = 0;
    }

    /// 当前窗口填充程度。
    pub fn fill(&self) -> usize {
        self.xs.len()
    }
}

// ═══════════════════════════════════════════════════════════════
// 测试
// ═══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn threat_response_flee_when_low_health() {
        assert_eq!(
            decide_threat_response(true, 4.0, true),
            ThreatResponse::Flee
        );
        assert_eq!(
            decide_threat_response(true, 20.0, false),
            ThreatResponse::Flee
        );
    }

    #[test]
    fn threat_response_fight_when_healthy_and_armed() {
        assert_eq!(
            decide_threat_response(true, 18.0, true),
            ThreatResponse::Fight
        );
    }

    #[test]
    fn threat_response_none_when_no_threat() {
        assert_eq!(
            decide_threat_response(false, 4.0, false),
            ThreatResponse::None
        );
    }

    #[test]
    fn food_priority_regen_when_low_health() {
        let (dec, pri) = food_priority(15, 10.0, true);
        assert_eq!(dec, FoodDecision::Regen);
        assert_eq!(pri, priorities::FOOD_REGEN);
    }

    #[test]
    fn food_priority_hunger_when_starving() {
        let (dec, pri) = food_priority(4, 20.0, true);
        assert_eq!(dec, FoodDecision::Hunger);
        assert_eq!(pri, priorities::FOOD_HUNGER);
    }

    #[test]
    fn food_priority_dormant_when_full() {
        let (dec, pri) = food_priority(20, 20.0, true);
        assert_eq!(dec, FoodDecision::Dormant);
        assert_eq!(pri, priorities::DORMANT);
    }

    #[test]
    fn food_priority_dormant_when_no_food() {
        let (dec, _) = food_priority(2, 5.0, false);
        assert_eq!(dec, FoodDecision::Dormant);
    }

    #[test]
    fn mlg_priority_triggers_on_long_fall() {
        assert_eq!(mlg_priority(false, 10.0, true), priorities::MLG);
    }

    #[test]
    fn mlg_priority_dormant_on_ground() {
        assert_eq!(mlg_priority(true, 0.0, true), priorities::DORMANT);
    }

    #[test]
    fn mlg_priority_dormant_without_bucket() {
        assert_eq!(mlg_priority(false, 10.0, false), priorities::DORMANT);
    }

    #[test]
    fn mlg_priority_dormant_for_short_fall() {
        assert_eq!(mlg_priority(false, 3.0, true), priorities::DORMANT);
    }

    #[test]
    fn unstuck_detector_not_stuck_when_idle() {
        // 即便窗口满，trying=false 不应判卡住
        let mut d = UnstuckDetector::new();
        for _ in 0..40 {
            d.record(0.0, 0.0, false);
        }
        assert!(!d.is_stuck(), "idle body should never be stuck");
    }

    #[test]
    fn unstuck_detector_not_stuck_when_moving() {
        let mut d = UnstuckDetector::new();
        for i in 0..40 {
            d.record(i as f64, 0.0, true);
        }
        assert!(!d.is_stuck(), "moving body should not be stuck");
    }

    #[test]
    fn unstuck_detector_stuck_when_trying_but_not_moving() {
        let mut d = UnstuckDetector::new();
        for _ in 0..40 {
            d.record(0.1, 0.1, true); // 一直在 trying 但位移极小
        }
        assert!(d.is_stuck(), "trying but not moving should be stuck");
    }

    #[test]
    fn unstuck_detector_not_stuck_when_window_not_full() {
        let mut d = UnstuckDetector::new();
        for _ in 0..30 {
            d.record(0.0, 0.0, true);
        }
        assert!(!d.is_stuck(), "window not full should not be stuck");
    }

    #[test]
    fn unstuck_detector_reset_clears_state() {
        let mut d = UnstuckDetector::new();
        for _ in 0..40 {
            d.record(0.0, 0.0, true);
        }
        assert!(d.is_stuck());
        d.reset();
        assert_eq!(d.fill(), 0);
        assert!(!d.is_stuck());
    }

    #[test]
    fn unstuck_detector_trying_fraction_guard() {
        // 50% trying 不应判卡住（阈值 80%）
        let mut d = UnstuckDetector::new();
        for i in 0..40 {
            d.record(0.0, 0.0, i % 2 == 0);
        }
        assert!(
            !d.is_stuck(),
            "50% trying should not be stuck (threshold 80%)"
        );
    }
}
