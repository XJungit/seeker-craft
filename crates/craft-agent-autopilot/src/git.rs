//! Git 操作

use anyhow::Result;
use std::path::Path;
use std::process::Command;

pub fn commit(message: &str) -> Result<()> {
    let _ = Command::new("git").args(["add", "-A"]).output();
    let output = Command::new("git")
        .args(["commit", "--no-verify", "-m", message])
        .output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("nothing to commit") {
            // Not an error
        } else {
            anyhow::bail!("git commit failed: {stderr}");
        }
    }
    Ok(())
}

pub fn revert_last() -> Result<()> {
    let _ = Command::new("git").args(["revert", "--no-verify", "HEAD"]).output();
    Ok(())
}

pub fn diff() -> Result<String> {
    let output = Command::new("git").args(["diff", "HEAD~1"]).output()?;
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

pub fn status() -> Result<String> {
    let output = Command::new("git").args(["status", "--short"]).output()?;
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}
