# New Contributor Guide

This page maps the fastest path from zero to productive.

## Suggested Reading Order

1. `getting-started.md` (v1.0 DSH bridge mode — clone/build/run)
2. `project-structure.md`
3. `adding-tools.md` or `adding-adapters.md` depending on your task
4. `agent-loop.md` only for historical context (in-bot loop, removed 2026-08-14)

## Key Documents for v1.0

- `../../ARCHITECTURE.md` — current architecture (DSH bridge runtime, azalea fork maintenance)
- `../../README.md` — overview, 54-tool table, 6-stage path
- `../../tools/dsh-bridge/README.md` — the DSH bridge plugin (game_state / bot_tool / set_goal)

## Crate-Level Docs

After the tutorials, read the crate README for the crate you'll work on:

- [`craft-agent`](../../crates/craft-agent/README.md) — core runtime abstractions
- [`craft-agent-minecraft`](../../crates/craft-agent-minecraft/README.md) — Minecraft adapter
- [`craft-agent-viewer`](../../crates/craft-agent-viewer/README.md) — Web dashboard (Axum + SSE)

## Common Tasks

- Run the azalea-bot session: see `getting-started.md`
- Add a Minecraft tool: see `adding-tools.md`
- Inspect runtime behavior: see `troubleshooting.md`

## Getting Help

- Open an issue for bugs or feature requests.
- Review `docs/design/` only if you need historical context.
