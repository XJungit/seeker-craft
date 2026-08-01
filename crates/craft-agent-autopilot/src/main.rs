//! Synchronous supervisor for the Viewer and Minecraft agent.

mod session_analysis;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use session_analysis::{SessionAnalysis, analyze_session};
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const MONITOR_INTERVAL: Duration = Duration::from_secs(10);
const STALL_TIMEOUT: Duration = Duration::from_secs(240);
const MAX_STATUS_FAILURES: u32 = 3;
const MIN_PROGRESS_DISTANCE: f64 = 2.0;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum SupervisorPhase {
    Starting,
    Monitoring,
    RecoverRuntime,
    SteeringStall,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SupervisorState {
    phase: SupervisorPhase,
    updated_at_ms: u128,
    recovery_count: u64,
    stall_count: u64,
    last_error: Option<String>,
}

impl Default for SupervisorState {
    fn default() -> Self {
        Self {
            phase: SupervisorPhase::Starting,
            updated_at_ms: now_ms(),
            recovery_count: 0,
            stall_count: 0,
            last_error: None,
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Craft-Agent Autopilot: goal=defeat_ender_dragon username=CraftAgent");

    let workspace_root = PathBuf::from(".");
    let session_path = workspace_root.join("sessions").join("mc_run.jsonl");
    let event_path = workspace_root
        .join("sessions")
        .join("events")
        .join("workflow.jsonl");
    let state_path = workspace_root
        .join("sessions")
        .join("events")
        .join("supervisor_state.json");
    let mut supervisor = load_supervisor_state(&state_path);
    supervisor.phase = SupervisorPhase::Starting;
    persist_supervisor_state(&state_path, &supervisor)?;
    let viewer_port = 8080;
    let base_url = format!("http://127.0.0.1:{viewer_port}");
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()?;

    print!("[startup] checking workspace... ");
    io::stdout().flush()?;
    let status = Command::new("cargo")
        .args(["check", "--workspace", "--exclude", "craft-agent-autopilot"])
        .current_dir(&workspace_root)
        .status()?;
    if !status.success() {
        return Err("workspace check failed; refusing to run an unverified bot".into());
    }
    println!("ok");

    let rollover_on_start = std::env::var_os("CRAFT_AGENT_ROLLOVER_SESSION").is_some();
    let mut viewer = ensure_viewer(
        &workspace_root,
        &base_url,
        viewer_port,
        rollover_on_start,
    )?;
    ensure_agent_started(&client, &base_url)?;

    let mut baseline = analyze_session(&session_path);
    let mut previous_game_state = try_get_json(&client, &format!("{base_url}/api/game-state"));
    let mut last_progress = Instant::now();
    let mut status_failures = 0_u32;
    supervisor.phase = SupervisorPhase::Monitoring;
    supervisor.last_error = None;
    persist_supervisor_state(&state_path, &supervisor)?;
    println!("[monitor] {}", baseline.summary);

    loop {
        std::thread::sleep(MONITOR_INTERVAL);

        if let Some(child) = viewer.as_mut()
            && let Some(exit) = child.try_wait()?
        {
            recover_runtime(
                &workspace_root,
                &client,
                &base_url,
                viewer_port,
                &state_path,
                &mut supervisor,
                format!("viewer exited unexpectedly: {exit}"),
                &mut viewer,
            )?;
            status_failures = 0;
            last_progress = Instant::now();
            continue;
        }

        let status = match get_json(&client, &format!("{base_url}/api/status")) {
            Ok(status) => {
                if status_failures > 0 {
                    println!(
                        "[monitor] status endpoint recovered after {status_failures} failure(s)"
                    );
                }
                status_failures = 0;
                status
            }
            Err(error) => {
                status_failures += 1;
                eprintln!(
                    "[monitor] status request failed ({status_failures}/{MAX_STATUS_FAILURES}): {error}"
                );
                if status_failures >= MAX_STATUS_FAILURES {
                    recover_runtime(
                        &workspace_root,
                        &client,
                        &base_url,
                        viewer_port,
                        &state_path,
                        &mut supervisor,
                        format!(
                            "status endpoint failed {status_failures} consecutive times: {error}"
                        ),
                        &mut viewer,
                    )?;
                    status_failures = 0;
                    last_progress = Instant::now();
                }
                continue;
            }
        };
        if !status["running"].as_bool().unwrap_or(false) {
            recover_runtime(
                &workspace_root,
                &client,
                &base_url,
                viewer_port,
                &state_path,
                &mut supervisor,
                "Agent stopped; replacing Viewer to avoid duplicate_login".into(),
                &mut viewer,
            )?;
            last_progress = Instant::now();
            continue;
        }

        let game_state = try_get_json(&client, &format!("{base_url}/api/game-state"));
        let current = analyze_session(&session_path);
        record_observation(&event_path, &status, game_state.as_ref(), &current)?;
        let game_progress = match (&previous_game_state, &game_state) {
            (Some(previous), Some(current)) => game_state_changed(previous, current),
            _ => false,
        };
        if game_progress || current.has_progress_since(&baseline) {
            println!(
                "[progress] {} game_state_changed={game_progress}",
                current.delta_summary(&baseline)
            );
            baseline = current;
            if game_state.is_some() {
                previous_game_state = game_state;
            }
            last_progress = Instant::now();
            continue;
        }

        let stalled_for = last_progress.elapsed();
        println!(
            "[monitor] step={} no verified progress for {}s",
            status["step"].as_u64().unwrap_or(0),
            stalled_for.as_secs()
        );
        if stalled_for >= STALL_TIMEOUT {
            supervisor.phase = SupervisorPhase::SteeringStall;
            supervisor.stall_count += 1;
            persist_supervisor_state(&state_path, &supervisor)?;
            steer_stalled_agent(&client, &base_url, supervisor.stall_count, &current)?;
            supervisor.phase = SupervisorPhase::Monitoring;
            persist_supervisor_state(&state_path, &supervisor)?;
            baseline = current;
            last_progress = Instant::now();
        }
    }
}

fn game_state_changed(previous: &Value, current: &Value) -> bool {
    position_changed(previous, current)
        || previous.get("inventory") != current.get("inventory")
        || previous.get("experience_level") != current.get("experience_level")
        || previous.get("gamemode") != current.get("gamemode")
        // P72: scene_desc 全文变化也算进展——inventory 顶层字段经常为 None
        // （BotEvent::State 不含背包），bot 小范围挖矿位置变化 <2m 时被误判停滞，
        // 导致 autopilot 每 3 分钟注入一次 steering goal 打断 LLM 节奏（实测 42 次）。
        // scene_desc 含"位置/背包/附近"实时行，任何实质变化都说明 LLM 在推进。
        // 误报（蝙蝠飞过也算）无害——只是不判停滞。
        || previous.get("scene_desc") != current.get("scene_desc")
}

fn position_changed(previous: &Value, current: &Value) -> bool {
    let Some(previous) = position(previous) else {
        return false;
    };
    let Some(current) = position(current) else {
        return false;
    };
    previous
        .iter()
        .zip(current)
        .map(|(a, b)| (a - b).powi(2))
        .sum::<f64>()
        >= MIN_PROGRESS_DISTANCE.powi(2)
}

fn position(state: &Value) -> Option<[f64; 3]> {
    let values = state.get("position")?.as_array()?;
    Some([
        values.first()?.as_f64()?,
        values.get(1)?.as_f64()?,
        values.get(2)?.as_f64()?,
    ])
}

fn record_observation(
    path: &Path,
    status: &Value,
    game_state: Option<&Value>,
    analysis: &SessionAnalysis,
) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let event = json!({
        "timestamp_ms": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis(),
        "status": status,
        "game_state_available": game_state.is_some(),
        "position": game_state.and_then(|state| state.get("position")),
        "health": game_state.and_then(|state| state.get("health")),
        "hunger": game_state.and_then(|state| state.get("hunger")),
        "experience_level": game_state.and_then(|state| state.get("experience_level")),
        "gamemode": game_state.and_then(|state| state.get("gamemode")),
        "inventory": game_state.and_then(|state| state.get("inventory")),
        "session": {
            "assistant_steps": analysis.assistant_steps,
            "tool_calls": analysis.tool_calls,
            "errors": analysis.errors,
            "productive_tools": analysis.successful_productive_tools,
        }
    });
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    serde_json::to_writer(&mut file, &event)?;
    file.write_all(b"\n")?;
    file.flush()?;
    Ok(())
}

fn ensure_viewer(
    workspace_root: &Path,
    base_url: &str,
    viewer_port: u16,
    rollover_session: bool,
) -> Result<Option<Child>, Box<dyn std::error::Error>> {
    if reqwest::blocking::get(format!("{base_url}/api/status")).is_ok() {
        println!("[viewer] reusing existing process");
        return Ok(None);
    }

    let viewer_exe = workspace_root
        .join("target")
        .join("debug")
        .join("craft-agent-viewer.exe");
    let mut command = Command::new(viewer_exe);
    command.args([
            "--goal",
            "优先解决食物保障并恢复饥饿，然后继续生存主线，最终击败末影龙",
            "--steps",
            "0",
            "--port",
            &viewer_port.to_string(),
            "--mc",
            "localhost:4444",
            "--username",
            "CraftAgent",
        ]);
    if rollover_session {
        command.arg("--rollover-session");
    }
    let child = command.current_dir(workspace_root).spawn()?;
    println!("[viewer] started pid={}", child.id());

    let deadline = Instant::now() + Duration::from_secs(60);
    while Instant::now() < deadline {
        if reqwest::blocking::get(format!("{base_url}/api/status")).is_ok() {
            return Ok(Some(child));
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    Err("viewer did not become ready within 60 seconds".into())
}

#[allow(clippy::too_many_arguments)]
fn recover_runtime(
    workspace_root: &Path,
    client: &reqwest::blocking::Client,
    base_url: &str,
    viewer_port: u16,
    state_path: &Path,
    supervisor: &mut SupervisorState,
    reason: String,
    viewer: &mut Option<Child>,
) -> Result<(), Box<dyn std::error::Error>> {
    supervisor.phase = SupervisorPhase::RecoverRuntime;
    supervisor.recovery_count += 1;
    supervisor.last_error = Some(reason.clone());
    persist_supervisor_state(state_path, supervisor)?;
    eprintln!("[recovery] {reason}");

    loop {
        if !minecraft_server_available() {
            supervisor.last_error = Some(
                "Minecraft server localhost:4444 is unavailable; preserving Viewer and waiting"
                    .into(),
            );
            persist_supervisor_state(state_path, supervisor)?;
            eprintln!("[recovery] Minecraft server unavailable; waiting without restarting Viewer");
            std::thread::sleep(Duration::from_secs(10));
            continue;
        }
        if let Some(child) = viewer.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        } else if let Err(error) = stop_viewer_on_port(viewer_port) {
            eprintln!("[recovery] failed to stop Viewer: {error}");
        }
        *viewer = None;
        if let Err(error) = wait_for_endpoint(base_url, false, Duration::from_secs(30)) {
            eprintln!("[recovery] waiting for old Viewer shutdown: {error}");
            std::thread::sleep(Duration::from_secs(5));
            continue;
        }
        match ensure_viewer(workspace_root, base_url, viewer_port, false)
            .and_then(|child| {
                *viewer = child;
                ensure_agent_started(client, base_url)
            }) {
            Ok(()) => break,
            Err(error) => {
                supervisor.last_error = Some(format!("runtime recovery retry: {error}"));
                persist_supervisor_state(state_path, supervisor)?;
                eprintln!("[recovery] retrying after error: {error}");
                std::thread::sleep(Duration::from_secs(10));
            }
        }
    }

    supervisor.phase = SupervisorPhase::Monitoring;
    supervisor.last_error = None;
    persist_supervisor_state(state_path, supervisor)?;
    Ok(())
}

fn minecraft_server_available() -> bool {
    std::net::TcpStream::connect_timeout(
        &"127.0.0.1:4444".parse().expect("static socket address"),
        Duration::from_secs(2),
    )
    .is_ok()
}

fn stop_viewer_on_port(port: u16) -> Result<(), Box<dyn std::error::Error>> {
    let query = format!(
        "$pids = Get-NetTCPConnection -State Listen -LocalPort {port} -ErrorAction SilentlyContinue | Select-Object -ExpandProperty OwningProcess -Unique; \
         foreach ($pidValue in $pids) {{ \
             $process = Get-Process -Id $pidValue -ErrorAction Stop; \
             if ($process.ProcessName -ne 'craft-agent-viewer') {{ \
                 throw \"Refusing to stop non-Viewer PID $pidValue ($($process.ProcessName)) on port {port}\" \
             }}; \
             Stop-Process -Id $pidValue -Force \
         }}"
    );
    let status = Command::new("powershell")
        .args(["-NoProfile", "-Command", &query])
        .status()?;
    if !status.success() {
        return Err(format!("refused or failed to stop Viewer on port {port}").into());
    }
    Ok(())
}

fn wait_for_endpoint(
    base_url: &str,
    available: bool,
    timeout: Duration,
) -> Result<(), Box<dyn std::error::Error>> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let is_available = reqwest::blocking::get(format!("{base_url}/api/status")).is_ok();
        if is_available == available {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Err(format!("Viewer endpoint did not become available={available} within {timeout:?}").into())
}

fn load_supervisor_state(path: &Path) -> SupervisorState {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn persist_supervisor_state(
    path: &Path,
    state: &SupervisorState,
) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut state = state.clone();
    state.updated_at_ms = now_ms();
    let temp = path.with_extension("json.tmp");
    std::fs::write(&temp, serde_json::to_vec_pretty(&state)?)?;
    if path.exists() {
        let backup = path.with_extension("json.bak");
        let _ = std::fs::remove_file(&backup);
        std::fs::rename(path, &backup)?;
        match std::fs::rename(&temp, path) {
            Ok(()) => {
                let _ = std::fs::remove_file(backup);
            }
            Err(error) => {
                let _ = std::fs::rename(backup, path);
                return Err(error.into());
            }
        }
    } else {
        std::fs::rename(temp, path)?;
    }
    Ok(())
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn ensure_agent_started(
    client: &reqwest::blocking::Client,
    base_url: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let status = get_json(client, &format!("{base_url}/api/status"))?;
    if status["running"].as_bool().unwrap_or(false) {
        println!("[agent] already running at step {}", status["step"]);
        return Ok(());
    }

    let response: Value = client
        .post(format!("{base_url}/api/start"))
        .send()?
        .error_for_status()?
        .json()?;
    if !response["ok"].as_bool().unwrap_or(false) {
        return Err(format!("agent start rejected: {response}").into());
    }

    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        let status = get_json(client, &format!("{base_url}/api/status"))?;
        if status["running"].as_bool().unwrap_or(false) {
            println!("[agent] started");
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Err("agent did not enter running state within 20 seconds".into())
}

fn steer_stalled_agent(
    client: &reqwest::blocking::Client,
    base_url: &str,
    stall_count: u64,
    analysis: &SessionAnalysis,
) -> Result<(), Box<dyn std::error::Error>> {
    let goal = format!(
        "检测到连续3分钟无真实进展（第{stall_count}次，{}）。立即停止重复动作，先perceive检查背包和位置；选择一个可验证的小里程碑（优先铁镐、铁甲、钻石、下界传送门、烈焰棒、末影之眼），执行后用perceive确认背包或位置确实变化。",
        analysis.summary
    );
    let response: Value = client
        .post(format!("{base_url}/api/goal"))
        .json(&json!({"goal": goal}))
        .send()?
        .error_for_status()?
        .json()?;
    println!("[recovery] steering result={response}");
    Ok(())
}

fn get_json(
    client: &reqwest::blocking::Client,
    url: &str,
) -> Result<Value, Box<dyn std::error::Error>> {
    Ok(client.get(url).send()?.error_for_status()?.json()?)
}

fn try_get_json(client: &reqwest::blocking::Client, url: &str) -> Option<Value> {
    match get_json(client, url) {
        Ok(value) => Some(value),
        Err(error) => {
            eprintln!("[monitor] optional observation failed: {error}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::game_state_changed;
    use serde_json::json;

    #[test]
    fn ignores_small_position_jitter() {
        let previous = json!({"position": [1.0, 64.0, 1.0], "inventory": []});
        let current = json!({"position": [1.4, 64.0, 1.3], "inventory": []});

        assert!(!game_state_changed(&previous, &current));
    }

    #[test]
    fn detects_meaningful_position_change() {
        let previous = json!({"position": [1.0, 64.0, 1.0], "inventory": []});
        let current = json!({"position": [3.0, 64.0, 1.0], "inventory": []});

        assert!(game_state_changed(&previous, &current));
    }

    #[test]
    fn detects_inventory_change_without_movement() {
        let previous = json!({"position": [1.0, 64.0, 1.0], "inventory": []});
        let current = json!({
            "position": [1.0, 64.0, 1.0],
            "inventory": [{"id": "minecraft:diamond", "count": 1}]
        });

        assert!(game_state_changed(&previous, &current));
    }
}
