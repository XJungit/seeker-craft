# Craft-Agent AGENTS.md

## Build

**Rust (azalea — 唯一路线):** `cargo build` / `cargo test --workspace` (edition 2024, nightly pinned via rust-toolchain.toml)

- `rust-toolchain.toml`（根 + `crates/craft-agent-minecraft/`）锁 `nightly-2026-07-21`。azalea (vendor commit c35b57eb) 依赖 nightly-only 特性（`generic_const_exprs` / `min_specialization` / `type_changing_struct_update`），stable 编不过。cargo 自动读该文件选工具链，**不要手动改 channel 为 stable**。
- azalea 依赖在 `crates/craft-agent-minecraft/Cargo.toml` 里**永久写 https git 源**（可移植，别人 clone 直接能编）。本地离线开发由根 `.cargo/config.toml` 的 `[patch."https://github.com/azalea-rs/azalea"]` 段重定向到 `file:///D:/Craft-Agent/vendor/azalea` 本地副本。**不要把 Cargo.toml 改成 file:// 源**——会让别人 clone 后编不过。
- **主入口（LLM 驱动 bot 端到端，带 Web 仪表盘）:**
  ```bash
  cargo run -p craft-agent-viewer -- --goal "收集木头做工作台" --steps 40 --port 8080
  ```
  → http://127.0.0.1:8080 （Agent 在后台线程跑，事件经 SSE 推前端，支持启停/单步）
- 底层 POC 示例：`azalea_adapter_demo` / `azalea_bot_demo` / `azalea_connect` / `azalea_place_demo` / `calibrate_4444`

> 旧路线 `mod-bridge`（Fabric mod TCP 桥接）与 `real`（VLM 截图 + enigo 键鼠）已从源码删除，仅保留 azalea 客户端协议层。

## Project layout

```
crates/
├── craft-agent/              — 核心：通用 Agent 框架（跨游戏抽象）
│   ├── src/
│   │   ├── lib.rs            — 导出 agent / core / adapters 三大模块
│   │   ├── agent/
│   │   │   ├── mod.rs        — Agent 主循环 + AgentConfig/Context/AgentEvent
│   │   │   ├── compaction.rs — token 估算 + 上下文压缩（专用模型→主模型→硬截断三级回退）
│   │   │   ├── prompt.rs     — 动态上下文构建（WorldInfo / Skill / Few-shot / obs 警告）
│   │   │   ├── modes.rs      — 模式响应（self_preservation / self_defense / unstuck）
│   │   │   └── session.rs    — 会话持久化 + manage_knowledge 工具实现
│   │   ├── core/
│   │   │   ├── types.rs      — WorldState / Action / MinecraftAction
│   │   │   ├── adapter.rs    — GameAdapter trait
│   │   │   ├── tool.rs       — GameTool trait + ToolRegistry + ToolEffects 副作用位掩码
│   │   │   ├── message.rs    — Message / AssistantResponse / Usage（ChatML 序列化）
│   │   │   ├── prompt.rs     — PromptBuilder + WorldInfo/WorldInfoLib
│   │   │   ├── session.rs    — Session JSONL 持久化 + Checkpoint
│   │   │   ├── memory.rs     — WorldMemory 空间-状态长期记忆（Resource/Structure/Container/...）
│   │   │   ├── skill.rs      — SkillLibrary 经验抽取
│   │   │   ├── world_model.rs— WorldModel trait 预留接口
│   │   │   └── mod.rs
│   │   └── adapters/mod.rs   — fake 适配器（离线测试）
├── craft-agent-model/        — LLM/VLM 多后端客户端（OpenAI 兼容）
│   └── src/{config,decision,vision,som}.rs
├── craft-agent-minecraft/    — MC 适配器（Azalea 客户端协议层）
│   ├── src/
│   │   ├── lib.rs            — 仅 `azalea-bot` 特性编译导出
│   │   ├── adapter_azalea.rs — GameAdapter 实现：Action 翻译为 BotCommand
│   │   ├── tools_azalea.rs   — 23 个 LLM 工具定义（见下）
│   │   └── azalea/
│   │       ├── mod.rs        — AzaleaBot + BotState + handler（命令队列模式）
│   │       ├── actions.rs    — 原子动作辅助
│   │       ├── auto_craft.rs — 沿配方图递归满足原料的高层合成
│   │       ├── client.rs     — 连接层
│   │       ├── craft.rs      — 2×2 / 3×3 / 熔炼 / 附魔
│   │       ├── ext_state.rs  — BotExtState（村民报价 + 配方书）Bevy plugin
│   │       ├── gather.rs     — 自动寻路采集
│   │       ├── perception.rs — 结构化世界状态快照
│   │       ├── place.rs      — 放置 / 开容器
│   │       ├── recipe_book.rs— vanilla 26.2 配方书模型（builtin_recipes.json 数据驱动）
│   │       ├── recipes.rs    — auto_craft 用的静态配方图（手写常见 26.2 配方）
│   │       ├── builtin_recipes.json — 内置全量 vanilla 26.2 配方数据
│   │       └── trade.rs      — 村民交易
│   └── examples/             — 6 个示例（见 Build 段）
└── craft-agent-viewer/       — Axum Web 仪表盘 + SSE 事件流 + 前端控制面板
    └── src/{main,agent_loop}.rs + index.html
config/agent.toml             — 多后端 LLM/VLM/Compaction 配置
```

## Core architecture

**Agent 主循环** (`crates/craft-agent/src/agent/mod.rs::run_one_turn`):
1. `drain_queues()` — steering / follow_up 队列
2. **压缩检查** — `messages.len() >= 10_000` 或 `estimate_tokens() > context_window - reserve` 触发 `compact()`（专用 Agnes 模型 → 主模型 → `hard_truncate()` 三级回退）
3. **易变注入清理** — `retain` 移除上一轮的 `【当前游戏状态（自动注入）】` / `【邻近世界记忆】` / `[当前目标]` user 消息（每轮重生，不累积）
4. **auto_perceive** — 调 `perceive` 工具注入结构化状态快照
5. **modes 反应** — `check_modes()` 注入 `[MODE: self_preservation/self_defense/unstuck]`
6. **SelfPrompter** — 重新注入 `[当前目标]`（每轮覆盖，避免目标漂移）
7. **动态上下文** — WorldInfo scan + Skill 示例 + Few-shot（词重叠 top-2）+ obs 警告
8. **WorldMemory 邻近记忆** — 以 `__self__` 锚点为中心、半径 64 渲染周边记忆
9. **LLM complete**（带 `RetryConfig` 退避重试 + `retry_abort: AtomicBool` 用户中止）
10. **纯文字回复检测** — 注入续跑 nudge（区分"伪工具调用"与"漏调用"）
11. **死循环检测** — `recent_calls`（容量 10）记录归一化签名（数字→#），4+ 重复注入 nudge。**nudge 必须在所有 tool result 之后注入**，否则插在 `assistant(tool_calls)` 与 `tool` 之间会触发 DeepSeek/OpenAI 400
12. **并行执行工具** — 按 `ToolEffects` 副作用分组（BARRIER 切批），批内 `std::thread::scope` 并行；`manage_knowledge` 串行单独处理
13. **技能抽取** — 非 obs 工具调用提取 SkillLibrary 经验

**System prompt 必须 byte-stable**：`obs_streak` / `knowledge_bootstrapped` 等易变状态已从 system prompt 移出，改为动态 user message 注入。改 prompt 构建逻辑时**不要把这些变量塞回 system prompt**——会破坏 DeepSeek prefix cache（已有回归测试 `regression_system_prompt_byte_stable_across_obs_streak`）。

**Azalea 集成** (`crates/craft-agent-minecraft/src/azalea/mod.rs`): 命令队列模式。Azalea 的 `Client` 只能在 handler 闭包内使用，外部无法持有，所以 `AzaleaBot` 把动作指令 push 进 `BotState.cmd_queue`，handler 每 tick drain 并执行。**所有 `AzaleaBot` 方法都是 fire-and-forget**，结果通过 `BotEvent` 事件通道异步回传。`push_cmd_and_wait()` 是同步阻塞变体（带超时，默认 120s）。handler 串行消费 pending 槽（每 tick 最多推进一条命令），超时 200 tick（≈10s）强制释放避免死锁。

**两层 modes 反应系统**:
- **Agent 层**（`modes.rs`）：每轮检查 perceive 文本，注入提示性 `[MODE: ...]` user 消息给 LLM（不直接执行动作）
- **handler 层**（`azalea/mod.rs` Tick）：直接执行动作，**不依赖 LLM**：
  - `self_preservation`：脚下方/头部是火/岩浆/岩浆块 → 自动 push Goto 脱困（每 tick 检查）
  - `self_defense`：空闲时自动攻击附近敌对生物（每 100 tick ≈5s 检查，zombie/skeleton/creeper/spider/...）

**23 个 LLM 工具** (`crates/craft-agent-minecraft/src/tools_azalea.rs::create_mc_azalea_tools`):

| 类别 | 工具 |
|---|---|
| 感知 | `perceive` / `memory`（save/anchor/query/forget）|
| 移动/挖掘 | `goto` / `mine` / `mine_below` / `interact_block` |
| 战斗 | `attack` |
| 合成/熔炼 | `craft`（2×2）/ `craft_3x3` / `smelt` / `auto_craft`（递归木链）/ `enchant` |
| 采集/放置 | `gather` / `place` / `open` |
| 交互 | `interact_entity` / `trade` |
| 沟通 | `chat` |
| 目标管理 | `set_goal`（写入 SelfPrompter，每轮重注）|
| 复合执行 | `run_plan`（JSON 步骤序列）/ `run_script`（rhai 嵌入式脚本）/ `build`（JSON 蓝图）|
| 知识查询 | `search_wiki`（中文 MC Wiki: wiki.biligame.com/mc）|

所有工具通过 `ArcAzaleaAdapter.execute(Action::Minecraft(...))` 代理到 `AzaleaBot`。**新增工具**：写 struct + impl `GameTool` + 在 `create_mc_azalea_tools` 末尾 `Box::new(...)`，不改 agent 核心。

**配方知识库（双层）**:
- `recipes.rs`：手写静态配方图（产物→输入+方法），驱动 `auto_craft` 沿图递归满足原料。azalea 未暴露配方查询 API，新增配方只需在 `RECIPES` 数组加一行。
- `recipe_book.rs` + `builtin_recipes.json`：vanilla 26.2 全量配方书（数据驱动，与 azalea 版本解耦）。`connect()` 时由 `load_builtin()` 载入 `BotExtState.recipes`，服务端 `RecipeBookAdd` 作为可选 overlay 叠加。

**WorldMemory** (`core/memory.rs`): 空间-状态长期记忆。`record_surroundings`（handler 每 20 tick 触发）扫描 bot 周围 8 格半径，TTL 30s 去重，资源点被挖时标 `depleted`，结构/容器消失时遗忘。MemoryKind: Resource/Structure/Container/Entity/Hazard/Portal/Note。Agent 每轮渲染 `__self__` 锚点周边 64 格注入 prompt。`place`/`open`/`mine` 工具执行后回写记忆。

**fold_tool_history 适配**: 默认 `[llm]` 后端（本地 OC-DSV4F 代理）的上游不支持多轮 tool-calling 历史，`OpenAiLlmClient::chat_tools` 内 `fold_tool_history()` 把 tool 历史折叠为纯文本发送。换用原生支持多轮 tool 的端点时（如 stepfun/agnes/longcat），可在该处关闭折叠。Agent 核心的多轮协议不受影响（它读自身内存 messages）。

## Compaction

使用专用 Agnes-2.0-flash 模型（512K context，需 `AGNES_API_KEY` 环境变量，`[compaction]` 段配置）。失败回退主模型，再失败用 `hard_truncate()`（丢弃旧消息无摘要，并注入系统提示告知 LLM 上下文已截断）。压缩触发条件：`messages.len() >= 10_000` 或 `estimate_tokens() > context_window - reserve`。

`estimate_tokens()`（`compaction.rs`）采用「实测优先 + 尾部启发式」避免重复计入：累加所有 assistant 消息的 `usage.total_tokens`（到最近一次 usage 为止），加上其后尾部消息的字符数估算（`CHARS_PER_TOKEN=2`，图像 `IMAGE_TOKENS=1200`）。压缩时序列化旧历史会**剔除易变注入**（perceive 快照 / 邻近记忆），避免过期坐标污染摘要。

## Critical git rules

- **Commit after every logical change** (`git add -A && git commit --no-verify`). No batching.
- **Never use PowerShell `Set-Content`/`Out-File` for Rust source files.** Use Edit tool. Reason: one `Set-Content -NoNewline` destroyed `CraftAgentBridge.java` (never committed → unrecoverable).

## Dependencies

- Azalea 依赖在 `vendor/azalea/`（git submodule，commit c35b57eb）。本地编译报 git 源错误时，确认根 `.cargo/config.toml` 的 `[patch]` 段指向正确路径，且 `[protocol] file.allow = "always"` 已设置。
- `crates/craft-agent-minecraft/rust-toolchain.toml` 是**第二份** toolchain 锁定文件（确保该 crate 独立编译时也走 nightly），不要删除。
- 工作区 resolver = "3"，所有公共依赖在根 `Cargo.toml` 的 `[workspace.dependencies]` 单点声明，成员用 `dep.workspace = true` 引用。
- `azalea-bot` 特性汇聚 azalea / azalea-client / azalea-inventory / azalea-registry / azalea-world / azalea-protocol / bevy_app / bevy_ecs / tokio（bevy 版本必须与 azalea 完全一致，否则 ecs 类型不兼容）。
- `rhai` 1.25.1（`sync` + `serde` 特性）驱动 `run_script` 工具。
- Windows 专用：`windows-sys` 仅 `[target.'cfg(windows)'.dependencies]` 引入（azalea 路线实际不依赖，保留供未来用）。

## Config

`config/agent.toml` 换后端只改这份文件，无需重编译。密钥优先用 `api_key_env`（从环境变量读），别把 sk-xxx 明文提交。三段独立配置：
- `[llm]` — 决策 LLM（active=deepseek，本地 OC-DSV4F 代理）
- `[vlm]` — 视觉理解后端（azalea 路线**不使用** VLM，perceive 工具返回结构化世界状态）
- `[compaction]` — 专用压缩模型（active=agnes，开启 thinking 提升摘要质量）
- `[perceive]` — 视觉后端模式（`vlm` / `multimodal`，仅 real 路线用，azalea 路线忽略）

注意：Agent 的 system prompt 与工具集**不**从此文件读取——避免"改配置却不生效"的假象。system prompt 在示例入口通过 `AgentConfig::new(...)` 注入；工具集在 `create_mc_azalea_tools` 注册。
