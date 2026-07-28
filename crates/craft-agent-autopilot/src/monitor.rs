//! 多源监控交叉验证

use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default)]
pub struct BotSnapshot {
    pub position: Option<Vec<f64>>,
    pub health: Option<f32>,
    pub hunger: Option<f32>,
    pub held_item: Option<String>,
    pub source: String,
    pub timestamp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceReliability {
    pub name: String,
    pub reliability: f64,
    pub last_error: Option<String>,
}

pub struct ConsistencyEngine {
    sources: Vec<SourceReliability>,
}

impl ConsistencyEngine {
    pub fn new() -> Self {
        Self {
            sources: vec![
                SourceReliability { name: "viewer_perceive".into(), reliability: 0.85, last_error: None },
                SourceReliability { name: "session_jsonl".into(), reliability: 0.90, last_error: None },
                SourceReliability { name: "bot_trace".into(), reliability: 0.95, last_error: None },
            ],
        }
    }

    pub fn cross_validate(&self, snapshots: &[BotSnapshot]) -> ValidationResult {
        if snapshots.len() < 2 {
            return ValidationResult::NeedMoreSources;
        }

        let mut discrepancies = vec![];
        for i in 0..snapshots.len() {
            for j in (i + 1)..snapshots.len() {
                if !self.consistent(&snapshots[i], &snapshots[j]) {
                    discrepancies.push((snapshots[i].source.clone(), snapshots[j].source.clone()));
                }
            }
        }

        if discrepancies.is_empty() {
            ValidationResult::Consistent
        } else {
            ValidationResult::Inconsistent(discrepancies)
        }
    }

    fn consistent(&self, a: &BotSnapshot, b: &BotSnapshot) -> bool {
        const POS_TOL: f64 = 1.0;
        const HEALTH_TOL: f32 = 1.0;

        let pos_match = match (&a.position, &b.position) {
            (Some(pa), Some(pb)) if pa.len() == 3 && pb.len() == 3 => {
                (pa[0] - pb[0]).abs() < POS_TOL
                    && (pa[1] - pb[1]).abs() < POS_TOL
                    && (pa[2] - pb[2]).abs() < POS_TOL
            }
            _ => true,
        };

        let health_match = match (&a.health, &b.health) {
            (Some(ha), Some(hb)) => (ha - hb).abs() < HEALTH_TOL,
            _ => true,
        };

        pos_match && health_match
    }

    pub fn degrade_source(&mut self, name: &str) {
        if let Some(source) = self.sources.iter_mut().find(|s| s.name == name) {
            source.reliability *= 0.8;
        }
    }

    pub fn active_sources(&self) -> Vec<&SourceReliability> {
        self.sources.iter().filter(|s| s.reliability > 0.3).collect()
    }
}

#[derive(Debug)]
pub enum ValidationResult {
    Consistent,
    Inconsistent(Vec<(String, String)>),
    NeedMoreSources,
}

/// Fetch bot state from viewer HTTP API
pub fn poll_viewer(addr: &str) -> Result<BotSnapshot> {
    let url = format!("http://{addr}/api/game-state");
    let resp = reqwest::blocking::get(&url)?;
    let json: serde_json::Value = resp.json()?;

    Ok(BotSnapshot {
        position: json.get("position").and_then(|v| v.as_array()).map(|v| {
            v.iter().filter_map(|x| x.as_f64()).collect()
        }),
        health: json.get("health").and_then(|v| v.as_f64()).map(|v| v as f32),
        hunger: json.get("hunger").and_then(|v| v.as_f64()).map(|v| v as f32),
        held_item: json.get("held_item").and_then(|v| v.as_str()).map(String::from),
        source: "viewer_perceive".into(),
        timestamp: chrono::Local::now().to_rfc3339(),
    })
}
