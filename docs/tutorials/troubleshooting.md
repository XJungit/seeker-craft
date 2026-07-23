# Troubleshooting

Common build, runtime, and configuration problems and fixes.

## Build Issues

| Symptom | Likely Cause | Fix |
|---|---|---|
| `cargo build` fails with edition 2024 error | Rust toolchain < 1.97.1 | `rustup update` to the pinned nightly (see `rust-toolchain.toml`) |
| `azalea-bot` feature not found | Feature flag missing | Add `--features azalea-bot` |
| azalea connect fails | MC server not running / wrong address | Start a vanilla 26.2 server and verify `localhost:4444` reachable |

## Runtime Issues

| Symptom | Likely Cause | Fix |
|---|---|---|
| Agent stalls (repeated tool failures) | Tool params wrong or game state unexpected | Check session logs for error details |
| Perception stale | azalea connection lost | Verify MC running and bot joined; check server address/port |
| LLM timeouts | Backend unreachable or slow | Check `config/agent.toml` endpoint and timeout_secs |
| LLM returns empty response | Finish reason caught as length | Provider handles this with retry; check `max_tokens` |
| Agent loops same action 10+ times | Self-prompt/obs_streak not triggering | Enable `enable_self_prompt` or check modes config |
| Compaction triggered every turn | `estimate_tokens` inflated by cache | Empty `usage` reset handles this; verify in logs |

## Connection Issues

- **Azalea bot**: Start the MC server *before* running the agent. The bot joins as a player on `localhost:4444` (configurable in `adapter_azalea.rs`).

## Performance

- **High token cost**: Enable compaction (default on). 
- **High latency per turn**: Reduce `context_window` or switch to a faster model.
- **Large session files**: Reduce `keep_recent` or compact more aggressively.

## DeepSeek Cache Debugging

Check cache hit rate in API response (`usage.prompt_cache_hit_tokens`):
- **Low rate**: System prompt likely changing between turns (check jailbreak/dynamic content).
- **Rate drops after compaction**: Expected — next turn after summary is partial miss, recovers in 2-3 turns.
- **Rate stays low**: Something is changing in the prefix every turn.

## Logs

- `RUST_LOG=debug` for detailed agent and tool logs.
- `RUST_LOG=trace` for raw LLM request/response bodies.
- Session JSONL is the source of truth for replay and debugging.
