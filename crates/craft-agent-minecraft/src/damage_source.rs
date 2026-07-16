//! 酒狐伤害系统 — DamageSource 标签 + ValueModifier + StunType（对齐 EpicFight 伤害体系）。
//!
//! 参考酒狐项目 BroadBladeSkill.java / BladeClash.java 的伤害 tag 体系：
//! - `DamageSource`：携带 11 种 tag（BypassArmor/Unblockable/Finisher/...）
//! - `ValueModifier`：伤害修改器（Multiplier/Flat/PercentageMaxHealth）
//! - `StunType`：7 种眩晕类型（None/Short/Long/Hold/Knockdown/Neutralize/Fall）
//!
//! 设计目标：
//! 1. tag 决定伤害是否可被格挡/闪避/护甲减免/盾牌格挡
//! 2. 技能可动态修改伤害倍率（单目标加成、百分比生命伤害）
//! 3. 不同攻击造成不同眩晕表现，可用于技能免疫

use std::collections::HashSet;

// ═══════════════════════════════════════════════════════════════
// StunType — 眩晕类型（酒狐 StunType 7 种）
// ═══════════════════════════════════════════════════════════════

/// 眩晕类型（对齐酒狐 StunType）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StunType {
    /// 无眩晕
    None,
    /// 短硬直
    Short,
    /// 长硬直
    Long,
    /// 持续保持
    Hold,
    /// 击倒
    Knockdown,
    /// 中和（取消当前动作）
    Neutralize,
    /// 倒地
    Fall,
}

impl Default for StunType {
    fn default() -> Self {
        StunType::Short
    }
}

// ═══════════════════════════════════════════════════════════════
// DamageTag — 伤害标签（酒狐 11 种 tag）
// ═══════════════════════════════════════════════════════════════

/// 伤害标签（对齐酒狐 DamageSource tags）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DamageTag {
    /// 绕过闪避
    BypassDodge,
    /// 破盾（穿透格挡）
    GuardPuncture,
    /// 不可格挡
    Unblockable,
    /// 终结技
    Finisher,
    /// 绕过护甲
    BypassArmor,
    /// 绕过无敌帧
    BypassInvulnerability,
    /// 绕过抗性
    BypassResistance,
    /// 绕过附魔
    BypassEnchantments,
    /// 绕过效果
    BypassEffects,
    /// 绕过冷却
    BypassCooldown,
    /// 绕过盾牌
    BypassShield,
    /// 爆炸伤害
    IsExplosion,
    /// 魔法伤害
    Magic,
    /// 火焰伤害
    IsFire,
}

impl DamageTag {
    /// 是否为"不可格挡"类 tag（酒狐 isBlockableSource 检查）。
    pub fn is_unblockable(self) -> bool {
        matches!(
            self,
            DamageTag::BypassInvulnerability
                | DamageTag::Unblockable
                | DamageTag::BypassArmor
                | DamageTag::IsExplosion
                | DamageTag::Magic
                | DamageTag::IsFire
        )
    }
}

// ═══════════════════════════════════════════════════════════════
// ValueModifier — 伤害修改器（酒狐 ValueModifier）
// ═══════════════════════════════════════════════════════════════

/// 伤害修改器（对齐酒狐 ValueModifier）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ValueModifier {
    /// 倍率（如 1.5 = 150% 伤害）
    Multiplier(f32),
    /// 固定加成（如 +5 伤害）
    Flat(f32),
    /// 目标最大生命百分比（如 0.05 = 5% max health）
    PercentageMaxHealth(f32),
}

impl ValueModifier {
    /// 应用修改器到基础伤害。
    pub fn apply(self, base: f32, target_max_health: f32) -> f32 {
        match self {
            ValueModifier::Multiplier(m) => base * m,
            ValueModifier::Flat(b) => base + b,
            ValueModifier::PercentageMaxHealth(p) => base + target_max_health * p,
        }
    }
}

// ═══════════════════════════════════════════════════════════════
// DamageSource — 伤害来源（对齐酒狐 EFSource）
// ═══════════════════════════════════════════════════════════════

/// 伤害来源（对齐酒狐 DamageSource / EFSource）。
#[derive(Debug, Clone)]
pub struct DamageSource {
    /// 攻击者实体类型
    pub attacker: String,
    /// 基础伤害
    pub amount: f32,
    /// 眩晕类型
    pub stun_type: StunType,
    /// 伤害标签集合
    pub tags: HashSet<DamageTag>,
    /// 冲击力（影响击退）
    pub impact: f32,
    /// 伤害修改器列表
    pub modifiers: Vec<ValueModifier>,
}

impl DamageSource {
    pub fn new(attacker: &str, amount: f32) -> Self {
        Self {
            attacker: attacker.into(),
            amount,
            stun_type: StunType::default(),
            tags: HashSet::new(),
            impact: 0.0,
            modifiers: vec![],
        }
    }

    /// 链式：设置眩晕类型。
    pub fn stun(mut self, s: StunType) -> Self {
        self.stun_type = s;
        self
    }

    /// 链式：添加 tag。
    pub fn tag(mut self, t: DamageTag) -> Self {
        self.tags.insert(t);
        self
    }

    /// 链式：设置冲击力。
    pub fn impact(mut self, i: f32) -> Self {
        self.impact = i;
        self
    }

    /// 链式：添加伤害修改器。
    pub fn modifier(mut self, m: ValueModifier) -> Self {
        self.modifiers.push(m);
        self
    }

    /// 是否可被格挡（酒狐 isBlockableSource）。
    pub fn is_blockable(&self) -> bool {
        !self.tags.iter().any(|t| t.is_unblockable())
    }

    /// 计算最终伤害（应用所有修改器）。
    pub fn final_amount(&self, target_max_health: f32) -> f32 {
        let mut amount = self.amount;
        for m in &self.modifiers {
            amount = m.apply(amount, target_max_health);
        }
        amount.max(0.0)
    }

    /// 是否包含某 tag。
    pub fn has_tag(&self, t: DamageTag) -> bool {
        self.tags.contains(&t)
    }
}

impl Default for DamageSource {
    fn default() -> Self {
        Self::new("unknown", 0.0)
    }
}

// ═══════════════════════════════════════════════════════════════
// 伤害计算工具
// ═══════════════════════════════════════════════════════════════

/// 计算击退力（酒狐 BladeClash.knockback 公式）。
///
/// `knockback = min(impact * 0.1, 1.0)`
pub fn calc_knockback(impact: f32) -> f32 {
    (impact * 0.1).min(1.0)
}

/// 判断是否为正面攻击（酒狐 isFrontAttack，点积判断）。
///
/// - `attacker_yaw`: 攻击者朝向
/// - `attacker_pos`: 攻击者位置
/// - `target_pos`: 目标位置
/// 返回 true 表示攻击来自目标正面。
pub fn is_front_attack(attacker_yaw: f32, attacker_pos: (f64, f64), target_pos: (f64, f64)) -> bool {
    // 攻击者朝向向量
    let yaw_rad = attacker_yaw.to_radians();
    let facing_x = -yaw_rad.sin();
    let facing_z = yaw_rad.cos();
    // 目标相对攻击者的方向（统一为 f32，方向判断不需要 f64 精度）
    let dx = (target_pos.0 - attacker_pos.0) as f32;
    let dz = (target_pos.1 - attacker_pos.1) as f32;
    // 点积 > 0 表示目标在攻击者前方
    facing_x * dx + facing_z * dz > 0.0
}

// ═══════════════════════════════════════════════════════════════
// 测试
// ═══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn damage_source_builder_chain() {
        let ds = DamageSource::new("zombie", 8.0)
            .stun(StunType::Long)
            .tag(DamageTag::BypassArmor)
            .impact(2.0)
            .modifier(ValueModifier::Multiplier(1.5));
        assert_eq!(ds.attacker, "zombie");
        assert_eq!(ds.amount, 8.0);
        assert_eq!(ds.stun_type, StunType::Long);
        assert!(ds.has_tag(DamageTag::BypassArmor));
        assert_eq!(ds.impact, 2.0);
        assert_eq!(ds.modifiers.len(), 1);
    }

    #[test]
    fn damage_source_blockable_check() {
        let blockable = DamageSource::new("zombie", 5.0);
        assert!(blockable.is_blockable());
        let unblockable = DamageSource::new("zombie", 5.0).tag(DamageTag::Unblockable);
        assert!(!unblockable.is_blockable());
        let fire = DamageSource::new("lava", 5.0).tag(DamageTag::IsFire);
        assert!(!fire.is_blockable());
    }

    #[test]
    fn value_modifier_multiplier() {
        let ds = DamageSource::new("zombie", 10.0).modifier(ValueModifier::Multiplier(1.5));
        assert_eq!(ds.final_amount(20.0), 15.0);
    }

    #[test]
    fn value_modifier_flat() {
        let ds = DamageSource::new("zombie", 10.0).modifier(ValueModifier::Flat(5.0));
        assert_eq!(ds.final_amount(20.0), 15.0);
    }

    #[test]
    fn value_modifier_percentage_max_health() {
        let ds = DamageSource::new("zombie", 10.0).modifier(ValueModifier::PercentageMaxHealth(0.05));
        // 10 + 20 * 0.05 = 11
        assert_eq!(ds.final_amount(20.0), 11.0);
    }

    #[test]
    fn value_modifier_stacked() {
        let ds = DamageSource::new("zombie", 10.0)
            .modifier(ValueModifier::Multiplier(1.5))   // 15
            .modifier(ValueModifier::Flat(3.0))          // 18
            .modifier(ValueModifier::PercentageMaxHealth(0.1)); // 18 + 20*0.1 = 20
        assert_eq!(ds.final_amount(20.0), 20.0);
    }

    #[test]
    fn calc_knockback_capped_at_1() {
        assert_eq!(calc_knockback(5.0), 0.5);
        assert_eq!(calc_knockback(20.0), 1.0); // capped
        assert_eq!(calc_knockback(0.0), 0.0);
    }

    #[test]
    fn is_front_attack_positive_dot() {
        // 攻击者朝南（yaw=0），目标在攻击者南方（z 更大）
        assert!(is_front_attack(0.0, (0.0, 0.0), (0.0, 5.0)));
        // 目标在攻击者北方（z 更小）→ 背后
        assert!(!is_front_attack(0.0, (0.0, 0.0), (0.0, -5.0)));
    }

    #[test]
    fn damage_tag_unblockable_classification() {
        assert!(DamageTag::BypassArmor.is_unblockable());
        assert!(DamageTag::Unblockable.is_unblockable());
        assert!(DamageTag::IsFire.is_unblockable());
        assert!(DamageTag::Magic.is_unblockable());
        assert!(!DamageTag::Finisher.is_unblockable());
        assert!(!DamageTag::BypassDodge.is_unblockable());
    }
}
