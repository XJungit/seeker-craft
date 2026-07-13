# Craft-Agent v0.4: Agent 核心重构设计

> 参考 pi_agent_rust (Rust, 43K+ 行已读) 和 SillyTavern (TypeScript, PromptManager + WorldInfo + ToolManager 已读)
>
> 目标: 把当前"硬编码玩具 Agent"升级为"可扩展通用游戏 Agent 框架"

---

## 一、现状诊断

### 当前架构问题

```
agent_multi_step.rs (90 行入口)
  └─ agent.rs (Agent::run, 80 行 loop)
       ├─ 工具执行: match name { "perceive" => ..., "aim_and_mine" => ..., ... }
       ├─ 消息存储: Vec<Value> (裸 JSON, 无类型安全)
       ├─ 系统 prompt: 单个字符串 blob (规则 + 示例 + 身份 混在一起)
       └─ 无会话树、无压缩、无工具副作用声明
```

| 问题 | 根因 | 影响 |
|------|------|------|
| 新增工具需改 3 个文件 | Action enum 硬编码 | 无法扩展 Minecraft 外的新工具 |
| 消息历史无类型 | `Vec<Value>` | 解析脆弱，调试困难 |
| Prompt 一团乱 | 单 blob 混所有信息 | LLM 行为不可控 |
| 无工具副作用 | 所有动作串行 | 无法并行感知+动作 |
| 无上下文压缩 | 线性增长 | 长会话 OOM |
| 无会话分支 | 无 id/parentId | 无法 /fork 实验不同策略 |

### 对比 pi_agent_rust

| 维度 | pi (参考标准) | 我们 (当前) | 差距 |
|------|:---:|:---:|:--:|
| 工具系统 | `Tool` trait + `ToolRegistry` | `Action` enum 硬编码 | 大 |
| 消息模型 | `Message` enum (User/Assistant/ToolResult) | `Vec<Value>` | 大 |
| Prompt 组装 | 多层 (system + AGENTS.md + skills) | 单 blob | 中 |
| 会话存储 | JSONL 树 (id/parentId) | 线性 Vec | 大 |
| 副作用声明 | `ToolEffects` 位掩码 | 无 | 中 |
| 扩展系统 | QuickJS 运行时 + 事件钩子 | 无 | 大 (远期) |
| **游戏适配** | **无 (编码专用)** | **GameAdapter trait** | **我们领先** |
| **视觉感知** | **无** | **VLM 截图理解** | **我们领先** |

---

## 二、目标架构 (v0.4)

```
┌─────────────────────────────────────────────────────┐
│  craft-agent (通用 Agent 框架, 不感知具体游戏)       │
│  ┌──────────┐  ┌────────────┐  ┌──────────────────┐ │
│  │ Agent    │  │ ToolRegistry│  │ SessionManager   │ │
│  │ run()    │  │ get(name)  │  │ tree/fork/compact │ │
│  │ messages │  │ register() │  │ JSONL persistence │ │
│  └────┬─────┘  └─────┬──────┘  └────────┬─────────┘ │
│       │              │                  │           │
│  ┌────┴──────────────┴──────────────────┴──────────┐│
│  │ Message enum (User/Assistant/ToolResult/System)  ││
│  │ Tool trait (name/desc/params/execute/effects)     ││
│  │ PromptBuilder (main+role+scenario+examples+jailbreak)││
│  └─────────────────────────────────────────────────┘│
└─────────────────────────────────────────────────────┘
                         │
                         ▼
┌─────────────────────────────────────────────────────┐
│  craft-agent-model (Provider 层, 不感知游戏)         │
│  ┌──────────────────┐  ┌──────────────────────────┐ │
│  │ VisionClient     │  │ DecisionClient            │ │
│  │ (VLM: stepfun/   │  │ (LLM: longcat/minicpm)   │ │
│  │  agnes/minicpm)  │  │ chat_tools()             │ │
│  └──────────────────┘  └──────────────────────────┘ │
└─────────────────────────────────────────────────────┘
                         │
                         ▼
┌─────────────────────────────────────────────────────┐
│  craft-agent-minecraft (Minecraft 实现层)            │
│  ┌──────────────────┐  ┌──────────────────────────┐ │
│  │ MinecraftAdapter │  │ MinecraftTools            │ │
│  │ (GameAdapter)    │  │ PerceiveTool              │ │
│  │ capture/screenshot│  │ AimAndMineTool            │ │
│  └──────────────────┘  │ MoveTool / LookTool       │ │
│                        └──────────────────────────┘ │
└─────────────────────────────────────────────────────┘
```

---

## 三、四个 Phase 的实施计划

### Phase 3.1: Message enum — 消息类型安全

**参考**: pi `model.rs` (1753 行)

**现状**: `Vec<serde_json::Value>` 裸 JSON
**目标**: 强类型 Message 枚举

```rust
// crates/craft-agent/src/core/message.rs (新建)

/// 对话消息 (pi 风格: 类型化, 带时间戳和元数据)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "role")]
pub enum Message {
    #[serde(rename = "system")]
    System(SystemMessage),
    #[serde(rename = "user")]
    User(UserMessage),
    #[serde(rename = "assistant")]
    Assistant(AssistantMessage),
    #[serde(rename = "tool")]
    ToolResult(ToolResultMessage),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemMessage {
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserMessage {
    pub content: String,
    pub timestamp: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssistantMessage {
    pub content: Option<String>,     // null when tool_calls present
    pub tool_calls: Vec<ToolCall>,   // empty when text response
    pub model: String,
    pub usage: Option<Usage>,
    pub timestamp: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResultMessage {
    pub tool_call_id: String,
    pub tool_name: String,
    pub content: String,             // LLM 可读的结果文本
    pub is_error: bool,
    pub timestamp: i64,
}

/// Token 用量追踪 (pi 的 Usage)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}
```

**改动范围**:
- 新建 `crates/craft-agent/src/core/message.rs`
- `agent.rs`: `Vec<Value>` → `Vec<Message>` + 提供 `to_chatml()` 转换
- `agent_multi_step.rs`: tool call/result 消息改用 Message 构造

**收益**: 类型安全, 调试可读, 为会话树和压缩打基础
**风险**: 低 (内部重构, 不影响 adapter/vision/decision 层)
**工作量**: ~150 行

---

### Phase 3.2: Tool trait — 工具可扩展

**参考**: pi `tools.rs` (12820 行, 但我们只需 200 行核心)

**现状**: `Action` enum 硬编码, `agent.rs` 里 `match name { "perceive" => ... }`
**目标**: Tool trait + ToolRegistry

```rust
// crates/craft-agent/src/core/tool.rs (新建)

/// 工具副作用声明 (pi 的 ToolEffects 简化版)
#[derive(Debug, Clone, Copy)]
pub struct ToolEffects {
    /// 只读: 不修改游戏状态 (perceive, look)
    pub is_readonly: bool,
    /// 修改游戏状态 (aim_and_mine, move)
    pub is_destructive: bool,
}

/// 游戏工具 trait (pi 的 Tool trait 游戏版)
pub trait GameTool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters(&self) -> serde_json::Value;  // JSON Schema
    fn effects(&self) -> ToolEffects;
    fn execute(
        &self,
        args: serde_json::Value,
    ) -> anyhow::Result<ToolResult>;
}

/// 工具执行结果
pub struct ToolResult {
    /// LLM 可读的描述文本
    pub message: String,
    /// 是否出错
    pub is_error: bool,
    /// 是否需要重新 perceive (视角变化后)
    pub need_reperceive: bool,
}

/// 工具注册表 (pi 的 ToolRegistry 简化版)
pub struct ToolRegistry {
    tools: Vec<Box<dyn GameTool>>,
}

impl ToolRegistry {
    pub fn new() -> Self;
    pub fn register(&mut self, tool: Box<dyn GameTool>);
    pub fn get(&self, name: &str) -> Option<&dyn GameTool>;
    pub fn tools(&self) -> &[Box<dyn GameTool>];
    pub fn to_openai_defs(&self) -> Vec<serde_json::Value>;
}
```

**Minecraft 工具实现** (放在 `craft-agent-minecraft/src/tools/`):
```rust
// PerceiveTool: 调用 VLM 感知
// AimAndMineTool: raw_mouse_rel + 左键
// MoveTool: W 键移动
// LookTool: raw_mouse_rel 转视角
```

每个工具一个 struct，impl `GameTool` trait。

**改动范围**:
- 新建 `crates/craft-agent/src/core/tool.rs`
- 新建 `crates/craft-agent-minecraft/src/tools/` (4 个工具 struct)
- `agent.rs`: `match name { ... }` → `self.tools.get(name)?.execute(args)`
- `agent_multi_step.rs`: 工具注册替代硬编码

**收益**: 新增工具不改 agent.rs, 跨游戏复用 GameTool trait
**风险**: 低 (内部重构)
**工作量**: ~300 行

---

### Phase 3.3: PromptBuilder — 五层组装

**参考**: 酒馆 `PromptManager.js` (12 层, 我们取 5 层核心)

**现状**: 单 blob 字符串
**目标**: 五层可配置 prompt

```rust
// crates/craft-agent/src/core/prompt.rs (新建)

pub struct PromptBuilder {
    /// 1. 身份: "你是 Minecraft AI 玩家"
    pub identity: String,
    /// 2. 角色描述: "你擅长采集资源, 优先挖树"
    pub role_desc: String,
    /// 3. 场景: (动态) "你在橡树平原, 夜晚即将来临"
    pub scenario: String,
    /// 4. 示例对话 (最重要的行为塑造手段)
    pub examples: Vec<String>,
    /// 5. 后置指令: "不要问问题。直接行动。"
    pub jailbreak: String,
}

impl PromptBuilder {
    pub fn build(&self) -> String {
        format!(
            "{identity}\n\n{role}\n\n{scenario}\n\n## Examples\n{examples}\n\n{jailbreak}",
            identity = self.identity,
            role = self.role_desc,
            scenario = self.scenario,
            examples = self.examples.iter()
                .map(|e| format!("- {e}"))
                .collect::<Vec<_>>()
                .join("\n"),
            jailbreak = self.jailbreak,
        )
    }
}
```

**World Info 动态注入** (酒馆 world-info.js 模式):
```rust
// 每次 perceive 后, 检测结果以 World Info 格式注入到上下文
pub struct WorldInfo {
    pub trigger: String,    // "tree", "stone", "creeper"
    pub content: String,    // "前方有橡树, 偏移(122,-103)。应 aim_and_mine tree。"
    pub sticky: u32,        // 保持激活轮数
}

// agent.run() 每个 turn 开始前:
// 扫描最近 N 条消息 → 匹配 WorldInfo trigger → 注入匹配条目
```

**改动范围**:
- 新建 `crates/craft-agent/src/core/prompt.rs`
- `AgentConfig`: 用 `PromptBuilder` 替代 `system_prompt: String`
- `agent.rs`: perceive 后将结果转为 WorldInfo 注入消息流

**收益**: LLM 行为质量质的提升, 示例对话比规则更有效
**风险**: 低
**工作量**: ~200 行

---

### Phase 3.4: Session 树 + Compaction (远期)

**参考**: pi `session.rs` (12751 行, 我们只需核心模式)

**目标**:
- 每个 turn 创建 `SessionEntry { id, parent_id, timestamp }`
- 支持 `/fork` 从任意 entry 分支实验不同策略
- Token 超限时 `compact()` 用 LLM 生成摘要替代旧消息

**这是远期目标**, 当前 20 轮以内的对话不需要压缩。

---

## 四、实施优先级

| Phase | 内容 | 收益 | 风险 | 工作量 | 优先级 |
|-------|------|:--:|:--:|:--:|:--:|
| 3.1 | Message enum | 类型安全 | 低 | 150 行 | 🔴 P0 |
| 3.2 | Tool trait | 可扩展 | 低 | 300 行 | 🔴 P0 |
| 3.3 | PromptBuilder | 行为质量 | 低 | 200 行 | 🟡 P1 |
| 3.4 | Session 树 | 长期会话 | 中 | 500+ 行 | 🟢 P2 |

---

## 五、与现有代码的兼容

所有 Phase 都是**内部重构**, 不影响:
- `MinecraftAdapter` (截图 + 感知 + 执行)
- `OpenAiVisionClient` / `OpenAiLlmClient`
- `config/agent.toml`
- `agent_multi_step.rs` 入口 (只改内部实现)

改动范围锁定在 `crates/craft-agent/src/`:
```
crates/craft-agent/src/
├── agent.rs          # 修改: 用 ToolRegistry 替代 match
├── core/
│   ├── types.rs      # 保留 (WorldState, Target 不变)
│   ├── adapter.rs    # 保留
│   ├── message.rs    # 新建: Message enum
│   ├── tool.rs       # 新建: GameTool trait + ToolRegistry
│   └── prompt.rs     # 新建: PromptBuilder + WorldInfo
└── adapters/fake.rs  # 更新测试
```
