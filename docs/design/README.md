# Design Docs

Project design documentation. Updated 2026-07-27 to reflect the azalea-bot
single-route architecture and P55-P58 fixes.

- [refactor-azalea-client-route.md](../legacy/refactor-azalea-client-route.md) — architecture shift from Fabric mod to Azalea client (implemented, 2026-07-23).
- [upgrade-to-mindcraft-parity-2026-07-15.md](../legacy/upgrade-to-mindcraft-parity-2026-07-15.md) — Capability comparison & remaining gaps.
- 已归档到 `../legacy/`：`mindcraft-parity-audit.md`（Java 时代快照，被 `../mindcraft-gap.md` 取代）、
  `refactor-numen-philosophy-baritone-base.md`（Baritone 方案，已弃用）、`game-agent-design.md`（v0.4 纯视觉设计）。

See also:
- [`ARCHITECTURE.md`](../../ARCHITECTURE.md) — Layered overview, DSH bridge runtime, 54 tools, P56-P58 governance (historical).
- [`PLAN.md`](../../PLAN.md) — Current project plan (azalea-bot route, Mindcraft philosophy,通关路径).
- [`AGENTS.md`](../../AGENTS.md) — Full automation workflow manual + section 9-bis Mindcraft philosophy rules.
- [`adr.md`](../adr.md) — ADR-001 (azalea-only) / ADR-004 (LLM-driven tools, supersedes old GoalEngine plan).

## Historical Context

The original `PLAN.md` proposed "LLM sends goals, Java Mod auto-decomposes" —
this was superseded by ADR-004 (2026-07-27) when the Java mod route was deleted.
The architecture evolved: LLM directly controls the bot via 54 atomic tools
(as of v1.1.0, 2026-08-15), following the Mindcraft philosophy where bot tools
only do what they can do and return `Err` with resolution steps when they can't.
Since 2026-08-14 the brain is DSH (DeepSeek Harness) via the viewer bridge; see
[`ARCHITECTURE.md`](../../ARCHITECTURE.md).
