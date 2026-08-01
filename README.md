# SeekerCraft

**An LLM-driven Minecraft bot that beats the Ender Dragon.**

SeekerCraft is a Rust implementation of an autonomous Minecraft agent. An LLM
observes structured game state, plans multi-step survival strategies, and
executes them through 44 typed tools built on the
[Azalea](https://github.com/azalea-rs/azalea) client protocol — no mods, no
screenshots, a real protocol-level player.

The goal is simple: from nothing, survive, craft, and defeat the Ender Dragon.

## Highlights

- **Real protocol client** — connects as a vanilla player via the Azalea Rust client (MC 26.2), with built-in pathfinding.
- **44 LLM tools** — perceive, goto, mine, craft (2x2/3x3/smelt/enchant/brew), place, build, containers, trading, and meta tools.
- **10 reactive modes** — self-defense, hunting, auto-pickup, auto-armor, torch-placing, unstuck, and more, running tick-level without LLM latency.
- **Structured task system** — 23 tiered tasks (wood → netherite → ender dragon) with machine-checkable completion conditions.
- **Spatial WorldMemory** — chunk-indexed memories (resources, structures, containers, hazards, portals) with TTL forgetting.
- **Byte-stable system prompt** — engineered for DeepSeek-style prefix caching; dynamic state is injected as user messages.
- **Probe mode** — a no-LLM tool-layer test harness that verifies tool behavior in seconds.
- **Ops console (`craft-agent-ctl`)** — process lifecycle, goal injection, session inspection.
- **Autopilot** — autonomous loop that builds, tests, triages anomalies, root-causes, and commits.

## Architecture

```
seeker-craft/
├── Cargo.toml                    # workspace root
├── crates/
│   ├── craft-agent/              # core agent: run_one_turn loop, modes, compaction, skills, WorldMemory
│   ├── craft-agent-minecraft/    # Azalea adapter: bot, 44 tools, crafting/smelting/enchanting
│   ├── craft-agent-model/        # LLM/VLM clients (OpenAI-compatible, multi-backend)
│   ├── craft-agent-viewer/       # web dashboard (Axum + SSE)
│   ├── craft-agent-autopilot/    # autonomous dev loop
│   └── craft-agent-ctl/          # ops console
├── data/config/agent.example.toml  # LLM backend config template (copy to agent.toml)
├── tasks/                        # 23 task JSONs (tier 1-6)
├── profiles/                     # 3-layer prompt templates
├── blueprints/                   # build blueprints
├── actions/                      # LLM-defined rhai scripts
└── vendor/azalea/                # pinned Azalea source (submodule, official upstream)
```

See [ARCHITECTURE.md](ARCHITECTURE.md) for the 13-step agent loop and
[AGENTS.md](AGENTS.md) for the development workflow.

## Quick Start

### Prerequisites

- Rust **nightly** (see `rust-toolchain.toml`; stable fails — azalea requires nightly)
- A Minecraft Java server the bot can join (any vanilla 1.20.4+ / MC 26.2 server, LAN included)

### Build

```bash
cargo build --workspace
```

### Configure LLM backends

```bash
cp data/config/agent.example.toml data/config/agent.toml
# edit data/config/agent.toml — set your API keys (or use api_key_env + env vars)
```

Any OpenAI-compatible endpoint works (DeepSeek, OpenAI, local gateways, ...).
Keys are never committed: `agent.toml` is gitignored.

### Run

```bash
# Web dashboard + agent (LLM-driven)
cargo run -p craft-agent-viewer --bin craft-agent-viewer \
  -- --goal "挖矿下探" --steps 0 --port 8080 --mc localhost:4444 --username CraftAgent
# open http://127.0.0.1:8080

# Probe mode — test tools without the LLM (seconds, not minutes)
cargo run -p craft-agent-minecraft --example azalea_probe --features azalea-bot -- 4444 --cmd "equip iron_helmet helmet"
```

### Test

```bash
cargo test -p craft-agent --lib
cargo test -p craft-agent-minecraft --features azalea-bot --lib
```

## Documentation

- [ARCHITECTURE.md](ARCHITECTURE.md) — layered architecture, 13-step loop, 44 tools
- [AGENTS.md](AGENTS.md) — autonomous development workflow (gap analysis → fix → verify)
- [docs/mindcraft-gap.md](docs/mindcraft-gap.md) — Mindcraft parity audit + priority queue
- [docs/CHANGELOG.md](docs/CHANGELOG.md) — change log
- [docs/adr.md](docs/adr.md) — architecture decision records
- [docs/README.md](docs/README.md) — full documentation index

## Related Projects

- [Mindcraft](https://github.com/mindcraft-bots/mindcraft) — JS + mineflayer LLM bot; reference for tasks/profiles/modes
- [Azalea](https://github.com/azalea-rs/azalea) — Rust Minecraft client protocol

## License

[MIT](LICENSE)
