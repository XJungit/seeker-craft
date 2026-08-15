# Craft-Agent Tutorials

Start here if you are new to the project.

## Quick Start (v1.0 · DSH bridge mode)

1. Read `getting-started.md` — clone, setup, start, drive from DSH.
2. Read `project-structure.md` — repo layout.
3. Run `.\scripts\setup.ps1` then `.\scripts\start.ps1` (see `getting-started.md`).
4. Open DSH → craft-bot preset session → use `game_state` / `bot_tool` / `set_goal`.
5. Read `troubleshooting.md` as needed.

## Full Path

- `new-contributor-guide.md` — curated onboarding path
- `getting-started.md` — clone/build/run (DSH bridge mode)
- `project-structure.md` — repo layout
- `agent-loop.md` — **historical** in-bot 13-step loop (removed 2026-08-14)
- `adding-tools.md` — add tools
- `adding-adapters.md` — add adapters
- `session-and-compaction.md` — sessions & cache
- `troubleshooting.md` — runtime issues

## Crate-level Docs

Each crate has its own README with API details:

| Crate | README |
|---|---|
| `craft-agent` | [`../../crates/craft-agent/README.md`](../../crates/craft-agent/README.md) |
| `craft-agent-minecraft` | [`../../crates/craft-agent-minecraft/README.md`](../../crates/craft-agent-minecraft/README.md) |
| `craft-agent-viewer` | [`../../crates/craft-agent-viewer/README.md`](../../crates/craft-agent-viewer/README.md) |

## DSH Bridge Plugin

The `tools/dsh-bridge/` plugin adds `game_state` / `bot_tool` / `set_goal` to DSH and
embeds a live bot dashboard. See [`../../tools/dsh-bridge/README.md`](../../tools/dsh-bridge/README.md).
