# Session and Compaction

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
