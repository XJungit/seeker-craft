# Troubleshooting

Common build, runtime, and configuration problems and fixes.

## Build Issues

| Symptom | Likely Cause | Fix |
|---|---|---|
| `cargo build` fails with edition 2024 error | Rust toolchain < 1.97.1 | `rustup update` to the pinned nightly (see `rust-toolchain.toml`) |
| `azalea-bot` feature not found | Feature flag missing | Add `--features azalea-bot` |
| azalea connect fails | MC server not running / wrong address | Start a vanilla 26.2 server and verify `localhost:4444` reachable |
| `cannot find trait LlmProvider` | Wrong path in nested `mod tests` | Use `use crate::agent::LlmProvider` (absolute path) |
| `method parallel_safe not found` | `ToolEffects::BARRIER` is `u8` const | Use `ToolEffects { bits: ToolEffects::BARRIER }` |
| cargo git cache stale after vendor change | azalea rev bumped but cache not cleared | `Remove-Item -Recurse "$env:USERPROFILE\.cargo\git\{db,checkouts}\azalea-*"` |

## Runtime Issues

| Symptom | Likely Cause | Fix |
|---|---|---|
| Agent stalls (repeated tool failures) | Tool params wrong or game state unexpected | Check session logs for error details |
| Perception stale | azalea connection lost | Verify MC running and bot joined; check server address/port |
| LLM timeouts | Backend unreachable or slow | Check `config/agent.toml` endpoint and `timeout_secs` |
| LLM returns empty response | Finish reason caught as length | Provider handles this with retry; check `max_tokens` |
| Agent loops same action 4+ times | Dead-loop detection triggers | Check `recent_calls` signatures; nudge injected automatically |
| Compaction triggered every turn | `estimate_tokens` inflated by cache | Empty `usage` reset handles this; verify in logs |

## Bot Behavior Issues (P55-P58)

| Symptom | Likely Cause | Fix |
|---|---|---|
| `gather` returns 14/16 partial | Resource depleted in area (P55 expected) | LLM should gather elsewhere or use what it has — partial success is `Ok` |
| LLM declares "任务完成 ✅" without tool calls | P56 premature completion | Check `is_premature_completion` keyword match in logs; nudge injected automatically |
| LLM calls `set_goal("")` to bypass P56 | P58 intercept triggers | Check `fake_completion_count` in logs; mandatory perceive nudge injected |
| `smelt(count=15)` times out at 120s | P57 batch limit (8 items/batch) | LLM should call `smelt` again for the remaining items — return message guides this |
| `gather` 100% failure rate | Returning `Err` for partial success (pre-P55) | Update to P55+ where partial success returns `Ok` |
| LLM outputs `【工具调用】goto(...)` text pseudo-call | Tool name alias / `fold_tool_history` | Use actual tool names (`go` not `goto`); `fold_tool_history` was removed |
| Bot self-attacks | `self_defense` no distance check | Now requires ≤4 blocks + `!is_busy()` |
| Bot mines to bedrock | `mine_below` no Y check | Auto-stops at Y≤-61 (deepslate layer) |
| Craft fails with "furnace 无工作台" | Tool block auto-craft forbidden (P9.2) | LLM must `craft_3x3('furnace')` manually with a placed crafting table |

## Connection Issues

- **Azalea bot**: Start the MC server *before* running the agent. The bot joins as a player on `localhost:4444` (configurable in `adapter_azalea.rs`).
- **Bot stuck in place**: Check if `go` target is >32m away (max distance limit) or pathfinding timed out (3s = 60 ticks).

## Performance

- **High token cost**: Enable compaction (default on).
- **High latency per turn**: Reduce `context_window` or switch to a faster model.
- **Large session files**: Reduce `keep_recent` or compact more aggressively.
- **Server TPS drops**: Bot `go` actions are capped at 32m / 3s to prevent TPS lag.

## DeepSeek Cache Debugging

Check cache hit rate in API response (`usage.prompt_cache_hit_tokens`):
- **Low rate**: System prompt likely changing between turns (check jailbreak/dynamic content).
- **Rate drops after compaction**: Expected — next turn after summary is partial miss, recovers in 2-3 turns.
- **Rate stays low**: Something is changing in the prefix every turn. Verify with `regression_system_prompt_byte_stable_across_obs_streak` test.

## Git Safety (2026-07-26 incident)

Never run destructive git commands on this repo — many fixes live as uncommitted
working-tree changes. See `AGENTS.md` section 8.1 for the full rules:

- ❌ `git checkout -- <file>` (silently overwrites working tree)
- ❌ `git checkout .` / `git restore <file>` / `git reset --hard` / `git clean -fd`
- ✅ To revert your own edit: use `Edit` tool to reverse the change
- ✅ Before risky experiments: `git add -A && git commit --no-verify -m "wip: checkpoint"`

## Logs

- `RUST_LOG=debug` for detailed agent and tool logs.
- `RUST_LOG=trace` for raw LLM request/response bodies.
- Session JSONL is the source of truth for replay and debugging.
- `tools/scan_run.ps1` analyzes a session file and prints tool call statistics.
- `tools/auto_diag.ps1` runs the full automation loop: build → test → e2e → analyze.
