# Design Docs

Project design documentation. Updated 2026-07-22 to reflect actual codebase
state (pathfinding/combat/GoalEngine exist on Java side; no Baritone).

- [mod-bridge.md](./mod-bridge.md) — Java mod architecture, TCP protocol, component map.
- [mindcraft-parity-audit.md](./mindcraft-parity-audit.md) — Tool coverage vs Mindcraft (62 tools, 44 PASS).
- [upgrade-to-mindcraft-parity-2026-07-15.md](./upgrade-to-mindcraft-parity-2026-07-15.md) — Capability comparison & remaining gaps.

See also:
- [`ARCHITECTURE.md`](../../ARCHITECTURE.md) — Layered overview and runtime flow.
- [`PLAN.md`](../../PLAN.md) — GoalEngine redesign plan (LLM sends goals, not tools).
