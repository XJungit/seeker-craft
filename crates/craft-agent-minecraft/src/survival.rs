//! Numen 生存自治层 — 零额外 LLM 调用的自主行为通知 + 结构化失败 + 反垂直挖坑评分。
//!
//! 参考 Numen 项目的三个核心抽象：
//! - `SurvivalJournal`：有界环形缓冲（6 条），零额外 LLM 调用通知自主行为
//! - `MiningEconomics`：反垂直挖坑评分（distance + 3.0 × depth_penalty）
//! - `FailureType`：13 个结构化失败类型（in-ladder vs kick-back-to-LLM）
//!
//! 设计目标：
//! 1. 自主行为（self_preservation/unstuck/food）不触发额外 LLM 调用，仅写入 journal
//! 2. 挖坑评分避免 agent 越挖越深无法回程
//! 3. 失败类型区分"阶梯内重试"和"踢回 LLM 重新决策"

use std::collections::VecDeque;
use std::time::{Duration, Instant};

// ═══════════════════════════════════════════════════════════════
// SurvivalJournal — 有界环形缓冲（Numen 风格）
// ═══════════════════════════════════════════════════════════════

/// 生存事件类型（自主行为通知，零 LLM 调用）。
#[derive(Debug, Clone, PartialEq)]
pub enum SurvivalEvent {
    /// 自我保护触发（血量低于阈值）
    SelfPreservation { health: f32, trigger: String },
    /// 卡住自救（move_stuck 检测）
    Unstuck {
        position: (f64, f64, f64),
        attempt: u32,
    },
    /// 自动进食（饥饿值低于阈值）
    AutoEat { hunger: u32, item: String },
    /// 自动防御（敌对实体接近）
    SelfDefense { entity_type: String, distance: f64 },
    /// 自动撤退（濒死）
    Retreat { health: f32, from: (f64, f64, f64) },
    /// 自动拾取（掉落物接近）
    ItemCollecting { item: String, count: u32 },
    /// 自动放火把（亮度低于阈值）
    TorchPlacing {
        position: (i32, i32, i32),
        light: u32,
    },
    /// 模式切换通知
    ModeSwitch {
        from: String,
        to: String,
        reason: String,
    },
}

impl SurvivalEvent {
    /// 渲染为简洁的单行文本（注入到 LLM 上下文）。
    pub fn to_brief(&self) -> String {
        match self {
            Self::SelfPreservation { health, trigger } => {
                format!("[survival] self_preservation: health={health:.1} trigger={trigger}")
            }
            Self::Unstuck { position, attempt } => {
                format!(
                    "[survival] unstuck: pos=({:.1},{:.1},{:.1}) attempt={attempt}",
                    position.0, position.1, position.2
                )
            }
            Self::AutoEat { hunger, item } => {
                format!("[survival] auto_eat: hunger={hunger} item={item}")
            }
            Self::SelfDefense {
                entity_type,
                distance,
            } => {
                format!("[survival] self_defense: entity={entity_type} dist={distance:.1}m")
            }
            Self::Retreat { health, from } => {
                format!(
                    "[survival] retreat: health={health:.1} from=({:.1},{:.1},{:.1})",
                    from.0, from.1, from.2
                )
            }
            Self::ItemCollecting { item, count } => {
                format!("[survival] item_collecting: {item} x{count}")
            }
            Self::TorchPlacing { position, light } => {
                format!(
                    "[survival] torch_placed: ({},{},{}) light_was={light}",
                    position.0, position.1, position.2
                )
            }
            Self::ModeSwitch { from, to, reason } => {
                format!("[survival] mode_switch: {from}→{to} reason={reason}")
            }
        }
    }
}

/// 有界环形缓冲日志（Numen SurvivalJournal 风格）。
///
/// 容量 6 条（Numen 原版值），超出后丢弃最旧。零额外 LLM 调用——
/// 仅在下一轮 perceive 时作为上下文前缀注入，避免每条事件单独唤醒 LLM。
#[derive(Debug, Clone)]
pub struct SurvivalJournal {
    events: VecDeque<SurvivalEvent>,
    capacity: usize,
    /// 事件计数（不重置，用于统计自主行为频率）。
    pub total_events: u64,
    /// 最近一次事件时间（用于退避判断）。
    pub last_event_at: Option<Instant>,
}

impl Default for SurvivalJournal {
    fn default() -> Self {
        Self::new(6)
    }
}

impl SurvivalJournal {
    pub fn new(capacity: usize) -> Self {
        Self {
            events: VecDeque::with_capacity(capacity),
            capacity,
            total_events: 0,
            last_event_at: None,
        }
    }

    /// 记录事件（环形覆盖最旧）。
    pub fn record(&mut self, event: SurvivalEvent) {
        if self.events.len() >= self.capacity {
            self.events.pop_front();
        }
        self.events.push_back(event);
        self.total_events += 1;
        self.last_event_at = Some(Instant::now());
    }

    /// 渲染为多行文本（注入 LLM 上下文前缀）。
    pub fn render(&self) -> String {
        if self.events.is_empty() {
            return String::new();
        }
        let mut out = String::from("[Recent autonomous events]\n");
        for e in &self.events {
            out.push_str(&format!("  {}\n", e.to_brief()));
        }
        out
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// 当前条数。
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// 清空（clear_chat 时调用）。
    pub fn clear(&mut self) {
        self.events.clear();
    }

    /// 距上次事件的间隔（用于退避判断，避免同 tick 连续触发）。
    pub fn elapsed_since_last(&self) -> Option<Duration> {
        self.last_event_at.map(|t| t.elapsed())
    }

    /// 是否在退避期内（默认 2 秒，避免抖动）。
    pub fn in_backoff(&self, backoff: Duration) -> bool {
        self.elapsed_since_last()
            .map(|d| d < backoff)
            .unwrap_or(false)
    }
}

// ═══════════════════════════════════════════════════════════════
// MiningEconomics — 反垂直挖坑评分（Numen 风格）
// ═══════════════════════════════════════════════════════════════

/// 挖掘候选评分（越低越优先）。
///
/// Numen 公式：score = distance + 3.0 × depth_penalty
/// depth_penalty = 垂直挖坑深度（player.y - block.y），惩罚"越挖越深无法回程"。
#[derive(Debug, Clone)]
pub struct MiningCandidate {
    pub block_id: String,
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub distance: f64,
    pub depth: i32,
    pub score: f64,
}

impl MiningCandidate {
    /// 计算评分（越低越优先）。
    /// distance: 玩家到方块的水平距离
    /// depth: 玩家 y - 方块 y（>0 表示在玩家下方，挖坑风险）
    pub fn score(distance: f64, depth: i32) -> f64 {
        let depth_penalty = if depth > 0 { depth as f64 } else { 0.0 };
        distance + 3.0 * depth_penalty
    }

    pub fn new(
        block_id: String,
        x: i32,
        y: i32,
        z: i32,
        player_x: f64,
        player_y: f64,
        player_z: f64,
    ) -> Self {
        let dx = x as f64 - player_x;
        let dz = z as f64 - player_z;
        let distance = (dx * dx + dz * dz).sqrt();
        let depth = (player_y - y as f64).round() as i32;
        let score = Self::score(distance, depth);
        Self {
            block_id,
            x,
            y,
            z,
            distance,
            depth,
            score,
        }
    }
}

/// 从候选列表中选出最优（评分最低）的方块。
pub fn pick_best_candidate(candidates: Vec<MiningCandidate>) -> Option<MiningCandidate> {
    candidates.into_iter().min_by(|a, b| {
        a.score
            .partial_cmp(&b.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    })
}

/// 过滤掉评分过高的候选（避免挖太深的坑）。
pub fn filter_by_score(candidates: Vec<MiningCandidate>, max_score: f64) -> Vec<MiningCandidate> {
    candidates
        .into_iter()
        .filter(|c| c.score <= max_score)
        .collect()
}

// ═══════════════════════════════════════════════════════════════
// FailureType — 13 个结构化失败类型（Numen 风格）
// ═══════════════════════════════════════════════════════════════

/// 结构化失败类型。
///
/// 区分两种处理路径：
/// - `InLadder`：阶梯内重试（同目标不同策略，不踢回 LLM）
/// - `KickBackToLlm`：踢回 LLM 重新决策（无法自主恢复）
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FailureType {
    // ═══ InLadder（阶梯内重试） ═══
    /// 路径被阻挡（可尝试跳跃/绕行）
    PathBlocked,
    /// 目标暂时不可达（可等待/换路径）
    TemporarilyUnreachable,
    /// 物品栏已满（可丢弃/整理）
    InventoryFull,
    /// 工具不合适（可换工具）
    WrongTool,
    /// 距离太远（可走近）
    TooFar,
    /// 方块已被破坏（可找下一个）
    BlockAlreadyBroken,
    /// 实体已消失（可找下一个）
    EntityGone,
    /// 容器已关闭（可重新打开）
    ContainerClosed,

    // ═══ KickBackToLlm（踢回 LLM） ═══
    /// 目标完全不可达（需要 LLM 重新规划）
    CompletelyUnreachable,
    /// 缺少关键材料（需要 LLM 规划采集）
    MissingMaterial,
    /// 无可用工具（需要 LLM 规划制作）
    NoTool,
    /// 未知错误（需要 LLM 诊断）
    Unknown,
    /// 被攻击中断（需要 LLM 决策战斗/逃跑）
    InterruptedByCombat,
}

impl FailureType {
    /// 是否可以阶梯内重试（不踢回 LLM）。
    pub fn is_in_ladder(&self) -> bool {
        matches!(
            self,
            FailureType::PathBlocked
                | FailureType::TemporarilyUnreachable
                | FailureType::InventoryFull
                | FailureType::WrongTool
                | FailureType::TooFar
                | FailureType::BlockAlreadyBroken
                | FailureType::EntityGone
                | FailureType::ContainerClosed
        )
    }

    /// 是否需要踢回 LLM 重新决策。
    pub fn is_kick_back(&self) -> bool {
        !self.is_in_ladder()
    }

    /// 渲染为 LLM 友好的文本。
    pub fn to_llm_text(&self, context: &str) -> String {
        match self {
            Self::PathBlocked => format!("Path blocked ({context}) — retrying with jump/detour"),
            Self::TemporarilyUnreachable => {
                format!("Temporarily unreachable ({context}) — waiting/repathing")
            }
            Self::InventoryFull => {
                format!("Inventory full ({context}) — discarding low-value items")
            }
            Self::WrongTool => format!("Wrong tool ({context}) — switching tool"),
            Self::TooFar => format!("Too far ({context}) — moving closer"),
            Self::BlockAlreadyBroken => format!("Block already broken ({context}) — finding next"),
            Self::EntityGone => format!("Entity gone ({context}) — finding next"),
            Self::ContainerClosed => format!("Container closed ({context}) — reopening"),
            Self::CompletelyUnreachable => {
                format!("CRITICAL: Cannot reach target ({context}) — needs LLM replanning")
            }
            Self::MissingMaterial => {
                format!("CRITICAL: Missing material ({context}) — needs LLM to plan gathering")
            }
            Self::NoTool => {
                format!("CRITICAL: No tool available ({context}) — needs LLM to plan crafting")
            }
            Self::Unknown => format!("CRITICAL: Unknown error ({context}) — needs LLM diagnosis"),
            Self::InterruptedByCombat => format!(
                "CRITICAL: Interrupted by combat ({context}) — needs LLM fight/flight decision"
            ),
        }
    }
}

/// 失败记录（含上下文 + 时间戳 + 重试次数）。
#[derive(Debug, Clone)]
pub struct FailureRecord {
    pub failure_type: FailureType,
    pub context: String,
    pub timestamp: Instant,
    pub retry_count: u32,
}

impl FailureRecord {
    pub fn new(failure_type: FailureType, context: String) -> Self {
        Self {
            failure_type,
            context,
            timestamp: Instant::now(),
            retry_count: 0,
        }
    }

    /// 增加重试次数。
    pub fn increment_retry(&mut self) {
        self.retry_count += 1;
    }

    /// 是否超过最大重试次数（in_ladder 类型的上限是 3 次）。
    pub fn should_kick_back(&self) -> bool {
        if self.failure_type.is_kick_back() {
            return true;
        }
        self.retry_count > 3
    }

    /// 渲染为 LLM 友好的文本。
    pub fn to_llm_text(&self) -> String {
        self.failure_type.to_llm_text(&self.context)
    }
}

/// 失败追踪器（同目标多次失败后踢回 LLM）。
#[derive(Debug, Default)]
pub struct FailureTracker {
    /// 最近一次失败记录（按目标 key）。
    recent: std::collections::HashMap<String, FailureRecord>,
}

impl FailureTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// 记录失败。返回 true 表示应踢回 LLM。
    pub fn record(&mut self, target_key: &str, failure_type: FailureType, context: String) -> bool {
        let record = self
            .recent
            .entry(target_key.to_string())
            .or_insert_with(|| FailureRecord::new(failure_type.clone(), context.clone()));
        if record.failure_type != failure_type {
            *record = FailureRecord::new(failure_type.clone(), context);
        } else {
            record.increment_retry();
        }
        record.should_kick_back()
    }

    /// 清除目标的失败记录（成功后调用）。
    pub fn clear(&mut self, target_key: &str) {
        self.recent.remove(target_key);
    }

    /// 清空所有失败记录。
    pub fn clear_all(&mut self) {
        self.recent.clear();
    }

    /// 获取目标的失败记录。
    pub fn get(&self, target_key: &str) -> Option<&FailureRecord> {
        self.recent.get(target_key)
    }
}

// ═══════════════════════════════════════════════════════════════
// 测试
// ═══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn survival_journal_ring_buffer() {
        let mut j = SurvivalJournal::new(3);
        j.record(SurvivalEvent::AutoEat {
            hunger: 6,
            item: "bread".into(),
        });
        j.record(SurvivalEvent::SelfDefense {
            entity_type: "zombie".into(),
            distance: 3.0,
        });
        j.record(SurvivalEvent::Unstuck {
            position: (1.0, 64.0, 2.0),
            attempt: 1,
        });
        assert_eq!(j.len(), 3);
        // 第 4 条应丢弃最旧
        j.record(SurvivalEvent::Retreat {
            health: 4.0,
            from: (1.0, 64.0, 2.0),
        });
        assert_eq!(j.len(), 3);
        assert_eq!(j.total_events, 4);
        // 最旧（AutoEat）应被丢弃
        let rendered = j.render();
        assert!(!rendered.contains("auto_eat"));
        assert!(rendered.contains("self_defense"));
    }

    #[test]
    fn mining_economics_penalizes_depth() {
        // 同样距离，更深的方块评分更高（更低优先级）
        let shallow = MiningCandidate::new("coal_ore".into(), 0, 60, 0, 5.0, 64.0, 0.0);
        let deep = MiningCandidate::new("coal_ore".into(), 0, 40, 0, 5.0, 64.0, 0.0);
        assert!(
            shallow.score < deep.score,
            "shallow should have lower score (higher priority)"
        );
        assert!(
            deep.score - shallow.score > 3.0 * 20.0 - 0.1,
            "depth penalty should be ~3.0 * depth_diff"
        );
    }

    #[test]
    fn failure_type_in_ladder_vs_kick_back() {
        assert!(FailureType::PathBlocked.is_in_ladder());
        assert!(!FailureType::PathBlocked.is_kick_back());
        assert!(FailureType::MissingMaterial.is_kick_back());
        assert!(!FailureType::MissingMaterial.is_in_ladder());
    }

    #[test]
    fn failure_tracker_kicks_back_after_3_retries() {
        let mut t = FailureTracker::new();
        // 前 3 次 in_ladder 失败不应踢回
        assert!(!t.record(
            "oak_log",
            FailureType::PathBlocked,
            "blocked by dirt".into()
        ));
        assert!(!t.record(
            "oak_log",
            FailureType::PathBlocked,
            "blocked by dirt".into()
        ));
        assert!(!t.record(
            "oak_log",
            FailureType::PathBlocked,
            "blocked by dirt".into()
        ));
        // 第 4 次应踢回
        assert!(t.record(
            "oak_log",
            FailureType::PathBlocked,
            "blocked by dirt".into()
        ));
        // kick_back 类型应立即踢回
        assert!(t.record(
            "diamond_ore",
            FailureType::MissingMaterial,
            "need iron pickaxe".into()
        ));
    }

    #[test]
    fn pick_best_candidate_returns_lowest_score() {
        let candidates = vec![
            MiningCandidate::new("coal_ore".into(), 0, 60, 0, 5.0, 64.0, 0.0),
            MiningCandidate::new("coal_ore".into(), 3, 62, 0, 2.0, 64.0, 0.0),
            MiningCandidate::new("coal_ore".into(), 0, 40, 0, 5.0, 64.0, 0.0),
        ];
        let best = pick_best_candidate(candidates).unwrap();
        assert_eq!(best.y, 62, "should pick shallow + close candidate");
    }
}
