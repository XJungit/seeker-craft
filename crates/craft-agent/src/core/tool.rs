//! 游戏工具 trait + 注册表 — pi 风格
//!
//! 参考 pi_agent_rust src/tools.rs:
//! - Tool trait (name/description/parameters/execute/effects)
//! - ToolRegistry (按名查找, config-driven 激活)
//! - ToolEffects 副作用声明 (READ/WRITE 位掩码)

use anyhow::Result;
use serde_json::Value;

// ── 副作用声明 ──

/// 工具副作用 (pi 的 ToolEffects 位掩码, 游戏版简化)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolEffects {
    /// 感知类: 不修改游戏世界 (perceive, look)
    pub is_readonly: bool,
    /// 修改类: 可能改变游戏状态 (aim_and_mine, move)
    pub is_destructive: bool,
}

impl ToolEffects {
    pub const fn read_only() -> Self {
        Self { is_readonly: true, is_destructive: false }
    }
    pub const fn destructive() -> Self {
        Self { is_readonly: false, is_destructive: true }
    }
}

// ── Tool trait ──

/// 工具执行结果
#[derive(Debug, Clone)]
pub struct ToolResult {
    /// LLM 可读的执行描述
    pub message: String,
    /// 是否出错
    pub is_error: bool,
}

/// 游戏工具 trait (pi 风格: 每个工具一个 struct, impl 这个 trait)
///
/// 新增工具: 写一个 struct + impl GameTool + 注册到 ToolRegistry。
/// 不改 agent.rs, 不改 types.rs。
pub trait GameTool {
    /// 工具名 (LLM function calling 中使用的 name)
    fn name(&self) -> &str;

    /// 工具描述 (给 LLM 看的)
    fn description(&self) -> &str;

    /// 参数 JSON Schema (OpenAI function calling 格式)
    fn parameters(&self) -> Value;

    /// 副作用声明
    fn effects(&self) -> ToolEffects { ToolEffects::destructive() }

    /// 执行工具
    fn execute(&self, args: Value) -> Result<ToolResult>;

    /// 转换为 OpenAI function calling 的完整定义
    fn to_openai_def(&self) -> Value {
        let params = self.parameters();
        if params.as_object().map_or(true, |o| o.is_empty()) {
            serde_json::json!({
                "type": "function",
                "function": {
                    "name": self.name(),
                    "description": self.description(),
                }
            })
        } else {
            serde_json::json!({
                "type": "function",
                "function": {
                    "name": self.name(),
                    "description": self.description(),
                    "parameters": params,
                }
            })
        }
    }
}

// ── ToolRegistry ──

/// 工具注册表 (pi 风格: 做 Vec<Box<dyn Tool>> 管理)
pub struct ToolRegistry {
    tools: Vec<Box<dyn GameTool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self { tools: Vec::new() }
    }

    /// 注册一个工具
    pub fn register(&mut self, tool: Box<dyn GameTool>) {
        self.tools.push(tool);
    }

    /// 按名称查找工具
    pub fn get(&self, name: &str) -> Option<&dyn GameTool> {
        self.tools.iter().find(|t| t.name() == name).map(AsRef::as_ref)
    }

    /// 获取所有工具
    pub fn tools(&self) -> &[Box<dyn GameTool>] {
        &self.tools
    }

    /// 生成 OpenAI function calling 的工具定义数组
    pub fn to_openai_defs(&self) -> Vec<Value> {
        self.tools.iter().map(|t| t.to_openai_def()).collect()
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}
