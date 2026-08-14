//! 控制面板核心控制状态（AgentController）与事件类型（AgentEvent）。
//!
//! DSH 桥接模式：viewer 不再运行 in-bot LLM 循环，只负责连接 azalea 客户端
//! （`/api/connect`）与状态呈现，bot 由 DSH/Cordis 经 `/api/bot_tool` 驱动。
//! `AgentController` 保留运行/暂停/停止/目标队列等控制面能力（被对应
//! `/api/*` 路由读取），以及 CLI 注入的 mode/individual profile 字段。

use craft_agent_minecraft::adapter_azalea::ArcAzaleaAdapter;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};

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

/// Agent 生命周期控制器。
#[allow(dead_code)]
pub struct AgentController {
    pub pause: Arc<AtomicBool>,
    pub stop: Arc<AtomicBool>,
    /// Shared with agent.retry_abort — set by stop button to cancel LLM retries instantly.
    pub abort: Arc<AtomicBool>,
    pub running: Arc<AtomicBool>,
    pub status: Mutex<Status>,
    goal_queue: Mutex<VecDeque<String>>,
    /// 共享的 azalea 适配器引用（bot 连接后填充，viewer 从中读取游戏状态）。
    pub game_adapter: Arc<RwLock<Option<ArcAzaleaAdapter>>>,
    /// 模式 profile 名（如 "survival" / "creative" / "assistant" / "god_mode"）。
    /// 加载 `profiles/defaults/{mode}.json` 叠加到 _default 之上。
    pub mode_profile: Option<String>,
    /// 个体 profile 名（如 "deepseek" / "claude" / "gpt"）。
    /// 加载 `profiles/{individual}.json` 叠加到 _default + mode 之上。
    pub individual_profile: Option<String>,
    /// Rotate the existing JSONL once, before attaching it to an Agent writer.
    pub rollover_session: bool,
}

impl AgentController {
    pub fn new(goal: String, max_steps: u32, session_path: String) -> Self {
        Self {
            pause: Arc::new(AtomicBool::new(false)),
            stop: Arc::new(AtomicBool::new(false)),
            abort: Arc::new(AtomicBool::new(false)),
            running: Arc::new(AtomicBool::new(false)),
            status: Mutex::new(Status {
                running: false,
                paused: false,
                step: 0,
                max_steps,
                goal,
                session_path,
            }),
            goal_queue: Mutex::new(VecDeque::new()),
            game_adapter: Arc::new(RwLock::new(None)),
            mode_profile: None,
            individual_profile: None,
            rollover_session: false,
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

    pub fn toggle_pause(&self) {
        let was = self.pause.load(Ordering::Relaxed);
        self.pause.store(!was, Ordering::Relaxed);
    }

    pub fn request_stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
        self.abort.store(true, Ordering::Relaxed);
    }

    /// Agent 循环取走待注入的目标（每步检查一次）。预留给未来 DSH 集成。
    #[allow(dead_code)]
    pub fn drain_goals(&self) -> Vec<String> {
        let mut q = self.goal_queue.lock().unwrap();
        let goals: Vec<_> = q.drain(..).collect();
        goals
    }
}

/// agent 循环发出的事件（通过 SSE 推给前端）。
///
/// 注：DSH/Cordis 接管大脑后，事件主要由桥接层（/api/bot_tool、/api/connect）
/// 在驱动 bot 时产生；此处先保留完整协议，未构造的变体为前端预留。
#[allow(dead_code)]
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
