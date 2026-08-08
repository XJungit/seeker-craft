# SeekerCraft (Craft-Agent)

**[English](README.md) | [中文](README.zh-CN.md)**

[![CI: fmt + clippy](https://img.shields.io/github/actions/workflow/status/XJungit/seeker-craft/ci.yml?label=CI&logo=github)](https://github.com/XJungit/seeker-craft/actions/workflows/ci.yml)
[![CI: tests](https://img.shields.io/github/actions/workflow/status/XJungit/seeker-craft/ci.yml?label=ci&logo=github&label=tests)](https://github.com/XJungit/seeker-craft/actions/workflows/ci.yml)
[![Security audit](https://img.shields.io/github/actions/workflow/status/XJungit/seeker-craft/audit.yml?label=cargo-audit&logo=github)](https://github.com/XJungit/seeker-craft/actions/workflows/audit.yml)
[![Docs](https://img.shields.io/github/actions/workflow/status/XJungit/seeker-craft/deploy-docs.yml?label=docs&logo=github)](https://github.com/XJungit/seeker-craft/actions/workflows/deploy-docs.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust: nightly-2026-07-21](https://img.shields.io/badge/rust-nightly--2026--07--21-orange.svg)](rust-toolchain.toml)

**An LLM-driven Minecraft bot that beats the Ender Dragon. Rust + Azalea protocol client, no mods, no screenshots — a real protocol-level player that observes, plans, and executes through typed tools.**

| | |
|---|---|
| **Core question** | Can an LLM autonomously survive, craft, and defeat the Ender Dragon from nothing? |
| **Runtime** | Pure Rust client via [Azalea](https://github.com/azalea-rs/azalea) (MC 26.2), no server mods |
| **Brain** | Any OpenAI-compatible LLM (DeepSeek cache-optimized), VLM optional |
| **Scale** | 6 crates, 53 LLM tools, 23 structured tasks, 10 reactive modes, spatial memory |
| **Dev loop** | Autonomous: gap analysis → fix → probe verify → commit (see [AGENTS.md](AGENTS.md)) |

> **Project nature.** This project is produced through AI-assisted development
> ("vibe coding") as a personal experiment — deliberately adopting a Rust-only
> toolchain the author had never used. Implementation draws on
> [Mindcraft](https://github.com/mindcraft-bots/mindcraft) (JS + mineflayer, reference
> for tasks/profiles/modes) and [Azalea](https://github.com/azalea-rs/azalea).

---

## Highlights

- **Real protocol client** — joins as a vanilla player via the Azalea Rust client (MC 26.2), built-in pathfinding; no mods, no screenshots.
- **53 typed LLM tools** — perceive, goto, mine, craft (2x2/3x3/smelt/enchant/brew), place, build, containers, trading, combat, meta-tools.
- **10 reactive modes** — self-defense, hunting, auto-pickup, torch-placing, unstuck, elbow-room, etc., running tick-level without LLM latency.
- **Structured task system** — 23 tiered tasks (wood → stone → iron → diamond → netherite → ender dragon) with machine-checkable completion conditions.
- **Spatial WorldMemory** — chunk-indexed memories (resources, structures, containers, hazards, portals) with TTL forgetting and named anchors.
- **Byte-stable system prompt** — engineered for DeepSeek-style prefix caching; dynamic state injected as user messages, >93% prefix cache hit.
- **Probe mode** — a no-LLM tool-layer test harness that verifies tool behavior in seconds (not minutes of LLM runtime).
- **Ops console (`craft-agent-ctl`)** — process lifecycle, goal injection, session inspection.
- **Autopilot** — autonomous dev loop that builds, tests, triages anomalies, root-causes, and commits.

## Architecture

```
seeker-craft/
├── Cargo.toml                     # workspace root (nightly-2026-07-21)
├── crates/
│   ├── craft-agent/               # core agent: run_one_turn loop, modes, compaction, skills, WorldMemory
│   ├── craft-agent-minecraft/     # Azalea adapter: bot + 53 tools (craft/smelt/enchant/brew/combat/farm)
│   ├── craft-agent-model/         # LLM/VLM clients (OpenAI-compatible, multi-backend)
│   ├── craft-agent-viewer/        # web dashboard (Axum + SSE)
│   ├── craft-agent-autopilot/     # autonomous dev loop (build/test/RCA/commit)
│   └── craft-agent-ctl/           # ops console
├── data/
│   ├── config/agent.example.toml  # LLM backend template (copy to agent.toml)
│   ├── tasks/                     # 23 task JSONs (tier 1-6)
│   ├── profiles/                  # 3-layer prompt templates
│   ├── blueprints/                # build blueprints
│   └── actions/                   # LLM-defined rhai scripts
└── vendor/azalea/                 # pinned Azalea source (submodule, official upstream)
```

### The 13-step agent loop

```
 1  drain queues ────────────► 2 compaction ──► 3 strip transient msgs
 5  reactive modes ──► 4 auto-perceive ────► 7 dynamic context (skills/examples)
 6. self-prompt ├── 8 WorldMemory (radius 64) ──► 9 LLM call (retry/backoff)
 10. plain-text check (nudge) ─ 11. dead-loop guard ─ 12. execute batch (READ parallel /
                                                               WRITE serial / slow-tool probe)
 13. skill extraction └─────────────────────────────────────────────────────────►

See [ARCHITECTURE.md](ARCHITECTURE.md) for the full 13-step loop details.
```
(drawn as flow → see ARCHITECTURE.md, the loop is implemented in `craft-agent/agent/run_one_turn.rs`.)

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

## 53 LLM tools

| Category | Tools |
|---|---|
| Perception | `perceive`, `memory`, `search_wiki`, `search_for_block` |
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

## Quick Start

### Prerequisites

- Rust **nightly** (see `rust-toolchain.toml`; stable fails — azalea requires nightly)
- A Minecraft Java server the bot can join (vanilla 1.20.4+ / MC 26.2, LAN included)

### Build & test

```bash
cargo build --workspace
cargo test -p craft-agent --lib
cargo test -p craft-agent-minecraft --features azalea-bot --lib
```

### Configure LLM backends

```bash
cp data/config/agent.example.toml data/config/agent.toml
# edit data/config/agent.toml — set your API keys (or use api_key_env + env vars)
```

Any OpenAI-compatible endpoint works (DeepSeek, OpenAI, local gateways, ...).
Keys are never committed: `agent.toml` is gitignored.

### Run the bot

```bash
# Web dashboard + agent (LLM-driven)
cargo run -p craft-agent-viewer --bin craft-agent-viewer \
  -- --goal "挖矿下探" --steps 0 --port 8080 --mc localhost:4444 --username CraftAgent
# open http://127.0.0.1:8080

# Probe mode — test tools WITHOUT the LLM (seconds, not minutes)
cargo run -p craft-agent-minecraft --example azalea_probe --features azalea-bot -- 4444 --cmd "equip iron_helmet helmet"
```

### Probe scripts

```bash
# Feature/end-to-end verification (see scripts/probe/*.json)
cargo run -p craft-agent-minecraft --example azalea_probe --features azalea-bot -- 4444 --script scripts\probe\smoke.json
```

## Documentation

| Doc | What it covers |
|---|---|
| [ARCHITECTURE.md](ARCHITECTURE.md) | Layered architecture, 13-step agent loop, module layout |
| [AGENTS.md](AGENTS.md) | Autonomous development workflow (gap analysis → fix → verify) |
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

## License

[MIT](LICENSE) — see [AUTHORS](AUTHORS) for maintainers. Cite via [CITATION.cff](CITATION.cff).