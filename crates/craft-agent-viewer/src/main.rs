//! craft-agent-viewer：Agent 控制面板 + 运行仪表盘。
//!
//! 启动后浏览器打开即可：启动/暂停/停止 agent，实时观察每轮决策与工具调用。
//! Agent 循环在后台线程运行（azalea 客户端路线），事件通过 SSE 推送到前端。
//!
//! ```bash
//! cargo run -p craft-agent-viewer -- \
//!   --goal "收集木头做工作台" --steps 40
//! ```

mod agent_loop;

use agent_loop::{AgentController, AgentEvent, spawn_agent_loop};
use axum::{
    Router,
    extract::State,
    http::{StatusCode},
    response::{
        Html, IntoResponse,
        sse::{Event, Sse},
    },
    routing::{get, post},
};
use craft_agent::core::message::Message;
use craft_agent::core::session::{SessionEntry, SessionHeader};
use craft_agent::core::types::WorldState;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::convert::Infallible;
use std::path::Path;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
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
    /// MC 服务器地址（azalea 连接用，如 localhost:4444）
    mc_addr: String,
    /// 最后一次成功拉取的游戏状态（离线时回退显示）
    last_state_cache: Mutex<Option<WorldState>>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();
    let args: Vec<String> = std::env::args().collect();

    let mut session_path = PathBuf::from("sessions/mc_run.jsonl");
    let mut port: u16 = 8080;
    let mut goal = "收集木头做工作台".to_string();
    let mut max_steps: u32 = 0; // 0 = 无限循环，仅手动停止才退出
    let mut config_path = "config/agent.toml".to_string();
    let mut mc_addr = "localhost:4444".to_string();

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
                if let Some(v) = get_val()
                    && let Ok(p) = v.parse()
                {
                    port = p;
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
                if let Some(v) = get_val()
                    && let Ok(n) = v.parse()
                {
                    max_steps = n;
                }
                i += if has_inline { 1 } else { 2 };
            }
            "--config" | "-c" => {
                if let Some(v) = get_val() {
                    config_path = v;
                }
                i += if has_inline { 1 } else { 2 };
            }
            "--mc" => {
                if let Some(v) = get_val() {
                    mc_addr = v;
                }
                i += if has_inline { 1 } else { 2 };
            }
            _ => i += 1,
        }
    }

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
        mc_addr,
        last_state_cache: Mutex::new(None),
    });

    let app = Router::new()
        .route("/", get(index))
        .route("/api/session", get(api_session))
        .route("/api/status", get(api_status))
        .route("/api/start", post(api_start))
        .route("/api/stop", post(api_stop))
        .route("/api/pause", post(api_pause))
        .route("/api/goal", post(api_goal))
        .route("/api/events", get(api_events))
        .route("/api/game-state", get(api_game_state))
        .with_state(state.clone());

    let addr = format!("127.0.0.1:{port}");
    let listener = TcpListener::bind(&addr).await?;
    println!("🎮 Craft-Agent 控制面板 → http://{addr}");
    let steps_text = if max_steps > 0 {
        max_steps.to_string()
    } else {
        "无限循环".to_string()
    };
    println!("   目标: {}", state.controller.get_status().goal);
    println!("   步数: {steps_text}");
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
        state.event_tx.clone(),
        state.mc_addr.clone(),
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

// ── 游戏状态 API ──

async fn api_game_state(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let adapter = state.controller.game_adapter.read().unwrap().clone();
    if let Some(arc) = adapter
    {
        // 实时拉取 azalea 世界状态（perceive_shared 返回最新 WorldState）
        if let Ok(st) = arc.perceive_shared() {
            if let Ok(mut cache) = state.last_state_cache.lock() {
                *cache = Some(st.clone());
            }
            return (StatusCode::OK, axum::Json(st)).into_response();
        }
    }
    // 降级：返回缓存（如果有）
    if let Ok(cache) = state.last_state_cache.lock()
        && let Some(st) = cache.clone()
    {
        return (StatusCode::OK, axum::Json(st)).into_response();
    }
    (
        StatusCode::OK,
        axum::Json(serde_json::json!({"status":"not_connected"})),
    )
        .into_response()
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

fn build_view(path: &Path) -> anyhow::Result<Value> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("无法读取 {}: {e}", path.display()))?;
    let mut lines = text.lines();

    let header: Option<SessionHeader> = lines
        .next()
        .and_then(|l| serde_json::from_str::<SessionHeader>(l).ok());

    let mut events: Vec<Value> = Vec::new();
    let mut tool_total: u32 = 0;
    let mut by_tool: BTreeMap<String, u32> = BTreeMap::new();
    let mut total_input_tokens: u64 = 0;
    let mut total_output_tokens: u64 = 0;
    let mut checkpoints: Vec<Value> = Vec::new();
    let mut branches: Vec<Value> = Vec::new();

    for l in lines {
        let t = l.trim();
        if t.is_empty() {
            continue;
        }
        let Ok(entry) = serde_json::from_str::<SessionEntry>(t) else {
            continue;
        };
        match entry {
            SessionEntry::Message(m) => {
                let id = m.id;
                let parent = m.parent_id;
                match m.message {
                    Message::User(u) => events.push(json!({
                        "kind":"user","content":u.content, "id": id, "parent": parent
                    })),
                    Message::Assistant(a) => {
                        total_input_tokens += a.usage.input_tokens;
                        total_output_tokens += a.usage.output_tokens;
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
                            "reasoning":a.reasoning,"tool_calls":calls,
                            "usage": a.usage, "id": id, "parent": parent
                        }));
                    }
                    Message::ToolResult(t) => {
                        tool_total += 1;
                        *by_tool.entry(t.tool_name.clone()).or_insert(0) += 1;
                        events.push(json!({
                            "kind":"tool","call_id":t.tool_call_id,"name":t.tool_name,
                            "content":t.content,"is_error":t.is_error,"shot":null,
                            "id": id, "parent": parent
                        }));
                    }
                }
            }
            SessionEntry::Checkpoint(cp) => {
                checkpoints.push(json!({
                    "id": cp.id, "label": cp.label, "turn": cp.snapshot.turn,
                    "input_tokens": cp.snapshot.usage.input_tokens,
                    "output_tokens": cp.snapshot.usage.output_tokens,
                    "messages": cp.snapshot.messages.len(),
                }));
                events.push(
                    json!({"kind":"checkpoint","label":cp.label,"id":cp.id,"parent": cp.parent_id}),
                );
            }
            SessionEntry::BranchSummary(bs) => {
                branches.push(json!({
                    "id": bs.id, "from_id": bs.from_id, "summary": bs.summary
                }));
                events.push(
                    json!({"kind":"branch","from": bs.from_id, "summary": bs.summary, "id": bs.id}),
                );
            }
            SessionEntry::WorldInfo(w) => {
                events.push(json!({"kind":"knowledge","action":w.action}))
            }
            SessionEntry::Compaction(comp) => {
                events.push(json!({"kind":"compaction","summary":comp.summary,"tokens_before":comp.tokens_before}));
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
        "total_input_tokens": total_input_tokens,
        "total_output_tokens": total_output_tokens,
        "checkpoints": checkpoints,
        "branches": branches,
        "events": events,
    }))
}

const INDEX_HTML: &str = include_str!("index.html");
