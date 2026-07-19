# Craft-Agent Architecture

This document explains the high-level architecture of Craft-Agent and how the
main crates interact at runtime.

## Layered Overview

```text
app / viewer / examples
        |
        v
  craft-agent-minecraft
        |
        v
    craft-agent
        |
        v
   external services
```

- `craft-agent` defines the generic game-agent runtime, session model,
  tool abstraction, and prompt/session management.
- `craft-agent-minecraft` adds Minecraft-specific adapters and tools.
- `craft-agent-model` provides vision/LLM clients used by adapters.
- `craft-agent-viewer` visualizes runtime sessions.

## Runtime Flow

1. Build or restore a `Session`.
2. Create an `Agent` with a provider, tools, and config.
3. Run turns until the goal is satisfied or the turn limit is reached.
4. Persist session entries for later review or resume.

## Key Abstractions

- `GameAdapter`: capture, perceive, execute.
- `GameTool`: name, description, parameters, effects, execute.
- `SessionEntry`: tool call, state snapshot, compaction metadata.
- `AgentEvent`: external hook for viewers and controllers.

See `game-agent-design.md` for the original product design and goals.
