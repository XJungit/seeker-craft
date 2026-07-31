# Design Docs

Project design documentation. Updated 2026-07-27 to reflect the azalea-bot
single-route architecture and P55-P58 fixes.

- [refactor-azalea-client-route.md](./refactor-azalea-client-route.md) — architecture shift from Fabric mod to Azalea client (implemented, 2026-07-23).
- [mindcraft-parity-audit.md](./mindcraft-parity-audit.md) — Tool coverage vs Mindcraft (44 tools, P48 RecipeBook integration).
- [upgrade-to-mindcraft-parity-2026-07-15.md](./upgrade-to-mindcraft-parity-2026-07-15.md) — Capability comparison & remaining gaps.

See also:
- [`ARCHITECTURE.md`](../../ARCHITECTURE.md) — Layered overview, 13-step main loop, 44 tools, P56-P58 governance.
- [`PLAN.md`](../../PLAN.md) — Current project plan (azalea-bot route, Mindcraft philosophy,通关路径).
- [`AGENTS.md`](../../AGENTS.md) — Full automation workflow manual + section 9-bis Mindcraft philosophy rules.
- [`adr.md`](../adr.md) — ADR-001 (azalea-only) / ADR-004 (LLM-driven tools, supersedes old GoalEngine plan).

## Historical Context

The original `PLAN.md` proposed "LLM sends goals, Java Mod auto-decomposes" —
this was superseded by ADR-004 (2026-07-27) when the Java mod route was deleted.
The current architecture: LLM directly controls the bot via 37 atomic tools,
following the Mindcraft philosophy where bot tools only do what they can do
and return `Err` with resolution steps when they can't.
