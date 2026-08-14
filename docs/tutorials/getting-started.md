# Getting Started (v1.0 · DSH bridge mode)

How to clone, build, and run SeekerCraft (Craft-Agent) v1.0 from scratch.
Verified on Windows PowerShell. The brain is **DeepSeek Harness (DSH)** — the viewer is
only an HTTP bridge.

## 1. Prerequisites

| Dependency | How to install / notes |
|---|---|
| Rust nightly | `rustup toolchain install nightly-2026-07-21` (see `rust-toolchain.toml`; stable fails) |
| Git | https://git-scm.com |
| Node.js ≥ 20 + pnpm | https://nodejs.org, then `npm install -g pnpm` |
| Minecraft Java 26.2 server | your own vanilla server; bot connects to `localhost:4444` by default |
| DeepSeek Harness (DSH) | your own install — https://github.com/deepseek-ai/deepseek-harness (this repo does not bundle it) |

## 2. Clone (with submodule)

```bash
git clone --recurse-submodules https://github.com/XJungit/seeker-craft.git
cd seeker-craft
```

> The azalea dependency is the maintained fork `XJungit/azalea` (https source + pinned rev).
> No local cargo patch is needed to build.

## 3. Install & configure (one shot)

```powershell
.\scripts\setup.ps1
```

This builds the workspace, registers the DSH bridge plugin, generates the `craft-bot`
preset, copies `.env.example` → `.env`, and verifies the plugin. Idempotent.

## 4. Start

```powershell
# 1) start your MC 26.2 server on localhost:4444
# 2) one-shot start (viewer + connect bot)
.\scripts\start.ps1
```

Or manually:

```powershell
cargo run -p craft-agent-ctl -- viewer "explore the world" 0   # viewer only
cargo run -p craft-agent-ctl -- start                          # connect bot
cargo run -p craft-agent-ctl -- status                         # verify running=true
```

## 5. Drive from DSH

1. Open DSH, create/enter a **craft-bot** preset session.
2. Use the three bridge tools:

```
game_state()                                   # perceive live state
bot_tool(name:"craft", args:{item:"stone_pickaxe"})   # run one of the 53 tools
set_goal("Collect 24 iron ore and smelt into ingots") # set the ops goal
```

3. Stop with `.\scripts\stop.ps1`.

## Build & test

```bash
cargo build --workspace
cargo test -p craft-agent --lib
cargo test -p craft-agent-minecraft --features azalea-bot --lib
```

## Probe mode (tool layer without the LLM)

```bash
cargo run -p craft-agent-minecraft --example azalea_probe --features azalea-bot -- 4444 --cmd "equip iron_helmet helmet"
cargo run -p craft-agent-minecraft --example azalea_probe --features azalea-bot -- 4444 --script scripts\probe\smoke.json
```

## Debug

Set `RUST_LOG=debug` for verbose output (azalea pathfinder logs included).
Session logs are written to `sessions/mc_run.jsonl` (viewer runtime data, gitignored).
Viewer/autopilot logs: `%TEMP%\opencode\viewer_run.log` (override with `SEEKER_LOG_DIR`).

## Troubleshooting

See [`troubleshooting.md`](troubleshooting.md). Common issues:

- **Viewer API not ready** — check `%TEMP%\opencode\viewer_run.log` / `viewer_run.err.log`.
- **Bot can't join** — confirm the MC server version is 26.2 and listening on the address in `-Mc`.
- **DSH tools not appearing** — re-run `setup.ps1` (it regenerates the preset and verifies the plugin).
