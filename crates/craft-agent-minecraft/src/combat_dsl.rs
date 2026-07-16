//! 酒狐（EpicFight_TouhouLittleMaid）战斗系统完整版 — 连招链 + 伤害体系 + 技能数据 + 生命周期。
//!
//! 深度对齐酒狐项目的核心抽象：
//! - `BehaviorSeries` + `BehaviorStep`：连招链（酒狐 CombatBehaviors.Builder + nextBehavior）
//! - `CombatCondition`：within(min,max) + custom() + health(ratio, comparator) + looping
//! - `WeaponBehaviorRegistry`：武器类别 → 行为集映射
//! - `SkillDataManager`：per-skill 二级 Map + register vs set 区分
//! - `LifecycleCallbacks`：14 个回调（含 on_hurt_target_pre/post + on_kill_target + on_init）
//! - `CallbackResult`：Cancelable 事件机制
//! - `LearnedSkills`：已学技能列表
//! - `CombatContext`：战斗上下文（stamina/attributes/owner/fight_mode）
//!
//! 设计目标：
//! 1. 战斗行为是"多段连招序列"而非单行为（BehaviorSeries + nextBehavior）
//! 2. 伤害系统完整（DamageSource tags + ValueModifier + StunType）
//! 3. 技能数据按技能隔离，register vs set 区分
//! 4. 事件可取消（格挡取消伤害、Pre 取消 hurt）

use crate::damage_source::{DamageSource, StunType};
use crate::stamina::{CombatAttributes, Stamina};
use std::any::Any;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Instant;

// ═══════════════════════════════════════════════════════════════
// HealthComparator — 血量比例比较器（酒狐 HealthPoint.Comparator）
// ═══════════════════════════════════════════════════════════════

/// 血量比例比较器（对齐酒狐 HealthPoint.Comparator）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthComparator {
    /// 血量比例 > threshold 时触发
    GreaterRatio,
    /// 血量比例 < threshold 时触发
    LessRatio,
}

// ═══════════════════════════════════════════════════════════════
// CombatCondition — 战斗行为条件（酒狐 CombatBehaviors.Builder 风格）
// ═══════════════════════════════════════════════════════════════

/// 战斗行为条件（升级版，支持 within(min,max) + custom + health_ratio + looping）。
#[derive(Clone)]
pub struct CombatCondition {
    pub cooldown_ms: u64,
    pub weight: f32,
    pub can_be_interrupted: bool,
    pub within_min: f64,
    pub within_max: f64,
    pub min_health: Option<f32>,
    pub max_health: Option<f32>,
    pub health_ratio: Option<(f32, HealthComparator)>,
    pub target_type: Option<String>,
    pub looping: bool,
    pub custom_condition: Option<Arc<dyn Fn(&CombatContext) -> bool + Send + Sync>>,
}

impl std::fmt::Debug for CombatCondition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CombatCondition")
            .field("cooldown_ms", &self.cooldown_ms)
            .field("weight", &self.weight)
            .field("can_be_interrupted", &self.can_be_interrupted)
            .field("within_min", &self.within_min)
            .field("within_max", &self.within_max)
            .field("min_health", &self.min_health)
            .field("max_health", &self.max_health)
            .field("health_ratio", &self.health_ratio)
            .field("target_type", &self.target_type)
            .field("looping", &self.looping)
            .field("custom_condition", &self.custom_condition.is_some())
            .finish()
    }
}

impl Default for CombatCondition {
    fn default() -> Self {
        Self {
            cooldown_ms: 0,
            weight: 1.0,
            can_be_interrupted: true,
            within_min: 0.0,
            within_max: 4.0,
            min_health: None,
            max_health: None,
            health_ratio: None,
            target_type: None,
            looping: false,
            custom_condition: None,
        }
    }
}

// ═══════════════════════════════════════════════════════════════
// CombatContext — 战斗上下文（酒狐 MaidPatch 状态）
// ═══════════════════════════════════════════════════════════════

/// 战斗上下文（对齐酒狐 MaidPatch 的运行时状态）。
pub struct CombatContext {
    pub health: f32,
    pub max_health: f32,
    pub distance: f64,
    pub target_type: String,
    pub stamina: f32,
    pub max_stamina: f32,
    pub is_fight_mode: bool,
    pub is_hugging: bool,
    pub is_sleeping: bool,
    pub is_sitting: bool,
    pub owner_id: Option<String>,
    pub target_owner_id: Option<String>,
    pub current_main_hand: String,
}

impl CombatContext {
    /// canExecute 守卫（酒狐 MaidSkill.canExecute：战斗模式 + 非拥抱/睡眠/坐下）。
    pub fn can_execute(&self) -> bool {
        self.is_fight_mode && !self.is_hugging && !self.is_sleeping && !self.is_sitting
    }

    /// 是否友军（酒狐 owner 保护：target 的 owner == 自己的 owner）。
    pub fn is_friendly(&self) -> bool {
        self.owner_id.is_some() && self.owner_id == self.target_owner_id
    }

    /// 血量比例。
    pub fn health_ratio(&self) -> f32 {
        if self.max_health > 0.0 {
            self.health / self.max_health
        } else {
            0.0
        }
    }
}

impl std::fmt::Debug for CombatContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CombatContext")
            .field("health", &self.health)
            .field("max_health", &self.max_health)
            .field("distance", &self.distance)
            .field("target_type", &self.target_type)
            .field("stamina", &self.stamina)
            .field("is_fight_mode", &self.is_fight_mode)
            .finish()
    }
}

// ═══════════════════════════════════════════════════════════════
// BehaviorStep — 连招段（酒狐 Behavior.nextBehavior）
// ═══════════════════════════════════════════════════════════════

/// 连招的一个步骤（对齐酒狐 Behavior）。
pub struct BehaviorStep {
    pub name: String,
    pub condition: CombatCondition,
    pub on_trigger: Option<Arc<dyn Fn(&mut CombatContext) + Send + Sync>>,
    pub last_triggered: Option<Instant>,
    pub trigger_count: u64,
}

impl BehaviorStep {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.into(),
            condition: CombatCondition::default(),
            on_trigger: None,
            last_triggered: None,
            trigger_count: 0,
        }
    }

    /// 链式：设置冷却。
    pub fn cooldown(mut self, ms: u64) -> Self {
        self.condition.cooldown_ms = ms;
        self
    }

    /// 链式：设置权重。
    pub fn weight(mut self, w: f32) -> Self {
        self.condition.weight = w;
        self
    }

    /// 链式：设置是否可中断。
    pub fn interruptible(mut self, b: bool) -> Self {
        self.condition.can_be_interrupted = b;
        self
    }

    /// 链式：设置距离区间（酒狐 withinDistance(min, max)）。
    pub fn within(mut self, min: f64, max: f64) -> Self {
        self.condition.within_min = min;
        self.condition.within_max = max;
        self
    }

    /// 链式：设置最小血量。
    pub fn min_health(mut self, h: f32) -> Self {
        self.condition.min_health = Some(h);
        self
    }

    /// 链式：设置最大血量（仅濒死时触发）。
    pub fn max_health(mut self, h: f32) -> Self {
        self.condition.max_health = Some(h);
        self
    }

    /// 链式：设置血量比例条件（酒狐 health(ratio, comparator)）。
    pub fn health_ratio(mut self, ratio: f32, cmp: HealthComparator) -> Self {
        self.condition.health_ratio = Some((ratio, cmp));
        self
    }

    /// 链式：设置目标类型限制。
    pub fn target_type(mut self, t: &str) -> Self {
        self.condition.target_type = Some(t.into());
        self
    }

    /// 链式：设置循环标志。
    pub fn looping(mut self, b: bool) -> Self {
        self.condition.looping = b;
        self
    }

    /// 链式：设置自定义条件谓词（酒狐 .custom(predicate)）。
    pub fn custom(mut self, pred: Arc<dyn Fn(&CombatContext) -> bool + Send + Sync>) -> Self {
        self.condition.custom_condition = Some(pred);
        self
    }

    /// 链式：设置触发回调（酒狐 .behavior(closure)）。
    pub fn on_trigger<F>(mut self, f: F) -> Self
    where
        F: Fn(&mut CombatContext) + Send + Sync + 'static,
    {
        self.on_trigger = Some(Arc::new(f));
        self
    }

    /// 检查是否可触发。
    pub fn can_trigger(&self, ctx: &CombatContext) -> bool {
        // 冷却检查
        if let Some(last) = self.last_triggered {
            if self.condition.cooldown_ms > 0 && last.elapsed().as_millis() < self.condition.cooldown_ms as u128 {
                return false;
            }
        }
        // 距离区间检查（酒狐 withinDistance(min, max)）
        if ctx.distance < self.condition.within_min || ctx.distance > self.condition.within_max {
            return false;
        }
        // 绝对血量检查
        if let Some(min_h) = self.condition.min_health {
            if ctx.health < min_h {
                return false;
            }
        }
        if let Some(max_h) = self.condition.max_health {
            if ctx.health > max_h {
                return false;
            }
        }
        // 血量比例检查（酒狐 health(ratio, comparator)）
        if let Some((ratio, cmp)) = self.condition.health_ratio {
            let actual = ctx.health_ratio();
            match cmp {
                HealthComparator::GreaterRatio => {
                    if actual <= ratio {
                        return false;
                    }
                }
                HealthComparator::LessRatio => {
                    if actual >= ratio {
                        return false;
                    }
                }
            }
        }
        // 目标类型检查
        if let Some(ref tt) = self.condition.target_type {
            if !ctx.target_type.contains(tt.as_str()) {
                return false;
            }
        }
        // 友军保护
        if ctx.is_friendly() {
            return false;
        }
        // 自定义条件
        if let Some(ref pred) = self.condition.custom_condition {
            if !pred(ctx) {
                return false;
            }
        }
        true
    }

    /// 标记触发并执行回调。
    pub fn trigger(&mut self, ctx: &mut CombatContext) {
        self.last_triggered = Some(Instant::now());
        self.trigger_count += 1;
        if let Some(ref f) = self.on_trigger {
            f(ctx);
        }
    }

    /// 评分。
    pub fn score(&self) -> f64 {
        let base = self.condition.weight as f64;
        if let Some(last) = self.last_triggered {
            let elapsed = last.elapsed().as_millis() as f64;
            let cooldown = self.condition.cooldown_ms as f64;
            if cooldown > 0.0 && elapsed < cooldown {
                return base * (elapsed / cooldown);
            }
        }
        base
    }
}

impl std::fmt::Debug for BehaviorStep {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BehaviorStep")
            .field("name", &self.name)
            .field("condition", &self.condition)
            .field("trigger_count", &self.trigger_count)
            .finish()
    }
}

// ═══════════════════════════════════════════════════════════════
// BehaviorSeries — 连招链（酒狐 CombatBehaviors + BehaviorSeries）
// ═══════════════════════════════════════════════════════════════

/// 连招序列（对齐酒狐 BehaviorSeries：多个 nextBehavior 串接）。
pub struct BehaviorSeries {
    pub name: String,
    pub cooldown_ms: u64,
    pub weight: f32,
    pub can_be_interrupted: bool,
    pub looping: bool,
    pub steps: Vec<BehaviorStep>,
    pub current_step: usize,
    pub last_triggered: Option<Instant>,
}

impl BehaviorSeries {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.into(),
            cooldown_ms: 0,
            weight: 1.0,
            can_be_interrupted: true,
            looping: false,
            steps: vec![],
            current_step: 0,
            last_triggered: None,
        }
    }

    /// 链式：设置冷却。
    pub fn cooldown(mut self, ms: u64) -> Self {
        self.cooldown_ms = ms;
        self
    }

    /// 链式：设置权重。
    pub fn weight(mut self, w: f32) -> Self {
        self.weight = w;
        self
    }

    /// 链式：设置是否可中断（series 级别）。
    pub fn interruptible(mut self, b: bool) -> Self {
        self.can_be_interrupted = b;
        self
    }

    /// 链式：设置循环。
    pub fn looping(mut self, b: bool) -> Self {
        self.looping = b;
        self
    }

    /// 添加连招段（酒狐 nextBehavior）。
    pub fn step(mut self, step: BehaviorStep) -> Self {
        self.steps.push(step);
        self
    }

    /// 尝试推进连招（返回当前应执行的 step 名称，None 表示连招结束/冷却中）。
    pub fn try_advance(&mut self, ctx: &mut CombatContext) -> Option<String> {
        // 冷却检查（series 级别）
        if let Some(last) = self.last_triggered {
            if self.cooldown_ms > 0 && last.elapsed().as_millis() < self.cooldown_ms as u128 {
                return None;
            }
        }
        // 检查当前 step
        if self.current_step >= self.steps.len() {
            if self.looping {
                self.current_step = 0;
            } else {
                return None;
            }
        }
        // 尝试触发当前 step
        // 先检查 can_trigger（不可变借用），再 trigger（可变借用）
        let can = self.steps.get(self.current_step).map(|s| s.can_trigger(ctx)).unwrap_or(false);
        if can {
            self.steps[self.current_step].trigger(ctx);
            let name = self.steps[self.current_step].name.clone();
            self.current_step += 1;
            self.last_triggered = Some(Instant::now());
            Some(name)
        } else {
            None
        }
    }

    /// 重置连招进度。
    pub fn reset(&mut self) {
        self.current_step = 0;
        self.last_triggered = None;
    }

    /// 连招段数量。
    pub fn step_count(&self) -> usize {
        self.steps.len()
    }
}

impl std::fmt::Debug for BehaviorSeries {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BehaviorSeries")
            .field("name", &self.name)
            .field("cooldown_ms", &self.cooldown_ms)
            .field("weight", &self.weight)
            .field("can_be_interrupted", &self.can_be_interrupted)
            .field("looping", &self.looping)
            .field("step_count", &self.steps.len())
            .field("current_step", &self.current_step)
            .finish()
    }
}

// ═══════════════════════════════════════════════════════════════
// CombatSelector — 行为选择器（从多个 BehaviorSeries 中选最优）
// ═══════════════════════════════════════════════════════════════

/// 行为选择器（从多个 BehaviorSeries 中按权重选最优）。
pub struct CombatSelector {
    series: Vec<BehaviorSeries>,
}

impl CombatSelector {
    pub fn new() -> Self {
        Self { series: vec![] }
    }

    /// 添加连招序列。
    pub fn add(&mut self, s: BehaviorSeries) -> &mut Self {
        self.series.push(s);
        self
    }

    /// 选择并推进最优连招（返回触发的 step 名称）。
    pub fn select_and_advance(&mut self, ctx: &mut CombatContext) -> Option<(String, String)> {
        // 找到权重最高的、且当前 step 可触发的 series（酒狐 CombatBehaviorPicker 风格）。
        // 不能只看 series 权重——还必须检查当前 step 的 can_trigger，
        // 否则高权重但条件不满足的 series（如 creeper_flee 对非苦力怕目标）会阻塞所有低权重 series。
        let mut best_idx: Option<(usize, f32)> = None;
        for (i, s) in self.series.iter().enumerate() {
            // series 级别冷却检查
            if let Some(last) = s.last_triggered {
                if s.cooldown_ms > 0 && last.elapsed().as_millis() < s.cooldown_ms as u128 {
                    continue;
                }
            }
            // 检查当前 step 是否可触发（跳过不能触发的 series）
            let step_idx = if s.current_step < s.steps.len() {
                s.current_step
            } else if s.looping && !s.steps.is_empty() {
                0
            } else {
                continue;
            };
            if !s.steps[step_idx].can_trigger(ctx) {
                continue;
            }
            if s.weight > best_idx.map(|(_, w)| w).unwrap_or(f32::NEG_INFINITY) {
                best_idx = Some((i, s.weight));
            }
        }
        if let Some((idx, _)) = best_idx {
            if let Some(step_name) = self.series[idx].try_advance(ctx) {
                return Some((self.series[idx].name.clone(), step_name));
            }
        }
        None
    }

    /// 重置所有连招进度（武器切换时调用，酒狐 resetAi）。
    pub fn reset_all(&mut self) {
        for s in &mut self.series {
            s.reset();
        }
    }

    /// 连招序列数量。
    pub fn series_count(&self) -> usize {
        self.series.len()
    }

    /// 获取所有连招名称。
    pub fn series_names(&self) -> Vec<&str> {
        self.series.iter().map(|s| s.name.as_str()).collect()
    }
}

impl Default for CombatSelector {
    fn default() -> Self {
        Self::new()
    }
}

/// 预设战斗选择器（参考酒狐 CombatBehaviors + mindcraft ModeController）。
pub fn default_combat_selector() -> CombatSelector {
    let mut sel = CombatSelector::new();
    // 近战 3 段连招
    sel.add(
        BehaviorSeries::new("melee_combo")
            .weight(10.0)
            .cooldown(1500)
            .interruptible(true)
            .step(BehaviorStep::new("jab").within(0.0, 3.5).weight(10.0))
            .step(BehaviorStep::new("swing").within(0.0, 3.5).weight(10.0))
            .step(BehaviorStep::new("finish").within(0.0, 3.5).weight(15.0)),
    );
    // 风筝（单步，中距离）
    sel.add(
        BehaviorSeries::new("kite")
            .weight(7.0)
            .cooldown(3000)
            .interruptible(true)
            .step(BehaviorStep::new("retreat_shot").within(5.0, 10.0).weight(7.0).min_health(10.0)),
    );
    // 撤退（单步，濒死时）
    sel.add(
        BehaviorSeries::new("retreat")
            .weight(15.0)
            .cooldown(5000)
            .interruptible(false)
            .step(BehaviorStep::new("flee").within(0.0, 15.0).weight(15.0).max_health(6.0)),
    );
    // 苦力怕特殊（单步，检测到苦力怕立即后撤）
    sel.add(
        BehaviorSeries::new("creeper_flee")
            .weight(20.0)
            .cooldown(1000)
            .interruptible(false)
            .step(BehaviorStep::new("back_away").within(0.0, 6.0).weight(20.0).target_type("creeper")),
    );
    sel
}

// ═══════════════════════════════════════════════════════════════
// WeaponBehaviorRegistry — 武器类别 → 行为集映射（酒狐 SetWeaponMotions）
// ═══════════════════════════════════════════════════════════════

/// 武器握法（酒狐 ONE_HAND / TWO_HAND）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WeaponStyle {
    OneHand,
    TwoHand,
}

/// 武器类别（酒狐 SWORD/LONGSWORD/GREATSWORD/TACHI/UCHIGATANA/SPEAR）。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum WeaponCategory {
    Sword,
    LongSword,
    GreatSword,
    Tachi,
    Uchigatana,
    Spear,
    Custom(String),
}

impl WeaponCategory {
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "sword" => Self::Sword,
            "longsword" => Self::LongSword,
            "greatsword" => Self::GreatSword,
            "tachi" => Self::Tachi,
            "uchigatana" => Self::Uchigatana,
            "spear" => Self::Spear,
            other => Self::Custom(other.into()),
        }
    }
}

/// 武器行为注册表（对齐酒狐 SetWeaponMotions + ItemAttackMotions）。
///
/// 支持三级查找：Item → (Category, Style) → 默认。
pub struct WeaponBehaviorRegistry {
    by_item: HashMap<String, CombatSelector>,
    by_category_style: HashMap<(WeaponCategory, WeaponStyle), CombatSelector>,
}

impl WeaponBehaviorRegistry {
    pub fn new() -> Self {
        Self {
            by_item: HashMap::new(),
            by_category_style: HashMap::new(),
        }
    }

    /// 按 item id 注册。
    pub fn register_item(&mut self, item_id: &str, selector: CombatSelector) {
        self.by_item.insert(item_id.into(), selector);
    }

    /// 按 (category, style) 注册。
    pub fn register_category(&mut self, category: WeaponCategory, style: WeaponStyle, selector: CombatSelector) {
        self.by_category_style.insert((category, style), selector);
    }

    /// 三级查找：Item → (Category, Style) → 默认。
    pub fn resolve(&self, item_id: &str, category: &WeaponCategory, style: WeaponStyle) -> Option<&CombatSelector> {
        self.by_item
            .get(item_id)
            .or_else(|| self.by_category_style.get(&(category.clone(), style)))
    }
}

impl Default for WeaponBehaviorRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ═══════════════════════════════════════════════════════════════
// SkillDataKey — 类型化数据键（酒狐 SkillDataKey，保留原版）
// ═══════════════════════════════════════════════════════════════

static KEY_COUNTER: AtomicU64 = AtomicU64::new(0);

/// 类型化数据键（酒狐 SkillDataKey 风格）。
pub struct SkillDataKey<T> {
    id: OnceLock<u64>,
    pub name: &'static str,
    _marker: std::marker::PhantomData<T>,
}

impl<T> SkillDataKey<T> {
    pub const fn new(name: &'static str) -> Self {
        Self {
            id: OnceLock::new(),
            name,
            _marker: std::marker::PhantomData,
        }
    }

    pub fn id(&self) -> u64 {
        *self.id.get_or_init(|| KEY_COUNTER.fetch_add(1, Ordering::Relaxed) + 1)
    }
}

impl<T> std::fmt::Debug for SkillDataKey<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SkillDataKey").field("name", &self.name).field("id", &self.id()).finish()
    }
}

// ═══════════════════════════════════════════════════════════════
// SkillDataManager — per-skill 二级 Map（酒狐 MaidPatch.SkillDataKey）
// ═══════════════════════════════════════════════════════════════

/// 技能 ID 类型。
pub type SkillId = String;

/// per-skill 类型安全数据存储（对齐酒狐 MaidPatch 的二级 Map 结构）。
///
/// 按 (skill_id, key_id) 索引，每个技能有自己的数据命名空间。
/// register 创建容器并设初值，set 仅在已注册时更新。
pub struct SkillDataManager {
    data: HashMap<SkillId, HashMap<u64, Box<dyn Any>>>,
}

impl SkillDataManager {
    pub fn new() -> Self {
        Self { data: HashMap::new() }
    }

    /// 注册数据（创建容器并设初值，酒狐 registerData）。
    pub fn register<T: Clone + 'static>(&mut self, skill_id: &str, key: &SkillDataKey<T>, value: T) {
        self.data
            .entry(skill_id.into())
            .or_default()
            .insert(key.id(), Box::new(value));
    }

    /// 设置数据（仅当已注册时更新，酒狐 setData）。
    /// 返回 true 表示成功更新，false 表示未注册。
    pub fn set<T: Clone + 'static>(&mut self, skill_id: &str, key: &SkillDataKey<T>, value: T) -> bool {
        if let Some(skill_data) = self.data.get_mut(skill_id) {
            if skill_data.contains_key(&key.id()) {
                skill_data.insert(key.id(), Box::new(value));
                return true;
            }
        }
        false
    }

    /// 获取数据（酒狐 getDataValue）。
    pub fn get<T: Clone + 'static>(&self, skill_id: &str, key: &SkillDataKey<T>) -> Option<T> {
        self.data
            .get(skill_id)
            .and_then(|sd| sd.get(&key.id()))
            .and_then(|v| v.downcast_ref::<T>())
            .cloned()
    }

    /// 获取引用。
    pub fn get_ref<T: 'static>(&self, skill_id: &str, key: &SkillDataKey<T>) -> Option<&T> {
        self.data
            .get(skill_id)
            .and_then(|sd| sd.get(&key.id()))
            .and_then(|v| v.downcast_ref::<T>())
    }

    /// 是否包含键（酒狐 hasData）。
    pub fn has(&self, skill_id: &str, key_id: u64) -> bool {
        self.data.get(skill_id).map(|sd| sd.contains_key(&key_id)).unwrap_or(false)
    }

    /// 移除某技能的所有数据（酒狐 removeData，WeaponInnateSkill.onRemove 调用）。
    pub fn remove_skill(&mut self, skill_id: &str) {
        self.data.remove(skill_id);
    }

    /// 清空所有。
    pub fn clear(&mut self) {
        self.data.clear();
    }

    /// 技能数量。
    pub fn skill_count(&self) -> usize {
        self.data.len()
    }
}

impl Default for SkillDataManager {
    fn default() -> Self {
        Self::new()
    }
}

// 预定义全局数据键
pub static HEALTH_KEY: SkillDataKey<f32> = SkillDataKey::new("health");
pub static HUNGER_KEY: SkillDataKey<u32> = SkillDataKey::new("hunger");
pub static COMBAT_MODE_KEY: SkillDataKey<String> = SkillDataKey::new("combat_mode");
pub static LAST_ATTACK_TIME_KEY: SkillDataKey<Instant> = SkillDataKey::new("last_attack_time");
pub static TARGET_ENTITY_KEY: SkillDataKey<String> = SkillDataKey::new("target_entity");
pub static STAMINA_KEY: SkillDataKey<f32> = SkillDataKey::new("stamina");
pub static MAX_STAMINA_KEY: SkillDataKey<f32> = SkillDataKey::new("max_stamina");
pub static STUN_ARMOR_KEY: SkillDataKey<f32> = SkillDataKey::new("stun_armor");
pub static COMBO_INDEX_KEY: SkillDataKey<u32> = SkillDataKey::new("combo_index");

// ═══════════════════════════════════════════════════════════════
// LearnedSkills — 已学技能列表（酒狐 MaidPatch.LearnedSkills）
// ═══════════════════════════════════════════════════════════════

/// 已学技能列表（对齐酒狐 MaidPatch.LearnedSkills）。
#[derive(Debug, Clone, Default)]
pub struct LearnedSkills {
    list: Vec<SkillId>,
}

impl LearnedSkills {
    pub fn new() -> Self {
        Self::default()
    }

    /// 学习技能（去重，返回是否新增）。
    pub fn add(&mut self, id: &str) -> bool {
        if self.contains(id) {
            return false;
        }
        self.list.push(id.into());
        true
    }

    /// 遗忘技能。
    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.list.len();
        self.list.retain(|s| s != id);
        self.list.len() < before
    }

    /// 是否已学。
    pub fn contains(&self, id: &str) -> bool {
        self.list.iter().any(|s| s == id)
    }

    /// 清空。
    pub fn clear(&mut self) {
        self.list.clear();
    }

    /// 已学数量。
    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    /// 获取所有已学技能 ID。
    pub fn all(&self) -> &[SkillId] {
        &self.list
    }
}

// ═══════════════════════════════════════════════════════════════
// CallbackResult — Cancelable 事件机制（酒狐 @Cancelable + setCanceled）
// ═══════════════════════════════════════════════════════════════

/// 回调返回值（对齐酒狐 Cancelable 事件机制）。
#[derive(Debug, Clone)]
pub enum CallbackResult {
    /// 继续后续处理器
    Continue,
    /// 取消原操作（如格挡取消伤害）
    Cancel,
    /// 取消原操作并产生新事件
    CancelWith(LifecycleEvent),
}

// ═══════════════════════════════════════════════════════════════
// LifecycleEvent + LifecycleCallbacks — 14 个回调（酒狐 MaidSkill 完整版）
// ═══════════════════════════════════════════════════════════════

/// 生命周期事件（对齐酒狐 14 个回调）。
#[derive(Debug, Clone)]
pub enum LifecycleEvent {
    Tick,
    Attack { target_type: String, distance: f64 },
    /// 造成伤害前（可取消，酒狐 MaidHurtTargetPre）
    HurtTargetPre { target: String, damage: DamageSource },
    /// 造成伤害后（酒狐 MaidHurtTargetPost）
    HurtTargetPost { target: String, damage: f32, hit_count: u32 },
    /// 被攻击时
    Hurt { damage: f32, source: String },
    /// 造成伤害时
    Damage { amount: f32, target: String },
    /// 击杀目标（酒狐 MaidKillTarget）
    KillTarget { target_type: String },
    /// 死亡时
    Death,
    /// 复活时
    Respawn,
    /// 技能初始化（酒狐 onInit）
    SkillInit { skill_id: SkillId },
    /// 主手武器切换（酒狐 MaidChangeItemOnHand）
    ChangeMainHand { old_item: String, new_item: String },
    /// 装备变更
    EquipChange { slot: String, item: String },
    /// 物品栏变更
    InventoryChange { item: String, count: i32 },
    /// 目标切换
    TargetSwitch { from: Option<String>, to: Option<String> },
    /// 模式切换
    ModeSwitch { from: String, to: String },
}

/// 生命周期回调 trait（酒狐 MaidSkill 14 个回调）。
pub trait LifecycleCallbacks: Send + Sync {
    fn on_tick(&self, _ctx: &CombatContext, _data: &SkillDataManager) -> CallbackResult {
        CallbackResult::Continue
    }

    fn on_attack(&self, _ctx: &CombatContext, _data: &SkillDataManager) -> CallbackResult {
        CallbackResult::Continue
    }

    /// 造成伤害前（可取消，酒狐 MaidHurtTargetPre）。
    fn on_hurt_target_pre(&self, _ctx: &CombatContext, _damage: &mut DamageSource, _data: &SkillDataManager) -> CallbackResult {
        CallbackResult::Continue
    }

    /// 造成伤害后（酒狐 MaidHurtTargetPost）。
    fn on_hurt_target_post(&self, _ctx: &CombatContext, _target: &str, _damage: f32, _hit_count: u32, _data: &SkillDataManager) -> CallbackResult {
        CallbackResult::Continue
    }

    fn on_hurt(&self, _ctx: &CombatContext, _damage: f32, _source: &str, _data: &SkillDataManager) -> CallbackResult {
        CallbackResult::Continue
    }

    fn on_damage(&self, _ctx: &CombatContext, _amount: f32, _target: &str, _data: &SkillDataManager) -> CallbackResult {
        CallbackResult::Continue
    }

    /// 击杀目标（酒狐 MaidKillTarget）。
    fn on_kill_target(&self, _ctx: &CombatContext, _target_type: &str, _data: &SkillDataManager) -> CallbackResult {
        CallbackResult::Continue
    }

    fn on_death(&self, _ctx: &CombatContext, _data: &SkillDataManager) -> CallbackResult {
        CallbackResult::Continue
    }

    fn on_respawn(&self, _ctx: &CombatContext, _data: &SkillDataManager) -> CallbackResult {
        CallbackResult::Continue
    }

    /// 技能初始化（酒狐 onInit，注册 SkillData）。
    fn on_init(&self, _skill_id: &str, _data: &mut SkillDataManager) {}

    /// 主手武器切换（酒狐 MaidChangeItemOnHand，触发武器固有技能重注册）。
    fn on_change_main_hand(&self, _ctx: &CombatContext, _old_item: &str, _new_item: &str, _data: &mut SkillDataManager) {}

    fn on_equip_change(&self, _ctx: &CombatContext, _slot: &str, _item: &str, _data: &SkillDataManager) -> CallbackResult {
        CallbackResult::Continue
    }

    fn on_inventory_change(&self, _ctx: &CombatContext, _item: &str, _count: i32, _data: &SkillDataManager) -> CallbackResult {
        CallbackResult::Continue
    }

    fn on_target_switch(&self, _ctx: &CombatContext, _from: &Option<String>, _to: &Option<String>, _data: &SkillDataManager) -> CallbackResult {
        CallbackResult::Continue
    }

    fn on_mode_switch(&self, _ctx: &CombatContext, _from: &str, _to: &str, _data: &SkillDataManager) -> CallbackResult {
        CallbackResult::Continue
    }
}

// ═══════════════════════════════════════════════════════════════
// SkillRegistry — 技能注册表（酒狐 MaidSkillManager）
// ═══════════════════════════════════════════════════════════════

/// 技能注册表（对齐酒狐 MaidSkillManager）。
pub struct SkillRegistry {
    skills: HashMap<SkillId, Arc<dyn LifecycleCallbacks>>,
    weapon_innate: HashMap<String, SkillId>,
}

impl SkillRegistry {
    pub fn new() -> Self {
        Self {
            skills: HashMap::new(),
            weapon_innate: HashMap::new(),
        }
    }

    /// 注册技能。
    pub fn register(&mut self, skill_id: &str, handler: Arc<dyn LifecycleCallbacks>) {
        self.skills.insert(skill_id.into(), handler);
    }

    /// 注册武器固有技能（item → skill_id）。
    pub fn register_weapon_innate(&mut self, item_id: &str, skill_id: &str) {
        self.weapon_innate.insert(item_id.into(), skill_id.into());
    }

    /// 查找技能。
    pub fn get(&self, skill_id: &str) -> Option<&Arc<dyn LifecycleCallbacks>> {
        self.skills.get(skill_id)
    }

    /// 查找武器的固有技能 ID。
    pub fn weapon_innate_for(&self, item_id: &str) -> Option<&SkillId> {
        self.weapon_innate.get(item_id)
    }

    /// 已注册技能数量。
    pub fn len(&self) -> usize {
        self.skills.len()
    }

    pub fn is_empty(&self) -> bool {
        self.skills.is_empty()
    }
}

impl Default for SkillRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ═══════════════════════════════════════════════════════════════
// EventBus — 事件分发（酒狐 EventBus，按已学技能过滤）
// ═══════════════════════════════════════════════════════════════

/// 事件分发器（对齐酒狐 EventBus，按已学技能过滤 + canExecute 守卫）。
pub struct EventBus {
    registry: SkillRegistry,
}

impl EventBus {
    pub fn new(registry: SkillRegistry) -> Self {
        Self { registry }
    }

    /// 分发事件（遍历已学技能，canExecute 守卫，支持 Cancelable）。
    pub fn dispatch(
        &self,
        event: &LifecycleEvent,
        ctx: &CombatContext,
        data: &SkillDataManager,
        learned: &LearnedSkills,
    ) -> (CallbackResult, Vec<LifecycleEvent>) {
        let mut new_events = vec![];
        for skill_id in learned.all() {
            // canExecute 守卫
            if !ctx.can_execute() {
                continue;
            }
            if let Some(handler) = self.registry.get(skill_id) {
                let result = match event {
                    LifecycleEvent::Tick => handler.on_tick(ctx, data),
                    LifecycleEvent::Attack { target_type, distance } => {
                        let mut ctx2 = CombatContext { distance: *distance, target_type: target_type.clone(), ..clone_ctx(ctx) };
                        let _ = &mut ctx2;
                        handler.on_attack(ctx, data)
                    }
                    LifecycleEvent::HurtTargetPre { target, damage } => {
                        let mut dmg = damage.clone();
                        let r = handler.on_hurt_target_pre(ctx, &mut dmg, data);
                        // 注意：dmg 的修改在此简化版中不回写（需要 &mut DamageSource 才能回写）
                        let _ = (target, dmg);
                        r
                    }
                    LifecycleEvent::HurtTargetPost { target, damage, hit_count } => {
                        handler.on_hurt_target_post(ctx, target, *damage, *hit_count, data)
                    }
                    LifecycleEvent::Hurt { damage, source } => handler.on_hurt(ctx, *damage, source, data),
                    LifecycleEvent::Damage { amount, target } => handler.on_damage(ctx, *amount, target, data),
                    LifecycleEvent::KillTarget { target_type } => handler.on_kill_target(ctx, target_type, data),
                    LifecycleEvent::Death => handler.on_death(ctx, data),
                    LifecycleEvent::Respawn => handler.on_respawn(ctx, data),
                    LifecycleEvent::SkillInit { skill_id: sid } => {
                        // on_init 需要 &mut data，这里跳过（应在注册时单独调用）
                        let _ = sid;
                        CallbackResult::Continue
                    }
                    LifecycleEvent::ChangeMainHand { .. } => {
                        // on_change_main_hand 需要 &mut data，这里跳过
                        CallbackResult::Continue
                    }
                    LifecycleEvent::EquipChange { slot, item } => handler.on_equip_change(ctx, slot, item, data),
                    LifecycleEvent::InventoryChange { item, count } => handler.on_inventory_change(ctx, item, *count, data),
                    LifecycleEvent::TargetSwitch { from, to } => handler.on_target_switch(ctx, from, to, data),
                    LifecycleEvent::ModeSwitch { from, to } => handler.on_mode_switch(ctx, from, to, data),
                };
                match result {
                    CallbackResult::Continue => {}
                    CallbackResult::Cancel => return (CallbackResult::Cancel, new_events),
                    CallbackResult::CancelWith(e) => {
                        new_events.push(e);
                        return (CallbackResult::Cancel, new_events);
                    }
                }
            }
        }
        (CallbackResult::Continue, new_events)
    }

    /// 注册表引用。
    pub fn registry(&self) -> &SkillRegistry {
        &self.registry
    }
}

/// 克隆 CombatContext（内部辅助）。
fn clone_ctx(ctx: &CombatContext) -> CombatContext {
    CombatContext {
        health: ctx.health,
        max_health: ctx.max_health,
        distance: ctx.distance,
        target_type: ctx.target_type.clone(),
        stamina: ctx.stamina,
        max_stamina: ctx.max_stamina,
        is_fight_mode: ctx.is_fight_mode,
        is_hugging: ctx.is_hugging,
        is_sleeping: ctx.is_sleeping,
        is_sitting: ctx.is_sitting,
        owner_id: ctx.owner_id.clone(),
        target_owner_id: ctx.target_owner_id.clone(),
        current_main_hand: ctx.current_main_hand.clone(),
    }
}

// ═══════════════════════════════════════════════════════════════
// 测试
// ═══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    fn make_ctx(health: f32, distance: f64, target: &str) -> CombatContext {
        CombatContext {
            health,
            max_health: 20.0,
            distance,
            target_type: target.into(),
            stamina: 20.0,
            max_stamina: 20.0,
            is_fight_mode: true,
            is_hugging: false,
            is_sleeping: false,
            is_sitting: false,
            owner_id: None,
            target_owner_id: None,
            current_main_hand: "sword".into(),
        }
    }

    #[test]
    fn behavior_step_within_min_max() {
        let step = BehaviorStep::new("jab").within(3.0, 5.0);
        // 在区间内
        assert!(step.can_trigger(&make_ctx(20.0, 4.0, "zombie")));
        // 太近
        assert!(!step.can_trigger(&make_ctx(20.0, 2.0, "zombie")));
        // 太远
        assert!(!step.can_trigger(&make_ctx(20.0, 6.0, "zombie")));
    }

    #[test]
    fn behavior_step_health_ratio() {
        let step = BehaviorStep::new("bloodlust").health_ratio(0.5, HealthComparator::GreaterRatio);
        // 血量 > 50%
        assert!(step.can_trigger(&make_ctx(15.0, 3.0, "zombie")));
        // 血量 = 50% （不满足 > 0.5）
        assert!(!step.can_trigger(&make_ctx(10.0, 3.0, "zombie")));
        // 血量 < 50%
        assert!(!step.can_trigger(&make_ctx(5.0, 3.0, "zombie")));
    }

    #[test]
    fn behavior_step_custom_condition() {
        let step = BehaviorStep::new("special")
            .within(0.0, 10.0)
            .custom(Arc::new(|ctx| ctx.stamina > 10.0));
        assert!(step.can_trigger(&make_ctx(20.0, 5.0, "zombie")));
    }

    #[test]
    fn behavior_step_on_trigger_callback() {
        let mut step = BehaviorStep::new("buff").on_trigger(|_ctx| {
            // 副作用回调
        });
        let mut ctx = make_ctx(20.0, 3.0, "zombie");
        assert!(step.can_trigger(&ctx));
        // trigger 应执行回调
        step.trigger(&mut ctx);
    }

    #[test]
    fn behavior_series_combo_chain() {
        let series = BehaviorSeries::new("combo")
            .weight(10.0)
            .cooldown(0)
            .step(BehaviorStep::new("hit1").within(0.0, 4.0))
            .step(BehaviorStep::new("hit2").within(0.0, 4.0))
            .step(BehaviorStep::new("hit3").within(0.0, 4.0));

        assert_eq!(series.step_count(), 3);
    }

    #[test]
    fn behavior_series_looping() {
        let mut series = BehaviorSeries::new("loop_combo")
            .weight(10.0)
            .cooldown(0)
            .looping(true)
            .step(BehaviorStep::new("hit").within(0.0, 4.0));

        let mut ctx = make_ctx(20.0, 3.0, "zombie");
        // 连续触发多次，looping 应循环
        for _ in 0..5 {
            let r = series.try_advance(&mut ctx);
            assert!(r.is_some(), "looping series should always advance when in range");
        }
    }

    #[test]
    fn behavior_series_non_looping_exhausts() {
        let mut series = BehaviorSeries::new("finite")
            .weight(10.0)
            .cooldown(0)
            .step(BehaviorStep::new("only").within(0.0, 4.0));

        let mut ctx = make_ctx(20.0, 3.0, "zombie");
        assert!(series.try_advance(&mut ctx).is_some());
        // 第二次应返回 None（连招结束，非 looping）
        assert!(series.try_advance(&mut ctx).is_none());
    }

    #[test]
    fn combat_selector_picks_highest_weight() {
        let mut sel = CombatSelector::new();
        sel.add(BehaviorSeries::new("weak").weight(1.0).step(BehaviorStep::new("h").within(0.0, 10.0)));
        sel.add(BehaviorSeries::new("strong").weight(10.0).step(BehaviorStep::new("h").within(0.0, 10.0)));
        let mut ctx = make_ctx(20.0, 5.0, "zombie");
        let r = sel.select_and_advance(&mut ctx);
        assert!(r.is_some());
        assert_eq!(r.unwrap().0, "strong");
    }

    #[test]
    fn combat_selector_creeper_flee() {
        let mut sel = default_combat_selector();
        let mut ctx = make_ctx(20.0, 5.0, "creeper");
        let r = sel.select_and_advance(&mut ctx);
        assert!(r.is_some());
        assert_eq!(r.unwrap().0, "creeper_flee");
    }

    #[test]
    fn combat_selector_retreat_low_health() {
        let mut sel = default_combat_selector();
        let mut ctx = make_ctx(4.0, 3.0, "zombie");
        let r = sel.select_and_advance(&mut ctx);
        assert!(r.is_some());
        assert_eq!(r.unwrap().0, "retreat");
    }

    #[test]
    fn skill_data_manager_per_skill_isolation() {
        let mut mgr = SkillDataManager::new();
        // 技能 A 的数据
        mgr.register("skill_a", &HEALTH_KEY, 18.0f32);
        mgr.register("skill_a", &STAMINA_KEY, 15.0f32);
        // 技能 B 的数据
        mgr.register("skill_b", &HEALTH_KEY, 10.0f32);

        assert_eq!(mgr.get("skill_a", &HEALTH_KEY), Some(18.0f32));
        assert_eq!(mgr.get("skill_b", &HEALTH_KEY), Some(10.0f32));
        assert_eq!(mgr.get("skill_a", &STAMINA_KEY), Some(15.0f32));
        assert_eq!(mgr.get("skill_b", &STAMINA_KEY), None); // B 没有 stamina

        assert_eq!(mgr.skill_count(), 2);
        mgr.remove_skill("skill_a");
        assert_eq!(mgr.skill_count(), 1);
        assert_eq!(mgr.get("skill_a", &HEALTH_KEY), None);
    }

    #[test]
    fn skill_data_manager_register_vs_set() {
        let mut mgr = SkillDataManager::new();
        // set 未注册的键应失败
        assert!(!mgr.set("skill_x", &HEALTH_KEY, 10.0f32));
        // register 后 set 才能成功
        mgr.register("skill_x", &HEALTH_KEY, 20.0f32);
        assert!(mgr.set("skill_x", &HEALTH_KEY, 15.0f32));
        assert_eq!(mgr.get("skill_x", &HEALTH_KEY), Some(15.0f32));
    }

    #[test]
    fn learned_skills_add_remove() {
        let mut ls = LearnedSkills::new();
        assert!(ls.add("blade_clash"));
        assert!(!ls.add("blade_clash")); // 去重
        assert!(ls.contains("blade_clash"));
        assert_eq!(ls.len(), 1);
        assert!(ls.remove("blade_clash"));
        assert!(!ls.contains("blade_clash"));
        assert!(ls.is_empty());
    }

    #[test]
    fn combat_context_can_execute_guard() {
        let mut ctx = make_ctx(20.0, 3.0, "zombie");
        assert!(ctx.can_execute());
        ctx.is_fight_mode = false;
        assert!(!ctx.can_execute());
        ctx.is_fight_mode = true;
        ctx.is_sleeping = true;
        assert!(!ctx.can_execute());
    }

    #[test]
    fn combat_context_friendly_fire_check() {
        let mut ctx = make_ctx(20.0, 3.0, "zombie");
        assert!(!ctx.is_friendly());
        ctx.owner_id = Some("player1".into());
        ctx.target_owner_id = Some("player1".into());
        assert!(ctx.is_friendly());
    }

    #[test]
    fn weapon_behavior_registry_three_level_lookup() {
        let mut registry = WeaponBehaviorRegistry::new();
        let sel_default = CombatSelector::new();
        let sel_sword = CombatSelector::new();
        let sel_diamond = CombatSelector::new();

        registry.register_category(WeaponCategory::Sword, WeaponStyle::OneHand, sel_default);
        registry.register_item("minecraft:diamond_sword", sel_diamond);
        let _ = sel_sword;

        // item 级查找
        assert!(registry.resolve("minecraft:diamond_sword", &WeaponCategory::Sword, WeaponStyle::OneHand).is_some());
        // category 级查找
        assert!(registry.resolve("minecraft:iron_sword", &WeaponCategory::Sword, WeaponStyle::OneHand).is_some());
        // 不存在
        assert!(registry.resolve("minecraft:bow", &WeaponCategory::Custom("bow".into()), WeaponStyle::OneHand).is_none());
    }

    #[test]
    fn skill_registry_register_and_lookup() {
        let mut registry = SkillRegistry::new();
        struct DummySkill;
        impl LifecycleCallbacks for DummySkill {}
        registry.register("dummy", Arc::new(DummySkill));
        registry.register_weapon_innate("minecraft:diamond_sword", "diamond_innate");

        assert!(registry.get("dummy").is_some());
        assert_eq!(registry.weapon_innate_for("minecraft:diamond_sword"), Some(&"diamond_innate".to_string()));
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn callback_result_cancel_stops_dispatch() {
        struct CancelOnHurt;
        impl LifecycleCallbacks for CancelOnHurt {
            fn on_hurt(&self, _ctx: &CombatContext, _damage: f32, _source: &str, _data: &SkillDataManager) -> CallbackResult {
                CallbackResult::Cancel
            }
        }

        let mut registry = SkillRegistry::new();
        registry.register("blocker", Arc::new(CancelOnHurt));
        let bus = EventBus::new(registry);
        let mut learned = LearnedSkills::new();
        learned.add("blocker");
        let ctx = make_ctx(20.0, 3.0, "zombie");
        let data = SkillDataManager::new();
        let event = LifecycleEvent::Hurt { damage: 5.0, source: "zombie".into() };
        let (result, _) = bus.dispatch(&event, &ctx, &data, &learned);
        assert!(matches!(result, CallbackResult::Cancel));
    }
}
