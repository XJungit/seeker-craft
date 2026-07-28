//! 异常检测 — 统计/模式/趋势

use crate::event_log::EventLog;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone)]
pub struct Anomaly {
    pub kind: AnomalyKind,
    pub metric: String,
    pub value: f64,
    pub expected: (f64, f64),
    pub severity: f64,
    pub timestamp: String,
}

#[derive(Debug, Clone)]
pub enum AnomalyKind {
    Statistical(f64),
    NovelPattern,
    TrendDecline,
}

impl Anomaly {
    pub fn features(&self) -> Vec<String> {
        let mut features = vec![];
        features.push(format!("metric:{}", self.metric));
        features.push(format!("kind:{:?}", self.kind));
        if self.severity > 3.0 {
            features.push("high_severity".into());
        }
        features
    }
}

pub struct AnomalyDetector {
    history: Vec<EventLog>,
    min_history: usize,
}

impl AnomalyDetector {
    pub fn new(min_history: usize) -> Self {
        Self {
            history: vec![],
            min_history,
        }
    }

    pub fn add_history(&mut self, events: EventLog) {
        self.history.push(events);
        if self.history.len() > 20 {
            self.history.remove(0);
        }
    }

    pub fn detect(&self, current: &EventLog) -> Vec<Anomaly> {
        let mut anomalies = vec![];
        if self.history.len() < self.min_history {
            return anomalies;
        }
        anomalies.extend(self.statistical_anomaly(current));
        anomalies.extend(self.pattern_anomaly(current));
        anomalies
    }

    fn statistical_anomaly(&self, current: &EventLog) -> Vec<Anomaly> {
        let mut result = vec![];
        let metrics = current.numeric_metrics();

        for (name, value) in metrics {
            let historical: Vec<f64> = self.history.iter()
                .filter_map(|h| h.numeric_metrics().get(&name).copied())
                .collect();

            if historical.len() >= 5 {
                let mean = historical.iter().sum::<f64>() / historical.len() as f64;
                let variance = historical.iter()
                    .map(|v| (v - mean).powi(2))
                    .sum::<f64>() / historical.len() as f64;
                let std = variance.sqrt();

                if std > 1e-10 {
                    let z_score = (value - mean).abs() / std;
                    if z_score > 2.5 {
                        result.push(Anomaly {
                            kind: AnomalyKind::Statistical(z_score),
                            metric: name,
                            value,
                            expected: (mean - 2.5 * std, mean + 2.5 * std),
                            severity: z_score,
                            timestamp: current.timestamp(),
                        });
                    }
                }
            }
        }

        result
    }

    fn pattern_anomaly(&self, current: &EventLog) -> Vec<Anomaly> {
        let current_ngrams: HashSet<Vec<String>> = current.event_ngrams(3);
        let historical_ngrams: HashSet<Vec<String>> = self.history.iter()
            .flat_map(|h| h.event_ngrams(3))
            .collect();

        let novel: Vec<_> = current_ngrams.difference(&historical_ngrams).collect();

        if novel.len() > 2 {
            vec![Anomaly {
                kind: AnomalyKind::NovelPattern,
                metric: "event_pattern".into(),
                value: novel.len() as f64,
                expected: (0.0, 2.0),
                severity: novel.len() as f64,
                timestamp: current.timestamp(),
            }]
        } else {
            vec![]
        }
    }

    pub fn history(&self) -> &[EventLog] {
        &self.history
    }
}
