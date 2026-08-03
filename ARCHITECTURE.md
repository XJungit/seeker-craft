# Craft-Agent Architecture

This document explains the high-level architecture of Craft-Agent and how the
main crates interact at runtime.

## Layered Overview

```text
app / viewer / examples
        |
        v
  craft-agent-minecraft  ──→  azalea (vendor)  ──→  MC server (TCP)
        |
        v
    craft-agent
        |
        v
   external LLM/VLM services
```

- [`craft-agent`](./crates/craft-agent/) — generic game-agent runtime: Agent main loop,
  tool registry, session, compaction, prompt assembly, modes, WorldMemory.
- [`craft-agent-minecraft`](./crates/craft-agent-minecraft/) — Minecraft-specific
  adapter and tool set built on the azalea protocol layer.
- [`craft-agent-model`](./crates/craft-agent-model/) — LLM/VLM clients and config model.
- [`craft-agent-viewer`](./crates/craft-agent-viewer/) — Axum + SSE Web dashboard.

## Runtime Flow

### Agent Main Loop (13 steps per turn)

File: `crates/craft-agent/src/agent/mod.rs::run_one_turn`

```
Step  1: drain_queues()              — steering/follow_up queues
Step  2: 压缩检查                     — msg.len≥10000 or token over budget → compact()
Step  3: 易变注入清理                 — remove last turn's 3 volatile user messages
Step  4: auto_perceive               — inject structured state snapshot
Step  5: modes reaction              — check_modes() → [MODE: ...] prompt
Step  6: SelfPrompter                — re-inject [当前目标]
Step  7: dynamic context             — WorldInfo + Skill + Few-shot + obs warning
Step  8: WorldMemory nearby          — render 64-block radius around __self__
Step  9: LLM complete                — with RetryConfig backoff
Step 10: plain-text reply detection  — inject continue nudge
Step 11: dead-loop detection         — 4+ repeat signatures → nudge
Step 12: execute_batch              — group by side effects: READ parallel, WRITE serial;
                                       slow tools (is_slow) run alone then abort the rest;
                                       failures abort the batch (finalize_abort → Reroute/Handoff)
Step 13: skill extraction            — non-obs tool calls extracted as experience
```

**Nudge injection rule**: nudges must be injected AFTER all tool results, never
between `assistant(tool_calls)` and `tool` messages (triggers DeepSeek/OpenAI 400).

**Batch execution (P2.1)**: `run_one_turn` delegates to `execute_batch` +
`finalize_abort`. `AbortDecision::{Reroute, Handoff}` unifies P89 (WRITE failure
→ re-plan same turn), P90 (steering interrupt → re-plan), P94 (tool budget 20 →
convergence nudge), P99 (slow tool finished → hand off, no re-call).

### Two-Layer Modes Reaction System

```
Agent Layer (modes.rs)
  └── checks perceive text each turn, injects [MODE: ...] prompt to LLM
  └── 10 modes: self_preservation / self_defense / unstuck /
       cowardice / hunting / item_collecting / torch_placing /
       elbow_room / idle_staring / cheat
  └── dedup: same mode_id consecutive triggers inject only once
  └── priority: self_preservation(1) > self_defense(2) > unstuck(3) > ...

Handler Layer (azalea/handler.rs Tick)
  └── directly executes actions without LLM involvement
  └── self_preservation: fire/lava → auto Goto escape (every tick)
  └── self_defense: hostile mob ≤4 blocks + !is_busy() → auto attack (every 100 tick)
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
  - `ToolEffects`: bit flags for batching (READ same batch, NETWORK+READ same batch,
    WRITE/APPEND/PROCESS each own batch — BARRIER splits).
- `SessionEntry`: tool call, state snapshot, compaction metadata.
- `AgentEvent`: external hook for viewers and controllers.
- `WorldMemory`: spatial memory keyed by `MemoryPos`, chunk-indexed for O(1) nearby
  queries, 6 types (Resource/Structure/Container/Entity/Hazard/Portal/Note),
  TTL 30s dedup, 64-block radius rendered each turn.

## 44 LLM Tools

| Category | Tools | Side Effect |
|---|---|---|
| Perception | `perceive` | READ |
| Memory | `memory` (save/anchor/query/forget) | READ/WRITE |
| Movement | `goto` / `mine_below` / `mine_above` / `pickup` / `follow` / `stop_follow` | WRITE |
| Mining | `mine` / `make_obsidian` | WRITE |
| Block Interaction | `interact_block` | WRITE |
| Combat | `attack` / `defend` | WRITE |
| Crafting | `craft` (2×2) / `craft_3x3` | WRITE |
| Smelting | `smelt` | WRITE (wait) |
| Auto-craft | `auto_craft` | WRITE (recursive) |
| Enchanting | `enchant` | WRITE |
| Gathering | `gather` | WRITE (pathfinding) |
| Placing | `place` | WRITE |
| Container | `open` / `chest_view` / `chest_withdraw` / `chest_deposit` | READ/WRITE |
| Equipment | `equip` / `discard` | WRITE |
| Consumption | `consume` | WRITE (long press) |
| Entity | `interact_entity` | WRITE |
| Trading | `trade` | WRITE |
| Chat | `chat` | NETWORK |
| Goal | `set_goal` / `pause_goal` / `resume_goal` | WRITE |
| Building | `build` / `build_blueprint` / `list_blueprints` | WRITE |
| Composite | `run_plan` | WRITE |
| Scripting | `run_script` | WRITE (rhai) |
| Custom Action | `new_action` / `list_actions` | WRITE (persist) |
| Knowledge | `search_wiki` | NETWORK |
| Social | `give` | WRITE |
| Task chain | `task_complete` / `task_retry` | WRITE |

## Critical Constraints

### System Prompt Byte Stability

DeepSeek prefix cache requires the system prompt to be byte-identical across
every API call. Regression test: `regression_system_prompt_byte_stable_across_obs_streak`.

- ❌ Never embed dynamic variables (obs_streak, knowledge_bootstrapped) in system prompt
- ✅ Dynamic content goes into `build_dynamic_instructions_msg()` as user message
- ✅ Perceived state goes as `【当前游戏状态（自动注入）】` user message

### Mindcraft Philosophy Alignment

Bot tools only do what they can do; if they can't, return `Err` and let the LLM
decide. See [`AGENTS.md`](./AGENTS.md) section 9-bis for full rules.

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

## P56-P58: Plain-Text Reply Governance

LLM may declare "task complete ✅" in plain text without tool calls, wasting turns.

- **P56**: `is_premature_completion` in `agent/mod.rs` detects 9+ keywords
  (✅, 任务完成, 已验证, 最终确认, etc.) and injects continue nudge.
- **P58**: When LLM calls `set_goal("")` to clear goal + text declares completion,
  agent refuses `stop_goal()` and injects mandatory perceive verification nudge.

## Recipe System (Dual Layer)

- `recipes.rs`: handwritten static recipe graph, drives `auto_craft` recursive
  material satisfaction.
- `recipe_book.rs` + `builtin_recipes.json`: vanilla 26.2 full recipe book (P48).
- `craft_3x3` lookup order: RecipeBook → handwritten SHAPED_RECIPES → Err.

## Test Infrastructure

```bash
cargo test --workspace --no-fail-fast        # full 234 tests
cargo test -p craft-agent --lib              # 122 core tests
cargo test -p craft-agent-minecraft --features azalea-bot --lib  # 118 adapter tests
cargo test -p craft-agent-model --lib        # 23 model tests
```

- All `regression_*` tests guard against reverting to old bugs.
- Mock container integration tests (`craft.rs::tests`) validate `do_smelt` and
  `do_craft_3x3` state machines without needing a Minecraft server.
- PowerShell scripts (`auto_diag.ps1` / `verify_build.ps1` / `scan_run.ps1`)
  drive the full automation loop: build → test → LLM e2e → analyze → fix → rerun.

See [`AGENTS.md`](./AGENTS.md) for the full automation workflow manual.
See [`docs/tutorials/`](./docs/tutorials/) for developer guides.
