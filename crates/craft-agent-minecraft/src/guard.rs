//! 酒狐（EpicFight_TouhouLittleMaid）格挡系统 + 目标选择 + 击退系统。
//!
//! 深度对齐酒狐项目的三大战斗子系统：
//! - `GuardSystem`：被动格挡 + 双向反震 + 体力惩罚递增（酒狐 BladeClash.java）
//! - `TargetingStrategy`：三级仇恨优先级 + 白名单/黑名单分流（酒狐 FightModeTask.java）
//! - `KnockbackCalculator`：击退强度公式 + 双向击退方向（酒狐 BladeClash + LivingEntityPatch）
//!
//! 设计要点（酒狐核心原则）：
//! 1. 格挡是被动技能：订阅被攻击事件，五重条件全满足才触发
//! 2. 格挡成功 = 双向击退（女仆和攻击者都被弹开）+ 取消原伤害
//! 3. 体力惩罚防滥用：每次格挡 +0.1，2 秒未格挡归零
//! 4. 目标选择三级优先级：自身复仇 > 主人攻击 > 主人被攻击
//! 5. 对友好生物白名单，对敌对生物黑名单
//! 6. 击退 = 0.1 基础 + min(impact * 0.1, 1.0)，硬封顶 1.1

use crate::damage_source::{DamageSource, DamageTag};
use crate::stamina::Stamina;

// ═══════════════════════════════════════════════════════════════
// GuardSystem — 被动格挡 + 双向反震（酒狐 BladeClash.java）
// ═══════════════════════════════════════════════════════════════

/// 格挡结果。
#[derive(Debug, Clone, PartialEq)]
pub enum GuardResult {
    /// 格挡成功（取消原伤害 + 双向击退）。
    Blocked { knockback: f32 },
    /// 格挡失败（不取消伤害）。
    PassedThrough { reason: GuardFailReason },
}

/// 格挡失败原因。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuardFailReason {
    /// 非正面攻击
    NotFrontAttack,
    /// 伤害源不可格挡（爆炸/魔法/火/穿甲）
    UnblockableSource,
    /// 阶段不在可格挡窗口（1-2）
    WrongPhase,
    /// 体力不足
    InsufficientStamina,
    /// 无敌/免疫状态（不触发格挡）
    Invulnerable,
    /// 攻击来自主人
    FriendlyFire,
    /// 无伤害源实体
    NoSourceEntity,
}

/// 可格挡窗口的阶段范围（酒狐 phaseLevel 1-2）。
pub const GUARD_PHASE_MIN: u32 = 1;
pub const GUARD_PHASE_MAX: u32 = 2;

/// 基础击退值（酒狐 knockback = 0.1F 基础）。
pub const KNOCKBACK_BASE: f32 = 0.1;
/// 击退冲击力系数（酒狐 knockback += min(impact * 0.1F, 1.0F)）。
pub const KNOCKBACK_IMPACT_COEFF: f32 = 0.1;
/// 击退冲击力上限（酒狐 Math.min(impact * 0.1F, 1.0F)）。
pub const KNOCKBACK_IMPACT_MAX: f32 = 1.0;

/// 体力惩罚递增量（酒狐 CLASH_PENALTY += 0.1F）。
pub const CLASH_PENALTY_INCREMENT: f32 = 0.1;
/// 体力惩罚恢复时间（酒狐 40 tick = 2 秒未格挡归零）。
pub const CLASH_PENALTY_RESTORE_TICKS: u32 = 40;

/// 格挡系统状态（酒狐 BladeClash 的 SkillData）。
#[derive(Debug, Clone)]
pub struct GuardSystem {
    /// 当前体力惩罚系数（每次格挡 +0.1，2 秒未格挡归零）。
    clash_penalty: f32,
    /// 上次格挡的 tick（用于惩罚恢复判断）。
    last_clash_tick: i64,
    /// 格挡总次数。
    total_blocks: u64,
}

impl Default for GuardSystem {
    fn default() -> Self {
        Self::new()
    }
}

impl GuardSystem {
    pub fn new() -> Self {
        Self {
            clash_penalty: 0.0,
            last_clash_tick: 0,
            total_blocks: 0,
        }
    }

    /// 尝试格挡（酒狐 BladeClash.MaidAttack 五重条件）。
    ///
    /// 返回 GuardResult::Blocked 表示格挡成功（应取消原伤害 + 双向击退）。
    /// 返回 GuardResult::PassedThrough 表示格挡失败（不取消伤害）。
    pub fn try_guard(
        &mut self,
        damage: &DamageSource,
        phase_level: u32,
        is_front_attack: bool,
        is_invulnerable: bool,
        is_immune: bool,
        is_friendly_fire: bool,
        stamina: &mut Stamina,
        current_tick: i64,
    ) -> GuardResult {
        // ① 无敌/免疫时不触发格挡（酒狐 EFNCompat.isInvulnerability / isImmunity）
        if is_invulnerable || is_immune {
            return GuardResult::PassedThrough { reason: GuardFailReason::Invulnerable };
        }

        // ② 排除主人攻击（酒狐 OwnerPatch.getOriginal().equals(Source.getEntity())）
        if is_friendly_fire {
            return GuardResult::PassedThrough { reason: GuardFailReason::FriendlyFire };
        }

        // ③ 阶段 1-2 才能格挡（酒狐 phaseLevel > 0 && phaseLevel < 3）
        if phase_level < GUARD_PHASE_MIN || phase_level > GUARD_PHASE_MAX {
            return GuardResult::PassedThrough { reason: GuardFailReason::WrongPhase };
        }

        // ④ 正面攻击（酒狐 isFrontAttack）
        if !is_front_attack {
            return GuardResult::PassedThrough { reason: GuardFailReason::NotFrontAttack };
        }

        // ⑤ 可格挡伤害源（酒狐 isBlockableSource）
        if !damage.is_blockable() {
            return GuardResult::PassedThrough { reason: GuardFailReason::UnblockableSource };
        }

        // 体力惩罚恢复：2 秒未格挡 → 归零（酒狐 MaidTick 检测）
        if current_tick - self.last_clash_tick > CLASH_PENALTY_RESTORE_TICKS as i64 {
            self.clash_penalty = 0.0;
        }

        // 计算体力消耗（酒狐 consumeAmount = penalty * impact）
        let consume_amount = self.clash_penalty * damage.impact;
        if !stamina.consume(consume_amount) {
            return GuardResult::PassedThrough { reason: GuardFailReason::InsufficientStamina };
        }

        // 格挡成功！
        self.clash_penalty += CLASH_PENALTY_INCREMENT;
        self.last_clash_tick = current_tick;
        self.total_blocks += 1;

        // 计算击退强度（酒狐 knockback = 0.1 + min(impact * 0.1, 1.0)）
        let knockback = calc_knockback(damage.impact);

        GuardResult::Blocked { knockback }
    }

    /// 当前体力惩罚系数。
    pub fn clash_penalty(&self) -> f32 {
        self.clash_penalty
    }

    /// 格挡总次数。
    pub fn total_blocks(&self) -> u64 {
        self.total_blocks
    }

    /// 重置（武器切换时调用，酒狐 resetAi）。
    pub fn reset(&mut self) {
        self.clash_penalty = 0.0;
        self.last_clash_tick = 0;
    }
}

/// 判断伤害源是否可格挡（酒狐 isBlockableSource）。
///
/// 以下类型不可格挡：BypassInvulnerability / Unblockable / BypassArmor / IsExplosion / Magic / IsFire
/// 注意：DamageSource 已有 `is_blockable()` 方法，此函数是对 tag 列表的语义封装。
pub fn is_blockable_source(damage: &DamageSource) -> bool {
    damage.is_blockable()
}

/// 计算击退强度（酒狐 knockback = 0.1 + min(impact * 0.1, 1.0)）。
pub fn calc_knockback(impact: f32) -> f32 {
    KNOCKBACK_BASE + (impact * KNOCKBACK_IMPACT_COEFF).min(KNOCKBACK_IMPACT_MAX)
}

// ═══════════════════════════════════════════════════════════════
// TargetingStrategy — 三级仇恨优先级（酒狐 FightModeTask.java）
// ═══════════════════════════════════════════════════════════════

/// 目标类型分类（酒狐 checkAttack 的分流逻辑）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetCategory {
    /// 玩家
    Player,
    /// 其他女仆/友方实体
    FriendlyMaid,
    /// 友好类别生物（动物等）
    FriendlyCreature,
    /// 敌对/中性生物
    Hostile,
}

/// 目标候选信息。
#[derive(Debug, Clone)]
pub struct TargetCandidate {
    pub entity_id: String,
    pub entity_type: String,
    pub category: TargetCategory,
    pub distance: f64,
    pub pos: (f64, f64, f64),
    pub is_alive: bool,
}

/// 仇恨记忆（酒狐 brain memory 中的攻击目标信息）。
#[derive(Debug, Clone, Default)]
pub struct AggroMemory {
    /// 女仆自身的复仇目标（getLastAttacker）。
    pub self_revenge: Option<String>,
    /// 主人正在攻击的目标（getOwner().getLastHurtMob()）。
    pub owner_attacking: Option<String>,
    /// 主人的复仇目标（getOwner().getLastAttacker()）。
    pub owner_revenge: Option<String>,
}

/// 目标选择策略（对齐酒狐 FightModeTask.checkAttack + checkTarget）。
pub struct TargetingStrategy {
    /// 活动范围半径（酒仆 getRestrictRadius）。
    pub restrict_radius: f64,
}

impl Default for TargetingStrategy {
    fn default() -> Self {
        Self { restrict_radius: 16.0 }
    }
}

impl TargetingStrategy {
    pub fn new(restrict_radius: f64) -> Self {
        Self { restrict_radius }
    }

    /// 从候选中选出最优目标（酒狐 findFirstValidAttackTarget + checkAttack）。
    ///
    /// 三级仇恨优先级：
    /// 1. 自身复仇目标（最高优先级）
    /// 2. 主人正在攻击的目标
    /// 3. 主人的复仇目标
    ///
    /// 对 Player/FriendlyMaid/FriendlyCreature：白名单（必须是三者之一）
    /// 对 Hostile：黑名单（非盟友即可）
    pub fn select_target<'a>(
        &self,
        candidates: &'a [TargetCandidate],
        memory: &AggroMemory,
    ) -> Option<&'a TargetCandidate> {
        // 按距离排序（酒狐 findClosest）
        let mut sorted: Vec<&TargetCandidate> = candidates
            .iter()
            .filter(|c| c.is_alive)
            .collect();
        sorted.sort_by(|a, b| a.distance.partial_cmp(&b.distance).unwrap_or(std::cmp::Ordering::Equal));

        // 酒狐三级仇恨优先级：跨类别生效。
        // 如果存在仇恨记忆，优先选仇恨目标（按 P1>P2>P3 顺序遍历候选），
        // 保证女仆优先处理对自己/主人的威胁，而非单纯按距离选最近。
        // 这对 Hostile 也生效：被 zombie_1 打了，即使更近处有 skeleton_1，也应优先打 zombie_1。
        let revenge_targets: Vec<&str> = [
            memory.self_revenge.as_deref(),
            memory.owner_attacking.as_deref(),
            memory.owner_revenge.as_deref(),
        ]
        .into_iter()
        .flatten()
        .collect();

        for revenge_id in &revenge_targets {
            for candidate in &sorted {
                if candidate.entity_id.as_str() == *revenge_id && self.check_attack(candidate, memory) {
                    return Some(candidate);
                }
            }
        }

        // 无仇恨目标命中 → 按距离选最近的可攻击目标
        for candidate in &sorted {
            if self.check_attack(candidate, memory) {
                return Some(candidate);
            }
        }
        None
    }

    /// 检查目标是否可攻击（酒狐 checkAttack + checkTarget）。
    pub fn check_attack(&self, target: &TargetCandidate, memory: &AggroMemory) -> bool {
        match target.category {
            TargetCategory::Player
            | TargetCategory::FriendlyMaid
            | TargetCategory::FriendlyCreature => {
                // 白名单：必须是三级仇恨之一
                self.check_target(&target.entity_id, memory)
            }
            TargetCategory::Hostile => {
                // 黑名单：非盟友即可攻击
                true
            }
        }
    }

    /// 三级仇恨优先级检查（酒狐 checkTarget）。
    ///
    /// P1: 自身复仇目标 > P2: 主人攻击目标 > P3: 主人复仇目标
    fn check_target(&self, entity_id: &str, memory: &AggroMemory) -> bool {
        if let Some(ref revenge) = memory.self_revenge {
            return entity_id == revenge.as_str(); // P1
        }
        if let Some(ref attacking) = memory.owner_attacking {
            return entity_id == attacking.as_str(); // P2
        }
        if let Some(ref revenge) = memory.owner_revenge {
            return entity_id == revenge.as_str(); // P3
        }
        false
    }

    /// 检查目标是否过远应脱战（酒狐 farAway）。
    ///
    /// 跟随模式：以主人为中心
    /// 驻守模式：以女仆为中心
    pub fn is_too_far(&self, target: &TargetCandidate, companion_pos: (f64, f64, f64), owner_pos: Option<(f64, f64, f64)>) -> bool {
        if !target.is_alive {
            return true;
        }
        let center = owner_pos.unwrap_or(companion_pos);
        let dx = target.pos.0 - center.0;
        let dz = target.pos.2 - center.2;
        let dist = (dx * dx + dz * dz).sqrt();
        dist > self.restrict_radius
    }
}

// ═══════════════════════════════════════════════════════════════
// 双向击退方向计算（酒狐 BladeClash 双向反震）
// ═══════════════════════════════════════════════════════════════

/// 双向击退方向（酒狐 BladeClash 的双向反震）。
#[derive(Debug, Clone)]
pub struct BidirectionalKnockback {
    /// 被攻击者（格挡者）的击退方向。
    pub defender_knockback: (f64, f64, f64),
    /// 攻击者的反击退方向。
    pub attacker_knockback: (f64, f64, f64),
    /// 击退强度。
    pub strength: f32,
}

/// 计算双向击退方向（酒狐 BladeClash 第 88-98 行）。
///
/// - 被攻击者：背离伤害源位置
/// - 攻击者：背离被攻击者位置
pub fn calc_bidirectional_knockback(
    defender_pos: (f64, f64, f64),
    attacker_pos: (f64, f64, f64),
    damage_source_pos: (f64, f64, f64),
    strength: f32,
) -> BidirectionalKnockback {
    // 被攻击者击退方向：背离伤害源（酒狐 MaidPatch.knockBackEntity(Source.getSourcePosition(), knockback)）
    let dx_def = defender_pos.0 - damage_source_pos.0;
    let dz_def = defender_pos.2 - damage_source_pos.2;
    let len_def = (dx_def * dx_def + dz_def * dz_def).sqrt().max(0.001);
    let defender_knockback = (dx_def / len_def, 0.0, dz_def / len_def);

    // 攻击者击退方向：背离被攻击者（酒狐 SourcePatch.knockBackEntity(Maid.position(), knockback)）
    let dx_atk = attacker_pos.0 - defender_pos.0;
    let dz_atk = attacker_pos.2 - defender_pos.2;
    let len_atk = (dx_atk * dx_atk + dz_atk * dz_atk).sqrt().max(0.001);
    let attacker_knockback = (dx_atk / len_atk, 0.0, dz_atk / len_atk);

    BidirectionalKnockback {
        defender_knockback,
        attacker_knockback,
        strength,
    }
}

// ═══════════════════════════════════════════════════════════════
// 状态效果门控（酒狐 EFNCompat 状态效果检查）
// ═══════════════════════════════════════════════════════════════

/// 状态效果类型（酒狐 MobEffect 应用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StatusEffect {
    /// 无敌
    Invulnerability,
    /// 眩晕免疫
    StunImmunity,
    /// 速度提升
    Speed,
    /// 渐愈（回血）
    GradualHeal,
    /// 嗜血
    BloodLust,
    /// 战续（伤害加成）
    BattleContinuation,
}

/// 状态效果实例（带持续时间）。
#[derive(Debug, Clone)]
pub struct ActiveEffect {
    pub effect: StatusEffect,
    pub duration_ticks: i32,
    pub amplifier: i32,
}

/// 状态效果管理器（酒狐 EntityMaid.addEffect/removeEffect/hasEffect）。
#[derive(Debug, Clone, Default)]
pub struct EffectManager {
    effects: Vec<ActiveEffect>,
}

impl EffectManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// 添加效果。
    pub fn add_effect(&mut self, effect: StatusEffect, duration_ticks: i32, amplifier: i32) {
        // 移除同类型旧效果（酒狐 removeEffect 语义）
        self.effects.retain(|e| e.effect != effect);
        self.effects.push(ActiveEffect { effect, duration_ticks, amplifier });
    }

    /// 检查是否有某效果。
    pub fn has_effect(&self, effect: StatusEffect) -> bool {
        self.effects.iter().any(|e| e.effect == effect && e.duration_ticks > 0)
    }

    /// 获取效果放大器（等级）。
    pub fn get_amplifier(&self, effect: StatusEffect) -> Option<i32> {
        self.effects
            .iter()
            .find(|e| e.effect == effect && e.duration_ticks > 0)
            .map(|e| e.amplifier)
    }

    /// 每 tick 更新（减少持续时间）。
    pub fn tick(&mut self) {
        for e in &mut self.effects {
            if e.duration_ticks > 0 {
                e.duration_ticks -= 1;
            }
        }
        // 清理过期效果
        self.effects.retain(|e| e.duration_ticks > 0 || e.duration_ticks == -1); // -1 = 永久
    }

    /// 移除效果。
    pub fn remove_effect(&mut self, effect: StatusEffect) {
        self.effects.retain(|e| e.effect != effect);
    }

    /// 检查无敌（酒狐 isInvulnerability）。
    pub fn is_invulnerable(&self) -> bool {
        self.has_effect(StatusEffect::Invulnerability)
    }

    /// 检查眩晕免疫（酒狐 isImmunity）。
    pub fn is_stun_immune(&self) -> bool {
        self.has_effect(StatusEffect::StunImmunity)
    }

    /// 击杀奖励（酒狐 BroadBladeSkill.MaidKillTarget：渐愈 IV + 速度 II）。
    pub fn on_kill_target(&mut self) {
        self.add_effect(StatusEffect::GradualHeal, 40, 3); // 40 tick, IV 级
        self.add_effect(StatusEffect::Speed, 100, 1);      // 100 tick, II 级
    }
}

// ═══════════════════════════════════════════════════════════════
// 测试
// ═══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::damage_source::{DamageSource, DamageTag, StunType};

    fn make_damage(impact: f32, tags: Vec<DamageTag>) -> DamageSource {
        let mut dmg = DamageSource::new("zombie", 10.0)
            .stun(StunType::Short)
            .impact(impact);
        for t in tags {
            dmg = dmg.tag(t);
        }
        dmg
    }

    #[test]
    fn guard_success_all_conditions_met() {
        let mut guard = GuardSystem::new();
        let mut stamina = Stamina::new(20.0, 1.0);
        let damage = make_damage(0.5, vec![]);

        let result = guard.try_guard(
            &damage,
            1,         // phase 1（可格挡窗口）
            true,      // 正面攻击
            false,     // 非无敌
            false,     // 非免疫
            false,     // 非友军伤害
            &mut stamina,
            0,
        );

        assert!(matches!(result, GuardResult::Blocked { .. }));
        if let GuardResult::Blocked { knockback } = result {
            // 0.1 + min(0.5 * 0.1, 1.0) = 0.1 + 0.05 = 0.15
            assert!((knockback - 0.15).abs() < 0.001);
        }
        assert_eq!(guard.total_blocks(), 1);
        assert!((guard.clash_penalty() - 0.1).abs() < 0.001);
    }

    #[test]
    fn guard_fail_not_front_attack() {
        let mut guard = GuardSystem::new();
        let mut stamina = Stamina::new(20.0, 1.0);
        let damage = make_damage(0.5, vec![]);

        let result = guard.try_guard(&damage, 1, false, false, false, false, &mut stamina, 0);
        assert_eq!(result, GuardResult::PassedThrough { reason: GuardFailReason::NotFrontAttack });
    }

    #[test]
    fn guard_fail_wrong_phase() {
        let mut guard = GuardSystem::new();
        let mut stamina = Stamina::new(20.0, 1.0);
        let damage = make_damage(0.5, vec![]);

        // phase 0（待机）不能格挡
        let result = guard.try_guard(&damage, 0, true, false, false, false, &mut stamina, 0);
        assert_eq!(result, GuardResult::PassedThrough { reason: GuardFailReason::WrongPhase });

        // phase 3（重击/硬直）不能格挡
        let result = guard.try_guard(&damage, 3, true, false, false, false, &mut stamina, 0);
        assert_eq!(result, GuardResult::PassedThrough { reason: GuardFailReason::WrongPhase });
    }

    #[test]
    fn guard_fail_unblockable_source() {
        let mut guard = GuardSystem::new();
        let mut stamina = Stamina::new(20.0, 1.0);

        // 爆炸不可格挡
        let damage = make_damage(0.5, vec![DamageTag::IsExplosion]);
        let result = guard.try_guard(&damage, 1, true, false, false, false, &mut stamina, 0);
        assert_eq!(result, GuardResult::PassedThrough { reason: GuardFailReason::UnblockableSource });

        // 魔法不可格挡
        let damage = make_damage(0.5, vec![DamageTag::Magic]);
        let result = guard.try_guard(&damage, 1, true, false, false, false, &mut stamina, 0);
        assert_eq!(result, GuardResult::PassedThrough { reason: GuardFailReason::UnblockableSource });

        // 穿甲不可格挡
        let damage = make_damage(0.5, vec![DamageTag::BypassArmor]);
        let result = guard.try_guard(&damage, 1, true, false, false, false, &mut stamina, 0);
        assert_eq!(result, GuardResult::PassedThrough { reason: GuardFailReason::UnblockableSource });
    }

    #[test]
    fn guard_fail_invulnerable() {
        let mut guard = GuardSystem::new();
        let mut stamina = Stamina::new(20.0, 1.0);
        let damage = make_damage(0.5, vec![]);

        let result = guard.try_guard(&damage, 1, true, true, false, false, &mut stamina, 0);
        assert_eq!(result, GuardResult::PassedThrough { reason: GuardFailReason::Invulnerable });
    }

    #[test]
    fn guard_fail_friendly_fire() {
        let mut guard = GuardSystem::new();
        let mut stamina = Stamina::new(20.0, 1.0);
        let damage = make_damage(0.5, vec![]);

        let result = guard.try_guard(&damage, 1, true, false, false, true, &mut stamina, 0);
        assert_eq!(result, GuardResult::PassedThrough { reason: GuardFailReason::FriendlyFire });
    }

    #[test]
    fn guard_penalty_resets_after_2_seconds() {
        let mut guard = GuardSystem::new();
        let mut stamina = Stamina::new(20.0, 1.0);
        let damage = make_damage(0.5, vec![]);

        // 格挡一次 → penalty = 0.1
        guard.try_guard(&damage, 1, true, false, false, false, &mut stamina, 0);
        assert!((guard.clash_penalty() - 0.1).abs() < 0.001);

        // 41 tick 后（> 40）→ penalty 归零
        let result = guard.try_guard(&damage, 1, true, false, false, false, &mut stamina, 41);
        assert!(matches!(result, GuardResult::Blocked { .. }));
        // penalty 应该是 0.0（归零后） + 0.1（本次格挡） = 0.1
        assert!((guard.clash_penalty() - 0.1).abs() < 0.001);
    }

    #[test]
    fn guard_penalty_accumulates() {
        let mut guard = GuardSystem::new();
        let mut stamina = Stamina::new(100.0, 1.0); // 足够体力
        let damage = make_damage(0.1, vec![]); // 低 impact

        // 连续格挡 3 次
        guard.try_guard(&damage, 1, true, false, false, false, &mut stamina, 0);
        guard.try_guard(&damage, 1, true, false, false, false, &mut stamina, 1);
        guard.try_guard(&damage, 1, true, false, false, false, &mut stamina, 2);

        // penalty 应该是 0.3
        assert!((guard.clash_penalty() - 0.3).abs() < 0.001);
        assert_eq!(guard.total_blocks(), 3);
    }

    #[test]
    fn knockback_calculation() {
        // impact = 0.5 → 0.1 + min(0.05, 1.0) = 0.15
        assert!((calc_knockback(0.5) - 0.15).abs() < 0.001);

        // impact = 10.0 → 0.1 + min(1.0, 1.0) = 1.1
        assert!((calc_knockback(10.0) - 1.1).abs() < 0.001);

        // impact = 0.0 → 0.1 + 0.0 = 0.1
        assert!((calc_knockback(0.0) - 0.1).abs() < 0.001);
    }

    #[test]
    fn is_blockable_source_checks() {
        // 无 tag → 可格挡
        assert!(is_blockable_source(&make_damage(0.5, vec![])));

        // IsFire → 不可格挡
        assert!(!is_blockable_source(&make_damage(0.5, vec![DamageTag::IsFire])));

        // Unblockable → 不可格挡
        assert!(!is_blockable_source(&make_damage(0.5, vec![DamageTag::Unblockable])));
    }

    #[test]
    fn bidirectional_knockback_directions() {
        let kb = calc_bidirectional_knockback(
            (0.0, 64.0, 0.0),  // defender
            (5.0, 64.0, 0.0),  // attacker
            (5.0, 64.0, 0.0),  // damage source = attacker
            0.15,
        );

        // defender 击退方向：背离 (5,0) → (-1, 0)
        assert!(kb.defender_knockback.0 < 0.0); // x 负方向
        assert!((kb.defender_knockback.1).abs() < 0.001); // y = 0

        // attacker 击退方向：背离 (0,0) → (1, 0)
        assert!(kb.attacker_knockback.0 > 0.0); // x 正方向

        assert!((kb.strength - 0.15).abs() < 0.001);
    }

    #[test]
    fn targeting_p1_self_revenge_highest() {
        let strategy = TargetingStrategy::new(16.0);
        let memory = AggroMemory {
            self_revenge: Some("zombie_1".into()),
            owner_attacking: Some("skeleton_1".into()),
            owner_revenge: Some("creeper_1".into()),
        };
        let candidates = vec![
            TargetCandidate {
                entity_id: "skeleton_1".into(),
                entity_type: "skeleton".into(),
                category: TargetCategory::Hostile,
                distance: 3.0,
                pos: (3.0, 64.0, 0.0),
                is_alive: true,
            },
            TargetCandidate {
                entity_id: "zombie_1".into(),
                entity_type: "zombie".into(),
                category: TargetCategory::Hostile,
                distance: 5.0,
                pos: (5.0, 64.0, 0.0),
                is_alive: true,
            },
        ];

        // P1: 自身复仇目标 zombie_1 应被选中（即使距离更远）
        let target = strategy.select_target(&candidates, &memory).unwrap();
        assert_eq!(target.entity_id, "zombie_1");
    }

    #[test]
    fn targeting_p2_owner_attacking_when_no_self_revenge() {
        let strategy = TargetingStrategy::new(16.0);
        let memory = AggroMemory {
            self_revenge: None,
            owner_attacking: Some("skeleton_1".into()),
            owner_revenge: Some("creeper_1".into()),
        };
        let candidates = vec![
            TargetCandidate {
                entity_id: "skeleton_1".into(),
                entity_type: "skeleton".into(),
                category: TargetCategory::Hostile,
                distance: 3.0,
                pos: (3.0, 64.0, 0.0),
                is_alive: true,
            },
            TargetCandidate {
                entity_id: "creeper_1".into(),
                entity_type: "creeper".into(),
                category: TargetCategory::Hostile,
                distance: 5.0,
                pos: (5.0, 64.0, 0.0),
                is_alive: true,
            },
        ];

        // P2: 主人攻击目标 skeleton_1 应被选中
        let target = strategy.select_target(&candidates, &memory).unwrap();
        assert_eq!(target.entity_id, "skeleton_1");
    }

    #[test]
    fn targeting_hostile_always_attackable() {
        let strategy = TargetingStrategy::new(16.0);
        let memory = AggroMemory::default(); // 无仇恨记忆
        let candidates = vec![TargetCandidate {
            entity_id: "zombie_1".into(),
            entity_type: "zombie".into(),
            category: TargetCategory::Hostile,
            distance: 5.0,
            pos: (5.0, 64.0, 0.0),
            is_alive: true,
        }];

        // Hostile 类别：无仇恨记忆也可攻击
        let target = strategy.select_target(&candidates, &memory).unwrap();
        assert_eq!(target.entity_id, "zombie_1");
    }

    #[test]
    fn targeting_friendly_needs_aggro() {
        let strategy = TargetingStrategy::new(16.0);
        let memory = AggroMemory::default(); // 无仇恨记忆
        let candidates = vec![TargetCandidate {
            entity_id: "player_1".into(),
            entity_type: "player".into(),
            category: TargetCategory::Player,
            distance: 3.0,
            pos: (3.0, 64.0, 0.0),
            is_alive: true,
        }];

        // Player 类别：无仇恨记忆不可攻击
        let target = strategy.select_target(&candidates, &memory);
        assert!(target.is_none());

        // 有自身复仇 → 可攻击
        let memory = AggroMemory {
            self_revenge: Some("player_1".into()),
            ..Default::default()
        };
        let target = strategy.select_target(&candidates, &memory).unwrap();
        assert_eq!(target.entity_id, "player_1");
    }

    #[test]
    fn targeting_too_far_follows_owner() {
        let strategy = TargetingStrategy::new(10.0);
        let target = TargetCandidate {
            entity_id: "zombie_1".into(),
            entity_type: "zombie".into(),
            category: TargetCategory::Hostile,
            distance: 15.0,
            pos: (15.0, 64.0, 0.0),
            is_alive: true,
        };

        // 跟随模式：以主人为中心
        let owner_pos = (0.0, 64.0, 0.0);
        assert!(strategy.is_too_far(&target, (5.0, 64.0, 0.0), Some(owner_pos)));

        // 驻守模式：以女仆为中心
        let companion_pos = (14.0, 64.0, 0.0);
        assert!(!strategy.is_too_far(&target, companion_pos, None));
    }

    #[test]
    fn effect_manager_add_and_check() {
        let mut mgr = EffectManager::new();
        assert!(!mgr.has_effect(StatusEffect::Invulnerability));

        mgr.add_effect(StatusEffect::Invulnerability, 100, 0);
        assert!(mgr.has_effect(StatusEffect::Invulnerability));
        assert!(mgr.is_invulnerable());
    }

    #[test]
    fn effect_manager_tick_expires() {
        let mut mgr = EffectManager::new();
        mgr.add_effect(StatusEffect::Speed, 2, 1);
        assert!(mgr.has_effect(StatusEffect::Speed));

        mgr.tick(); // 1 tick left
        assert!(mgr.has_effect(StatusEffect::Speed));

        mgr.tick(); // 0 tick → 过期
        assert!(!mgr.has_effect(StatusEffect::Speed));
    }

    #[test]
    fn effect_manager_on_kill_target() {
        let mut mgr = EffectManager::new();
        mgr.on_kill_target();

        // 应有渐愈 IV + 速度 II
        assert!(mgr.has_effect(StatusEffect::GradualHeal));
        assert_eq!(mgr.get_amplifier(StatusEffect::GradualHeal), Some(3)); // IV = amplifier 3

        assert!(mgr.has_effect(StatusEffect::Speed));
        assert_eq!(mgr.get_amplifier(StatusEffect::Speed), Some(1)); // II = amplifier 1
    }

    #[test]
    fn effect_manager_stun_immunity_check() {
        let mut mgr = EffectManager::new();
        assert!(!mgr.is_stun_immune());

        mgr.add_effect(StatusEffect::StunImmunity, 100, 0);
        assert!(mgr.is_stun_immune());
    }
}
