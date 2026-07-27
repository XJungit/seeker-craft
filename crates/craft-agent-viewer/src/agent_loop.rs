//! Agent 循环控制器：管理 agent 的生命周期（启动/停止/暂停），
//! 通过 broadcast channel 向前端推送实时事件。
//!
//! 唯一路线：Azalea 客户端协议层（Rust 全栈 bot 连入普通 MC 服务器）。
//! 旧 mod-bridge / real（Fabric mod TCP 桥接 + 真机 VLM 键鼠）路线已从源码删除。

use craft_agent::agent::{Agent, AgentConfig, CompactionConfig, LlmProvider, RetryConfig};
use craft_agent::core::message::AssistantResponse;
use craft_agent::core::session::Session;
use craft_agent::core::tool::ToolRegistry;
use craft_agent_minecraft::action_lib::ActionLibrary;
use craft_agent_minecraft::adapter_azalea::ArcAzaleaAdapter;
use craft_agent_minecraft::blueprint::BlueprintLibrary;
use craft_agent_minecraft::tools_azalea::create_mc_azalea_tools_full;
use craft_agent_model::config::AgentConfig as ModelConfig;
use craft_agent_model::decision::real::OpenAiLlmClient;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::RwLock;
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
    /// 当前游戏状态（perceive 快照），实时推送给前端展示 LLM 视角。
    #[serde(rename = "perceive")]
    Perceive {
        /// 结构化状态文本（同 LLM 收到的 perceive 注入内容）。
        state: String,
    },
    /// 世界记忆库快照（资源点/结构/容器/锚点），供前端可视化。
    #[serde(rename = "memory")]
    Memory {
        /// 完整 JSON（cells + anchors），前端按需渲染。
        json: String,
        /// 坐标记忆条数。
        cells: usize,
        /// 锚点数。
        anchors: usize,
    },
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
    /// 共享的 azalea 适配器引用（agent 启动后填充，viewer 从中读取游戏状态）。
    pub game_adapter: Arc<RwLock<Option<ArcAzaleaAdapter>>>,
    /// 模式 profile 名（如 "survival" / "creative" / "assistant" / "god_mode"）。
    /// 加载 `profiles/defaults/{mode}.json` 叠加到 _default 之上。
    pub mode_profile: Option<String>,
    /// 个体 profile 名（如 "deepseek" / "claude" / "gpt"）。
    /// 加载 `profiles/{individual}.json` 叠加到 _default + mode 之上。
    pub individual_profile: Option<String>,
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
            game_adapter: Arc::new(RwLock::new(None)),
            mode_profile: None,
            individual_profile: None,
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
    event_tx: broadcast::Sender<AgentEvent>,
    mc_addr: String,
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
            &ctrl,
            &tx,
            &abort,
            &mc_addr,
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
    max_steps: u32,
    session_path: &str,
    cfg_path: &str,
    ctrl: &AgentController,
    event_tx: &broadcast::Sender<AgentEvent>,
    abort: &Arc<AtomicBool>,
    mc_addr: &str,
) -> anyhow::Result<()> {
    let model_cfg = ModelConfig::load(cfg_path)?;
    let perceive_cfg = model_cfg.perceive.unwrap_or_default();
    let _ = &perceive_cfg; // azalea 路线无需 image_max_side

    let llm_group = model_cfg
        .llm
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("缺少 [llm] 配置"))?;
    let llm_backend = llm_group.active_backend()?;
    let llm = Arc::new(OpenAiLlmClient::from_config(llm_backend)?);

    // 共享世界记忆库：适配器（自动扫描回填）+ 工具（LLM 显式记录）+ Agent（每轮注入）共用同一实例。
    let world_mem = craft_agent::core::memory::WorldMemory::new();

    // 记忆可视化：后台线程每 2s 把世界记忆快照推送到前端（SSE "memory" 事件）。
    {
        let wm = world_mem.clone();
        let tx = event_tx.clone();
        let stop_flag = ctrl.stop.clone();
        std::thread::spawn(move || {
            loop {
                if stop_flag.load(Ordering::Relaxed) {
                    break;
                }
                let snapshot = wm.to_json();
                let cells = wm.len();
                let anchors = wm.anchors().len();
                let _ = tx.send(AgentEvent::Memory {
                    json: snapshot,
                    cells,
                    anchors,
                });
                std::thread::sleep(std::time::Duration::from_secs(2));
            }
        });
    }

    // 连接 azalea adapter：azalea 内部用独立 OS 线程跑自己的 runtime，
    // 此处仅用一次性局部 runtime 把 async connect 跑完，拿到句柄后立即 drop。
    let adapter = {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        rt.block_on(ArcAzaleaAdapter::connect_with_memory(
            mc_addr,
            "craftbot",
            world_mem.clone(),
        ))
        .map_err(|e| {
            anyhow::anyhow!(
                "azalea adapter 连接失败（确认服为纯 vanilla 26.2 且地址 {mc_addr} 开放）: {e}"
            )
        })?
    };
    *ctrl.game_adapter.write().unwrap() = Some(adapter.clone());

    let mut registry = ToolRegistry::new();
    // 加载蓝图库 + LLM 自定义动作库（P2-1 + P2-4）：
    // 优先从工作目录的 blueprints/ 与 actions/ 子目录载入；缺失时退化为空库。
    let blueprints = BlueprintLibrary::load_dir(Path::new("blueprints"));
    let actions = ActionLibrary::load_dir(Path::new("actions"));
    eprintln!(
        "[agent_loop] 加载蓝图 {} 个，自定义动作 {} 个",
        blueprints.len(),
        actions.len()
    );
    for tool in create_mc_azalea_tools_full(adapter.clone(), world_mem.clone(), blueprints, actions)
    {
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
                let comp_cw = comp_backend.context_window;
                compaction.context_window = comp_cw;
                let reserve = (comp_cw as f64 * 0.35) as u32;
                let keep_recent = (comp_cw as f64 * 0.5) as u32;
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

    // ============= Prompt Profile 加载（三层叠加：_default → mode → individual）=============
    // 学习自 Mindcraft 的 profile 系统：system prompt 从 JSON 文件加载，无需重编译即可调优。
    // 默认从 ./profiles/_default.json 加载，叠加 defaults/{mode}.json，再叠加 {individual}.json
    //
    // CLI 参数 --profile 可指定 individual profile 名（如 deepseek/claude/gpt）。
    // CLI 参数 --mode 可指定模式 profile 名（如 survival/creative/assistant/god_mode）。
    // 都不指定时只加载 _default.json。
    //
    // 路径解析顺序：$CRAFT_AGENT_PROFILES_DIR → cwd/profiles → exe 父目录向上查找 profiles/
    // （这样从任何 cwd 启动 viewer 都能找到项目根的 profiles/）。
    let profiles_path = resolve_profiles_path();
    let profile = craft_agent::profile::Profile::load(
        &profiles_path,
        ctrl.mode_profile.as_deref(),
        ctrl.individual_profile.as_deref(),
    )
    .unwrap_or_else(|e| {
        let _ = event_tx.send(AgentEvent::Log {
            text: format!("⚠ Profile 加载失败，回退默认空 prompt: {e}"),
        });
        craft_agent::profile::Profile::default()
    });

    let _ = event_tx.send(AgentEvent::Log {
        text: format!(
            "📄 Profile 加载: name={} modes={:?} cooldown={}ms examples={}",
            profile.name,
            profile.modes,
            profile.cooldown_ms,
            profile.conversation_examples.len()
        ),
    });

    // 渲染最终 system prompt（替换 $NAME / $SELF_PROMPT 等占位符）
    let mut replacements = std::collections::HashMap::new();
    replacements.insert("NAME".to_string(), "craftbot".to_string());
    replacements.insert("SELF_PROMPT".to_string(), "".to_string()); // SelfPrompter 在 agent loop 内每轮重注
    replacements.insert("MEMORY".to_string(), "".to_string());
    replacements.insert("STATS".to_string(), "".to_string());
    replacements.insert("INVENTORY".to_string(), "".to_string());
    replacements.insert("COMMAND_DOCS".to_string(), "".to_string());
    replacements.insert("EXAMPLES".to_string(), "".to_string());
    let system_prompt = profile.render(&replacements);

    let _ = event_tx.send(AgentEvent::Log {
        text: format!(
            "📄 System prompt 长度: {} 字符",
            system_prompt.chars().count()
        ),
    });
    let agent_cfg = AgentConfig::new(system_prompt, 1) // 每步 1 轮，外循环控制步数
        .with_compaction(compaction)
        .with_retry(RetryConfig {
            enabled: true,
            max_retries: 1,
            base_delay_ms: 500,
            backoff_multiplier: 2.0,
        })
        .with_auto_perceive(true)
        // WI 模板已修复为真实工具名（gather/attack/goto/mine），可安全开启给 LLM 场景化提示。
        // MC_KNOWLEDGE_BASE 仍关闭（azalea 路线用 perceive 结构化数据 + 上方 mc_knowledge 替代）。
        .with_knowledge_base(None)
        .with_knowledge_tool(false);

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
        let mut agent = Agent::new(Box::new(Lp { llm }), registry, agent_cfg)
            .with_world_memory(world_mem)
            .with_session(sess);
        agent.retry_abort = abort.clone();
        agent
    };

    let _ = event_tx.send(AgentEvent::Log {
        text: format!("已连接 azalea | LLM: {}", llm_backend.model),
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
        // P4 修复：max_steps=0 表示无限循环；>0 时达到上限自动停止。
        // 原 bug：参数名为 _max_steps（被忽略），导致 step 计数超过 max_steps 仍继续跑。
        if max_steps > 0 && step >= max_steps {
            let _ = event_tx.send(AgentEvent::Done {
                reason: format!("已达到最大步数 {max_steps}"),
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
            agent.set_self_prompt(&new_goal);
            let _ = event_tx.send(AgentEvent::Log {
                text: format!("📋 目标已更新 (SelfPrompter 已设置): {new_goal}"),
            });
        }

        // 单步执行
        let _ = event_tx.send(AgentEvent::Log {
            text: format!("🤔 第 {step} 步: 正在思考..."),
        });
        let step_stop = Arc::new(AtomicBool::new(false));
        let progress_tx = event_tx.clone();
        let progress_step = step;
        let progress_abort = abort.clone();
        let progress_handle = {
            let stop_flag = step_stop.clone();
            std::thread::spawn(move || {
                let mut waited_secs = 0u64;
                std::thread::sleep(std::time::Duration::from_secs(10));
                while !stop_flag.load(Ordering::Relaxed) {
                    waited_secs += 10;
                    let _ = progress_tx.send(AgentEvent::Log {
                        text: format!("⏳ 第 {progress_step} 步: LLM 思考中 (已等 {waited_secs}s)"),
                    });
                    if waited_secs >= 120 && !progress_abort.load(Ordering::Relaxed) {
                        progress_abort.store(true, Ordering::Relaxed);
                        let _ = progress_tx.send(AgentEvent::Log {
                            text: format!(
                                "⚠ 第 {progress_step} 步: 已等 {waited_secs}s，自动中止 LLM 重试（判定卡死）"
                            ),
                        });
                    }
                    if waited_secs >= 180 {
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_secs(10));
                }
            })
        };
        // 消费聊天消息（玩家在 MC 里发的消息注入到 agent）
        let chat_msgs = adapter.drain_chat();
        for msg in &chat_msgs {
            agent.queue_steering(format!("玩家说: {msg}"));
            let _ = event_tx.send(AgentEvent::Log {
                text: format!("💬 收到聊天: {msg}"),
            });
        }

        let step_result = agent.step();
        step_stop.store(true, Ordering::Relaxed);
        let _ = progress_handle.join();
        let (step_log, should_continue) = step_result?;
        for line in &step_log {
            let _ = event_tx.send(AgentEvent::Log { text: line.clone() });
        }

        // 实时推送 perceive 状态给前端（LLM 当前看到的游戏世界）
        if let Ok(ws) = adapter.perceive_shared() {
            let _ = event_tx.send(AgentEvent::Perceive {
                state: ws.scene_desc,
            });
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

        // 空闲自提示循环：有目标时自动持续推进，无需用户输入
        if agent.has_goal() {
            std::thread::sleep(std::time::Duration::from_millis(500));
            agent.queue_steering("继续推进当前目标，用工具采取下一步行动".to_string());
        }
    }
    Ok(())
}

/// 解析 profiles/ 目录路径。
///
/// 查找顺序：
/// 1. `$CRAFT_AGENT_PROFILES_DIR` 环境变量（绝对路径优先）
/// 2. `cwd/profiles`（默认相对路径，从启动 cwd 解析）
/// 3. 从可执行文件所在目录向上查找，直到找到含 `profiles/_default.json` 的目录
///    （让 viewer 从任何 cwd 启动都能定位到项目根的 profiles/）
///
/// 都找不到时返回 `cwd/profiles`，让上层报错信息可读。
fn resolve_profiles_path() -> PathBuf {
    // 1. 环境变量优先
    if let Ok(dir) = std::env::var("CRAFT_AGENT_PROFILES_DIR") {
        let p = PathBuf::from(&dir);
        if p.join("_default.json").exists() {
            return p;
        }
    }

    // 2. cwd/profiles
    let cwd_profiles = PathBuf::from("profiles");
    if cwd_profiles.join("_default.json").exists() {
        return cwd_profiles;
    }

    // 3. 从 exe 父目录向上查找
    if let Ok(exe) = std::env::current_exe() {
        let mut dir = exe.parent().map(|p| p.to_path_buf());
        while let Some(d) = dir {
            let candidate = d.join("profiles");
            if candidate.join("_default.json").exists() {
                return candidate;
            }
            dir = d.parent().map(|p| p.to_path_buf());
        }
    }

    // 回退：返回相对路径让上层报错
    cwd_profiles
}
