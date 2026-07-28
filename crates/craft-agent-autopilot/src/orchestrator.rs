//! 主循环编排

use crate::anomaly::{AnomalyDetector, Anomaly};
use crate::event_log::{Event, EventLog};
use crate::experiment::ExperimentRunner;
use crate::hypothesis::Hypothesis;
use crate::knowledge::KnowledgeBase;
use crate::monitor::ConsistencyEngine;
use crate::root_cause::RootCauseAnalyzer;
use anyhow::Result;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

pub struct Orchestrator {
    workspace_root: PathBuf,
    mc_addr: String,
    viewer_port: u16,
    event_log: EventLog,
    anomaly_detector: AnomalyDetector,
    knowledge_base: KnowledgeBase,
    consistency_engine: ConsistencyEngine,
    experiment_runner: ExperimentRunner,
    sessions_dir: PathBuf,
    round: u64,
}

#[derive(Debug, Default)]
pub struct RoundResult {
    pub round_id: u64,
    pub build_ok: bool,
    pub test_ok: bool,
    pub test_passed: u32,
    pub test_failed: u32,
    pub llm_steps: u32,
    pub llm_stuck_steps: u32,
    pub tool_calls: u32,
    pub tool_errors: u32,
    pub anomalies: Vec<Anomaly>,
    pub hypotheses_tested: u32,
    pub hypotheses_succeeded: u32,
    pub knowledge_added: u32,
    pub current_phase: String,
    pub current_phase_complete: bool,
    pub open_issues: u32,
    pub has_progress: bool,
    pub ender_dragon_defeated: bool,
}

impl RoundResult {
    pub fn new(round_id: u64) -> Self {
        Self {
            round_id,
            current_phase: "early_game".into(),
            ..Default::default()
        }
    }

    pub fn summary(&self) -> String {
        format!(
            "build={} test={}/{} anomalies={} hypotheses={}/{} knowledge={}",
            if self.build_ok { "OK" } else { "FAIL" },
            self.test_passed,
            self.test_passed + self.test_failed,
            self.anomalies.len(),
            self.hypotheses_succeeded,
            self.hypotheses_tested,
            self.knowledge_added,
        )
    }
}

impl Orchestrator {
    pub fn new(workspace_root: PathBuf, mc_addr: String, viewer_port: u16) -> Self {
        let sessions_dir = workspace_root.join("sessions");
        let event_log = EventLog::new(1, &sessions_dir);
        let anomaly_detector = AnomalyDetector::new(3);
        let knowledge_base = KnowledgeBase::load(workspace_root.join("tools/knowledge_base.json")).unwrap_or_default();
        let consistency_engine = ConsistencyEngine::new();
        let experiment_runner = ExperimentRunner::new(workspace_root.clone());

        Self {
            workspace_root,
            mc_addr,
            viewer_port,
            event_log,
            anomaly_detector,
            knowledge_base,
            consistency_engine,
            experiment_runner,
            sessions_dir,
            round: 0,
        }
    }

    pub async fn run_round(&mut self, round_id: u64) -> Result<RoundResult> {
        self.round = round_id;
        let mut result = RoundResult::new(round_id);
        self.event_log = EventLog::new(round_id, &self.sessions_dir);

        // Phase 0: Build + Test
        self.run_phase_0(&mut result)?;

        // Phase 2b: LLM real-machine test (always run if build+test passed)
        if result.build_ok && result.test_ok {
            self.run_phase_2b(&mut result).await?;
        }

        // Phase 2: Anomaly Detection (after all phases have run)
        self.run_phase_2(&mut result)?;

        // Phase 3: Root cause analysis (if anomalies found)
        if !result.anomalies.is_empty() {
            self.run_phase_3(&mut result)?;
        }

        // Phase 4: Knowledge update
        self.anomaly_detector.add_history(self.event_log.clone());

        // Determine if progress was made
        result.has_progress = result.llm_steps > 0 || result.hypotheses_succeeded > 0;

        Ok(result)
    }

    fn run_phase_0(&mut self, result: &mut RoundResult) -> Result<()> {
        // cargo check (not build, to avoid self-deadlock)
        let build_start = Instant::now();
        let build_output = Command::new("cargo")
            .args(["check", "--workspace", "--exclude", "craft-agent-autopilot"])
            .current_dir(&self.workspace_root)
            .output()?;
        let build_ms = build_start.elapsed().as_millis() as u64;
        result.build_ok = build_output.status.success();

        let error_count = if result.build_ok { 0 } else {
            String::from_utf8_lossy(&build_output.stderr).lines().filter(|l| l.contains("error")).count()
        };
        self.event_log.log(Event::BuildResult { success: result.build_ok, error_count })?;
        self.event_log.log(Event::PhaseStart { phase: "build".into() })?;
        self.event_log.log(Event::PhaseEnd { phase: "build".into(), success: result.build_ok, duration_ms: build_ms })?;

        // cargo test (exclude autopilot and viewer to avoid issues)
        let test_start = Instant::now();
        let test_output = Command::new("cargo")
            .args(["test", "--workspace", "--no-fail-fast", "--exclude", "craft-agent-autopilot"])
            .current_dir(&self.workspace_root)
            .output()?;
        let test_ms = test_start.elapsed().as_millis() as u64;
        result.test_ok = test_output.status.success();

        let stdout = String::from_utf8_lossy(&test_output.stdout);
        for line in stdout.lines() {
            if line.contains("... ok") { result.test_passed += 1; }
            if line.contains("... FAILED") { result.test_failed += 1; }
        }
        self.event_log.log(Event::TestResult { passed: result.test_passed, failed: result.test_failed })?;
        self.event_log.log(Event::PhaseStart { phase: "test".into() })?;
        self.event_log.log(Event::PhaseEnd { phase: "test".into(), success: result.test_ok, duration_ms: test_ms })?;

        Ok(())
    }

    fn run_phase_2(&mut self, result: &mut RoundResult) -> Result<()> {
        let anomalies = self.anomaly_detector.detect(&self.event_log);
        for anomaly in &anomalies {
            self.event_log.log(Event::AnomalyDetected {
                kind: format!("{:?}", anomaly.kind),
                metric: anomaly.metric.clone(),
                value: anomaly.value,
            })?;
        }
        result.anomalies = anomalies;
        Ok(())
    }

    async fn run_phase_2b(&mut self, result: &mut RoundResult) -> Result<()> {
        let viewer_addr = format!("127.0.0.1:{}", self.viewer_port);

        // Archive old session
        let session_path = self.sessions_dir.join("mc_run.jsonl");
        if session_path.exists() {
            let archive_dir = self.sessions_dir.join("archive");
            std::fs::create_dir_all(&archive_dir)?;
            let archive_path = archive_dir.join(format!("mc_run_{}.jsonl", self.round));
            let _ = std::fs::rename(&session_path, &archive_path);
        }

        // Kill existing viewer
        let _ = Command::new("taskkill").args(["/F", "/IM", "craft-agent-viewer.exe"]).output();

        // Use pre-built binary instead of cargo run (faster startup)
        let viewer_exe = self.workspace_root.join("target").join("debug").join("craft-agent-viewer.exe");
        let viewer_result = if viewer_exe.exists() {
            Command::new(&viewer_exe)
                .args([
                    "--goal", "继续推进当前任务",
                    "--steps", "0",
                    "--port", &self.viewer_port.to_string(),
                    "--mc", &self.mc_addr,
                ])
                .current_dir(&self.workspace_root)
                .spawn()
        } else {
            // Fallback to cargo run
            Command::new("cargo")
                .args([
                    "run", "-p", "craft-agent-viewer", "--",
                    "--goal", "继续推进当前任务",
                    "--steps", "0",
                    "--port", &self.viewer_port.to_string(),
                    "--mc", &self.mc_addr,
                ])
                .current_dir(&self.workspace_root)
                .spawn()
        };

        let mut viewer_proc = match viewer_result {
            Ok(proc) => proc,
            Err(e) => {
                eprintln!("Failed to start viewer: {e}");
                return Ok(());
            }
        };

        // Wait for viewer ready (max 60s)
        let deadline = Instant::now() + Duration::from_secs(60);
        let mut ready = false;
        while Instant::now() < deadline {
            if reqwest::blocking::get(format!("http://{viewer_addr}/api/status")).is_ok() {
                ready = true;
                break;
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }

        if !ready {
            eprintln!("[Round {}] Viewer not ready within 60s, skipping LLM test", self.round);
            let _ = viewer_proc.kill();
            self.event_log.log(crate::event_log::Event::LlmError {
                error: "Viewer failed to start within timeout".into(),
            })?;
            result.llm_stuck_steps = 1;
            return Ok(());
        }

        // Start agent
        let _ = reqwest::blocking::Client::new()
            .post(format!("http://{viewer_addr}/api/start"))
            .send();

        // Poll until done or timeout (5 min global)
        let deadline = Instant::now() + Duration::from_secs(300);
        while Instant::now() < deadline {
            tokio::time::sleep(Duration::from_secs(5)).await;

            match reqwest::blocking::get(format!("http://{viewer_addr}/api/status")) {
                Ok(resp) => {
                    if let Ok(status) = resp.json::<serde_json::Value>() {
                        if !status["running"].as_bool().unwrap_or(false) {
                            break;
                        }
                        result.llm_steps = status["step"].as_u64().unwrap_or(0) as u32;
                    }
                }
                Err(_) => break,
            }

            // Record bot state
            if let Ok(resp) = reqwest::blocking::get(format!("http://{viewer_addr}/api/game-state")) {
                if let Ok(json) = resp.json::<serde_json::Value>() {
                    let _ = self.event_log.log(Event::BotState {
                        position: json.get("position").and_then(|v| v.as_array()).map(|v| {
                            v.iter().filter_map(|x| x.as_f64()).collect()
                        }),
                        health: json.get("health").and_then(|v| v.as_f64()).map(|v| v as f32),
                        hunger: json.get("hunger").and_then(|v| v.as_f64()).map(|v| v as f32),
                    });
                }
            }
        }

        // Stop viewer
        let _ = reqwest::blocking::Client::new()
            .post(format!("http://{viewer_addr}/api/stop"))
            .send();
        let _ = viewer_proc.kill();

        Ok(())
    }

    fn run_phase_3(&mut self, result: &mut RoundResult) -> Result<()> {
        for anomaly in &result.anomalies.clone() {
            // Check knowledge base first
            if let Some(entry) = self.knowledge_base.lookup(anomaly) {
                self.event_log.log(Event::KnowledgeAdded {
                    root_cause: format!("Reused known fix for: {}", entry.root_cause),
                })?;
                result.knowledge_added += 1;
                continue;
            }

            // Generate hypotheses
            let hypotheses = RootCauseAnalyzer::analyze(
                anomaly, &self.event_log, self.anomaly_detector.history(),
            );

            for hypothesis in hypotheses {
                result.hypotheses_tested += 1;

                match self.experiment_runner.verify(&hypothesis) {
                    Ok(exp_result) if exp_result.improved => {
                        result.hypotheses_succeeded += 1;
                        self.knowledge_base.learn(anomaly, &hypothesis.cause, &hypothesis.test_description, true);
                        self.event_log.log(Event::HypothesisVerified {
                            cause: hypothesis.cause.clone(),
                            improved: true,
                        })?;
                        break;
                    }
                    _ => {
                        self.knowledge_base.learn(anomaly, &hypothesis.cause, &hypothesis.test_description, false);
                        continue;
                    }
                }
            }
        }

        self.knowledge_base.prune();
        let _ = self.knowledge_base.save();

        Ok(())
    }
}
