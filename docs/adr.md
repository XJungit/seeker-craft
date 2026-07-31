# Architecture Decision Records

This directory tracks important architecture decisions.

## Current Decisions

- **ADR-001: azalea-bot as the only supported runtime** (supersedes old ADR-001/002)
  - Date: 2026-07-23
  - Status: accepted
  - Context: Three runtime paths existed (mod-bridge Java mod, real VLM+enigo,
    azalea-bot Rust client). Maintenance burden was high and only azalea-bot
    supported MC 26.2 natively.
  - Decision: Delete mod-bridge and real routes from source. Only azalea-bot
    remains, gated by the `azalea-bot` cargo feature.
  - Consequences: Single codepath, no Java dependency, native 26.2 support.
    All tutorials and docs updated to reflect the single route.

- **ADR-002: Send + Sync tool boundary for parallel execution**
  - Date: 2026-07-15
  - Status: accepted
  - Context: Tools execute in parallel batches grouped by `ToolEffects`.
    Need thread-safe tool handles.
  - Decision: `GameTool` requires `Send + Sync`. Tools share state via
    `Arc<Mutex<...>>` adapters.
  - Consequences: READ tools run in parallel; WRITE tools run sequentially
    with BARRIER splitting.

- **ADR-003: System prompt byte stability for DeepSeek prefix cache**
  - Date: 2026-07-18
  - Status: accepted
  - Context: DeepSeek prefix cache requires byte-identical system prompts.
    Dynamic variables (obs_streak, knowledge_bootstrapped) were breaking cache.
  - Decision: System prompt is static; dynamic content goes into
    `build_dynamic_instructions_msg()` user messages.
  - Consequences: 94%+ cache hit rate. Regression test
    `regression_system_prompt_byte_stable_across_obs_streak` guards against regression.

- **ADR-004: LLM-driven tool calls instead of Java GoalEngine** (supersedes PLAN.md v1)
  - Date: 2026-07-27
  - Status: accepted
  - Context: Original PLAN.md proposed "LLM sends goals, Java Mod auto-decomposes".
    This required a Java mod (deleted in ADR-001) and removed LLM agency.
  - Decision: LLM directly controls the bot via 44 atomic tools. Bot tools
    only do what they can; failures return `Err` with resolution steps for
    the LLM to plan around (Mindcraft philosophy).
  - Consequences: No goal decomposition engine needed. LLM plans multi-step
    synthesis. Bot tools stay simple and atomic. See `AGENTS.md` section 9-bis.

## Template

```text
## ADR-XXX: Title

Date: YYYY-MM-DD
Status: accepted

Context
Decision
Consequences
```

## Historical (superseded)

- ~~ADR-001: Mod-bridge as primary structured control path~~ — superseded by
  current ADR-001 (azalea-bot only, 2026-07-23).
- ~~ADR-002: Real machine path preserved for visual validation~~ — superseded
  by current ADR-001 (real route deleted, 2026-07-23).
