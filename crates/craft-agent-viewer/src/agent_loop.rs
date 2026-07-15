//! Agent 循环控制器：管理 agent 的生命周期（启动/停止/暂停），
//! 通过 broadcast channel 向前端推送实时事件。

use craft_agent::agent::{Agent, AgentConfig, CompactionConfig, LlmProvider, RetryConfig};
use craft_agent::core::message::AssistantResponse;
use craft_agent::core::session::Session;
use craft_agent::core::tool::ToolRegistry;
use craft_agent_minecraft::adapter_mod::MinecraftModAdapter;
use craft_agent_minecraft::bridge::DEFAULT_PORT;
use craft_agent_minecraft::tools_mod::create_mc_mod_tools;
use craft_agent_model::config::AgentConfig as ModelConfig;
use craft_agent_model::decision::real::OpenAiLlmClient;
use craft_agent_model::vision::VisionClient;
use craft_agent_model::vision::real::OpenAiVisionClient;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::broadcast;

/// 前端可获取的运行状态快照。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Status {
    pub running: bool,
    pub paused: bool,
    pub step: u32,
    pub max_steps: u32,
    pub goal: String,
    pub session_path: String,
}

/// agent 循环发出的事件（通过 SSE 推给前端）。
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind")]
pub enum AgentEvent {
    #[serde(rename = "log")]
    Log { text: String },
    #[serde(rename = "step")]
    Step {
        step: u32,
        action: String,
        detail: String,
    },
    #[serde(rename = "done")]
    Done { reason: String },
    #[serde(rename = "error")]
    Error { message: String },
}

/// Agent 生命周期控制器。
pub struct AgentController {
    pub pause: Arc<AtomicBool>,
    pub stop: Arc<AtomicBool>,
    /// Shared with agent.retry_abort — set by stop button to cancel LLM retries instantly.
    pub abort: Arc<AtomicBool>,
    pub running: Arc<AtomicBool>,
    pub status: std::sync::Mutex<Status>,
    goal_queue: std::sync::Mutex<VecDeque<String>>,
}

impl AgentController {
    pub fn new(goal: String, max_steps: u32, session_path: String) -> Self {
        Self {
            pause: Arc::new(AtomicBool::new(false)),
            stop: Arc::new(AtomicBool::new(false)),
            abort: Arc::new(AtomicBool::new(false)),
            running: Arc::new(AtomicBool::new(false)),
            status: std::sync::Mutex::new(Status {
                running: false,
                paused: false,
                step: 0,
                max_steps,
                goal,
                session_path,
            }),
            goal_queue: std::sync::Mutex::new(VecDeque::new()),
        }
    }

    pub fn get_status(&self) -> Status {
        let mut s = self.status.lock().unwrap();
        s.running = self.running.load(Ordering::Relaxed);
        s.paused = self.pause.load(Ordering::Relaxed);
        s.clone()
    }

    /// UI 推送新目标（运行时动态修改，不中断正在进行的步骤）。
    pub fn push_goal(&self, new_goal: String) {
        self.goal_queue.lock().unwrap().push_back(new_goal.clone());
        self.status.lock().unwrap().goal = new_goal;
    }

    /// Agent 循环取走待注入的目标（每步检查一次）。
    pub fn drain_goals(&self) -> Vec<String> {
        let mut q = self.goal_queue.lock().unwrap();
        let goals: Vec<_> = q.drain(..).collect();
        goals
    }

    pub fn toggle_pause(&self) {
        let was = self.pause.load(Ordering::Relaxed);
        self.pause.store(!was, Ordering::Relaxed);
    }

    pub fn request_stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
        self.abort.store(true, Ordering::Relaxed);
    }
}

/// 启动 agent 循环（在独立 OS 线程中运行，使用 spawn_blocking 兼容 tokio）。
pub fn spawn_agent_loop(
    controller: Arc<AgentController>,
    model_config_path: String,
    use_vision: bool,
    shots_dir: Option<PathBuf>,
    event_tx: broadcast::Sender<AgentEvent>,
) -> anyhow::Result<()> {
    if controller.running.swap(true, Ordering::Relaxed) {
        let _ = event_tx.send(AgentEvent::Error {
            message: "Agent 已在运行中".into(),
        });
        return Ok(());
    }

    // 重置控制标志
    controller.stop.store(false, Ordering::Relaxed);
    controller.pause.store(false, Ordering::Relaxed);

    // 读取配置快照（避免引用生命周期问题）
    let status = controller.get_status();
    let goal = status.goal.clone();
    let max_steps = status.max_steps;
    let session_path = status.session_path.clone();
    let ctrl = controller.clone();
    let tx = event_tx.clone();

    let steps_text = if max_steps > 0 {
        format!("最大步数: {max_steps}")
    } else {
        "无限循环".to_string()
    };
    let _ = tx.send(AgentEvent::Log {
        text: format!("Agent 启动 | 目标: {goal} | {steps_text}"),
    });

    // Share abort signal with controller for instant stop
    let abort = controller.abort.clone();
    std::thread::spawn(move || {
        let goal = controller.status.lock().unwrap().goal.clone();
        let _ = dotenvy::dotenv();
        if let Err(e) = run_agent(
            &goal,
            max_steps,
            &session_path,
            &model_config_path,
            use_vision,
            shots_dir,
            &ctrl,
            &tx,
            &abort,
        ) {
            let _ = tx.send(AgentEvent::Error {
                message: format!("{e}"),
            });
        }
        ctrl.running.store(false, Ordering::Relaxed);
        ctrl.pause.store(false, Ordering::Relaxed);
    });

    Ok(())
}

/// Agent 主循环（阻塞运行在独立线程中）。
fn run_agent(
    goal: &str,
    max_steps: u32,
    session_path: &str,
    cfg_path: &str,
    use_vision: bool,
    shots_dir: Option<PathBuf>,
    ctrl: &AgentController,
    event_tx: &broadcast::Sender<AgentEvent>,
    abort: &Arc<AtomicBool>,
) -> anyhow::Result<()> {
    let model_cfg = ModelConfig::load(cfg_path)?;
    let perceive_cfg = model_cfg.perceive.unwrap_or_default();

    let llm_group = model_cfg
        .llm
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("缺少 [llm] 配置"))?;
    let llm_backend = llm_group.active_backend()?;
    let llm = Arc::new(OpenAiLlmClient::from_config(llm_backend)?);

    // VLM（可选）
    let vision: Option<Box<dyn VisionClient>> = if use_vision {
        let vlm_backend = model_cfg.vlm.active_backend()?;
        Some(Box::new(OpenAiVisionClient::from_config(vlm_backend)?))
    } else {
        None
    };

    // 连接 MC mod
    let adapter = Rc::new(RefCell::new(MinecraftModAdapter::connect_with_vision(
        "127.0.0.1",
        DEFAULT_PORT,
        vision,
    )?));

    let mut registry = ToolRegistry::new();
    for tool in create_mc_mod_tools(
        adapter.clone(),
        perceive_cfg.image_max_side,
        shots_dir,
        use_vision,
    ) {
        registry.register(tool);
    }

    struct Lp {
        llm: Arc<OpenAiLlmClient>,
    }
    impl LlmProvider for Lp {
        fn complete(&self, m: &[Value], t: &[Value]) -> anyhow::Result<AssistantResponse> {
            self.llm
                .chat_tools(&Value::Array(m.to_vec()), &Value::Array(t.to_vec()))
        }
    }

    let cw = llm_backend.context_window;
    let compaction = CompactionConfig {
        context_window: cw,
        reserve: (cw as f64 * 0.2) as u32,
        keep_recent: (cw as f64 * 0.2) as u32,
    };
    let sys = String::from(
        "You are a Minecraft AI bot that can see, move, mine, build, and interact with the world by using tools.\n\
         Be effective and efficient. Don't pretend to act, use tools immediately.\n\
         Key tool: collect(target, count) — automatically finds, aims at, walks to, and mines blocks. Use this for gathering resources.\n\
         Also available: look, press, mine for fine control. Do NOT describe what you will do, just call the tool.\n\
         Every response MUST contain a tool call, never text-only.",
    );
    let agent_cfg = AgentConfig::new(sys, 1) // 每步 1 轮，外循环控制步数
        .with_compaction(compaction)
        .with_retry(RetryConfig::default())
        .with_auto_perceive(true);

    let mut agent = {
        let path = Path::new(session_path);
        let sess = if path.exists() {
            Session::open(path)?
        } else {
            let mut s = Session::new("minecraft-control-panel");
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            s.save_to(path)?;
            s
        };
        let mut agent = Agent::new(Box::new(Lp { llm }), registry, agent_cfg).with_session(sess);
        agent.retry_abort = abort.clone();
        agent
    };

    let _ = event_tx.send(AgentEvent::Log {
        text: format!("已连接 mod | LLM: {}", llm_backend.model),
    });

    // 第一步: push goal message + run 1 turn
    let log = agent.run(goal.to_string())?;
    for line in &log {
        let _ = event_tx.send(AgentEvent::Log { text: line.clone() });
    }

    let mut step = 1u32;
    ctrl.status.lock().unwrap().step = step;
    let _ = event_tx.send(AgentEvent::Step {
        step,
        action: format!("第 {step} 步"),
        detail: String::new(),
    });
    if let Some(ref mut sess) = agent.session {
        let _ = std::fs::create_dir_all(Path::new(session_path).parent().unwrap_or(Path::new(".")));
        let _ = sess.save_to(Path::new(session_path));
    }

    loop {
        if ctrl.stop.load(Ordering::Relaxed) {
            let _ = event_tx.send(AgentEvent::Done {
                reason: "用户手动停止".into(),
            });
            break;
        }
        if ctrl.pause.load(Ordering::Relaxed) {
            let _ = event_tx.send(AgentEvent::Log {
                text: "⏸ 已暂停".into(),
            });
            while ctrl.pause.load(Ordering::Relaxed) {
                if ctrl.stop.load(Ordering::Relaxed) {
                    let _ = event_tx.send(AgentEvent::Done {
                        reason: "用户手动停止".into(),
                    });
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
            if ctrl.stop.load(Ordering::Relaxed) {
                break;
            }
            let _ = event_tx.send(AgentEvent::Log {
                text: "▶ 恢复运行".into(),
            });
        }

        step += 1;
        ctrl.status.lock().unwrap().step = step;

        // 检查是否有运行时注入的新目标
        for new_goal in ctrl.drain_goals() {
            agent.queue_steering(format!("【目标更新】{new_goal}"));
            let _ = event_tx.send(AgentEvent::Log {
                text: format!("📋 目标已更新: {new_goal}"),
            });
        }

        // 单步执行
        let (step_log, should_continue) = agent.step()?;
        for line in &step_log {
            let _ = event_tx.send(AgentEvent::Log { text: line.clone() });
        }

        let _ = event_tx.send(AgentEvent::Step {
            step,
            action: format!("第 {step} 步"),
            detail: String::new(),
        });

        // 保存 session
        if let Some(ref mut sess) = agent.session {
            let _ =
                std::fs::create_dir_all(Path::new(session_path).parent().unwrap_or(Path::new(".")));
            let _ = sess.save_to(Path::new(session_path));
        }

        if !should_continue {
            let _ = event_tx.send(AgentEvent::Done {
                reason: "目标达成或达到最大步数".into(),
            });
            break;
        }
    }
    Ok(())
}
