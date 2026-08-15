# SeekerCraft (Craft-Agent)

**[English](README.md) | [中文](README.zh-CN.md)**

[![CI: fmt + clippy](https://img.shields.io/github/actions/workflow/status/XJungit/seeker-craft/ci.yml?label=CI&logo=github)](https://github.com/XJungit/seeker-craft/actions/workflows/ci.yml)
[![CI: tests](https://img.shields.io/github/actions/workflow/status/XJungit/seeker-craft/ci.yml?label=ci&logo=github&label=tests)](https://github.com/XJungit/seeker-craft/actions/workflows/ci.yml)
[![Security audit](https://img.shields.io/github/actions/workflow/status/XJungit/seeker-craft/audit.yml?label=cargo-audit&logo=github)](https://github.com/XJungit/seeker-craft/actions/workflows/audit.yml)
[![Docs](https://img.shields.io/github/actions/workflow/status/XJungit/seeker-craft/deploy-docs.yml?label=docs&logo=github)](https://github.com/XJungit/seeker-craft/actions/workflows/deploy-docs.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust: nightly-2026-07-21](https://img.shields.io/badge/rust-nightly--2026--07--21-orange.svg)](rust-toolchain.toml)
[![Release: v1.1.0](https://img.shields.io/badge/release-v1.1.0-blue.svg)](https://github.com/XJungit/seeker-craft/releases)

**An LLM-driven Minecraft bot that beats the Ender Dragon. Rust + Azalea protocol client, no mods, no screenshots — a real protocol-level player that observes, plans, and executes through typed tools.**

| | |
|---|---|
| **Core question** | Can an LLM autonomously survive, craft, and defeat the Ender Dragon from nothing? |
| **Runtime** | Pure Rust client via [Azalea](https://github.com/azalea-rs/azalea) (MC 26.2), no server mods |
| **Brain** | Any OpenAI-compatible LLM (DeepSeek cache-optimized); DSH (DeepSeek Harness) bridge mode as of 2026-08-14 |
| **Scale** | 5 crates, 54 LLM tools, 23 structured tasks, 10 reactive modes, spatial memory |
| **Dev loop** | Autonomous: gap analysis → fix → probe verify → commit (workflow notes kept locally, not shipped) |

> **Project nature.** This project is produced through AI-assisted development
> ("vibe coding") as a personal experiment — deliberately adopting a Rust-only
> toolchain the author had never used. Implementation draws on
> [Mindcraft](https://github.com/mindcraft-bots/mindcraft) (JS + mineflayer, reference
> for tasks/profiles/modes) and [Azalea](https://github.com/azalea-rs/azalea).

---

## Highlights

- **Real protocol client** — joins as a vanilla player via the Azalea Rust client (MC 26.2), built-in pathfinding; no mods, no screenshots.
- **54 typed LLM tools** — perceive, goto, mine, craft (2x2/3x3/smelt/enchant/brew), place, build, containers, trading, combat, meta-tools.
- **10 reactive modes** — self-defense, hunting, auto-pickup, torch-placing, unstuck, elbow-room, etc., running tick-level without LLM latency (bot-side; LLM posture switched via `set_mode`).
- **Structured task system** — 23 tiered tasks (wood → stone → iron → diamond → netherite → ender dragon) with machine-checkable completion conditions.
- **Spatial WorldMemory** — chunk-indexed memories (resources, structures, containers, hazards, portals) with TTL forgetting and named anchors.
- **DSH bridge mode** — since 2026-08-14 the in-bot LLM loop is removed; DSH (DeepSeek Harness) is the sole brain driving the bot through the viewer bridge (`/api/connect` + `/api/bot_tool` + `/api/game-state` + `/api/goal`).
- **Probe mode** — a no-LLM tool-layer test harness that verifies tool behavior in seconds (not minutes of LLM runtime).
- **Ops console (`craft-agent-ctl`)** — process lifecycle, goal injection, session inspection.
- **Autopilot** — ops supervisor (10s polling): brings up viewer + connects bot, stall steering, crash recovery, anomaly detection (no code-editing logic).

## Architecture

```
seeker-craft/
├── Cargo.toml                     # workspace root (nightly-2026-07-21)
├── crates/
│   ├── craft-agent/               # pure logic lib: types/GameTool/ToolRegistry/WorldMemory/session/task/profile/skill
│   ├── craft-agent-minecraft/     # Azalea adapter: bot + 54 tools (craft/smelt/enchant/brew/combat/farm)
│   ├── craft-agent-viewer/        # web dashboard (Axum + SSE) + DSH bridge (connect/bot_tool/game-state/goal)
│   ├── craft-agent-autopilot/     # ops supervisor (10s polling: viewer+connect, stall steering, crash recovery)
│   └── craft-agent-ctl/           # ops console
├── data/
│   ├── config/agent.example.toml  # legacy LLM template (in-bot era; not used in DSH mode)
│   ├── tasks/                     # 23 task JSONs (tier 1-6)
│   ├── profiles/                  # 3-layer prompt templates
│   ├── blueprints/                # build blueprints
│   ├── actions/                   # LLM-defined rhai scripts
│   └── dsh/craft-bot-preset/      # DSH craft-bot preset template (setup.ps1 generates into ~/.dsh)
├── scripts/
│   ├── setup.ps1                  # one-shot install & configure (build + DSH bridge + preset + verify)
│   ├── start.ps1                  # one-shot start viewer + connect bot
│   ├── stop.ps1                   # one-shot stop
│   └── probe/*.json               # tool-layer live-test scripts (no LLM)
├── tools/dsh-bridge/              # DSH bridge plugin (game_state/bot_tool/set_goal + dashboard)
└── vendor/azalea/                 # local mirror of the maintained azalea fork (submodule)
```

> **azalea dependency**: the manifest declares the maintained fork `XJungit/azalea`
> (`craft-agent` branch) as an https source with a pinned rev — upstream main lacks the
> archery/equipping APIs. `vendor/azalea` is the local offline mirror (submodule); during
> development a gitignored `.cargo/config.toml` `[patch]` redirects to it. **Fresh clones
> compile without that patch.** Fork-update workflow: see [ARCHITECTURE.md](ARCHITECTURE.md).

### DSH bridge runtime (since 2026-08-14)

```
DSH (DeepSeek Harness) brain ──HTTP──► craft-agent-viewer bridge
  │  /api/connect    → azalea client joins MC (account CraftAgent)
  │  /api/bot_tool   → dispatch one of 54 tools (GameTool::execute)
  │  /api/game-state → real-time BotState snapshot (perceive format)
  │  /api/goal       → update ops goal
  ▼
craft-agent-minecraft (54 tools + WorldMemory per-20-tick scan + handler.rs reactive modes)
  ▼
azalea (vendor) ──► MC server (TCP)
```

> **The in-bot 13-step agent loop was removed** (2026-08-14, phase-3 cleanup): `run_one_turn`,
> auto-perceive, SelfPrompter, execute_batch, and per-turn dynamic-context injection no longer
> exist in Rust. The brain (DSH) now owns decision/planning/context-injection/system-prompt
> stability; Rust only exposes real-time bot capabilities through the viewer bridge.
> See [ARCHITECTURE.md](ARCHITECTURE.md) for details.

## The 6-stage path to beating the dragon

| Tier | Stage | Tasks |
|---|---|---|
| 1 | **Wood & tools** | gather wood, crafting table, wooden pickaxe, stone pickaxe |
| 2 | **Iron age** | furnace, iron pickaxe |
| 3 | **Survival gear** | bread, iron armor, iron sword, shield |
| 4 | **Diamond age** | diamond pickaxe/sword/armor, mine to bedrock |
| 5 | **Nether & magic** | enchanting table, enchant sword, brewing stand, nether portal |
| 6 | **Finale** | netherite ingot/pickaxe, shulker box, elytra, ender dragon |

All 23 tasks (6 tiers) ship as machine-checkable JSON in [`data/tasks/`](data/tasks/).

## Current Progress (2026-08-15 · v1.0.0)

**Verified end-to-end (live server, no mods):**

| Stage | Status | Evidence |
|---|---|---|
| Tier 1–2: wood → stone → iron pickaxe chain | ✅ live | bot autonomously gathered wood, crafted planks/sticks/pickaxes, and crafted an iron pickaxe via a nearby crafting table |
| Tier 3: survival gear | ✅ live | full iron armor equipped + diamond sword + shield; HP/hunger fully recovered |
| Tier 4: diamond age | 🔄 in progress | bot followed the Y-layer hint (mine_below to Y≤16), descended to the diamond layer (Y=-59), and located diamond_ore blocks with `search_for_block` |
| Tier 5: nether & magic | ⬜ next | nether portal / enchanting / brewing not yet end-to-end verified |
| Tier 6: finale | ⬜ pending | netherite / shulker / elytra / ender dragon |

**Milestones recently shipped (v1.0.0 release baseline):**

- **v1.0.0 (2026-08-15)** — 1.0 release: DSH bridge mode is the only supported usage (one-shot setup/start/stop scripts +
  craft-bot preset); azalea dependency moved to a maintained fork (`XJungit/azalea`) with a pinned rev — **fresh clones compile
  with no local patch**; `craft-agent-ctl` paths are derived at runtime (no machine-specific paths); repo URL corrected.
- **P154** — equip falls back to vanilla right-click equipping (use_item_air) when left_click fails
- **P152/P151/P150** — mine approach-branch intermediate results, look_at before mining, approach before mining far targets (fixes dropped loot)
- **P149/P148/P147** — pickup supports vertical drops; goto underground nav fix; auto-pickup after mining
- **P135/P136** — recipe & Y-layer knowledge-base fixes (below)

**Verification discipline:** every tool-layer behavior is probe-verified against the live server (see `scripts/probe/*.json`) before push; Y-hint correctness was probe-verified for diamond (out-of-range hint), emerald (biome hint), and iron/coal (no false positives in-range). Full milestone table: [`docs/benchmarks.md`](docs/benchmarks.md).

## 54 LLM tools

| Category | Tools |
|---|---|
| Perception | `perceive`, `memory`, `remember`, `search_wiki`, `search_for_block` |
| Movement | `goto`, `goto_player`, `move_away`, `mine_below`, `mine_above`, `pickup`, `follow`, `stop_follow` |
| Modes | `set_mode` |
| Mining | `mine`, `make_obsidian` |
| Interaction | `interact_block`, `interact_entity`, `attack`, `defend`, `use_item`, `shoot`, `sleep` |
| Craft | `craft`, `craft_3x3`, `smelt`, `auto_craft`, `enchant` |
| Gathering | `gather`, `till_and_sow`, `harvest` |
| Placement | `place`, `build`, `build_blueprint`, `list_blueprints` |
| Containers | `open`, `chest_view`, `chest_withdraw`, `chest_deposit` |
| Inventory | `equip`, `discard`, `consume` |
| NPC/Social | `trade`, `give` |
| Meta | `chat`, `set_goal`, `run_plan`, `run_script`, `new_action`, `list_actions`, `pause_goal`, `resume_goal`, `task_complete`, `task_retry` |

## Quick Start (v1.0 · DSH bridge mode)

> **In v1.0 the usage is DSH bridge mode**: `craft-agent-viewer` only provides the
> HTTP bridge (`/api/connect` + `/api/bot_tool` + `/api/game-state` + `/api/goal`),
> and the **brain is DeepSeek Harness (DSH)** — you drive the bot from DSH using
> three tools (`game_state` / `bot_tool` / `set_goal`). The steps below are verified
> on Windows PowerShell.

### 1. Prerequisites

| Dependency | Notes |
|---|---|
| **Rust nightly** | Pinned `nightly-2026-07-21` in `rust-toolchain.toml` (azalea needs nightly; stable fails) |
| **Git** | For cloning the repo and submodules |
| **Node.js ≥ 20 + pnpm** | For the DSH bridge plugin |
| **Minecraft Java 26.2 server** | Bring your own vanilla server (LAN is fine); bot connects to `localhost:4444` by default |
| **DeepSeek Harness (DSH)** | Bring your own install (https://github.com/deepseek-ai/deepseek-harness); this repo only generates the craft-bot preset |

> **Why DSH is not bundled**: DSH is the external "brain" (the same harness you code with);
> bundling it would duplicate the whole toolchain and pin a version. You install DSH once;
> `setup.ps1` then registers the craft-bot preset into your existing `~/.dsh`.

### 2. Clone (with azalea submodule)

```bash
git clone --recurse-submodules https://github.com/XJungit/seeker-craft.git
cd seeker-craft
```

> The azalea dependency is a maintained fork (`XJungit/azalea`, `craft-agent` branch) —
> upstream lacks the APIs the bot needs for archery/equipping. The manifest declares
> the fork's https source with a pinned rev, so **a fresh clone compiles with no local
> patch**. See [ARCHITECTURE.md](ARCHITECTURE.md) → "azalea fork maintenance".

### 3. One-shot install & configure (setup.ps1)

```powershell
.\scripts\setup.ps1
```

Idempotent and repeatable. It:

1. Checks prerequisites (cargo / git / node / pnpm; prompts installs if missing)
2. Runs `cargo build --workspace`
3. Configures the DSH bridge plugin (registers into `~/.dsh` + links deps + `pnpm install`)
4. Generates the **craft-bot preset** (`~/.dsh/.agent-presets/craft-bot`), substituting local path placeholders
5. Copies `.env.example` → `.env` if absent
6. Runs the DSH plugin verification script

> Only want to build, not touch DSH? `.\scripts\setup.ps1 -SkipDsh` (skips steps 3/4).
> Skip the build? `-SkipBuild`.

### 4. Start MC server + viewer + connect bot

```powershell
# First start your MC 26.2 server (listening on localhost:4444)

# One shot: build viewer → start viewer → connect bot (polls until ready)
.\scripts\start.ps1
```

`start.ps1` parameters (all have defaults):

| Parameter | Default | Notes |
|---|---|---|
| `-Goal` | explore the world… | Operational goal shown in the viewer |
| `-Steps` | `0` (infinite) | Number of steps |
| `-Port` | `8080` | Viewer HTTP port |
| `-Mc` | `localhost:4444` | MC server address |
| `-Username` | `CraftAgent` | Bot name |

Or step-by-step with `craft-agent-ctl`:

```powershell
cargo run -p craft-agent-ctl -- viewer "explore the world" 0   # viewer only
cargo run -p craft-agent-ctl -- start                          # connect bot
cargo run -p craft-agent-ctl -- status                         # verify running=true
```

### 5. Drive the bot from DSH (core usage)

1. Open **DeepSeek Harness**, create/enter a **craft-bot** preset session
2. A **Craft Bot dashboard** embeds on the right side (live bot state)
3. Call the three tools in conversation:

```
game_state()                    # perceive: position/health/hunger/inventory/nearby/memory
bot_tool(name:"mine", args:{x:.., y:.., z:..})   # execute one of the 54 tools
set_goal("Collect 24 iron ore and smelt into ingots")           # set the ops goal
```

> Tool names are a stable contract (`tools_azalea.rs::ALL_TOOL_NAMES`, 54 total).
> Auto-corrections (mine-on-air → nearest solid; interaction → auto-approach ≤2.5m)
> are built in — pass the intended target directly.

### 6. Stop

```powershell
.\scripts\stop.ps1        # stops viewer/autopilot (does not affect MC server or DSH)
```

### Build & test (for development)

```bash
cargo build --workspace
cargo test -p craft-agent --lib
cargo test -p craft-agent-minecraft --features azalea-bot --lib
```

### Probe mode (test the tool layer WITHOUT the LLM, seconds not minutes)

```bash
# Single command
cargo run -p craft-agent-minecraft --example azalea_probe --features azalea-bot -- 4444 --cmd "equip iron_helmet helmet"
# Script (see scripts/probe/*.json)
cargo run -p craft-agent-minecraft --example azalea_probe --features azalea-bot -- 4444 --script scripts\probe\smoke.json
```

> The LLM is provided by **DSH** (the brain) — there is no separate LLM backend
> configuration file in this repo. `data/config/agent.example.toml` is a legacy
> template from the in-bot era (kept for reference only; not used in DSH mode).

## Documentation

| Doc | What it covers |
|---|---|
| [ARCHITECTURE.md](ARCHITECTURE.md) | Layered architecture, DSH bridge runtime, module layout |
| [docs/mindcraft-gap.md](docs/mindcraft-gap.md) | Mindcraft parity audit + prioritized backlog |
| [docs/benchmarks.md](docs/benchmarks.md) | Test baselines, probe coverage, cache hit rates, Ender-Dragon progress |
| [docs/adr.md](docs/adr.md) | Architecture decision records |
| [docs/README.md](docs/README.md) | Full documentation index (tutorials, design archive) |
| [CHANGELOG.md](CHANGELOG.md) | Versioned change log |
| [SECURITY.md](SECURITY.md) | Security policy & vulnerability reporting |
| [CONTRIBUTING.md](CONTRIBUTING.md) | How to develop, test & submit a fix |

Docs are also published to **GitHub Pages** (rustdoc + docs) — see the
[docs workflow](https://github.com/XJungit/seeker-craft/actions/workflows/deploy-docs.yml).

## Related Projects

- [Mindcraft](https://github.com/mindcraft-bots/mindcraft) — JS + mineflayer LLM bot; reference for tasks/profiles/modes
- [Azalea](https://github.com/azalea-rs/azalea) — Rust Minecraft client protocol library
- [XJungit/azalea](https://github.com/XJungit/azalea) — maintained fork used by this project (adds `stop_use_item` /
  `use_item_air` / `force_miss` for archery & equipping; `craft-agent` branch)

## License

[MIT](LICENSE) — see [AUTHORS](AUTHORS) for maintainers. Cite via [CITATION.cff](CITATION.cff).