# Agent Loop

This guide explains the main agent loop and how a turn is executed.

## Turn Flow (13 steps)

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
Step 12: parallel tool execution     — group by side effects, parallel within batch
Step 13: skill extraction            — non-obs tool calls extracted as experience
```

**Nudge injection rule**: nudges must be injected AFTER all tool results, never
between `assistant(tool_calls)` and `tool` messages (triggers DeepSeek/OpenAI 400).

## Step Details

### Step 2: Compaction
When estimated context exceeds `context_window - reserve`, compaction triggers:
keep `keep_recent` tokens of recent messages intact, summarize older messages
via an LLM call, replace them with a `CompactionEntry` + summary.

### Step 3: Volatile Injection Cleanup
Three types of volatile user messages from the previous turn are removed:
- `【当前游戏状态（自动注入）】` (auto_perceive snapshot)
- `[当前目标]` (SelfPrompter injection)
- `[MODE: ...]` (modes reaction)

This keeps the message stream clean and prevents stale state from accumulating.

### Step 4: auto_perceive
Injects a structured snapshot of the current game state as a user message:
position, health, hunger, biome, nearby blocks, inventory (aggregated by item ID).

### Step 5: Modes Reaction
The Agent layer checks the perceive text against 10 modes
(self_preservation / self_defense / unstuck / cowardice / hunting /
item_collecting / torch_placing / elbow_room / idle_staring / cheat).
Matching modes inject `[MODE: ...]` prompts. Same `mode_id` consecutive triggers
inject only once. Priority: `self_preservation(1) > self_defense(2) > unstuck(3) > ...`.

### Step 9: LLM Call
Calls the active LLM backend with the assembled context and tool definitions.
`RetryConfig` handles transient failures with exponential backoff.

### Step 10: Plain-Text Reply Detection (P56)
If the LLM returns a text-only reply without `tool_calls`, and the text contains
premature-completion keywords (✅, 任务完成, 已验证, 最终确认, etc.), a continue
nudge is injected forcing the LLM to keep producing tool calls.

### Step 11: Dead-Loop Detection
If `recent_calls` shows 4+ identical call signatures, a nudge is injected
forcing the LLM to try a different approach.

### Step 12: Parallel Tool Execution
Tools are grouped by `ToolEffects`:
- READ tools run in the same batch (parallel)
- NETWORK + READ run in the same batch (parallel)
- WRITE / APPEND / PROCESS each get their own batch (BARRIER splits)

### Step 13: Skill Extraction
Non-`perceive` tool calls are extracted as skill examples for future few-shot
prompting. Skills are persisted to the skill library.

## Extension Points

- `queue_steering()` injects follow-up instructions.
- `queue_follow_up()` queues follow-up turns.
- `abort()` stops retry loops.

## P58: set_goal("") Bypass Intercept

When the LLM calls `set_goal(goal="")` to clear the goal while its text declares
"task complete ✅", the agent:
1. Refuses to call `stop_goal()` (preserves the original goal)
2. Increments `fake_completion_count`
3. Injects a mandatory perceive-verification nudge

This closes a bypass of P56 where the LLM used `set_goal("")` to "complete"
a goal without actually verifying it via `perceive`.

## Observability

- `AgentEvent` emits start/update/end events for tool execution.
- `Usage` tracks LLM token usage.
- Session JSONL records every message, tool call, and compaction entry.
- `RUST_LOG=debug` enables detailed agent logs; `RUST_LOG=trace` logs raw
  LLM request/response bodies.
