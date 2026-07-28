//! 根因分析

use crate::anomaly::Anomaly;
use crate::event_log::EventLog;
use crate::hypothesis::Hypothesis;

pub struct RootCauseAnalyzer;

impl RootCauseAnalyzer {
    pub fn analyze(
        anomaly: &Anomaly,
        events: &EventLog,
        history: &[EventLog],
    ) -> Vec<Hypothesis> {
        let mut hypotheses = vec![];

        // Method 1: Find first change before anomaly
        if let Some(change) = Self::find_first_change(events) {
            hypotheses.push(Hypothesis {
                cause: format!("Event change: {change}"),
                prediction: "This change may have triggered the anomaly".into(),
                test_description: format!("Revert: {change}"),
                impact_scope: 2,
                confidence: 0.6,
            });
        }

        // Method 2: Compare with last normal round
        if let Some(normal) = history.iter().rev().find(|h| h.events().is_empty() || h.events().iter().all(|e| !matches!(e, crate::event_log::Event::AnomalyDetected { .. }))) {
            let current_types: Vec<_> = events.event_types();
            let normal_types: Vec<_> = normal.event_types();
            if current_types != normal_types {
                hypotheses.push(Hypothesis {
                    cause: "Different event pattern from last normal round".into(),
                    prediction: "Aligning patterns should help".into(),
                    test_description: "Adjust workflow to match normal pattern".into(),
                    impact_scope: 3,
                    confidence: 0.5,
                });
            }
        }

        // Method 3: Metric-specific hypotheses
        match anomaly.metric.as_str() {
            "llm_duration_ms" => {
                hypotheses.push(Hypothesis {
                    cause: "LLM response time too long".into(),
                    prediction: "Reducing timeout or max_tokens should help".into(),
                    test_description: "Reduce timeout_secs from 60 to 30".into(),
                    impact_scope: 2,
                    confidence: 0.7,
                });
            }
            "build_errors" => {
                hypotheses.push(Hypothesis {
                    cause: "Build errors detected".into(),
                    prediction: "Fixing compilation errors should resolve".into(),
                    test_description: "Fix build errors".into(),
                    impact_scope: 5,
                    confidence: 0.9,
                });
            }
            "test_failed" => {
                hypotheses.push(Hypothesis {
                    cause: "Test failures detected".into(),
                    prediction: "Fixing test failures should resolve".into(),
                    test_description: "Fix failing tests".into(),
                    impact_scope: 4,
                    confidence: 0.85,
                });
            }
            _ => {
                hypotheses.push(Hypothesis {
                    cause: format!("Anomaly in {}", anomaly.metric),
                    prediction: "Investigate root cause".into(),
                    test_description: format!("Deep analysis of {}", anomaly.metric),
                    impact_scope: 1,
                    confidence: 0.3,
                });
            }
        }

        hypotheses.sort_by(|a, b| {
            let score_a = a.confidence * a.impact_scope as f64;
            let score_b = b.confidence * b.impact_scope as f64;
            score_b.partial_cmp(&score_a).unwrap_or(std::cmp::Ordering::Equal)
        });

        hypotheses
    }

    fn find_first_change(events: &EventLog) -> Option<String> {
        for event in events.events() {
            match event {
                crate::event_log::Event::LlmError { error } => return Some(format!("LLM error: {error}")),
                crate::event_log::Event::ToolError { name, error } => return Some(format!("Tool {name} error: {error}")),
                crate::event_log::Event::BuildResult { success: false, error_count } => {
                    return Some(format!("Build failed with {error_count} errors"));
                }
                crate::event_log::Event::TestResult { failed, .. } if *failed > 0 => {
                    return Some(format!("{failed} tests failed"));
                }
                _ => {}
            }
        }
        None
    }
}
