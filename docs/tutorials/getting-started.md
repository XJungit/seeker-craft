# Getting Started

Build and run Craft-Agent locally.

## Prerequisites

- Rust nightly (edition 2024, see `rust-toolchain.toml`).
- A running Minecraft Java 26.2 server (vanilla/forge/fabric) for the azalea bot to join.
- The azalea route (`azalea-bot` feature) is the only supported path. The old
  `mod-bridge` (Fabric mod TCP bridge) and `real` (VLM + enigo) routes have been
  removed from source.

## Build

```bash
cargo build --workspace
cargo build -p craft-agent-minecraft --features azalea-bot
```

## Run

```bash
cargo run -p craft-agent-viewer -- --goal "collect wood" --steps 40 --port 8080
```

打开浏览器 http://127.0.0.1:8080 查看实时对话与启停控制。

The bot joins the MC server at `localhost:4444` (configurable in `adapter_azalea.rs`).

## Debug

Set `RUST_LOG=debug` for verbose output (azalea pathfinder logs included).
Session logs are written to the path passed via `--session` (default `sessions/mc_run.jsonl`).

## Test

```bash
# Full workspace tests (234 tests)
cargo test --workspace --no-fail-fast

# Core crate (122 tests)
cargo test -p craft-agent --lib

# Minecraft adapter (118 tests, includes mock container integration tests)
cargo test -p craft-agent-minecraft --features azalea-bot --lib
```

## Automation Loop

```bash
# Full automation: build → test → LLM e2e → analyze → fix → rerun
.\tools\auto_diag.ps1 -Goal "..." -Steps 40 -TimeoutMin 20

# Analyze a session file
.\tools\scan_run.ps1
```

See `AGENTS.md` for the full automation workflow manual.
