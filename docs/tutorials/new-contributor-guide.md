# New Contributor Guide

This page maps the fastest path from zero to productive.

## Suggested Reading Order

1. `getting-started.md`
2. `project-structure.md`
3. `configuration.md`
4. `agent-loop.md`
5. `adding-tools.md` or `adding-adapters.md` depending on your task

## Crate-Level Docs

After the tutorials, read the crate README for the crate you'll work on:

- [`craft-agent`](../../crates/craft-agent/README.md) — core runtime abstractions
- [`craft-agent-minecraft`](../../crates/craft-agent-minecraft/README.md) — Minecraft adapter
- [`craft-agent-model`](../../crates/craft-agent-model/README.md) — LLM/VLM clients
- [`craft-agent-viewer`](../../crates/craft-agent-viewer/README.md) — Web dashboard (Axum + SSE)

## Common Tasks

- Run the azalea-bot session: see `getting-started.md`
- Add a Minecraft tool: see `adding-tools.md`
- Inspect runtime behavior: see `troubleshooting.md`

## Getting Help

- Open an issue for bugs or feature requests.
- Review `docs/design/` only if you need historical context.
