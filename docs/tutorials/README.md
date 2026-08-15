# Developer Tutorials

This directory contains hands-on guides for working on SeekerCraft (Craft-Agent) v1.0.

## Start Here

- `INDEX.md` — tutorial map
- `new-contributor-guide.md` — fastest onboarding path
- `getting-started.md` — clone/build/run (DSH bridge mode)

## Core

1. `getting-started.md` — clone, build, run, drive from DSH.
2. `project-structure.md` — repo layout.
3. `agent-loop.md` — **historical** in-bot 13-step loop (removed 2026-08-14); kept for reference.

## Extending

4. `adding-tools.md` — add a new Minecraft tool.
5. `adding-adapters.md` — add or swap a game adapter.
6. `session-and-compaction.md` — session persistence & cache design.

## Operations

7. `troubleshooting.md` — runtime problems and fixes.

## Crate-level Docs

| Crate | README |
|---|---|
| `craft-agent` | [`../../crates/craft-agent/README.md`](../../crates/craft-agent/README.md) |
| `craft-agent-minecraft` | [`../../crates/craft-agent-minecraft/README.md`](../../crates/craft-agent-minecraft/README.md) |
| `craft-agent-viewer` | [`../../crates/craft-agent-viewer/README.md`](../../crates/craft-agent-viewer/README.md) |

## Quick Start

```powershell
.\scripts\setup.ps1      # build + DSH bridge + craft-bot preset
.\scripts\start.ps1      # viewer + connect bot
# then open DSH → craft-bot preset session
```
