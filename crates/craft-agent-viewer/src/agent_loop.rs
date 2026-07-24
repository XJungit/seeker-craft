//! Agent 循环控制器：管理 agent 的生命周期（启动/停止/暂停），
//! 通过 broadcast channel 向前端推送实时事件。
//!
//! 唯一路线：Azalea 客户端协议层（Rust 全栈 bot 连入普通 MC 服务器）。
//! 旧 mod-bridge / real（Fabric mod TCP 桥接 + 真机 VLM 键鼠）路线已从源码删除。

use craft_agent::agent::{Agent, AgentConfig, CompactionConfig, LlmProvider, RetryConfig};
use craft_agent::core::message::AssistantResponse;
use craft_agent::core::session::Session;
use craft_agent::core::tool::ToolRegistry;
use craft_agent_minecraft::adapter_azalea::ArcAzaleaAdapter;
use craft_agent_minecraft::tools_azalea::create_mc_azalea_tools;
use craft_agent_model::config::AgentConfig as ModelConfig;
use craft_agent_model::decision::real::OpenAiLlmClient;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::VecDeque;
use std::path::Path;
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
    _max_steps: u32,
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
        .map_err(|e| anyhow::anyhow!("azalea adapter 连接失败（确认服为纯 vanilla 26.2 且地址 {mc_addr} 开放）: {e}"))?
    };
    *ctrl.game_adapter.write().unwrap() = Some(adapter.clone());

    let mut registry = ToolRegistry::new();
    for tool in create_mc_azalea_tools(adapter.clone(), world_mem.clone()) {
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

    let system_prompt = String::from(
        "你是 Minecraft AI 玩家，通过 azalea 客户端协议控制 bot（纯 vanilla 26.2）。\n\
         可用工具：\n\
         - perceive()：读坐标/背包/附近玩家（无参数）。\n\
         - goto(x,y,z)：A* 导航到坐标。\n\
         - mine_below()：挖脚下方块（向下探矿，会持续挖直到你改指令）。\n\
         - mine(x,y,z)：挖掉指定世界坐标的方块（精确挖掘）。\n\
         - interact_block(x,y,z)：对着指定坐标方块交互（放置/右键激活）。\n\
         - attack(target)：攻击最近的生物（自卫/狩猎），target 可填 nearest。\n\
          - craft(item,count)：2×2 背包合成（无需工作台），如 craft(\"oak_planks\",4)。\n\
          - craft_3x3(item,count)：3×3 工作台合成（需先右键打开工作台），如 craft_3x3(\"furnace\")。\n\
          - smelt(output,fuel,count)：熔炼（需先右键打开熔炉），如 smelt(\"iron_ingot\",\"coal\")。\n\
          - gather(item,count)：走到最近方块并挖掘（早期采集），如 gather(\"oak_log\",4) / gather(\"stone\",8)。\n\
          - place(item,x,y,z)：把手持物品放到坐标旁（如 place(\"crafting_table\",x,y,z) 造工作台）。\n\
          - open(x,y,z)：打开坐标处容器（工作台/熔炉），随后可 craft_3x3 / smelt。\n\
           - auto_craft(item,count)：高层一键造任意已登记物品（推荐），如 auto_craft(\"chest\",1) / auto_craft(\"iron_ingot\",3)，bot 自主采集+合成+熔炼+放置容器。\n\
           - enchant(item,level)：附魔（需先 open 打开附魔台，且背包有 item 与青金石 lapis_lazuli），level 取 1/2/3，如 enchant(\"iron_sword\",2)。\n\
           - interact_entity(kind)：右键交互最近的实体（如 villager）。先走到村民附近再用。\n\
           - trade(offer)：与最近的村民交易，选第 offer 个报价（0 起）。需先靠近村民。\n\
           - chat(content)：发聊天消息，用于向玩家汇报进度。\n\
           - memory(action,kind,pos,label,anchor)：世界长期记忆（空间-状态）。action=save 记录资源点/结构/容器坐标；action=query 查询附近记忆；action=anchor 设置当前位置锚点（__self__）；action=forget 删除。采集到重要坐标或建好设施后用 save 记录，决策前用 query 回忆。\n\
          行为准则：\n\
         1) 每轮尽量在一次回复里连续输出多个工具调用完成一个小目标（如：先 perceive 确认状态，\n\
            再 goto 到树旁，再 gather 木头，最后 craft 木板），不要每轮只发一个动作等下一轮。\n\
            参考 Mindcraft：一个 LLM 决策应推进一整段子任务；优先用高层工具 gather/auto_craft，而非逐个 mine。\n\
         2) 下探任务：连续调 mine_below 2~3 次后，调一次 chat 汇报当前 Y 坐标与进度，\n\
             再继续 mine_below。穿插 chat 汇报，不要无脑连续调同一工具超过 3 次。\n\
         3) 若 perceive 返回含 \"卡住计数=N\"（N>=3，Y 坐标连续不变，可能挖到基岩或脚下无可破坏方块），\n\
             必须停止下探：改用 goto 侧前方 3 格空地或跳跃脱困，再重新 perceive；\n\
             不要原地反复 perceive 或假装还在挖。确实无法推进时用 chat 向玩家说明后，以纯文本结束。\n\
         4) perceive 每轮开头调一次确认状态即可，不必每次都调。\n\
         5) 工具没回报\"实际获得X\"就当作没获得，不得虚构成功。\n\
         6) 任务确实无法推进时，允许纯文本结束（说明原因），这不算错误。",
    );
    let agent_cfg = AgentConfig::new(system_prompt, 1) // 每步 1 轮，外循环控制步数
        .with_compaction(compaction)
        .with_retry(RetryConfig {
            enabled: true,
            max_retries: 1,
            base_delay_ms: 500,
            backoff_multiplier: 2.0,
        })
        .with_auto_perceive(true)
        .with_knowledge_base(None)
        .with_world_info(None)
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
        let step_result = agent.step();
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
