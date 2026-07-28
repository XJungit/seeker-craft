//! 知识库 — 从经验中学习

use crate::anomaly::Anomaly;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeEntry {
    pub problem_features: Vec<String>,
    pub root_cause: String,
    pub fix: String,
    pub verified: bool,
    pub occurrences: u32,
    pub last_used: String,
    pub success_history: Vec<bool>,
}

#[derive(Serialize, Deserialize)]
pub struct KnowledgeBase {
    entries: Vec<KnowledgeEntry>,
    path: PathBuf,
}

impl KnowledgeBase {
    pub fn load(path: PathBuf) -> Result<Self> {
        if path.exists() {
            let content = std::fs::read_to_string(&path)?;
            let entries = serde_json::from_str(&content).unwrap_or_default();
            Ok(Self { entries, path })
        } else {
            Ok(Self { entries: vec![], path })
        }
    }

    pub fn save(&self) -> Result<()> {
        let content = serde_json::to_string_pretty(&self.entries)?;
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&self.path, content)?;
        Ok(())
    }

    pub fn lookup(&self, anomaly: &Anomaly) -> Option<&KnowledgeEntry> {
        let features = anomaly.features();
        self.entries.iter()
            .filter(|e| Self::matches(&e.problem_features, &features))
            .max_by_key(|e| e.occurrences)
    }

    pub fn learn(&mut self, anomaly: &Anomaly, cause: &str, fix: &str, success: bool) {
        let features = anomaly.features();

        if let Some(entry) = self.entries.iter_mut()
            .find(|e| Self::similar_features(&e.problem_features, &features))
        {
            entry.occurrences += 1;
            entry.last_used = chrono::Local::now().to_rfc3339();
            entry.success_history.push(success);
        } else {
            self.entries.push(KnowledgeEntry {
                problem_features: features,
                root_cause: cause.to_string(),
                fix: fix.to_string(),
                verified: success,
                occurrences: 1,
                last_used: chrono::Local::now().to_rfc3339(),
                success_history: vec![success],
            });
        }
    }

    pub fn prune(&mut self) {
        self.entries.retain(|e| {
            if e.success_history.len() < 2 {
                return true;  // Keep new entries
            }
            let success_rate = e.success_history.iter().filter(|&&s| s).count() as f64
                / e.success_history.len() as f64;
            e.occurrences > 2 || success_rate > 0.3
        });
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    fn matches(a: &[String], b: &[String]) -> bool {
        a.iter().any(|x| b.contains(x))
    }

    fn similar_features(a: &[String], b: &[String]) -> bool {
        // Simple Jaccard similarity
        let set_a: std::collections::HashSet<_> = a.iter().collect();
        let set_b: std::collections::HashSet<_> = b.iter().collect();
        let intersection = set_a.intersection(&set_b).count();
        let union = set_a.union(&set_b).count();
        if union == 0 { return false; }
        intersection as f64 / union as f64 > 0.5
    }
}

impl Default for KnowledgeBase {
    fn default() -> Self {
        Self {
            entries: vec![],
            path: PathBuf::from("tools/knowledge_base.json"),
        }
    }
}
