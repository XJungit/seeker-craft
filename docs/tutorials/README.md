# Developer Tutorials

This directory contains hands-on guides for working on Craft-Agent.

## Start Here

- `INDEX.md` -- tutorial map
- `new-contributor-guide.md` -- fastest onboarding path

## Core

1. `getting-started.md` -- build, run, and debug the workspace.
2. `project-structure.md` -- repo layout.
3. `configuration.md` -- runtime configuration.
4. `agent-loop.md` -- core loop.

## Extending

5. `adding-tools.md` -- add a new Minecraft tool.
6. `adding-adapters.md` -- add or swap a game adapter.
7. `session-and-compaction.md` -- understand session persistence and compaction.

## Operations

8. `troubleshooting.md` -- runtime problems and fixes.

## Crate-level Docs

| Crate | README |
|---|---|
| `craft-agent` | [`../../crates/craft-agent/README.md`](../../crates/craft-agent/README.md) |
| `craft-agent-minecraft` | [`../../crates/craft-agent-minecraft/README.md`](../../crates/craft-agent-minecraft/README.md) |
| `craft-agent-model` | [`../../crates/craft-agent-model/README.md`](../../crates/craft-agent-model/README.md) |
| `craft-agent-viewer` | [`../../crates/craft-agent-viewer/README.md`](../../crates/craft-agent-viewer/README.md) |

## Quick Start

```bash
cargo build --workspace
cargo run -p craft-agent-minecraft --example agent_azalea_demo --features azalea-bot \
  -- --steps=40 --goal="collect wood" --session=sessions/mc_run_azalea.jsonl
```
