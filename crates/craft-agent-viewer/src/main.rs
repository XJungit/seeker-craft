//! craft-agent-viewer：Agent 控制面板 + 运行仪表盘。
//!
//! 启动后浏览器打开即可：启动/暂停/停止 agent，实时观察每轮决策与工具调用。
//! Agent 循环在后台线程运行（mod 桥接路径），事件通过 SSE 推送到前端。
//!
//! ```bash
//! cargo run -p craft-agent-viewer --features mod-bridge -- \
//!   --goal "收集木头做工作台" --steps 40 --vision
//! ```

mod agent_loop;

use agent_loop::{AgentController, AgentEvent, spawn_agent_loop};
use axum::{
    Router,
    extract::{Path as AxumPath, State},
    http::{HeaderMap, StatusCode, header},
    response::{
        Html, IntoResponse,
        sse::{Event, Sse},
    },
    routing::{get, post},
};
use craft_agent::core::message::Message;
use craft_agent::core::session::{SessionEntry, SessionHeader};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::convert::Infallible;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;

/// 跨 handler 共享状态。
struct AppState {
    session_path: PathBuf,
    controller: Arc<AgentController>,
    event_tx: broadcast::Sender<AgentEvent>,
    /// Agent 配置
    model_config_path: String,
    use_vision: bool,
    shots_dir: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();
    let args: Vec<String> = std::env::args().collect();

    let mut session_path = PathBuf::from("sessions/mc_run.jsonl");
    let mut port: u16 = 8080;
    let mut goal = "收集木头做工作台".to_string();
    let mut max_steps: u32 = 0; // 0 = 无限循环，仅手动停止才退出
    let mut use_vision = false;
    let mut config_path = "config/agent.toml".to_string();

    let mut i = 1;
    while i < args.len() {
        let (key, inline_val) = match args[i].split_once('=') {
            Some((k, v)) => (k, Some(v.to_string())),
            None => (args[i].as_str(), None),
        };
        let has_inline = inline_val.is_some();
        let get_val = || inline_val.clone().or_else(|| args.get(i + 1).cloned());
        match key {
            "--session" | "-s" => {
                if let Some(v) = get_val() {
                    session_path = PathBuf::from(v);
                }
                i += if has_inline { 1 } else { 2 };
            }
            "--port" | "-p" => {
                if let Some(v) = get_val() {
                    if let Ok(p) = v.parse() {
                        port = p;
                    }
                }
                i += if has_inline { 1 } else { 2 };
            }
            "--goal" | "-g" => {
                if let Some(v) = get_val() {
                    goal = v;
                }
                i += if has_inline { 1 } else { 2 };
            }
            "--steps" | "-n" => {
                if let Some(v) = get_val() {
                    if let Ok(n) = v.parse() {
                        max_steps = n;
                    }
                }
                i += if has_inline { 1 } else { 2 };
            }
            "--vision" | "-v" => {
                use_vision = true;
                i += 1;
            }
            "--config" | "-c" => {
                if let Some(v) = get_val() {
                    config_path = v;
                }
                i += if has_inline { 1 } else { 2 };
            }
            _ => i += 1,
        }
    }

    let shots_dir: Option<PathBuf> = Some({
        let p = Path::new(&session_path);
        let parent = p.parent().unwrap_or_else(|| Path::new("."));
        let stem = p
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "session".to_string());
        parent.join(format!("{stem}.shots"))
    });

    let (event_tx, _) = broadcast::channel::<AgentEvent>(128);
    let controller = Arc::new(AgentController::new(
        goal,
        max_steps,
        session_path.display().to_string(),
    ));

    let state = Arc::new(AppState {
        session_path: session_path.clone(),
        controller: controller.clone(),
        event_tx: event_tx.clone(),
        model_config_path: config_path,
        use_vision,
        shots_dir,
    });

    let app = Router::new()
        .route("/", get(index))
        .route("/api/session", get(api_session))
        .route("/api/shot/{name}", get(api_shot))
        .route("/api/status", get(api_status))
        .route("/api/start", post(api_start))
        .route("/api/stop", post(api_stop))
        .route("/api/pause", post(api_pause))
        .route("/api/goal", post(api_goal))
        .route("/api/events", get(api_events))
        .with_state(state.clone());

    let addr = format!("127.0.0.1:{port}");
    let listener = TcpListener::bind(&addr).await?;
    println!("🎮 Craft-Agent 控制面板 → http://{addr}");
    let steps_text = if max_steps > 0 {
        max_steps.to_string()
    } else {
        "无限循环".to_string()
    };
    println!("   目标: {}", &state.controller.get_status().goal);
    println!("   步数: {steps_text}");
    println!("   VLM: {}", if use_vision { "启用" } else { "关闭" });
    println!("   Session: {}", session_path.display());
    axum::serve(listener, app).await?;
    Ok(())
}

async fn index() -> Html<&'static str> {
    Html(INDEX_HTML)
}

// ── 控制 API ──

async fn api_status(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    axum::Json(state.controller.get_status())
}

async fn api_start(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let result = spawn_agent_loop(
        state.controller.clone(),
        state.model_config_path.clone(),
        state.use_vision,
        state.shots_dir.clone(),
        state.event_tx.clone(),
    );
    match result {
        Ok(()) => axum::Json(json!({"ok": true})),
        Err(e) => axum::Json(json!({"ok": false, "error": format!("{e}")})),
    }
}

async fn api_stop(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    state.controller.request_stop();
    axum::Json(json!({"ok": true}))
}

async fn api_pause(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    state.controller.toggle_pause();
    let s = state.controller.get_status();
    axum::Json(json!({"ok": true, "paused": s.paused}))
}

async fn api_goal(
    State(state): State<Arc<AppState>>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> impl IntoResponse {
    let goal = body.get("goal").and_then(|v| v.as_str()).unwrap_or("");
    if goal.is_empty() {
        return axum::Json(json!({"ok": false, "error": "goal 不能为空"}));
    }
    state.controller.push_goal(goal.to_string());
    axum::Json(json!({"ok": true, "goal": goal}))
}

// ── SSE 事件流 ──

async fn api_events(
    State(state): State<Arc<AppState>>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>> {
    let rx = state.event_tx.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|r| match r {
        Ok(event) => {
            let data = serde_json::to_string(&event).unwrap_or_default();
            Some(Ok(Event::default().data(data)))
        }
        Err(_) => None,
    });
    Sse::new(stream)
}

// ── 现有仪表盘 API ──

async fn api_session(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    match build_view(&state.session_path) {
        Ok(v) => (StatusCode::OK, axum::Json(v)).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("读取 session 失败: {e}"),
        )
            .into_response(),
    }
}

fn shots_dir_for(session: &Path) -> PathBuf {
    let parent = session.parent().unwrap_or_else(|| Path::new("."));
    let stem = session
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "session".to_string());
    parent.join(format!("{stem}.shots"))
}

fn extract_shot(content: &str, shots_dir: &Path) -> Option<String> {
    let pos = content.find("已落盘 ")?;
    let start = pos + "已落盘 ".len();
    let rest = &content[start..];
    let end = rest.find([']', '）']).unwrap_or(rest.len());
    let p = &rest[..end];
    if p.is_empty() {
        return None;
    }
    let base = Path::new(p).file_name()?.to_string_lossy().to_string();
    if shots_dir.join(&base).exists() {
        Some(base)
    } else {
        None
    }
}

async fn api_shot(
    State(state): State<Arc<AppState>>,
    AxumPath(name): AxumPath<String>,
) -> impl IntoResponse {
    let base = Path::new(&name)
        .file_name()
        .map(|f| f.to_string_lossy().to_string());
    let Some(base) = base else {
        return (StatusCode::BAD_REQUEST, "invalid name").into_response();
    };
    let full = shots_dir_for(&state.session_path).join(&base);
    match std::fs::read(&full) {
        Ok(bytes) => {
            let mut headers = HeaderMap::new();
            headers.insert(header::CONTENT_TYPE, "image/png".parse().unwrap());
            (StatusCode::OK, headers, bytes).into_response()
        }
        Err(_) => (StatusCode::NOT_FOUND, "not found").into_response(),
    }
}

fn build_view(path: &Path) -> anyhow::Result<Value> {
    let shots_dir = shots_dir_for(path);
    let text = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("无法读取 {}: {e}", path.display()))?;
    let mut lines = text.lines();

    let header: Option<SessionHeader> = lines
        .next()
        .and_then(|l| serde_json::from_str::<SessionHeader>(l).ok());

    let mut events: Vec<Value> = Vec::new();
    let mut tool_total: u32 = 0;
    let mut by_tool: BTreeMap<String, u32> = BTreeMap::new();

    for l in lines {
        let t = l.trim();
        if t.is_empty() {
            continue;
        }
        let Ok(entry) = serde_json::from_str::<SessionEntry>(t) else {
            continue;
        };
        match entry {
            SessionEntry::Message(m) => match m.message {
                Message::User(u) => events.push(json!({"kind":"user","content":u.content})),
                Message::Assistant(a) => {
                    let calls: Vec<Value> = a
                        .tool_calls
                        .iter()
                        .map(|tc| {
                            let args = if tc.arguments.is_null() {
                                String::new()
                            } else {
                                serde_json::to_string_pretty(&tc.arguments).unwrap_or_default()
                            };
                            json!({"id":tc.id,"name":tc.name,"args":args})
                        })
                        .collect();
                    events.push(json!({
                        "kind":"assistant","content":a.content,
                        "reasoning":a.reasoning,"tool_calls":calls
                    }));
                }
                Message::ToolResult(t) => {
                    tool_total += 1;
                    *by_tool.entry(t.tool_name.clone()).or_insert(0) += 1;
                    let shot = if t.tool_name == "perceive" {
                        extract_shot(&t.content, &shots_dir)
                    } else {
                        None
                    };
                    events.push(json!({
                        "kind":"tool","call_id":t.tool_call_id,"name":t.tool_name,
                        "content":t.content,"is_error":t.is_error,"shot":shot
                    }));
                }
            },
            SessionEntry::WorldInfo(w) => {
                events.push(json!({"kind":"knowledge","action":w.action}))
            }
            _ => {}
        }
    }

    let turns = events.iter().filter(|e| e["kind"] == "user").count()
        + events.iter().filter(|e| e["kind"] == "assistant").count();

    Ok(json!({
        "path": path.display().to_string(),
        "game": header.as_ref().map(|h| h.game.clone()),
        "knowledge_bootstrapped": header.as_ref().map(|h| h.knowledge_bootstrapped).unwrap_or(false),
        "turns": turns,
        "tool_total": tool_total,
        "by_tool": by_tool,
        "events": events,
    }))
}

const INDEX_HTML: &str = include_str!("index.html");
