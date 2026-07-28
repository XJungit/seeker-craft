//! 全量事件记录

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event_type")]
pub enum Event {
    LlmResponse {
        has_tool_calls: bool,
        input_tokens: u32,
        output_tokens: u32,
        duration_ms: u64,
    },
    LlmError {
        error: String,
    },
    LlmTimeout {
        waited_ms: u64,
    },
    TextOnlyResponse {
        content_preview: String,
    },
    ToolCall {
        name: String,
    },
    ToolResult {
        name: String,
        success: bool,
        duration_ms: u64,
    },
    ToolError {
        name: String,
        error: String,
    },
    BotState {
        position: Option<Vec<f64>>,
        health: Option<f32>,
        hunger: Option<f32>,
    },
    PhaseStart {
        phase: String,
    },
    PhaseEnd {
        phase: String,
        success: bool,
        duration_ms: u64,
    },
    BuildResult {
        success: bool,
        error_count: usize,
    },
    TestResult {
        passed: u32,
        failed: u32,
    },
    AnomalyDetected {
        kind: String,
        metric: String,
        value: f64,
    },
    HypothesisVerified {
        cause: String,
        improved: bool,
    },
    KnowledgeAdded {
        root_cause: String,
    },
}

#[derive(Clone)]
pub struct EventLog {
    round_id: u64,
    file_path: PathBuf,
    events: Vec<Event>,
}

impl EventLog {
    pub fn new(round_id: u64, sessions_dir: &Path) -> Self {
        let file_path = sessions_dir.join("events").join(format!("{round_id}.jsonl"));
        Self {
            round_id,
            file_path,
            events: vec![],
        }
    }

    pub fn log(&mut self, event: Event) -> Result<()> {
        self.events.push(event.clone());
        let line = serde_json::to_string(&event)?;
        std::fs::create_dir_all(self.file_path.parent().unwrap_or(Path::new(".")))?;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.file_path)?;
        use std::io::Write;
        writeln!(file, "{line}")?;
        Ok(())
    }

    pub fn events(&self) -> &[Event] {
        &self.events
    }

    pub fn numeric_metrics(&self) -> std::collections::HashMap<String, f64> {
        let mut metrics = std::collections::HashMap::new();
        for event in &self.events {
            match event {
                Event::LlmResponse { input_tokens, output_tokens, duration_ms, .. } => {
                    metrics.insert("llm_input_tokens".into(), *input_tokens as f64);
                    metrics.insert("llm_output_tokens".into(), *output_tokens as f64);
                    metrics.insert("llm_duration_ms".into(), *duration_ms as f64);
                }
                Event::ToolResult { duration_ms, .. } => {
                    metrics.insert("tool_duration_ms".into(), *duration_ms as f64);
                }
                Event::BuildResult { success, error_count } => {
                    metrics.insert("build_success".into(), if *success { 1.0 } else { 0.0 });
                    metrics.insert("build_errors".into(), *error_count as f64);
                }
                Event::TestResult { passed, failed } => {
                    metrics.insert("test_passed".into(), *passed as f64);
                    metrics.insert("test_failed".into(), *failed as f64);
                }
                _ => {}
            }
        }
        metrics
    }

    pub fn event_types(&self) -> Vec<String> {
        self.events
            .iter()
            .map(|e| serde_json::to_value(e).unwrap()["event_type"].as_str().unwrap_or("").to_string())
            .collect()
    }

    pub fn event_ngrams(&self, n: usize) -> std::collections::HashSet<Vec<String>> {
        let types = self.event_types();
        types.windows(n).map(|w| w.to_vec()).collect()
    }

    pub fn timestamp(&self) -> String {
        chrono::Local::now().to_rfc3339()
    }
}
