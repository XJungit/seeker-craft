# Troubleshooting

Common build, runtime, and configuration problems and fixes.

## Build Issues

| Symptom | Likely Cause | Fix |
|---|---|---|
| `cargo build` fails with edition 2024 error | Rust toolchain < 1.85 | `rustup update stable` |
| `mod-bridge` feature not found | Feature flag missing | Add `--features mod-bridge` |
| `real` feature fails on Windows | `windows-sys` crate issue | Ensure latest Rust; open issue if persists |
| Gradle build fails with mapping errors | MC version mismatch | Check AGENTS.md for 26.2 API signatures |
| Gradle build SSL/cert error | Network/proxy blocking Maven | See `mods/craft-agent-bridge/tools/README.md` |

## Runtime Issues

| Symptom | Likely Cause | Fix |
|---|---|---|
| Agent stalls (repeated tool failures) | Tool params wrong or game state unexpected | Check session logs for error details |
| Perception stale | Mod-bridge connection lost | Verify MC running with mod loaded; check port 25567 |
| LLM timeouts | Backend unreachable or slow | Check `config/agent.toml` endpoint and timeout_secs |
| LLM returns empty response | Finish reason caught as length | Provider handles this with retry; check `max_tokens` |
| Agent loops same action 10+ times | Self-prompt/obs_streak not triggering | Enable `enable_self_prompt` or check modes config |
| Compaction triggered every turn | `estimate_tokens` inflated by cache | Empty `usage` reset handles this; verify in logs |

## Connection Issues

- **Mod-bridge**: MC must be running *before* starting the agent. Verify `CraftAgentBridge` mod is in `mods/` folder.
- **Mod-bridge port conflict**: Change port in mod Java source and agent `--port` arg.
- **Real path**: Ensure no other application is capturing the mouse/keyboard.

## Performance

- **High token cost**: Enable compaction; prefer `mod-bridge` over `real`.
- **High latency per turn**: Reduce `context_window` or switch to V4 Flash.
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
