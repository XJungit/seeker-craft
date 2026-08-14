# Troubleshooting

Common build, install, runtime, and configuration problems and fixes (v1.0 · DSH bridge mode).

## Install / Clone Issues

| Symptom | Likely Cause | Fix |
|---|---|---|
| `git clone --recurse-submodules` fails on submodule | `.gitmodules` URL or gitlink mismatch | Confirm `vendor/azalea` resolves to `https://github.com/XJungit/azalea.git`; run `git submodule update --init --recursive` |
| `cargo build` fails with edition 2024 error | Rust toolchain < nightly-2026-07-21 | `rustup toolchain install nightly-2026-07-21` (pinned in `rust-toolchain.toml`; stable fails) |
| `azalea-bot` feature not found | Feature flag missing | Add `--features azalea-bot` |
| Cargo fetches azalea from `file:///...` | Local `.cargo/config.toml` patch left in a committed lock | Rebuild the lock from the https source: temporarily move `.cargo/config.toml`, `cargo update -p azalea`, verify `Cargo.lock` shows `git+https://github.com/XJungit/azalea`, restore the patch (see ARCHITECTURE.md → azalea fork maintenance) |

## Build Issues

| Symptom | Likely Cause | Fix |
|---|---|---|
| `cannot find trait LlmProvider` | Wrong path in nested `mod tests` | Use `use crate::agent::LlmProvider` (absolute path) — legacy, most such code was removed with the in-bot loop |
| azalea connect fails | MC server not running / wrong address | Start a vanilla 26.2 server and verify `localhost:4444` reachable |
| cargo git cache stale after vendor change | azalea rev bumped but cache not cleared | `Remove-Item -Recurse "$env:USERPROFILE\.cargo\git\checkouts" -Force` |

## Setup / DSH Issues

| Symptom | Likely Cause | Fix |
|---|---|---|
| `setup.ps1` reports missing deps | cargo/git/node/pnpm not installed | Install them (see getting-started.md prerequisites) and re-run |
| DSH tools (`game_state`/`bot_tool`/`set_goal`) not appearing | Plugin not registered / preset not generated | Re-run `.\scripts\setup.ps1` (regenerates preset + verifies plugin); confirm DSH is installed first |
| `setup.ps1` can't find `@deepseek-ai` deps | DSH not run once yet (deps not installed) | Run DSH once to install its dependencies, then re-run `setup.ps1`; or set `DSH_NPX_ROOT` |
| Viewer API not ready after `start.ps1` | Viewer failed to start or still compiling | Check `%TEMP%\opencode\viewer_run.log` / `viewer_run.err.log` (override dir with `SEEKER_LOG_DIR`) |

## Runtime Issues

| Symptom | Likely Cause | Fix |
|---|---|---|
| Bot stalls (repeated tool failures) | Tool params wrong or game state unexpected | Check viewer session logs / `craft-agent-ctl session` for error details |
| Perception stale | azalea connection lost | Verify MC running and bot joined; check server address/port |
| Bot can't join MC | Server version ≠ 26.2 or wrong address | Confirm `-Mc` matches your server (default `localhost:4444`) |

## Bot Behavior Issues (tool layer)

| Symptom | Likely Cause | Fix |
|---|---|---|
| `gather` returns partial (e.g. 14/16) | Resource depleted in area (expected) | LLM should gather elsewhere or use what it has — partial success is `Ok` |
| `mine` reports air at target | Air target auto-corrected to nearest solid (P101) | Read the correction notice in the tool result; the actual target was mined |
| `till_and_sow` reports correction | Non-tillable target auto-corrected (P102) | Read the correction notice; till+sow proceeded on the corrected block |
| Bot self-attacks | `self_defense` needs ≤4 blocks + `!is_busy()` | Set posture via `set_mode` if it's not appropriate |
| Bot mines to bedrock | `mine_below` has no Y check | It auto-stops at the bedrock layer (Y≈-59..-58) |
| Craft fails "furnace 无工作台" | Tool-block auto-craft forbidden (P9.2) | LLM must `craft_3x3('furnace')` manually with a placed crafting table |

## Connection Issues

- **Azalea bot**: Start the MC server *before* running `start.ps1`. The bot joins as a player on `localhost:4444`.
- **Bot stuck in place**: Check if the `goto` target is >32m away (max distance limit) or pathfinding timed out.

## Performance

- **High token cost**: The DSH brain manages context/compaction now — tune in DSH, not here.
- **High latency per action**: Tool-layer slow actions (goto/mine) are async by nature; probe mode verifies them in seconds without the LLM.

## Logs

- `RUST_LOG=debug` for detailed bot/tool logs (azalea pathfinder included).
- Viewer/autopilot logs: `%TEMP%\opencode\viewer_run.log` / `.err` (override with `SEEKER_LOG_DIR`).
- Session JSONL (`sessions/mc_run.jsonl`) is the source of truth for replay and debugging.
- `craft-agent-ctl session N` / `craft-agent-ctl tail <log> <N>` / `craft-agent-ctl status` for ops.
