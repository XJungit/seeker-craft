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
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex;
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
    #[serde(rename = "compaction")]
    Compaction { summary: String, tokens_before: u64 },
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
    /// 共享的 mod 适配器引用（agent 启动后填充，viewer 从中读取游戏状态）。
    pub game_adapter: Arc<std::sync::RwLock<Option<Arc<std::sync::Mutex<MinecraftModAdapter>>>>>,
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
            game_adapter: Arc::new(std::sync::RwLock::new(None)),
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
#[allow(clippy::too_many_arguments)]
fn run_agent(
    goal: &str,
    _max_steps: u32,
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
    let vision: Option<Arc<dyn VisionClient>> = if use_vision {
        let vlm_backend = model_cfg.vlm.active_backend()?;
        Some(Arc::new(OpenAiVisionClient::from_config(vlm_backend)?))
    } else {
        None
    };

    // 连接 MC mod
    let adapter = Arc::new(std::sync::Mutex::new(
        MinecraftModAdapter::connect_with_vision("127.0.0.1", DEFAULT_PORT, vision)?,
    ));
    *ctrl.game_adapter.write().unwrap() = Some(adapter.clone());

    let pending_goal: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));

    let mut registry = ToolRegistry::new();
    for tool in create_mc_mod_tools(
        adapter.clone(),
        perceive_cfg.image_max_side,
        shots_dir,
        use_vision,
        Some(pending_goal.clone()),
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
    let mut compaction = CompactionConfig {
        context_window: cw,
        reserve: (cw as f64 * 0.2) as u32,
        keep_recent: (cw as f64 * 0.2) as u32,
        ..Default::default()
    };

    // 专用压缩模型：从 [compaction] 后端组构造，隔离主模型。
    // 用免费、512K 上下文的 agnes-2.0-flash 做压缩，避免小模型因上下文过长卡死。
    if let Some(comp_group) = model_cfg.compaction.as_ref()
        && let Ok(comp_backend) = comp_group.active_backend()
    {
        match OpenAiLlmClient::from_config(comp_backend) {
            Ok(comp_llm) => {
                let thinking = comp_backend
                    .extra_body
                    .as_ref()
                    .and_then(|eb| eb.get("chat_template_kwargs"))
                    .and_then(|kt| kt.get("enable_thinking"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                compaction.compaction_provider = Some(Box::new(Lp {
                    llm: Arc::new(comp_llm),
                }));
                compaction.compaction_thinking = thinking;
                // 关键：用【压缩模型自身】的上下文窗口来算压缩预算，
                // 而非主模型的。这样「压缩模型能吃多大」真正约束「一次压多少旧消息」，
                // 避免旧消息超过 agnes 上限导致压缩请求直接报错。
                let comp_cw = comp_backend.context_window;
                compaction.context_window = comp_cw;
                // 压缩模型需同时装下「旧消息 + 旧摘要 + 系统提示 + 新摘要输出」，
                // 因此保留量取压缩窗口的一部分，给输出与系统提示留余量。
                let reserve = (comp_cw as f64 * 0.35) as u32; // 35% 留给摘要输出/系统提示/安全余量
                let keep_recent = (comp_cw as f64 * 0.5) as u32; // 单次最多压掉约 50% 窗口的旧消息
                compaction.reserve = reserve;
                compaction.keep_recent = keep_recent;
                let _ = event_tx.send(AgentEvent::Log {
                    text: format!(
                        "🗜 专用压缩模型: {} (thinking={}, 窗口={}tok, 单次压旧≤{}tok)",
                        comp_backend.model, thinking, comp_cw, keep_recent
                    ),
                });
            }
            Err(e) => {
                let _ = event_tx.send(AgentEvent::Log {
                    text: format!("⚠ 压缩模型构造失败，回退主模型: {e}"),
                });
            }
        }
    }
    let sys = String::from(
        "你是 Minecraft AI 玩家，通过服务端 mod 桥接（ServerPlayer 架构）精确控制角色。每轮会自动注入游戏状态（perceive），无需手动调用 perceive。\n\
         感知返回的是结构化状态（精确物品栏数量/方块与生物的世界坐标与距离/玩家坐标朝向/血量饥饿），直接据数据决策，不要靠看图猜。\n\
         核心工具: collect(target,count) 自动找→走→挖; move_to(x,y,z) 精确导航; look_at(x,y,z) 瞄准坐标; craft(item,count) 合成; place(item) 放置; combat(mode,ticks) 战斗。\n\
         禁止调用 look(dx,dy)/press(keys)/mine(ticks)/craftable() —— 这些工具未注册，会返回 Unknown tool。\n\
         始终用中文回复，每次回复必须以工具调用结尾。",
    );
    let agent_cfg = AgentConfig::new(sys, 1) // 每步 1 轮，外循环控制步数
        .with_compaction(compaction)
        .with_retry(RetryConfig {
            enabled: true,
            max_retries: 1, // 最多 2 次尝试（原 3 次），配合 timeout=60s 最坏 ~120s
            base_delay_ms: 500,
            backoff_multiplier: 2.0,
        })
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
    let _ = event_tx.send(AgentEvent::Log {
        text: "🤔 正在思考...".into(),
    });
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
        if let Err(e) = sess.save_to(Path::new(session_path)) {
            let _ = event_tx.send(AgentEvent::Error {
                message: format!("session 保存失败: {e}"),
            });
            eprintln!("[agent_loop] session save_to 失败: {e}");
        }
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

        // 检查是否有运行时注入的新目标（Web UI）
        for new_goal in ctrl.drain_goals() {
            agent.queue_steering(format!("【目标更新】{new_goal}"));
            // 设置 SelfPrompter 持续目标注入，每轮自动提醒 LLM
            agent.set_self_prompt(&new_goal);
            let _ = event_tx.send(AgentEvent::Log {
                text: format!("📋 目标已更新 (SelfPrompter 已设置): {new_goal}"),
            });
        }

        // 检查 LLM 通过 set_goal 工具设置的 goal
        if let Ok(mut llm_goal) = pending_goal.lock()
            && let Some(goal) = llm_goal.take()
        {
            agent.queue_steering(format!("【目标更新】{goal}"));
            agent.set_self_prompt(&goal);
            let _ = event_tx.send(AgentEvent::Log {
                text: format!("📋 LLM 目标已注入 SelfPrompter: {goal}"),
            });
        }

        // 单步执行
        let _ = event_tx.send(AgentEvent::Log {
            text: format!("🤔 第 {step} 步: 正在思考..."),
        });
        // 在阻塞 step 期间，定时推送进度事件，让前端知道"还在等 LLM"而不是卡死。
        // 用独立线程 + 共享 stop 标志实现，step 返回后立即停止。
        // ≥120s 自动触发 abort：单次 timeout=60s + 1 次重试最坏 ~120s，超过即判定卡死。
        let step_stop = Arc::new(AtomicBool::new(false));
        let progress_tx = event_tx.clone();
        let progress_step = step;
        let progress_abort = abort.clone();
        let progress_handle = {
            let stop_flag = step_stop.clone();
            std::thread::spawn(move || {
                let mut waited_secs = 0u64;
                // 第一次 10s 后开始报进度，之后每 10s 一次
                std::thread::sleep(std::time::Duration::from_secs(10));
                while !stop_flag.load(Ordering::Relaxed) {
                    waited_secs += 10;
                    let _ = progress_tx.send(AgentEvent::Log {
                        text: format!("⏳ 第 {progress_step} 步: LLM 思考中 (已等 {waited_secs}s)"),
                    });
                    // ≥120s 自动 abort：LongCat 正常响应 <30s，超过 120s 必定卡死
                    // （timeout 60s + 1 次重试 = 120s 是配置上限，超过说明 timeout 没生效或 API 端异常）
                    if waited_secs >= 120 && !progress_abort.load(Ordering::Relaxed) {
                        progress_abort.store(true, Ordering::Relaxed);
                        let _ = progress_tx.send(AgentEvent::Log {
                            text: format!(
                                "⚠ 第 {progress_step} 步: 已等 {waited_secs}s，自动中止 LLM 重试（判定卡死）"
                            ),
                        });
                    }
                    if waited_secs >= 180 {
                        // 再等 60s 让 abort 生效（重试循环检测到 abort 会跳出），然后停止进度线程
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_secs(10));
                }
            })
        };
        let step_result = agent.step();
        // 停止进度线程
        step_stop.store(true, Ordering::Relaxed);
        let _ = progress_handle.join();
        let (step_log, should_continue) = step_result?;
        for line in &step_log {
            let _ = event_tx.send(AgentEvent::Log { text: line.clone() });
        }

        // 实时反馈：压缩事件
        if let Some(comp) = agent.last_compaction.take() {
            let _ = event_tx.send(AgentEvent::Compaction {
                summary: comp.summary,
                tokens_before: comp.tokens_before,
            });
        }

        let _ = event_tx.send(AgentEvent::Step {
            step,
            action: format!("第 {step} 步"),
            detail: String::new(),
        });

        // 保存 session（走增量 append，避免每次全量重写）
        if let Some(ref mut sess) = agent.session
            && let Err(e) = sess.save()
        {
            eprintln!("[agent_loop] session save 失败: {e}");
            // save 失败时降级到 save_to 全量重写
            if let Err(e2) = sess.save_to(Path::new(session_path)) {
                let _ = event_tx.send(AgentEvent::Error {
                    message: format!("session 保存失败: {e2}"),
                });
            }
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
