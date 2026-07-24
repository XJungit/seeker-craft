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

// MC 常识知识库（注入 system prompt，提升 bot 的玩法认知，降低"瞎操作"概率）。
    let mc_knowledge = String::from(
        "\n\
         ===== Minecraft 常识（vanilla 26.2）=====\n\
         矿物分布（Y 层，越深越多）：煤 coal 在 Y=0~136 随处可见，Y=90 附近多；\n\
         铁 iron 在 Y=-24~80，Y=15 附近最富；铜 copper 同 iron；金 gold 在 Y=-64~32；\n\
         红石 redstone 在 Y=-59~-32；青金石 lapis 在 Y=-64~-32；钻石 diamond 在 Y=-58~-51 最富；\n\
         绿宝石 emerald 只在山地 biome。深层（Y<0）矿物密度远高于地表，\n\
         想挖矿就 mine_below 一路下到 Y≈-20~-50。\n\
         工具规则：徒手挖木头/沙子/泥土/砾石；挖石头/矿石必须先用 wooden_pickaxe 以上镐，\n\
         否则不掉掉落物。先用 gather(\"oak_log\") 砍树→auto_craft(\"crafting_table\")→auto_craft(\"wooden_pickaxe\")→下矿。\n\
         合成链路：要铁锭先挖 iron_ore→开熔炉 smelt(\"iron_ingot\",\"coal\")； coal 既是燃料也是熔炼燃料。\n\
         没煤炭可烧木炭：gather 木头→auto_craft(\"charcoal\")（熔炉里烧木头得木炭当燃料）。\n\
         脱困：若卡在方块里或悬空，先 mine_below 挖脚下方块下落；若被墙挡 goto 不过去，\n\
         改 goto 到侧前方 3~5 格空地（x±3 或 z±3），不要反复 goto 同一个到不了的点。\n\
         照明与怪物：黑暗处（亮度<7）会刷怪，下矿前 auto_craft(\"torch\") 并沿途 place；\n白天安全、夜里或洞穴有僵尸/骷髅/苦力怕，遇怪用 attack 或逃跑 goto 到亮处。\n\
         体力：吃饱才跑得快，饥饿见底会缓慢掉血；有食材先 auto_craft 熟食。\n\
         目标拆解：拿到一个大目标（如\"造铁镐\"）先想依赖链——\n\
         树→木板→木棍→工具台→木镐→下矿挖铁→熔铁→铁镐，按链用高层工具逐段推进。\n\
         优先用 auto_craft/gather/run_plan：它们内部已自主完成多步任务，比手写单个工具可靠。\n\
         首日生存：1) gather oak_log 4-8 → 2) auto_craft crafting_table → 3) place crafting_table →\n\
         4) auto_craft wooden_pickaxe → 5) gather stone 8 → 6) auto_craft stone_pickaxe\n\
         7) 天黑前 gather 羊/牛/猪 3-4 只获取食物 → 8) 挖个 2x1 地洞插火把过夜\n\
         ",
    );
    let system_prompt = String::from(
        "You are a Minecraft bot. You see the world through auto-injected perceive state each turn.\n\n\
         RULES:\n\
         - Use function calling. Never write tool calls in text.\n\
         - Call multiple tools per turn — they run sequentially.\n\
         - If a tool fails, try something different. Don't repeat the same failed action.\n\
         - Modes handle survival (fire, lava, mobs). Focus on your goals.\n\
         - Use set_goal() for goals. The bot keeps working on them.\n\
         - For complex tasks, use run_script() with rhai code:\n\
           Available functions: goto(x,y,z), mine(x,y,z), mine_below(), gather(item,count),\n\
           craft(item,count), place(item,x,y,z), open(x,y,z), chat(msg), attack(),\n\
           smelt(output,fuel,count), interact(x,y,z), sleep(ms), print(msg).\n\
           Example: let r = gather(\"oak_log\", 4); print(r); craft(\"oak_planks\", 4)\n\
         - For sequential plans, use run_plan().\n\
         - search_wiki() for game knowledge.\n\n\
         SURVIVAL:\n\
         - Day 1: gather oak_log 4 → craft crafting_table → place → craft wooden_pickaxe\n\
         - Then: gather stone 8 → craft stone_pickaxe → gather coal → craft torches\n\
         - Shelter before night. Food when hungry. Torches in dark areas.\n\
         - Stuck? Try different direction. Jump. Dig around you.",
    ) + mc_knowledge.as_str();
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
