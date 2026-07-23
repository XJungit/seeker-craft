# Getting Started

Build and run Craft-Agent locally.

## Prerequisites

- Rust nightly (edition 2024, see `rust-toolchain.toml`)
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
cargo run -p craft-agent-minecraft --example agent_azalea_demo --features azalea-bot \
  -- --steps=40 --goal="collect wood" --session=sessions/mc_run_azalea.jsonl
```

## Debug

Set `RUST_LOG=debug` for verbose output (azalea pathfinder logs included).
Session logs are written to the path passed via `--session`.
