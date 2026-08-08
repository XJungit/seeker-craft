//! Anomaly detection for the autopilot monitor loop.
//!
//! Detects high-value runtime anomalies from structured `game-state` snapshots so
//! long-running sessions leave auditable evidence instead of silently forgetting
//! incidents (e.g. the "armor vanished / maybe fell into lava" investigation of
//! Round 38 had to be re-derived by hand from archived session JSONL).
//!
//! Signals used (all from `/api/game-state`, never from LLM tool text):
//! - `health`: 0 → death; 0→20 recovery = respawn
//! - `position`: sudden large jump = respawn teleport
//! - armor summary line in `scene_desc` (`装备: [头盔: X, 胸甲: Y, 护腿: Z, 靴子: W]`)
//!   full set → empty = armor loss
//!
//! The detector is stateful across consecutive snapshots: it reports each anomaly
//! once per incident (no repeated noise), and it also reports the *lurking*
//! near-death case (health < 4 recovering to full without a 0 frame in between,
//! which commonly happens when the poll interval skips the death frame).

use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnomalyKind {
    Death,
    Respawn,
    ArmorLoss,
    NearDeath,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Anomaly {
    pub kind: AnomalyKind,
    pub detail: String,
    pub timestamp_ms: u128,
}

impl Anomaly {
    pub fn kind_name(&self) -> &'static str {
        match self.kind {
            AnomalyKind::Death => "death",
            AnomalyKind::Respawn => "respawn",
            AnomalyKind::ArmorLoss => "armor_loss",
            AnomalyKind::NearDeath => "near_death",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct AnomalyState {
    prev_health: Option<f64>,
    prev_position: Option<[f64; 3]>,
    prev_armor_count: Option<usize>,
    /// health observed at 0 since last snapshot; latch until observed recovery
    was_dead: bool,
    /// last snapshot timestamp (for stable anomaly ordering in tests)
    pub last_timestamp_ms: u128,
}

const RESPAWN_TELEPORT_DISTANCE: f64 = 50.0;
const ARMOR_SLOTS: usize = 4;

/// Feed a fresh game-state snapshot; returns anomalies detected since last feed.
pub fn detect_anomalies(
    state: &mut AnomalyState,
    game_state: &Value,
    now_ms: u128,
) -> Vec<Anomaly> {
    let mut anomalies = Vec::new();
    let health = health_of(game_state);
    let position = position_of(game_state);
    let armor_count = armor_count_of(game_state);

    // ── Death: health hits 0 (may be a single frame between polls) ──
    if let Some(h) = health {
        if h <= 0.0 {
            if !state.was_dead {
                state.was_dead = true;
                anomalies.push(Anomaly {
                    kind: AnomalyKind::Death,
                    timestamp_ms: now_ms,
                    detail: format!(
                        "health hit 0{}",
                        armor_loss_suffix(state.prev_armor_count, armor_count)
                    ),
                });
            }
        } else if state.was_dead {
            // Respawn: back above 0 after an observed death frame
            state.was_dead = false;
            let mut detail = format!("health recovered to {h:.0}");
            detail.push_str(&armor_respawn_suffix(state.prev_armor_count, armor_count));
            detail.push_str(&teleport_suffix(state.prev_position, position));
            anomalies.push(Anomaly {
                kind: AnomalyKind::Respawn,
                timestamp_ms: now_ms,
                detail,
            });
        } else if h >= 20.0
            && let Some(prev) = state.prev_health
            && prev < 4.0
        {
            // Near-death recovered between polls: previous frame low, now full again,
            // and no death frame was observed (poll interval likely skipped it).
            anomalies.push(Anomaly {
                kind: AnomalyKind::NearDeath,
                timestamp_ms: now_ms,
                detail: format!("health jumped {prev:.1} → {h:.0}"),
            });
        }
    }

    // ── Armor loss: previously wore >0 armor pieces, now wears none (non-death) ──
    // 重生帧已包含 armor lost 细节，避免重复报。
    let respawn_reported = anomalies.iter().any(|a| a.kind == AnomalyKind::Respawn);
    if !state.was_dead
        && !respawn_reported
        && let Some(prev_armor) = state.prev_armor_count
        && prev_armor > 0
        && armor_count == Some(0)
    {
        anomalies.push(Anomaly {
            kind: AnomalyKind::ArmorLoss,
            timestamp_ms: now_ms,
            detail: format!(
                "equipment went from {prev_armor}/{ARMOR_SLOTS} pieces to 0{}",
                teleport_suffix(state.prev_position, position),
            ),
        });
    }

    state.prev_health = health;
    state.prev_position = position;
    // 死亡帧 scene_desc 可能为空导致 armor_count 解析不到（None）：此时保留上次
    // 已知的装备数，重生时才能对比"死前穿甲 → 重生后裸装"。
    if armor_count.is_some() {
        state.prev_armor_count = armor_count;
    }
    state.last_timestamp_ms = now_ms;
    anomalies
}

fn health_of(game_state: &Value) -> Option<f64> {
    game_state.get("health").and_then(|v| v.as_f64())
}

fn position_of(game_state: &Value) -> Option<[f64; 3]> {
    let values = game_state.get("position")?.as_array()?;
    Some([
        values.first()?.as_f64()?,
        values.get(1)?.as_f64()?,
        values.get(2)?.as_f64()?,
    ])
}

/// Parse the armor summary line from scene_desc (`装备: [头盔: X, 胸甲: Y, ...]`
/// with `无` marking empty slots) and return the number of worn pieces.
fn armor_count_of(game_state: &Value) -> Option<usize> {
    let scene = game_state.get("scene_desc")?.as_str()?;
    let line = scene.lines().find(|l| l.contains("装备:"))?;
    let empty_slots = line.matches("无").count().min(ARMOR_SLOTS);
    Some(ARMOR_SLOTS - empty_slots)
}

fn armor_loss_suffix(prev: Option<usize>, cur: Option<usize>) -> String {
    match (prev, cur) {
        (Some(p), Some(c)) if p > 0 && c == 0 => format!("; armor lost {p} → {c} pieces"),
        _ => String::new(),
    }
}

/// Respawn suffix: compares armor *before* death (kept across the death frame)
/// with armor after respawn.
fn armor_respawn_suffix(prev: Option<usize>, cur: Option<usize>) -> String {
    match (prev, cur) {
        (Some(p), Some(c)) if p > 0 && c == 0 => format!("; armor lost {p} → {c} pieces"),
        _ => String::new(),
    }
}

fn teleport_suffix(prev: Option<[f64; 3]>, cur: Option<[f64; 3]>) -> String {
    match (prev, cur) {
        (Some(p), Some(c)) if distance(p, c) >= RESPAWN_TELEPORT_DISTANCE => {
            format!(
                "; position teleported ({:.0},{:.0},{:.0}) → ({:.0},{:.0},{:.0})",
                p[0], p[1], p[2], c[0], c[1], c[2]
            )
        }
        _ => String::new(),
    }
}

fn distance(a: [f64; 3], b: [f64; 3]) -> f64 {
    (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn bare_state() -> Value {
        json!({
            "health": 20.0,
            "position": [1.0, 64.0, 1.0],
            "inventory": [],
            "scene_desc": "装备: [头盔: 无, 胸甲: 无, 护腿: 无, 靴子: 无]\n背包: [cobblestone:64]"
        })
    }

    fn armored_state() -> Value {
        json!({
            "health": 20.0,
            "position": [1.0, 64.0, 1.0],
            "inventory": [],
            "scene_desc": "装备: [头盔: iron_helmet, 胸甲: iron_chestplate, 护腿: iron_leggings, 靴子: iron_boots]"
        })
    }

    #[test]
    fn death_then_respawn_detected() {
        let mut det = AnomalyState::default();
        let _ = detect_anomalies(&mut det, &bare_state(), 1);
        let dead =
            json!({"health": 0.0, "position": [1.0, 64.0, 1.0], "inventory": [], "scene_desc": ""});
        let anomalies = detect_anomalies(&mut det, &dead, 2);
        assert_eq!(anomalies.len(), 1);
        assert_eq!(anomalies[0].kind, AnomalyKind::Death);

        let respawn = json!({"health": 20.0, "position": [100.0, 64.0, 100.0], "inventory": [], "scene_desc": "装备: [头盔: 无, 胸甲: 无, 护腿: 无, 靴子: 无]"});
        let anomalies = detect_anomalies(&mut det, &respawn, 3);
        assert_eq!(anomalies.len(), 1);
        assert_eq!(anomalies[0].kind, AnomalyKind::Respawn);
        assert!(anomalies[0].detail.contains("teleport"));
    }

    #[test]
    fn death_with_armor_loss_reported_on_respawn() {
        let mut det = AnomalyState::default();
        let _ = detect_anomalies(&mut det, &armored_state(), 1);
        let dead =
            json!({"health": 0.0, "position": [1.0, 64.0, 1.0], "inventory": [], "scene_desc": ""});
        let death = detect_anomalies(&mut det, &dead, 2);
        assert_eq!(death.len(), 1);
        assert_eq!(death[0].kind, AnomalyKind::Death);
        // death 帧 scene 为空 → armor 无法解析，detail 无 armor 信息
        assert!(!death[0].detail.contains("armor lost"));
        // 重生帧对比"死前 4 件 → 重生后 0 件"，armor lost 在此报告
        let respawn = json!({"health": 20.0, "position": [1.0, 64.0, 1.0], "inventory": [], "scene_desc": "装备: [头盔: 无, 胸甲: 无, 护腿: 无, 靴子: 无]"});
        let respawn_anomalies = detect_anomalies(&mut det, &respawn, 3);
        assert_eq!(respawn_anomalies.len(), 1);
        assert_eq!(respawn_anomalies[0].kind, AnomalyKind::Respawn);
        assert!(respawn_anomalies[0].detail.contains("armor lost 4 → 0"));
    }

    #[test]
    fn armor_loss_detected_without_death() {
        let mut det = AnomalyState::default();
        let _ = detect_anomalies(&mut det, &bare_state(), 1);
        let _ = detect_anomalies(&mut det, &armored_state(), 2);
        let anomalies = detect_anomalies(&mut det, &bare_state(), 3);
        assert_eq!(anomalies.len(), 1);
        assert_eq!(anomalies[0].kind, AnomalyKind::ArmorLoss);
    }

    #[test]
    fn near_death_recovery_detected_without_death_frame() {
        let mut det = AnomalyState::default();
        let low =
            json!({"health": 2.0, "position": [1.0, 64.0, 1.0], "inventory": [], "scene_desc": ""});
        let _ = detect_anomalies(&mut det, &low, 1);
        let anomalies = detect_anomalies(&mut det, &bare_state(), 2);
        assert_eq!(anomalies.len(), 1);
        assert_eq!(anomalies[0].kind, AnomalyKind::NearDeath);
    }

    #[test]
    fn no_false_positive_for_stable_state() {
        let mut det = AnomalyState::default();
        let _ = detect_anomalies(&mut det, &bare_state(), 1);
        let anomalies = detect_anomalies(&mut det, &bare_state(), 2);
        assert!(anomalies.is_empty());
    }

    #[test]
    fn single_death_not_reported_twice() {
        let mut det = AnomalyState::default();
        let _ = detect_anomalies(&mut det, &bare_state(), 1);
        let dead =
            json!({"health": 0.0, "position": [1.0, 64.0, 1.0], "inventory": [], "scene_desc": ""});
        let first = detect_anomalies(&mut det, &dead, 2);
        let second = detect_anomalies(&mut det, &dead, 3);
        assert_eq!(first.len(), 1);
        assert!(second.is_empty());
    }

    #[test]
    fn armor_count_parses_chinese_line() {
        let state = armored_state();
        assert_eq!(armor_count_of(&state), Some(4));
        assert_eq!(armor_count_of(&bare_state()), Some(0));
        let mixed =
            json!({"scene_desc": "装备: [头盔: iron_helmet, 胸甲: 无, 护腿: 无, 靴子: 无]"});
        assert_eq!(armor_count_of(&mixed), Some(1));
    }
}
