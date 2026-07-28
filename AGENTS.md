# Craft-Agent

An LLM-driven Minecraft bot using the Azalea Rust client protocol.

## Build & Run

```bash
# Build
cargo build --workspace
cargo build -p craft-agent-minecraft  # MC adapter (needs azalea-bot feature)

# Run viewer (Web UI at http://127.0.0.1:8080)
# Kill old process first! Old binary keeps holding port 8080.
cargo run -p craft-agent-viewer

# Test
cargo test -p craft-agent --lib
cargo test -p craft-agent-minecraft --features azalea-bot --lib
cargo test regression_system_prompt_byte_stable  # After changing prompt/system prompt
```

## Architecture (4 crates)

```
craft-agent              Core agent framework
  agent/                 run_one_turn, compaction, prompt, modes, session
  core/                  types, GameTool trait, ToolRegistry, memory, skill

craft-agent-minecraft    MC adapter (azalea protocol)
  azalea/mod.rs          AzaleaBot + tick handler (command queue, modes)
  adapter_azalea.rs      GameAdapter impl, perceive, execute
  tools_azalea.rs        20 GameTool impls (goto, mine, gather, craft, etc.)

craft-agent-model        LLM client (OpenAI-compatible)
  decision.rs            chat_tools(), fold_tool_history(), parse_chat_tools_response()

craft-agent-viewer       Web dashboard (Axum + SSE)
  agent_loop.rs          Main loop: agent.step() → chat drain → idle loop
```

## Agent Loop (run_one_turn, 13 steps)

1. `drain_queues()` — steering/follow_up messages
2. Compaction — messages ≥ 10000 or token over budget → compact()
3. Strip ephemeral messages — perceive, memory, goal from last turn
4. Auto-perceive — inject structured state snapshot
5. Modes — check_modes() → [MODE: ...] text hint
6. SelfPrompter — inject [当前目标]
7. Dynamic context — WorldInfo + Skill + Few-shot + obs warnings
8. WorldMemory — render nearby memories (radius 64)
9. LLM call — with retry (exponential backoff)
10. Text-only check — if no tool_calls, inject nudge + return (continue)
11. Dead-loop check — 4+ same normalized signature → nudge
12. Execute tools — grouped by effect (READ parallel, WRITE sequential)
13. Skill extraction — learn from non-obs tool calls

**Nudge rule:** Must inject AFTER all tool results, never between `assistant(tool_calls)` and `tool` (triggers 400).

## System Prompt Must Be Byte-Stable

DeepSeek prefix cache requires system prompt to be identical every call.
- **Static content** → system prompt (identity, role_desc, jailbreak, knowledge)
- **Dynamic content** → user messages, injected each turn and stripped the next:
  - `【当前游戏状态（自动注入）】` — perceive state
  - `【邻近世界记忆】` — WorldMemory nearby
  - `[当前目标]` — self_prompt goal
  - `【参考示例】` — few-shot examples
  - `【工具执行】...` — folded tool history (from `fold_tool_history`)

**Regression test:** `regression_system_prompt_byte_stable_across_obs_streak`

## Tool Calling

- All tools use OpenAI function calling (JSON `tool_calls`), never text commands
- `fold_tool_history` in `decision.rs` strips tool_calls from history for DeepSeek compat
  - Converts `tool_calls` + `role:tool` messages to `【工具执行】name(args) → result` text
  - This format was recently changed from `[工具 ...]` to stop teaching the LLM to write pseudo-calls
- Tools return sync results — `result_tx` channel bridges tick handler to tool thread
- 20 tools total: perceive, goto, mine, mine_below, interact_block, attack, craft, craft_3x3, smelt, gather, place, open, auto_craft, enchant, trade, interact_entity, chat, memory, set_goal, run_plan, run_script, build, search_wiki

## Code Execution

- **`run_script`** → rhai engine (embedded, Rust-native, no subprocess)
  - Functions: goto, mine, mine_below, gather, craft, place, open, chat, attack, smelt, interact, sleep, print
  - Supports variables, loops, conditionals
  - Safer than Mindcraft's JS sandbox — no arbitrary code execution
- **`run_plan`** → JSON multi-step plan (sequential, synchronous)
- Replaced Node.js subprocess approach (was too slow, required file I/O)

## Chat System

- Player messages in Minecraft chat → `Event::Chat` → `BotEvent::Chat` → `chat_queue`
- Agent loop drains `chat_queue` before each step via `queue_steering("玩家说: ...")`
- Bot replies via `chat(msg)` tool → `bot.chat(&content)` → appears in Minecraft chat
- `adapter_azalea.rs::drain_chat()` for agent loop to consume

## Modes (Reactive, Tick-Level)

Executed in `azalea/mod.rs` `Event::Tick` handler, independent of LLM:
- **self_preservation:** Fire/lava under bot → auto push Goto to escape
- **self_defense:** Hostile mobs nearby → attack every 100 ticks, check entity exists first
- Both are fire-and-forget (push to cmd_queue with no result_tx)

## WorldMemory

- Spatial memory: Resource, Structure, Container, Entity, Hazard, Portal, Note
- Chunk-indexed (`chunk_key`) for O(1) nearby queries
- `record_surroundings` scans radius 8, TTL 30s, every 20 ticks
- Agent renders radius 64 around `__self__` anchor each turn

## Critical Constraints

### Don't modify vendor/azalea
- `vendor/azalea` is a separate git repo + workspace
- All changes must use azalea's public API only
- To modify: commit in vendor → update SHA in both `.cargo/config.toml` [patch] and `Cargo.toml`
- Prefer solving in upper layers first

### Cargo network
- Uses `rsproxy.cn` mirror (`.cargo/config.toml`). If down, temporarily rename config file to use crates.io directly.
- azalea is a git dependency with local vendor patch — offline builds work if cache is populated.

### Git safety
- ❌ Never `git checkout -- <file>`, `git checkout .`, `git restore`, `git reset --hard`, `git clean -fd`
- ❌ Never `git stash` without immediate pop plan
- ✅ Use SearchReplace to revert code (visible, line-by-line)
- ✅ `git add -A && git commit --no-verify -m "wip: checkpoint"` before risky experiments

### Perceive format
- `adapter_azalea.rs` builds `scene` string with multi-line format:
  ```
  位置: (-489, 86, -169)
  生命: 20/20  饱食: 20/20  主手: dirt
  ...
  ```
- Block names use `BlockKind` (not `BlockState` Debug format)
- Stuck detection: 15+ ticks of no position change → `⚠ 卡住!`
- 10x10 scan filters to show only useful blocks (ore, wood, etc.)

## Common Build Errors

| Error | Fix |
|-------|-----|
| `cannot find trait LlmProvider` | `use crate::agent::LlmProvider` |
| `cannot find module azalea_block` | Use `azalea::block::BlockState` |
| `mismatched closing delimiter` | Check for duplicate code blocks from merge errors |
| `failed to get anyhow as dependency` | rsproxy.cn down, rename `.cargo/config.toml` temporarily |
| `tried to attack entity which isn't in EntityIdIndex` | Self-defense mode needs entity existence check |