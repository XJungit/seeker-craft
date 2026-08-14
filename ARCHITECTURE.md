# Craft-Agent Architecture

This document explains the high-level architecture of Craft-Agent and how the
main crates interact at runtime.

## Layered Overview

```text
DSH (DeepSeek Harness, 外部大脑)          ← 唯一 LLM 大脑（决策/规划/自进化）
        |  HTTP 桥（DSH 侧 dsh-bridge 插件）
        v
craft-agent-viewer (Axum + SSE 桥)        ← /api/connect / /api/bot_tool / /api/game-state / /api/goal
        |
        v
craft-agent-minecraft  ──→  azalea (vendor)  ──→  MC server (TCP)
        |
        v
   craft-agent (纯逻辑库：types/GameTool/ToolRegistry/WorldMemory/session/task/profile/skill)
```

> **DSH 桥接模式（2026-08-14 起）**：in-bot LLM 循环（`craft-agent/src/agent/mod.rs::run_one_turn`
> 的 13 步主循环）已彻底移除。DSH 成为唯一大脑，经 viewer 桥驱动 bot；Rust 侧不再组装 LLM
> prompt、不再做每轮自动注入。详见 AGENTS.md「DSH 桥接模式」段。

- [`craft-agent`](./crates/craft-agent/) — 纯逻辑库：types / GameTool / ToolRegistry /
  WorldMemory（空间记忆）/ session（JSONL 归档格式）/ task（23 任务）/ profile / skill。
  **不含** Agent 主循环或 LLM 调用。
- [`craft-agent-minecraft`](./crates/craft-agent-minecraft/) — Minecraft 适配器与 53 工具集
  （`tools_azalea.rs::ALL_TOOL_NAMES`），构建于 azalea 协议层；bot 端实时能力（WorldMemory
  每 20 tick 扫描、perceive 快照）在此。
- [`craft-agent-model`](./crates/craft-agent-model/) — LLM/VLM 客户端与配置模型（in-bot 循环时代
  使用；DSH 模式下由 DSH 大脑自带 LLM 客户端，此 crate 为兼容保留）。
- [`craft-agent-viewer`](./crates/craft-agent-viewer/) — Axum + SSE Web 仪表盘 + DSH 桥
  （连接 bot / 驱动工具 / 状态呈现）。无 LLM 逻辑。

## Runtime Flow

### DSH Bridge Runtime (current)

```text
DSH 大脑（外部 Cordis 插件，tools/dsh-bridge/）
  │  POST /api/connect          → viewer 把 azalea 客户端连上 MC（account CraftAgent）
  │  POST /api/bot_tool {name,args}  → viewer 派发 53 工具之一（GameTool::execute）
  │  GET  /api/game-state       → viewer 实时拉 BotState 快照（perceive 格式）
  │  POST /api/goal {text}      → viewer 更新运营目标
  ▼
craft-agent-viewer（Axum 桥）
  └── GameAdapter：capture / perceive / execute
      └── craft-agent-minecraft 53 工具（P100/P101/P102/P132 派发时自动修正）
      └── azalea 客户端（订阅 MC 服务端实体/方块/库存更新，维护 ECS world）
      └── handler.rs tick：bot 端反应式模式（self_preservation / self_defense 自动执行，无需 LLM）
      └── WorldMemory：每 20 tick 扫描附近方块/实体，锚点 + 结构/容器/危险记忆（TTL 30s）
```

**每轮「自动注入世界状态/记忆/上下文」由 DSH 大脑承担**（in-bot 循环时代这部分在
`auto_perceive` / `build_dynamic_instructions_msg` / `self_prompt` 里，现已删除）。Rust 侧
只提供 bot 端**实时**快照（`/api/game-state`）与世界记忆（WorldMemory），由 DSH 决定何时注入。

> **历史架构（已移除，2026-08-14）**：原 in-bot `run_one_turn` 13 步主循环（drain_queues →
> 压缩 → 易变注入清理 → auto_perceive → modes → SelfPrompter → 动态上下文 → WorldMemory →
> LLM → 纯文字检测 → 死循环检测 → execute_batch → 技能抽取）。其中 `execute_batch` /
> `finalize_abort` / Agent Layer `modes.rs` / `auto_perceive` / `self_prompt` 注入逻辑已随
> 主循环删除；`GameTool::effects` / `is_slow` / `ToolEffects` 位掩码仍作为工具定义保留
> （兼容与文档用途），但运行时批处理执行已不存在（DSH 经 `/api/bot_tool` 逐工具驱动）。

### Bot-side Reactive Modes (handler.rs tick)

```
Handler Layer (azalea/handler.rs Tick) — 直接执行，无需 LLM 介入
  └── self_preservation: 火/熔岩 → 自动 Goto 逃生（每 tick）
  └── self_defense: 敌对生物 ≤4 格 + !is_busy() → 自动攻击（每 100 tick）
DSH 大脑可经 set_mode 工具切换姿态（self_preservation / self_defense / unstuck /
cowardice / hunting / item_collecting / torch_placing / idle_staring / cheat …）
```

> **P2.2 module layout**: the azalea adapter is split across
> `azalea/mod.rs` (AzaleaBot + connect + action API, ~2000 lines),
> `azalea/commands.rs` (BotCommand 33 variants + QueuedCommand + parse_chat_command),
> `azalea/handler.rs` (BotState + tick handler + helpers), plus domain modules:
> craft / place / gather / auto_craft / recipes / till / harvest / sleep /
> perception / actions / smart_actions.

## Key Abstractions

- `GameAdapter`: capture / perceive / execute.
- `GameTool`: name / description / parameters / effects / execute.
  - `ToolEffects`: bit flags carried on each tool for documentation/compat
    (READ / WRITE / APPEND / NETWORK / PROCESS / BARRIER). NOTE: the in-bot
    batch executor (`execute_batch`/`finalize_abort`) that consumed these flags
    was removed with the agent loop; DSH now drives tools one-by-one via
    `/api/bot_tool`, so the flags are informational only.
- `SessionEntry`: tool call, state snapshot, compaction metadata.
- `AgentEvent`: external hook for viewers and controllers.
- `WorldMemory`: spatial memory keyed by `MemoryPos`, chunk-indexed for O(1) nearby
  queries, 6 types (Resource/Structure/Container/Entity/Hazard/Portal/Note),
  TTL 30s dedup, 64-block radius rendered each turn.

## 53 LLM Tools

> 权威清单：`tools_azalea.rs::ALL_TOOL_NAMES`（53 个，与 `create_mc_azalea_tools_full` vec 一一对应）。

| Category | Tools | Side Effect |
|---|---|---|
| Perception | `perceive` | READ |
| Memory | `memory` (save/anchor/query/forget) | READ/WRITE |
| Block Search | `search_for_block` | READ |
| Knowledge | `search_wiki` | NETWORK |
| Movement | `goto` / `goto_player` / `move_away` / `mine_below` / `mine_above` / `pickup` / `follow` / `stop_follow` | WRITE |
| Mining | `mine` / `make_obsidian` | WRITE |
| Mode | `set_mode` | WRITE |
| Block Interaction | `interact_block` / `interact_entity` | WRITE |
| Combat | `attack` / `defend` / `use_item` / `shoot` | WRITE |
| Sleeping | `sleep` | WRITE |
| Crafting | `craft` (2×2) / `craft_3x3` | WRITE |
| Smelting | `smelt` | WRITE (wait) |
| Auto-craft | `auto_craft` | WRITE (recursive) |
| Enchanting | `enchant` | WRITE |
| Gathering | `gather` / `till_and_sow` / `harvest` | WRITE (pathfinding) |
| Placing | `place` / `build` / `build_blueprint` / `list_blueprints` | WRITE |
| Container | `open` / `chest_view` / `chest_withdraw` / `chest_deposit` | READ/WRITE |
| Equipment | `equip` / `discard` / `consume` | WRITE |
| Trading | `trade` | WRITE |
| Social | `give` | WRITE |
| Chat | `chat` | NETWORK |
| Goal | `set_goal` / `pause_goal` / `resume_goal` | WRITE |
| Composite | `run_plan` / `run_script` | WRITE |
| Custom Action | `new_action` / `list_actions` | WRITE (persist) |
| Task Chain | `task_complete` / `task_retry` | WRITE |

## Critical Constraints

### System Prompt Byte Stability (DSH-era reframing)

> **历史约束（in-bot 循环时代）**：DeepSeek 前缀缓存要求系统提示逐字节一致，动态内容
> （`【当前游戏状态】` / `【邻近世界记忆】` / `[当前目标]` / `【参考示例】`）走用户消息、
> 轮间剔除，回归测试 `regression_system_prompt_byte_stable_across_obs_streak` 守护。
> **现状**：该测试与注入逻辑已随 in-bot 循环删除；系统提示的字节稳定性现由 **DSH 大脑**
> 的 prompt 装配负责（Rust 侧不再组装 LLM prompt）。本段保留为设计原则备忘，不对应现存代码。

- ❌ 绝不在系统提示嵌入动态变量（obs_streak、knowledge_bootstrapped 等）
- ✅ 动态内容走用户消息，逐轮注入、下一轮剔除
- ✅ 感知状态作为 `【当前游戏状态（自动注入）】` 用户消息

### Mindcraft Philosophy Alignment

Bot tools only do what they can do; if they can't, return `Err` and let the LLM
decide (full rules kept in local workflow notes, not shipped).

- ❌ Never auto-craft tool blocks (furnace/pickaxe/axe/sword) inside tools
- ❌ Never auto-satisfy material dependencies (no `gather → auto_craft → smelt` chains)
- ✅ Error messages must list complete resolution steps
- ✅ LLM plans multi-step synthesis; bot tools are atomic operations

### Tool Name Discipline

| Actual Name | Forbidden Aliases |
|---|---|
| `go` | goto, move_to, walk_to, moveto (goto is also a rhai reserved keyword) |
| `gather` | collect, pickup_item, get |
| `mine` | dig, break, destroy |
| `attack` | combat, fight, hit, kill |
| `craft` | make, create, produce |
| `place` | put, set, build_block |

Forbidden aliases get emitted by the LLM as text pseudo-calls (`【工具调用】goto(...)`)
instead of real `tool_calls` JSON.

## P56-P58: Plain-Text Reply Governance (historical)

> **历史治理（in-bot 循环时代）**：LLM 可能在纯文本里宣称「任务完成 ✅」而不调用工具，
> 浪费回合。`is_premature_completion`（`agent/mod.rs`）检测 9+ 关键词注入 continue nudge；
> `set_goal("")` + 文本声明完成则拒绝 `stop_goal()` 并强制 perceive 验证。**现状**：该逻辑
> 随主循环删除，纯文本完成的治理现由 **DSH 大脑**的 guardrails 负责。本段保留为设计原则备忘。

- P56: detect premature-completion keywords → continue nudge.
- P58: `set_goal("")` + text completion → refuse stop, inject mandatory perceive nudge.

## Recipe System (Dual Layer)

- `recipes.rs`: handwritten static recipe graph, drives `auto_craft` recursive
  material satisfaction.
- `recipe_book.rs` + `builtin_recipes.json`: vanilla 26.2 full recipe book (P48).
- `craft_3x3` lookup order: RecipeBook → handwritten SHAPED_RECIPES → Err.

## Test Infrastructure

```bash
cargo test --workspace --no-fail-fast        # 全量测试（当前约 318，随测试增长）
cargo test -p craft-agent --lib              # 核心逻辑测试
cargo test -p craft-agent-minecraft --features azalea-bot --lib  # 适配器测试（需 azalea-bot feature）
cargo test -p craft-agent-model --lib        # 模型测试
```

- All `regression_*` tests guard against reverting to old bugs (e.g.
  `regression_every_registered_tool_maps_to_action`, `task_manager_lifecycle`,
  `regression_all_tasks_dir_json_loads`).
- Mock container integration tests (`craft.rs::tests`) validate `do_smelt` and
  `do_craft_3x3` state machines without needing a Minecraft server.
- 运维脚本见 `craft-agent-ctl`（`status`/`stop`/`build`/`viewer`/`goal`/`session`/`tail`）；
  原 `auto_diag.ps1` / `verify_build.ps1` / `scan_run.ps1` 为 legacy PowerShell 诊断流程，
  现由 ctl + autopilot 取代。

See [`docs/mindcraft-gap.md`](./docs/mindcraft-gap.md) for the automation workflow records.
See [`docs/tutorials/`](./docs/tutorials/) for developer guides.
