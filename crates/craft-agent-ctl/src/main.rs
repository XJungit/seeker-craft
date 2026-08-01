//! craft-agent-ctl：Craft-Agent 运维控制台。
//! 单命令快速返回，避免频繁 shell 长命令/后台进程句柄问题。
//!
//! 用法：
//!   craft-agent-ctl status      # 全面状态：进程 + API + game-state 摘要 + 会话最近动作
//!   craft-agent-ctl stop        # 停止所有 craft-agent 进程
//!   craft-agent-ctl build       # 编译 viewer + autopilot exe
//!   craft-agent-ctl deploy      # stop → build → 启动 viewer + autopilot → start agent
//!   craft-agent-ctl goal "<g>"  # 注入新 goal
//!   craft-agent-ctl start       # POST /api/start
//!   craft-agent-ctl session N   # 分析会话最近 N 个工具结果（默认 10）
//!   craft-agent-ctl tail F N    # 打印日志文件尾部 N 行
//!   craft-agent-ctl health      # 持续健康检查（最多 10 分钟，检测到进步就退出）

use std::process::{Command, Stdio};
use std::time::Duration;

const BASE_URL: &str = "http://127.0.0.1:8080";
const VIEWER_EXE: &str = "D:\\Craft-Agent\\target\\debug\\craft-agent-viewer.exe";
const AUTOPILOT_EXE: &str = "D:\\Craft-Agent\\target\\debug\\craft-agent-autopilot.exe";
const SESSION: &str = "D:\\Craft-Agent\\sessions\\mc_run.jsonl";
const WORKSPACE: &str = "D:\\Craft-Agent";
const LOG_DIR: &str = "C:\\Windows\\TEMP\\opencode";
const GOAL: &str = "优先解决食物保障并恢复饥饿，然后继续生存主线，最终击败末影龙";

fn http_get(path: &str) -> Option<serde_json::Value> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .ok()?;
    client.get(format!("{BASE_URL}{path}")).send().ok()?.json().ok()
}

fn http_post(path: &str, body: serde_json::Value) -> Option<serde_json::Value> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()
        .ok()?;
    client
        .post(format!("{BASE_URL}{path}"))
        .json(&body)
        .send()
        .ok()?
        .json()
        .ok()
}

fn list_procs() -> Vec<(u32, String)> {
    let out = Command::new("tasklist")
        .args(["/FO", "CSV", "/NH"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();
    let mut procs = Vec::new();
    for line in out.lines() {
        if line.contains("craft-agent") {
            let parts: Vec<&str> = line.split(',').collect();
            if parts.len() >= 2 {
                let name = parts[0].trim_matches('"').to_string();
                let pid: u32 = parts[1].trim_matches('"').parse().unwrap_or(0);
                if pid > 0 {
                    procs.push((pid, name));
                }
            }
        }
    }
    procs
}

fn kill_all() {
    let procs = list_procs();
    if procs.is_empty() {
        println!("[ctl] no craft-agent processes running");
        return;
    }
    for (pid, name) in &procs {
        let _ = Command::new("taskkill")
            .args(["/F", "/PID", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        println!("[ctl] killed {name} pid={pid}");
    }
}

fn spawn_detached(exe: &str, args: &[&str], out_log: &str) -> bool {
    let out = std::fs::File::create(format!("{LOG_DIR}\\{out_log}")).ok();
    let err = std::fs::File::create(format!("{LOG_DIR}\\{out_log}.err")).ok();
    let mut cmd = Command::new(exe);
    cmd.args(args);
    if let Some(f) = out {
        cmd.stdout(f);
    }
    if let Some(f) = err {
        cmd.stderr(f);
    }
    match cmd.spawn() {
        Ok(child) => {
            println!("[ctl] spawned {exe} pid={}", child.id());
            true
        }
        Err(e) => {
            println!("[ctl] spawn failed: {e}");
            false
        }
    }
}

fn tail_file(path: &str, n: usize) -> Vec<String> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    let lines: Vec<&str> = content.lines().collect();
    let start = lines.len().saturating_sub(n);
    lines[start..].iter().map(|s| s.to_string()).collect()
}

fn cmd_status() {
    let procs = list_procs();
    if procs.is_empty() {
        println!("[status] no craft-agent processes running");
    } else {
        for (pid, name) in &procs {
            println!("[status] proc {name} pid={pid}");
        }
    }
    match http_get("/api/status") {
        Some(v) => {
            let run = v["running"].as_bool().unwrap_or(false);
            let step = v["step"].as_u64().unwrap_or(0);
            let paused = v["paused"].as_bool().unwrap_or(false);
            let goal = v["goal"].as_str().unwrap_or("").to_string();
            println!("[status] api running={run} paused={paused} step={step}");
            println!("[status] goal: {}", truncate(&goal, 90));
        }
        None => println!("[status] api unreachable at {BASE_URL}"),
    }
    if let Some(v) = http_get("/api/game-state") {
        if let Some(desc) = v["scene_desc"].as_str() {
            println!("[status] --- game-state ---");
            for line in desc.lines().take(14) {
                println!("[status]   {line}");
            }
        }
    } else {
        println!("[status] game-state unavailable");
    }
    for (name, path) in [
        ("autopilot", "auto5_out.log"),
        ("viewer", "viewer_out.log"),
    ] {
        let lines = tail_file(&format!("{LOG_DIR}\\{path}"), 3);
        if !lines.is_empty() {
            println!("[status] --- {name} log tail ---");
            for l in lines {
                println!("[status]   {l}");
            }
        }
    }
}

fn cmd_session(n: usize) {
    let content = match std::fs::read_to_string(SESSION) {
        Ok(c) => c,
        Err(e) => {
            println!("[session] read failed: {e}");
            return;
        }
    };
    let lines: Vec<&str> = content.lines().collect();
    let mut shown = 0usize;
    let mut buf: Vec<(usize, String)> = Vec::new();
    for (i, l) in lines.iter().enumerate() {
        let o: Option<serde_json::Value> = serde_json::from_str(l).ok();
        let Some(o) = o else { continue };
        let Some(role) = o["message"]["role"].as_str() else { continue };
        if role == "toolresult" {
            let c = o["message"]["content"].as_str().unwrap_or("").to_string();
            buf.push((i, truncate(&c, 150)));
        } else if role == "assistant" {
            let calls = o["message"]["tool_calls"].as_array().cloned().unwrap_or_default();
            for t in &calls {
                let name = t["function"]["name"].as_str().unwrap_or("?");
                buf.push((i, format!("CALL {name}")));
            }
        }
    }
    for (i, c) in buf.iter().rev() {
        println!("[session] L{i} {c}");
        shown += 1;
        if shown >= n {
            break;
        }
    }
    if shown == 0 {
        println!("[session] no recent tool activity");
    }
}

fn cmd_goal(goal: &str) {
    match http_post("/api/goal", serde_json::json!({"goal": goal})) {
        Some(v) => println!("[ctl] goal response: {v}"),
        None => println!("[ctl] goal request failed"),
    }
}

fn cmd_start() {
    match http_post("/api/start", serde_json::json!({})) {
        Some(v) => println!("[ctl] start response: {v}"),
        None => println!("[ctl] start request failed"),
    }
}

fn cmd_build() {
    for pkg in ["craft-agent-viewer", "craft-agent-autopilot"] {
        let mut cmd = Command::new("cargo");
        cmd.args(["build", "-p", pkg]);
        cmd.current_dir(WORKSPACE);
        let out = cmd.output().expect("cargo build");
        let text = String::from_utf8_lossy(&out.stderr).to_string();
        if text.contains("Finished") {
            println!("[build] {pkg} OK");
        } else {
            println!("[build] {pkg} FAILED:");
            for l in text.lines().filter(|l| l.contains("error")).take(8) {
                println!("  {l}");
            }
        }
    }
}

fn cmd_deploy() {
    println!("[deploy] stopping all");
    kill_all();
    std::thread::sleep(Duration::from_secs(3));
    println!("[deploy] building");
    cmd_build();
    println!("[deploy] spawning viewer");
    spawn_detached(
        VIEWER_EXE,
        &[
            "--goal",
            GOAL,
            "--steps",
            "0",
            "--port",
            "8080",
            "--mc",
            "localhost:4444",
            "--username",
            "CraftAgent",
        ],
        "viewer_out.log",
    );
    std::thread::sleep(Duration::from_secs(1));
    println!("[deploy] spawning autopilot");
    spawn_detached(AUTOPILOT_EXE, &[], "auto5_out.log");
    std::thread::sleep(Duration::from_secs(12));
    // 等 autopilot 自动 start；若未 running 则手动 start
    let running = http_get("/api/status")
        .map(|v| v["running"].as_bool().unwrap_or(false))
        .unwrap_or(false);
    if !running {
        cmd_start();
    }
    println!("[deploy] done");
    cmd_status();
}

fn cmd_health(max_secs: u64) {
    let deadline = std::time::Instant::now() + Duration::from_secs(max_secs);
    let mut last_step: u64 = u64::MAX;
    while std::time::Instant::now() < deadline {
        let Some(v) = http_get("/api/status") else {
            println!("[health] api unreachable");
            std::thread::sleep(Duration::from_secs(5));
            continue;
        };
        let running = v["running"].as_bool().unwrap_or(false);
        let step = v["step"].as_u64().unwrap_or(0);
        if !running {
            println!("[health] agent NOT running (step={step})");
            return;
        }
        if step != last_step {
            println!("[health] step={step} (+{})", step.saturating_sub(if last_step == u64::MAX { step } else { last_step }));
            last_step = step;
        }
        std::thread::sleep(Duration::from_secs(5));
    }
    println!("[health] {max_secs}s elapsed, last step={last_step}");
}

fn truncate(s: &str, n: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= n {
        s.to_string()
    } else {
        chars[..n].iter().collect::<String>() + "…"
    }
}

fn usage() {
    println!(
        "usage: craft-agent-ctl <status|stop|build|deploy|goal|start|session|tail|health>"
    );
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(|s| s.as_str()).unwrap_or("status");
    match cmd {
        "status" => cmd_status(),
        "stop" => kill_all(),
        "build" => cmd_build(),
        "deploy" => cmd_deploy(),
        "goal" => {
            if let Some(g) = args.get(2) {
                cmd_goal(g);
            } else {
                usage();
            }
        }
        "start" => cmd_start(),
        "session" => {
            let n = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(10);
            cmd_session(n);
        }
        "tail" => {
            let path = args.get(2).map(|s| s.as_str()).unwrap_or("auto5_out.log");
            let n = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(10);
            for l in tail_file(&format!("{LOG_DIR}\\{path}"), n) {
                println!("{l}");
            }
        }
        "health" => {
            let secs = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(300);
            cmd_health(secs);
        }
        _ => usage(),
    }
}
