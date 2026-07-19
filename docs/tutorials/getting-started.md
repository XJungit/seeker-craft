# Getting Started

Build and run Craft-Agent locally.

## Prerequisites

- Rust stable with edition 2024
- Windows for `real` features; other platforms can use `mod-bridge`
- For `mod-bridge`: a running Minecraft with `craft-agent-bridge`

## Build

```bash
cargo build --workspace
cargo build --workspace --features mod-bridge
cargo build --workspace --features real
```

## Run

```bash
cargo run -p craft-agent-minecraft --example agent_multi_step_mod --features mod-bridge \
  -- --steps=40 --goal="collect wood" --session=sessions/mc_run_mod.jsonl
```

## Debug

Set `RUST_LOG=debug` for verbose output.
Session logs are written to the path passed via `--session`.
