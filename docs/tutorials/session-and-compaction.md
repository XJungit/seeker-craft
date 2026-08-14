# Session and Compaction

> **现状说明**：本文档描述 **in-bot 循环时代**（`Session` / `CompactionConfig` /
> P56/P58 nudge 注入）的会话持久化与上下文压缩。DSH 桥接模式（2026-08-14 起）下，
> 会话与上下文管理由 **DSH 大脑**负责；Rust 侧仅保留 `sessions/mc_run.jsonl` 的
> JSONL 归档（viewer `/api/session` 只读展示）。本页保留为历史设计参考，
> 不要据此修改代码。

This guide explains session persistence, context compaction, and cache optimization.

## Session

`Session` stores message history in JSONL format:
- User/Assistant/ToolResult messages
- Compaction entries with summary text
- Entry IDs for precise truncation/summary references

```rust
let mut sess = Session::open("sessions/mc_run.jsonl")?;
// ... run agent ...
sess.save_to("sessions/mc_run.jsonl")?;
```

Always set `--session` for long runs — enables resume and replay.

## Compaction

When the estimated context exceeds `context_window - reserve`, compaction triggers:

1. Keep `keep_recent` tokens of the latest messages intact.
2. Send older messages to the LLM for summarization.
3. Replace summarized messages with a `CompactionEntry` + summary.
4. Reset token estimates to prevent re-triggering.

```rust
let compaction = CompactionConfig {
    context_window: 500_000,
    reserve: 100_000,      // 20%
    keep_recent: 300_000,  // 60%
};
```

## Cache Impact

Compaction uses **summarization** (not truncation): old messages are rewritten as a new summary text.

This **breaks DeepSeek's prefix cache** for the next API call — the byte stream changes at the summary point. Cache recovery takes 1-2 turns as the new prefix stabilizes.

For optimal DeepSeek cache hit rates:
- Keep system prompt **byte-identical** across all turns (no embedded variables).
- Prefer fewer, larger turns over many small turns.
- Compaction frequency naturally decreases with larger `context_window`.
- See `docs/tutorials/configuration.md` for DeepSeek-specific tuning.

## Tips

- Inspect `session_entries` for tool call history and compaction boundaries.
- Use `Agent::abort()` to stop retries on unrecoverable errors.
- Session files are plain JSONL — greppable and diffable.

## P56/P58: Plain-Text Reply & set_goal("") Bypass

The session records two special nudge injections:

- **P56 nudge** (`[tN] 注入续跑 nudge`): triggered when the LLM returns a
  text-only reply (no `tool_calls`) containing premature-completion keywords
  (✅, 任务完成, 已验证, 最终确认, smelt/craft/gather/mine 任务, etc.).
  The nudge forces the LLM to keep producing tool calls.
- **P58 nudge** (`[tN] P58 拦截: set_goal("") + 文字宣告完成`): triggered when
  the LLM calls `set_goal(goal="")` while its text declares "task complete ✅".
  The agent refuses `stop_goal()` and injects a mandatory perceive-verification
  nudge. `fake_completion_count` increments each time this fires.

These nudges appear in the JSONL as user messages from the agent itself
(content starts with `【续跑】` or `【P58 拦截】`). When replaying a session,
they help identify where the LLM tried to "give up" mid-task.
