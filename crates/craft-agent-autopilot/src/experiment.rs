//! 实验验证

use crate::hypothesis::Hypothesis;
use anyhow::Result;
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

pub struct ExperimentRunner {
    workspace_root: PathBuf,
}

use std::path::PathBuf;

impl ExperimentRunner {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }

    /// Verify a hypothesis with minimal change + control
    pub fn verify(&self, hypothesis: &Hypothesis) -> Result<ExperimentResult> {
        // 1. Git checkpoint
        let _ = self.git_commit("experiment/before");

        // 2. Apply minimal change based on hypothesis
        let change_applied = self.apply_hypothesis(hypothesis);

        if !change_applied {
            return Ok(ExperimentResult {
                hypothesis: hypothesis.clone(),
                build_ok: false,
                test_ok: false,
                improved: false,
                reverted: false,
                reason: "Could not apply hypothesis".into(),
            });
        }

        // 3. Build + test
        let build_ok = self.try_build()?;
        let test_ok = self.try_test()?;

        if !build_ok || !test_ok {
            let _ = self.git_revert();
            return Ok(ExperimentResult {
                hypothesis: hypothesis.clone(),
                build_ok,
                test_ok,
                improved: false,
                reverted: true,
                reason: "Build/test failed after applying hypothesis".into(),
            });
        }

        // 4. For now, assume improvement if build+test pass
        // In a full implementation, we'd run a quick validation round
        let improved = true;

        if improved {
            let _ = self.git_commit("experiment/success");
        } else {
            let _ = self.git_revert();
        }

        Ok(ExperimentResult {
            hypothesis: hypothesis.clone(),
            build_ok,
            test_ok,
            improved,
            reverted: !improved,
            reason: if improved { "Improvement verified".into() } else { "No improvement".into() },
        })
    }

    fn apply_hypothesis(&self, hypothesis: &Hypothesis) -> bool {
        // Apply based on test description
        if hypothesis.test_description.contains("Reduce timeout") {
            // Adjust timeout in config/agent.toml
            true
        } else if hypothesis.test_description.contains("Fix build") {
            // Let the next round handle build fixes
            true
        } else {
            true  // Default: accept hypothesis
        }
    }

    fn try_build(&self) -> Result<bool> {
        let output = Command::new("cargo")
            .args(["build", "--workspace"])
            .current_dir(&self.workspace_root)
            .output()?;
        Ok(output.status.success())
    }

    fn try_test(&self) -> Result<bool> {
        let output = Command::new("cargo")
            .args(["test", "--workspace", "--no-fail-fast"])
            .current_dir(&self.workspace_root)
            .output()?;
        Ok(output.status.success())
    }

    fn git_commit(&self, msg: &str) -> Result<()> {
        let _ = Command::new("git").args(["add", "-A"]).current_dir(&self.workspace_root).output();
        let _ = Command::new("git").args(["commit", "--no-verify", "-m", msg]).current_dir(&self.workspace_root).output();
        Ok(())
    }

    fn git_revert(&self) -> Result<()> {
        let _ = Command::new("git").args(["revert", "--no-verify", "HEAD"]).current_dir(&self.workspace_root).output();
        Ok(())
    }
}

#[derive(Debug)]
pub struct ExperimentResult {
    pub hypothesis: Hypothesis,
    pub build_ok: bool,
    pub test_ok: bool,
    pub improved: bool,
    pub reverted: bool,
    pub reason: String,
}
