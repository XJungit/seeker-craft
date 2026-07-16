//! 酒狐体力与属性系统 — Stamina + CombatAttributes（对齐 MaidPatch 属性体系）。
//!
//! 参考酒狐项目 MaidPatch.java 的属性注册和体力系统：
//! - `Stamina`：非线性回复体力（越满回复越快），技能消耗资源
//! - `CombatAttributes`：10 个 EpicFight 属性（armor_negation/impact/max_strikes/stun_armor/...）
//!
//! 设计目标：
//! 1. 体力是技能资源（格挡/闪避消耗），回复有非线性曲线
//! 2. 属性决定伤害计算（护甲穿透、冲击力、最大命中数、眩晕护甲）

// ═══════════════════════════════════════════════════════════════
// Stamina — 体力系统（酒狐 MaidPatch.Stamina 风格）
// ═══════════════════════════════════════════════════════════════

/// 体力系统（对齐酒狐 MaidPatch 的 Stamina）。
///
/// 非线性回复：`staminaFactor = 1 + (stamina / (max - max*0.5))^2`
/// 即体力越满回复越快（后半段加速回复）。
#[derive(Debug, Clone)]
pub struct Stamina {
    current: f32,
    max: f32,
    regen: f32,
}

impl Stamina {
    pub fn new(max: f32, regen: f32) -> Self {
        Self { current: max, max, regen }
    }

    /// 当前体力。
    pub fn current(&self) -> f32 {
        self.current
    }

    /// 最大体力。
    pub fn max(&self) -> f32 {
        self.max
    }

    /// 是否有足够体力。
    pub fn has(&self, amount: f32) -> bool {
        self.current >= amount
    }

    /// 消耗体力，成功返回 true。
    pub fn consume(&mut self, amount: f32) -> bool {
        if self.current < amount {
            return false;
        }
        self.current -= amount;
        true
    }

    /// 强制消耗（可透支到 0）。
    pub fn force_consume(&mut self, amount: f32) {
        self.current = (self.current - amount).max(0.0);
    }

    /// 每 tick 回复（酒狐非线性公式）。
    ///
    /// `staminaFactor = 1 + (stamina / (max - max*0.5))^2`
    /// `setStamina(stamina + max * 0.01 * factor * regen)`
    pub fn tick_regen(&mut self) {
        if self.current >= self.max {
            self.current = self.max;
            return;
        }
        let half_max = self.max * 0.5;
        let denom = self.max - half_max;
        let factor = if denom > 0.0 {
            1.0 + (self.current / denom).powi(2)
        } else {
            1.0
        };
        let gain = self.max * 0.01 * factor * self.regen;
        self.current = (self.current + gain).min(self.max);
    }

    /// 重置为满。
    pub fn reset(&mut self) {
        self.current = self.max;
    }

    /// 体力比例（0.0 ~ 1.0）。
    pub fn ratio(&self) -> f32 {
        if self.max > 0.0 {
            self.current / self.max
        } else {
            0.0
        }
    }
}

impl Default for Stamina {
    fn default() -> Self {
        Self::new(20.0, 1.0)
    }
}

// ═══════════════════════════════════════════════════════════════
// CombatAttributes — EpicFight 属性体系（酒狐 MaidPatch.initAttribute）
// ═══════════════════════════════════════════════════════════════

/// 战斗属性（对齐酒狐 MaidPatch 注册的 10 个 EpicFight 属性）。
#[derive(Debug, Clone)]
pub struct CombatAttributes {
    /// 护甲穿透（0~1，绕过护甲的比例）
    pub armor_negation: f32,
    /// 冲击力（影响击退和硬直）
    pub impact: f32,
    /// 最大命中数（单次攻击最多命中几个实体）
    pub max_strikes: u32,
    /// 眩晕护甲（抵御眩晕的阈值）
    pub stun_armor: f32,
    /// 副手攻击速度
    pub offhand_attack_speed: f32,
    /// 副手最大命中数
    pub offhand_max_strikes: u32,
    /// 副手护甲穿透
    pub offhand_armor_negation: f32,
    /// 副手冲击力
    pub offhand_impact: f32,
    /// 最大体力
    pub max_stamina: f32,
    /// 体力回复
    pub stamina_regen: f32,
    /// 重量（影响闪避消耗：weight * 0.1）
    pub weight: f32,
}

impl Default for CombatAttributes {
    fn default() -> Self {
        // 酒狐默认值
        Self {
            armor_negation: 0.0,
            impact: 0.0,
            max_strikes: 999,
            stun_armor: 20.0,
            offhand_attack_speed: 0.0,
            offhand_max_strikes: 0,
            offhand_armor_negation: 0.0,
            offhand_impact: 0.0,
            max_stamina: 20.0,
            stamina_regen: 1.0,
            weight: 0.0,
        }
    }
}

impl CombatAttributes {
    /// 计算闪避消耗（酒狐 Step.java：`weight * 0.1`）。
    pub fn dodge_cost(&self) -> f32 {
        self.weight * 0.1
    }

    /// 计算格挡消耗（酒狐 BladeClash.java：基于 impact）。
    pub fn block_cost(&self, attacker_impact: f32) -> f32 {
        (attacker_impact * 0.1).min(1.0)
    }
}

// ═══════════════════════════════════════════════════════════════
// DodgeDirection — 闪避方向决策（酒狐 Step.java）
// ═══════════════════════════════════════════════════════════════

/// 闪避方向（对齐酒狐 Step 的 forward/backward/left/right）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DodgeDirection {
    Forward,
    Backward,
    Left,
    Right,
}

/// 根据目标朝向和相对位置决定闪避方向（酒狐 Step.java 的方向决策）。
///
/// - `target_yaw`: 目标朝向
/// - `target_pos`: 目标位置
/// - `self_pos`: 自身位置
pub fn decide_dodge_direction(
    target_yaw: f32,
    target_pos: (f64, f64),
    self_pos: (f64, f64),
) -> DodgeDirection {
    let yaw_rad = target_yaw.to_radians();
    // 目标朝向向量
    let facing_x = -yaw_rad.sin();
    let facing_z = yaw_rad.cos();
    // 自身相对目标的方向（统一为 f32，方向判断不需要 f64 精度）
    let dx = (self_pos.0 - target_pos.0) as f32;
    let dz = (self_pos.1 - target_pos.1) as f32;
    // 点积：>0 在前方，<0 在后方
    let dot = facing_x * dx + facing_z * dz;
    // 叉积 y 分量：>0 在右侧，<0 在左侧
    let cross_y = facing_x * dz - facing_z * dx;
    if dot > 0.0 {
        // 在目标前方 → 向后闪避
        DodgeDirection::Backward
    } else if cross_y > 0.0 {
        // 在目标右侧 → 向左闪避
        DodgeDirection::Left
    } else {
        // 在目标左侧 → 向右闪避
        DodgeDirection::Right
    }
}

// ═══════════════════════════════════════════════════════════════
// 测试
// ═══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stamina_consume_success() {
        let mut s = Stamina::new(20.0, 1.0);
        assert!(s.consume(5.0));
        assert_eq!(s.current(), 15.0);
        assert!(s.has(10.0));
        assert!(!s.has(20.0));
    }

    #[test]
    fn stamina_consume_fail_when_insufficient() {
        let mut s = Stamina::new(10.0, 1.0);
        assert!(!s.consume(15.0));
        assert_eq!(s.current(), 10.0);
    }

    #[test]
    fn stamina_force_consume_clamps_to_zero() {
        let mut s = Stamina::new(5.0, 1.0);
        s.force_consume(10.0);
        assert_eq!(s.current(), 0.0);
    }

    #[test]
    fn stamina_nonlinear_regen() {
        let mut s = Stamina::new(20.0, 1.0);
        s.force_consume(20.0); // 清零
        assert_eq!(s.current(), 0.0);
        // 回复多个 tick，应逐渐恢复
        for _ in 0..100 {
            s.tick_regen();
        }
        assert!(s.current() > 5.0, "should regen over 100 ticks");
        // 不应超过 max
        assert!(s.current() <= 20.0);
    }

    #[test]
    fn stamina_regen_faster_when_higher() {
        // 低体力时回复慢，高体力时回复快（非线性）
        let mut low = Stamina::new(20.0, 1.0);
        low.force_consume(20.0); // current=0
        let mut high = Stamina::new(20.0, 1.0);
        high.force_consume(5.0); // current=15
        let low_before = low.current();
        let high_before = high.current();
        for _ in 0..10 {
            low.tick_regen();
            high.tick_regen();
        }
        // 高体力的回复量应更大（非线性加速）
        assert!(high.current() - high_before > low.current() - low_before);
    }

    #[test]
    fn stamina_ratio() {
        let s = Stamina::new(20.0, 1.0);
        assert_eq!(s.ratio(), 1.0);
    }

    #[test]
    fn combat_attributes_defaults() {
        let a = CombatAttributes::default();
        assert_eq!(a.max_strikes, 999);
        assert_eq!(a.stun_armor, 20.0);
        assert_eq!(a.max_stamina, 20.0);
    }

    #[test]
    fn combat_attributes_dodge_cost() {
        let mut a = CombatAttributes::default();
        a.weight = 5.0;
        assert_eq!(a.dodge_cost(), 0.5);
    }

    #[test]
    fn combat_attributes_block_cost_capped() {
        let a = CombatAttributes::default();
        assert_eq!(a.block_cost(5.0), 0.5);
        assert_eq!(a.block_cost(20.0), 1.0); // capped
    }

    #[test]
    fn dodge_direction_backward_when_in_front() {
        // 目标朝南（yaw=0），自身在目标南方（z 更大）→ 在前方 → 向后闪避
        let dir = decide_dodge_direction(0.0, (0.0, 0.0), (0.0, 5.0));
        assert_eq!(dir, DodgeDirection::Backward);
    }

    #[test]
    fn dodge_direction_left_or_right_when_behind() {
        // 目标朝南（yaw=0），自身在目标北方（z 更小）→ 在后方 → 左或右
        let dir_left = decide_dodge_direction(0.0, (0.0, 0.0), (-5.0, -5.0));
        let dir_right = decide_dodge_direction(0.0, (0.0, 0.0), (5.0, -5.0));
        // 只要不是 Backward 就行
        assert_ne!(dir_left, DodgeDirection::Backward);
        assert_ne!(dir_right, DodgeDirection::Backward);
    }
}
