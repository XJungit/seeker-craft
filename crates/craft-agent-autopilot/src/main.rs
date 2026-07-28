//! Craft-Agent 全自动化自进化主循环
//!
//! 无人值守连续运行，自动：
//! 1. 编译 + 测试
//! 2. LLM 实机测试
//! 3. 全量事件记录
//! 4. 异常检测
//! 5. 根因分析
//! 6. 假设验证
//! 7. 知识沉淀
//! 8. 归档 + git commit
//!
//! 循环直至击败末影龙或手动停止

mod anomaly;
mod decision;
mod event_log;
mod experiment;
mod git;
mod hypothesis;
mod knowledge;
mod monitor;
mod orchestrator;
mod root_cause;
mod web_research;

use anyhow::Result;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct AutopilotConfig {
    pub workspace_root: PathBuf,
    pub max_runtime_hours: Option<u64>,
    pub mc_addr: String,
    pub viewer_port: u16,
    pub llm_goal: String,
}

impl Default for AutopilotConfig {
    fn default() -> Self {
        Self {
            workspace_root: PathBuf::from("."),
            max_runtime_hours: None,
            mc_addr: "localhost:4444".into(),
            viewer_port: 8080,
            llm_goal: "继续推进当前任务".into(),
        }
    }
}

pub struct Autopilot {
    config: AutopilotConfig,
    orchestrator: orchestrator::Orchestrator,
    round: u64,
}

impl Autopilot {
    pub fn new(config: AutopilotConfig) -> Self {
        let orchestrator = orchestrator::Orchestrator::new(
            config.workspace_root.clone(),
            config.mc_addr.clone(),
            config.viewer_port,
        );
        Self {
            config,
            orchestrator,
            round: 0,
        }
    }

    pub async fn run_round(&mut self) -> Result<orchestrator::RoundResult> {
        self.round += 1;
        self.orchestrator.run_round(self.round).await
    }

    pub async fn commit_and_archive(&self, result: &orchestrator::RoundResult) -> Result<()> {
        git::commit(&format!("auto: round {}, {}", self.round, result.summary()))?;
        Ok(())
    }

    pub async fn global_review(&self) -> Result<()> {
        println!("[GlobalReview] 触发全局回顾...");
        // TODO: 全面分析瓶颈
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    println!("╔══════════════════════════════════════════════════════════╗");
    println!("║     Craft-Agent Autopilot — 全自动化自进化系统           ║");
    println!("║     目标: 击败末影龙                                      ║");
    println!("╚══════════════════════════════════════════════════════════╝");

    let config = AutopilotConfig::default();
    let mut autopilot = Autopilot::new(config);

    let start_time = std::time::Instant::now();
    let mut no_progress_streak = 0u32;

    loop {
        // 检查运行时长限制
        if let Some(max_hours) = autopilot.config.max_runtime_hours {
            if start_time.elapsed().as_secs() > max_hours * 3600 {
                println!("⏰ 达到最大运行时长 {} 小时，停止", max_hours);
                break;
            }
        }

        println!("\n=== Round {} ===", autopilot.round + 1);

        match autopilot.run_round().await {
            Ok(result) => {
                println!("Round {}: {}", autopilot.round, result.summary());

                if result.ender_dragon_defeated {
                    println!("🎉🎉🎉 末影龙已被击败！任务完成！🎉🎉🎉");
                    break;
                }

                if result.has_progress {
                    no_progress_streak = 0;
                } else {
                    no_progress_streak += 1;
                }

                if no_progress_streak >= 5 {
                    println!("⚠ 连续 5 轮无进展，触发全局回顾...");
                    autopilot.global_review().await?;
                    no_progress_streak = 0;
                }

                if let Err(e) = autopilot.commit_and_archive(&result).await {
                    eprintln!("Commit failed: {e}");
                }
            }
            Err(e) => {
                eprintln!("Round {} failed: {e}", autopilot.round + 1);
                tokio::time::sleep(Duration::from_secs(30)).await;
            }
        }
    }

    Ok(())
}
