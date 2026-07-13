//! VLM / LLM 最小客户端（与 game-agent-design.md §4.1 / §4.4 对齐）
//!
//! - [`vision::VisionClient`]：截图 + 标记元素 → 场景自然语言描述
//! - [`decision::DecisionClient`]：WorldState → 抽象 Action
//!
//! 默认带 **mock 实现**（离线单测用，不依赖网络/密钥）；
//! `--features real` 启用基于 reqwest 的真实实现（agnes-vision / LLM API）。

pub mod config;
pub mod decision;
pub mod som;
pub mod vision;
