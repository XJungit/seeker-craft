//! Agent 循环事件：`AgentEvent` 定义 + 事件推送 helper（P3.3 拆分，事件推送从 agent_loop.rs 抽出）。
//!
//! `EventSender` 封装 broadcast sender，统一 `let _ = tx.send(AgentEvent::...)` 样板，
//! 并让 run_agent 内的调用点只关心"发什么"，不关心"怎么发"。

use serde::Serialize;
use tokio::sync::broadcast;

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

/// 事件推送 helper：`event_tx.log(...)` / `event_tx.error(...)` 等。
#[derive(Clone)]
pub struct EventSender {
    tx: broadcast::Sender<AgentEvent>,
}

impl EventSender {
    pub fn new(tx: broadcast::Sender<AgentEvent>) -> Self {
        Self { tx }
    }

    /// 日志事件（`let _ = tx.send(AgentEvent::Log {...})` 的替代）。
    pub fn log(&self, text: impl Into<String>) {
        let _ = self.tx.send(AgentEvent::Log { text: text.into() });
    }

    pub fn step(&self, step: u32) {
        let _ = self.tx.send(AgentEvent::Step {
            step,
            action: format!("第 {step} 步"),
            detail: String::new(),
        });
    }

    pub fn compaction(&self, summary: String, tokens_before: u64) {
        let _ = self.tx.send(AgentEvent::Compaction {
            summary,
            tokens_before,
        });
    }

    pub fn done(&self, reason: impl Into<String>) {
        let _ = self.tx.send(AgentEvent::Done {
            reason: reason.into(),
        });
    }

    pub fn error(&self, message: impl Into<String>) {
        let _ = self.tx.send(AgentEvent::Error {
            message: message.into(),
        });
    }

    pub fn perceive(&self, state: String) {
        let _ = self.tx.send(AgentEvent::Perceive { state });
    }

    pub fn memory(&self, json: String, cells: usize, anchors: usize) {
        let _ = self.tx.send(AgentEvent::Memory {
            json,
            cells,
            anchors,
        });
    }
}
