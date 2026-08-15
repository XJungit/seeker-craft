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

    /// 慢动作标记（P99，2026-08-02）：工具执行耗时秒~分钟级（MC 的 goto/mine/
    /// gather 等异步命令）。标记后 agent 在批处理中执行完该工具即中止剩余
    /// 预测调用（基于旧状态的后续调用不再有意义），结果回填历史，下一轮
    /// LLM 基于动作完成后的真实状态重新决策（opencode 式等待慢命令结果）。
    /// 快工具（perceive/equip/craft 等毫秒~秒级）保持批量，不受影响。
    fn is_slow(&self) -> bool {
        false
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
        let params = if params.as_object().is_some_and(|o| !o.is_empty()) {
            params
        } else {
            // OpenAI 规范要求 function 必须带 parameters 字段；
            // 缺省时补空对象，避免严格端点（如本地 OC-DSV4F 代理）返回 400。
            serde_json::json!({ "type": "object", "properties": {} })
        };
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

/// 工具知识分组表：LLM 工具按域分类展示（输出 `## {group} Tools` 段头）。
///
/// 对应 `craft-agent-minecraft` 实际注册的工具集（`tools_azalea.rs` 的
/// `create_mc_azalea_tools_full`）。新增工具时必须归入某个分组，否则落
/// `## 其他工具` 兜底并被回归测试拦截：
/// - `crates/craft-agent-minecraft` 的 `regression_all_tool_names_in_knowledge_group`
///   （每个注册工具名都要在某个分组里）
/// - 本模块的 `regression_knowledge_groups_well_formed`（无重复/无空组）
///
/// ⚠️ 这是打进系统提示（knowledge_string）的一部分——改分组名/顺序会碎一次
/// DeepSeek 前缀缓存（一次性，C8 knowledge_cache 之后自动稳定）。
pub const TOOL_GROUPS: &[(&str, &[&str])] = &[
    (
        "感知",
        &[
            "perceive",
            "memory",
            "remember",
            "search_wiki",
            "search_for_block",
        ],
    ),
    (
        "移动",
        &[
            "goto",
            "goto_player",
            "move_away",
            "mine_below",
            "mine_above",
            "pickup",
            "follow",
            "stop_follow",
        ],
    ),
    ("挖矿", &["mine", "make_obsidian"]),
    ("模式", &["set_mode"]),
    (
        "交互",
        &[
            "interact_block",
            "interact_entity",
            "attack",
            "defend",
            "use_item",
            "shoot",
            "sleep",
        ],
    ),
    (
        "合成",
        &["craft", "craft_3x3", "smelt", "auto_craft", "enchant"],
    ),
    ("采集", &["gather", "till_and_sow", "harvest"]),
    (
        "建造",
        &["place", "build", "build_blueprint", "list_blueprints"],
    ),
    (
        "容器",
        &["open", "chest_view", "chest_withdraw", "chest_deposit"],
    ),
    ("背包", &["equip", "discard", "consume"]),
    ("社交", &["trade", "give"]),
    (
        "元操作",
        &[
            "chat",
            "run_plan",
            "run_script",
            "new_action",
            "list_actions",
        ],
    ),
];

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
        // 按 group 分组展示（分组表见 TOOL_GROUPS，与本项目真实工具域对齐）
        let mut grouped: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for (group_name, tool_names) in TOOL_GROUPS {
            let mut group_lines = Vec::new();
            for name in *tool_names {
                if let Some(tool) = self.get(name) {
                    grouped.insert(tool.name());
                    group_lines.push(format!("{}(...) — {}", tool.name(), tool.description()));
                }
            }
            if !group_lines.is_empty() {
                lines.push(String::new());
                lines.push(format!("## {group_name}"));
                lines.extend(group_lines);
            }
        }
        // 未分组的剩余工具
        let ungrouped: Vec<String> = self
            .tools
            .iter()
            .filter(|t| !grouped.contains(t.name()))
            .map(|t| format!("{}(...) — {}", t.name(), t.description()))
            .collect();
        if !ungrouped.is_empty() {
            lines.push(String::new());
            lines.push("## 其他工具".to_string());
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
        let a = ToolEffects::append();
        let n = ToolEffects::network();
        let p = ToolEffects::process();
        assert!(r.parallel_safe());
        assert!(!w.parallel_safe()); // WRITE 是屏障
        assert!(!a.parallel_safe()); // APPEND 是屏障
        assert!(!p.parallel_safe()); // PROCESS 是屏障
        assert!(n.parallel_safe()); // NETWORK 不是屏障，可并行
        assert!(r.compatible_with(&r)); // read + read 可并行 (同批)
        assert!(!r.compatible_with(&w)); // read + write 不可并行
        assert!(!w.compatible_with(&w)); // write+write 都非 parallel_safe → 不同批
        assert!(n.compatible_with(&r)); // network + read 可并行
        assert!(!n.compatible_with(&w)); // network + write 不可并行
        assert!(!a.compatible_with(&r)); // append + read 不可并行
        assert!(n.compatible_with(&n)); // network + network 可并行
    }

    #[test]
    fn effects_labels() {
        let r = ToolEffects::read();
        let labels = r.labels();
        assert!(labels.contains(&"read"));
        assert_eq!(labels.len(), 1);

        let rw = r.union(ToolEffects::write());
        let labels = rw.labels();
        assert!(labels.contains(&"read"));
        assert!(labels.contains(&"write"));
    }

    #[test]
    fn effects_barrier_union() {
        let bar = ToolEffects {
            bits: ToolEffects::BARRIER,
        };
        assert!(!bar.parallel_safe());
        // BARRIER = WRITE | APPEND | PROCESS, 不含 READ 和 NETWORK
        assert!(!bar.reads(), "BARRIER 不含 READ");
        assert!(bar.writes(), "BARRIER 含 WRITE");
        assert!(bar.appends(), "BARRIER 含 APPEND");
        assert!(bar.processes(), "BARRIER 含 PROCESS");
        assert!(!bar.networks(), "BARRIER 不含 NETWORK");
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
    fn plan_batches_all_reads_in_one_batch() {
        let eff = [
            ToolEffects::read(),
            ToolEffects::read(),
            ToolEffects::read(),
        ];
        let b = plan_tool_effect_batches(&eff);
        assert_eq!(b, vec![vec![0, 1, 2]]);
    }

    #[test]
    fn plan_batches_network_and_read_together() {
        let eff = [
            ToolEffects::read(),
            ToolEffects::network(),
            ToolEffects::read(),
        ];
        let b = plan_tool_effect_batches(&eff);
        assert_eq!(b, vec![vec![0, 1, 2]]);
    }

    #[test]
    fn plan_batches_every_write_separate() {
        let eff = [
            ToolEffects::write(),
            ToolEffects::write(),
            ToolEffects::write(),
        ];
        let b = plan_tool_effect_batches(&eff);
        assert_eq!(b, vec![vec![0], vec![1], vec![2]]);
    }

    #[test]
    fn plan_batches_mixed_network_read_write() {
        let eff = [
            ToolEffects::read(),
            ToolEffects::network(),
            ToolEffects::write(),
            ToolEffects::read(),
        ];
        let b = plan_tool_effect_batches(&eff);
        assert_eq!(b, vec![vec![0, 1], vec![2], vec![3]]);
    }

    #[test]
    fn plan_batches_append_and_process_barrier() {
        let eff = [
            ToolEffects::read(),
            ToolEffects::append(),
            ToolEffects::process(),
            ToolEffects::read(),
        ];
        let b = plan_tool_effect_batches(&eff);
        assert_eq!(b, vec![vec![0], vec![1], vec![2], vec![3]]);
    }

    #[test]
    fn plan_batches_single_element() {
        let eff = [ToolEffects::read()];
        let b = plan_tool_effect_batches(&eff);
        assert_eq!(b, vec![vec![0]]);
    }

    #[test]
    fn plan_batches_empty() {
        let eff: [ToolEffects; 0] = [];
        let b = plan_tool_effect_batches(&eff);
        assert!(b.is_empty());
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

    #[test]
    fn registry_to_openai_defs_returns_valid_json() {
        struct Dummy;
        impl GameTool for Dummy {
            fn name(&self) -> &str {
                "dummy"
            }
            fn description(&self) -> &str {
                "a test tool"
            }
            fn parameters(&self) -> Value {
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "arg1": {"type": "string"}
                    }
                })
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
        reg.register(Box::new(Dummy));
        let defs = reg.to_openai_defs();
        assert_eq!(defs.len(), 1);
        let def = &defs[0];
        assert_eq!(def["function"]["name"], "dummy");
        assert!(
            def["function"]["description"]
                .as_str()
                .unwrap()
                .contains("test tool")
        );
    }

    /// 分组表形态校验：工具名不跨组重复、无空组、无重复组名。
    /// 覆盖性（每个注册工具都在某组）由 craft-agent-minecraft 侧的
    /// `regression_all_tool_names_in_knowledge_group` 兜底。
    #[test]
    fn regression_knowledge_groups_well_formed() {
        assert!(!TOOL_GROUPS.is_empty(), "分组表不能为空");
        let mut seen_group_names = std::collections::HashSet::new();
        let mut seen_tools = std::collections::HashSet::new();
        for (group_name, tools) in TOOL_GROUPS {
            assert!(
                seen_group_names.insert(*group_name),
                "分组名重复: {group_name}"
            );
            assert!(!tools.is_empty(), "分组 {group_name} 无工具");
            for t in *tools {
                assert!(
                    seen_tools.insert(*t),
                    "工具 `{t}` 出现在多个分组（跨组重复）"
                );
            }
        }
        assert!(
            seen_tools.len() >= 40,
            "分组表覆盖率异常：仅 {} 个工具被分组（应覆盖全部注册工具）",
            seen_tools.len()
        );
    }

    /// 知识字符串渲染：全部已分组工具都出现在对应组段头下，
    /// 且注册工具全被分组时不会出现 `## 其他工具` 兜底段。
    #[test]
    fn regression_knowledge_string_groups_all_registered_tools() {
        // 用分组表里全部工具名注册 stub 工具（模拟完整工具集）
        struct Stub(&'static str);
        impl GameTool for Stub {
            fn name(&self) -> &str {
                self.0
            }
            fn description(&self) -> &str {
                "stub"
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
        for (_, tools) in TOOL_GROUPS {
            for name in *tools {
                reg.register(Box::new(Stub(name)));
            }
        }
        let knowledge = reg.to_knowledge_string();
        // 每个组段头都渲染（非空组必然有注册工具）
        for (group_name, _) in TOOL_GROUPS {
            assert!(
                knowledge.contains(&format!("## {group_name}")),
                "缺少组段头 ## {group_name}"
            );
        }
        // 注册工具全覆盖 → 不应出现 其他工具 兜底段
        assert!(
            !knowledge.contains("## 其他工具"),
            "全部工具已分组，不应出现 ## 其他工具 兜底段"
        );
        // 每个工具都在其组段头之后渲染（抽查：组头行号 < 组内工具行号）
        let lines: Vec<&str> = knowledge.lines().collect();
        for (group_name, tools) in TOOL_GROUPS {
            let header_pos = lines
                .iter()
                .position(|l| *l == format!("## {group_name}"))
                .expect("组段头存在");
            for t in *tools {
                let tool_pos = lines
                    .iter()
                    .position(|l| l.starts_with(&format!("{t}(...)")))
                    .unwrap_or_else(|| panic!("工具 `{t}` 未渲染在知识字符串中"));
                assert!(
                    tool_pos > header_pos,
                    "工具 `{t}` 应渲染在组 {group_name} 段头之后"
                );
            }
        }
    }
}
