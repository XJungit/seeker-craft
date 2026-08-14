# Getting Started (v1.0 · DSH bridge mode)

How to clone, build, and run SeekerCraft (Craft-Agent) v1.0 from scratch.
Verified on Windows PowerShell. The brain is **DeepSeek Harness (DSH)** — the viewer is
only an HTTP bridge.

## Architecture in one paragraph

```
DSH (DeepSeek Harness)  ←  the LLM brain (you install this yourself)
   │  via the dsh-bridge plugin (shipped in tools/dsh-bridge/)
   │    game_state() / bot_tool() / set_goal()
   ▼
craft-agent-viewer  ←  HTTP bridge (Rust, part of this repo)
   ▼
craft-agent-minecraft  ←  53 typed tools (Rust, part of this repo)
   ▼
azalea (fork, vendored)  ←  Minecraft protocol client
   ▼
Minecraft Java 26.2 server  ←  you install/run this yourself
```

Three things you must bring yourself: **Rust nightly**, **a Minecraft server**, and
**DeepSeek Harness**. Everything else (azalea fork, viewer, tools, the DSH bridge
plugin, the craft-bot preset) is shipped in this repository.

---

## 1. Prerequisites

| Dependency | Why | How to install |
|---|---|---|
| **Rust nightly** | builds the workspace (azalea needs nightly) | `rustup toolchain install nightly-2026-07-21` (pinned in `rust-toolchain.toml`; stable fails) |
| **Git** | clone repo + submodule | https://git-scm.com |
| **Node.js ≥ 20 + pnpm** | DSH bridge plugin installation | https://nodejs.org, then `npm install -g pnpm` |
| **Minecraft Java 26.2 server** | the bot joins it | run your own vanilla server (LAN is fine); default address `localhost:4444` |
| **DeepSeek Harness (DSH)** | the LLM brain | install from https://github.com/deepseek-ai/deepseek-harness — **not bundled here** |

> **Why DSH is not bundled**: this project treats DSH as the external "brain" — the same
> harness you use for coding. Bundling it would duplicate the whole harness toolchain and
> pin a version that may not match your setup. You install DSH once; `setup.ps1` then
> registers the craft-bot preset into your existing `~/.dsh`.

## 2. Clone (azalea is vendored, not downloaded)

```bash
git clone --recurse-submodules https://github.com/XJungit/seeker-craft.git
cd seeker-craft
```

- The **azalea protocol client is vendored** in `vendor/azalea/` as a git submodule.
- It points to the **maintained fork** `XJungit/azalea` (`craft-agent` branch): upstream
  azalea lacks the archery (`stop_use_item`) and equipping (`use_item_air`) APIs this bot uses.
- The manifest pins that fork's rev; `Cargo.lock` records the https source, so **a fresh
  clone builds with no local cargo patch**.

> If you already cloned without `--recurse-submodules`:
> `git submodule update --init --recursive`

## 3. Install & configure (one shot)

```powershell
.\scripts\setup.ps1
```

Idempotent and repeatable. What it does (in order):

1. **Checks prerequisites** — cargo / git / node / pnpm; tells you what to install if missing.
2. **Builds the workspace** — `cargo build --workspace`.
3. **Registers the DSH bridge plugin** (`tools/dsh-bridge/`) into `~/.dsh/profiles/web`:
   - adds the `dsh-bridge` link dependency to the profile `package.json`
   - appends the plugin config override to `cordis.patch.yml`
   - links `@deepseek-ai/dsh-tools` / `@deepseek-ai/schemastery` into the plugin's `node_modules`
   - runs `pnpm install` and the plugin verification script
4. **Generates the craft-bot preset** into `~/.dsh/.agent-presets/craft-bot` from
   `data/dsh/craft-bot-preset/` (substituting `{{PROJECT_ROOT}}` / `{{DSH_PKG_ROOT}}`).
5. **Copies `.env.example` → `.env`** if absent.
6. **Verifies** the DSH plugin loads in the harness module graph.

Flags: `-SkipBuild` (env check only), `-SkipDsh` (build only, don't touch DSH).

> What this means for you: **you do not install the DSH plugin manually** — the repo
> ships the plugin source and `setup.ps1` wires it into your harness. If you ever want to
> understand or redo it by hand, see `tools/dsh-bridge/README.md` (it documents the manual
> equivalent of steps 3–4).

## 4. Start

```powershell
# 1) start your MC 26.2 server on localhost:4444
# 2) one-shot start (build viewer → start viewer → connect bot → poll ready)
.\scripts\start.ps1
```

`start.ps1` parameters (all defaulted): `-Goal`, `-Steps` (0 = infinite), `-Port` (8080),
`-Mc` (localhost:4444), `-Username` (CraftAgent).

Manual alternative with the ops console:

```powershell
cargo run -p craft-agent-ctl -- viewer "explore the world" 0   # viewer only
cargo run -p craft-agent-ctl -- start                          # connect bot
cargo run -p craft-agent-ctl -- status                         # verify running=true
```

## 5. Drive from DSH

1. Open **DeepSeek Harness**, create/enter a **craft-bot** preset session.
2. A **Craft Bot dashboard** embeds on the right (live bot state).
3. Use the three bridge tools (registered by the dsh-bridge plugin):

```
game_state()                                   # perceive live state
bot_tool(name:"craft", args:{item:"stone_pickaxe"})   # run one of the 54 tools
set_goal("Collect 24 iron ore and smelt into ingots") # set the ops goal
```

The tools appear in the DSH tool catalog when the plugin mounts. `bot_tool` auto-corrects
(mining air → nearest solid; interactions auto-approach ≤2.5 m), so pass the intended target.

4. Stop with `.\scripts\stop.ps1`.

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
Session logs: `sessions/mc_run.jsonl` (viewer runtime data, gitignored).
Viewer/autopilot logs: `%TEMP%\opencode\viewer_run.log` (override with `SEEKER_LOG_DIR`).

## Troubleshooting

See [`troubleshooting.md`](troubleshooting.md). Common issues:

- **Viewer API not ready** — check `%TEMP%\opencode\viewer_run.log` / `viewer_run.err.log`.
- **Bot can't join** — confirm the MC server version is 26.2 and listening on the address in `-Mc`.
- **DSH tools not appearing** — re-run `setup.ps1` (it regenerates the preset and verifies the plugin).
- **Submodule checkout fails** — confirm `vendor/azalea` resolves to `XJungit/azalea`
  (see `.gitmodules`); run `git submodule update --init --recursive`.
