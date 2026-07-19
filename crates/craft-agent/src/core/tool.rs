//! 游戏工具 trait + 注册表 — pi 风格 (严格对应 pi_agent_rust src/tools.rs)
//!
//! 与 pi 同构:
//! - Tool trait: name / label / description / parameters / execute / effects
//! - ToolEffects: 手写位掩码 (READ/WRITE/APPEND/NETWORK/PROCESS/BARRIER)
//! - ToolRegistry: Vec<Box<dyn Tool>>, 按名线性查找
//! - plan_tool_effect_batches: 按副作用分组 (pi agent.rs:417 plan_tool_effect_batches)

use anyhow::Result;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

// ── 副作用位掩码 (pi tools.rs L36-155, 手写非 bitflags crate) ──

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolEffects {
    bits: u8,
}

impl ToolEffects {
    const READ: u8 = 1 << 0; // 只读本地态
    const WRITE: u8 = 1 << 1; // 替换/变更
    const APPEND: u8 = 1 << 2; // 追加
    const NETWORK: u8 = 1 << 3; // 网络 I/O (不变更本地)
    const PROCESS: u8 = 1 << 4; // 启动本地进程 (视作调度屏障)
    /// 写/追加/进程 = 不可并行的复合屏障
    const BARRIER: u8 = Self::WRITE | Self::APPEND | Self::PROCESS;

    pub const fn read() -> Self {
        Self { bits: Self::READ }
    }
    pub const fn write() -> Self {
        Self { bits: Self::WRITE }
    }
    pub const fn append() -> Self {
        Self { bits: Self::APPEND }
    }
    pub const fn network() -> Self {
        Self {
            bits: Self::NETWORK,
        }
    }
    pub const fn process() -> Self {
        Self {
            bits: Self::PROCESS,
        }
    }

    /// 组合两个副作用 (pi: union)
    pub const fn union(self, other: Self) -> Self {
        Self {
            bits: self.bits | other.bits,
        }
    }

    pub fn reads(&self) -> bool {
        self.bits & Self::READ != 0
    }
    pub fn writes(&self) -> bool {
        self.bits & Self::WRITE != 0
    }
    pub fn appends(&self) -> bool {
        self.bits & Self::APPEND != 0
    }
    pub fn networks(&self) -> bool {
        self.bits & Self::NETWORK != 0
    }
    pub fn processes(&self) -> bool {
        self.bits & Self::PROCESS != 0
    }

    /// 能否并行 (pi: parallel_safe = bits != 0 && bits & BARRIER == 0)
    pub fn parallel_safe(&self) -> bool {
        self.bits != 0 && self.bits & Self::BARRIER == 0
    }

    /// 两个工具能否同批并发 (pi: compatible_with)
    pub fn compatible_with(&self, other: &Self) -> bool {
        self.parallel_safe() && other.parallel_safe()
    }

    pub fn labels(&self) -> Vec<&'static str> {
        let mut v = Vec::new();
        if self.reads() {
            v.push("read");
        }
        if self.writes() {
            v.push("write");
        }
        if self.appends() {
            v.push("append");
        }
        if self.networks() {
            v.push("network");
        }
        if self.processes() {
            v.push("process");
        }
        v
    }
}

// ── Tool 执行结果 ──

#[derive(Debug, Clone)]
pub struct ToolResult {
    /// LLM 可读的执行描述
    pub message: String,
    /// 是否出错
    pub is_error: bool,
    /// 可选图像段：base64 data URI（如 `data:image/png;base64,...`）。
    /// 非空时由 Agent 以 `tool_result_with_images` 落历史，并以
    /// OpenAI ChatML `image_url` 内容段形式直接发给决策 LLM（多模态直读场景）。
    /// 纯文本工具保持为空。
    pub images: Vec<String>,
}

/// 增量结果回调 (pi: on_update: Option<Box<dyn Fn(ToolUpdate) + Send + Sync>>)
/// 长任务 (如长跑 bash) 可边执行边回传进度。游戏工具通常为瞬时, 传 None。
pub type ToolUpdateFn = Arc<dyn Fn(&str) + Send + Sync>;

/// 游戏工具 trait (pi Tool trait, L157)
///
/// 新增工具: 写一个 struct + impl GameTool + 注册到 ToolRegistry。
/// 不改 agent.rs, 不改 types.rs。
pub trait GameTool: Send + Sync {
    /// 工具名 (LLM function calling 中使用的 name)
    fn name(&self) -> &str;

    /// 显示名 (pi: label, 默认 = name)
    fn label(&self) -> &str {
        self.name()
    }

    /// 工具描述 (给 LLM 看的)
    fn description(&self) -> &str;

    /// 参数 JSON Schema (OpenAI function calling 格式)
    fn parameters(&self) -> Value;

    /// 副作用声明 (pi: 默认 write() = 保守串行化, fail-closed)
    fn effects(&self) -> ToolEffects {
        ToolEffects::write()
    }

    /// 执行工具 (pi: tool.execute(&tool_call_id, input, on_update))
    fn execute(
        &self,
        call_id: &str,
        args: Value,
        on_update: Option<ToolUpdateFn>,
    ) -> Result<ToolResult>;

    /// 转换为 OpenAI function calling 的完整定义
    fn to_openai_def(&self) -> Value {
        let params = self.parameters();
        if params.as_object().is_none_or(|o| o.is_empty()) {
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

// ── ToolRegistry (pi tools.rs L2646) ──

/// 工具注册表 — HashMap 索引 O(1) 查找
pub struct ToolRegistry {
    tools: Vec<Box<dyn GameTool>>,
    /// name → index 映射（register/extend/push 后重建）
    index: HashMap<String, usize>,
}

fn build_index(tools: &[Box<dyn GameTool>]) -> HashMap<String, usize> {
    tools
        .iter()
        .enumerate()
        .map(|(i, t)| (t.name().to_string(), i))
        .collect()
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: Vec::new(),
            index: HashMap::new(),
        }
    }

    /// 注册一个工具
    pub fn register(&mut self, tool: Box<dyn GameTool>) {
        self.tools.push(tool);
        self.index = build_index(&self.tools);
    }

    /// 追加一批工具 (pi: extend)
    pub fn extend(&mut self, others: Vec<Box<dyn GameTool>>) {
        self.tools.extend(others);
        self.index = build_index(&self.tools);
    }

    /// 追加单个工具 (pi: push)
    pub fn push(&mut self, tool: Box<dyn GameTool>) {
        self.tools.push(tool);
        self.index = build_index(&self.tools);
    }

    pub fn len(&self) -> usize {
        self.tools.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    /// O(1) 按名称查找工具
    pub fn get(&self, name: &str) -> Option<&dyn GameTool> {
        self.index
            .get(name)
            .and_then(|i| self.tools.get(*i))
            .map(AsRef::as_ref)
    }

    /// 获取所有工具
    pub fn tools(&self) -> &[Box<dyn GameTool>] {
        &self.tools
    }

    /// 生成 OpenAI function calling 的工具定义数组
    pub fn to_openai_defs(&self) -> Vec<Value> {
        self.tools.iter().map(|t| t.to_openai_def()).collect()
    }

    /// 自动生成工具参考知识字符串（替代硬编码 MC_KNOWLEDGE 的工具部分）
    /// 每个工具一行 `name(params) — description`
    pub fn to_knowledge_string(&self) -> String {
        if self.tools.is_empty() {
            return String::new();
        }
        let mut lines = Vec::new();
        // 按 group 分组展示
        let groups = [
            (
                "High-Level",
                &[
                    "collect",
                    "craft",
                    "place",
                    "build",
                    "blueprints",
                    "combat",
                    "attack",
                ] as &[&str],
            ),
            ("Utility", &["equip", "consume", "discard", "smeltItem"]),
            (
                "Navigation",
                &["searchForBlock", "move_to", "moveAway", "digDown"],
            ),
            ("Aim", &["look_at", "look_at_player", "look_at_position"]),
            ("Query", &["perceive", "visual_perceive", "savedPlaces"]),
            ("Memory", &["rememberHere", "goToRememberedPlace"]),
        ];
        for (group_name, tool_names) in &groups {
            let mut group_lines = Vec::new();
            for name in *tool_names {
                if let Some(tool) = self.get(name) {
                    group_lines.push(format!("{}(...) — {}", tool.name(), tool.description()));
                }
            }
            if !group_lines.is_empty() {
                lines.push(String::new());
                lines.push(format!("## {} Tools", group_name));
                lines.extend(group_lines);
            }
        }
        // 未分组的剩余工具
        let all_grouped: std::collections::HashSet<&str> =
            groups.iter().flat_map(|(_, ns)| *ns).copied().collect();
        let ungrouped: Vec<String> = self
            .tools
            .iter()
            .filter(|t| !all_grouped.contains(t.name()))
            .map(|t| format!("{}(...) — {}", t.name(), t.description()))
            .collect();
        if !ungrouped.is_empty() {
            lines.push(String::new());
            lines.push("## Other Tools".to_string());
            lines.extend(ungrouped);
        }
        lines.join("\n")
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ── 副作用分组 (pi agent.rs L417 plan_tool_effect_batches) ──
//
// 把连续兼容的工具分进同一批; 遇到屏障 (BARRIER: write/append/process) 就切新批。
// 兼容批内可并行 (pi 用 buffer_unordered); 游戏输入设备单一, 我们串行执行 (见 agent.rs)。
pub fn plan_tool_effect_batches(effects: &[ToolEffects]) -> Vec<Vec<usize>> {
    let mut batches: Vec<Vec<usize>> = Vec::new();
    let mut active: Option<ToolEffects> = None;

    for (i, e) in effects.iter().enumerate() {
        let compatible = match active {
            None => true,
            Some(a) => a.compatible_with(e),
        };
        if compatible {
            match batches.last_mut() {
                Some(b) => b.push(i),
                None => batches.push(vec![i]),
            }
            active = Some(match active {
                Some(a) => a.union(*e),
                None => *e,
            });
        } else {
            batches.push(vec![i]);
            active = Some(*e);
        }
    }
    batches
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effects_bitmask() {
        let r = ToolEffects::read();
        let w = ToolEffects::write();
        assert!(r.parallel_safe());
        assert!(!w.parallel_safe()); // WRITE 是屏障
        assert!(r.compatible_with(&r)); // read + read 可并行 (同批)
        assert!(!r.compatible_with(&w)); // read + write 不可并行
        assert!(!w.compatible_with(&w)); // write+write 都非 parallel_safe → 不同批 (串行屏障)
    }

    #[test]
    fn plan_batches_groups_compatible() {
        // read, read, write, read → [[0,1],[2],[3]]
        let eff = [
            ToolEffects::read(),
            ToolEffects::read(),
            ToolEffects::write(),
            ToolEffects::read(),
        ];
        let b = plan_tool_effect_batches(&eff);
        assert_eq!(b, vec![vec![0, 1], vec![2], vec![3]]);
    }

    #[test]
    fn registry_lookup_and_extend() {
        struct T;
        impl GameTool for T {
            fn name(&self) -> &str {
                "x"
            }
            fn description(&self) -> &str {
                "d"
            }
            fn parameters(&self) -> Value {
                serde_json::json!({})
            }
            fn execute(&self, _: &str, _: Value, _: Option<ToolUpdateFn>) -> Result<ToolResult> {
                Ok(ToolResult {
                    message: "ok".into(),
                    is_error: false,
                    images: vec![],
                })
            }
        }
        let mut reg = ToolRegistry::new();
        reg.register(Box::new(T));
        assert!(reg.get("x").is_some());
        assert!(reg.get("y").is_none());
        reg.extend(vec![Box::new(T)]);
        assert_eq!(reg.len(), 2);
    }
}
