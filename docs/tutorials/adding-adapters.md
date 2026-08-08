# Adding Adapters

This guide explains how to add a new game adapter for a non-Minecraft game.
The current Minecraft implementation lives in `craft-agent-minecraft` and uses
the azalea-bot route exclusively.

## GameAdapter trait

Implement `GameAdapter` with:
- `perceive() -> WorldState` — return structured game state for the LLM
- `execute(action) -> ExecResult` — execute an action and return result
- optional `capture() -> Screenshot` — for visual debugging (optional)

## Current Reference Implementation

The only supported runtime is **azalea-bot**:

- Adapter: `MinecraftAzaleaAdapter` in `crates/craft-agent-minecraft/src/adapter_azalea.rs`
- Tools: 53 LLM tools registered in `create_mc_azalea_tools()` in `tools_azalea.rs`
- Bot runtime: `AzaleaBot` in `crates/craft-agent-minecraft/src/azalea/mod.rs`

The old `mod-bridge` (Fabric mod TCP bridge) and `real` (VLM + enigo) routes have
been removed from source. See `docs/adr.md` ADR-004 for the deprecation rationale.

## Steps to Add a New Game Adapter

1. Create a new crate `craft-agent-<game>` under `crates/`.
2. Add the adapter module: `crates/craft-agent-<game>/src/adapter_<game>.rs`.
3. Implement `GameAdapter` for your game's state/execution model.
4. Add tooling in `crates/craft-agent-<game>/src/tools_<game>.rs`.
5. Register tools in a factory function (e.g. `create_mc_<game>_tools()`).
6. Add a `README.md` to the new crate describing tools and modules.
7. Wire up the new crate in the workspace `Cargo.toml`.

## Example

See `adapter_azalea.rs` and `tools_azalea.rs` for the Minecraft reference
implementation. The azalea-bot route is the canonical example of:

- Connecting to a game server (azalea TCP protocol)
- Polling game state for `perceive()` (position, health, inventory, nearby blocks)
- Executing tool calls via game-native APIs (azalea pathfinder, mining, container ops)
- Handling background reactions (modes system: fire escape, auto-attack)

## Key Patterns from the Azalea Adapter

- **Two-layer modes**: Agent layer injects `[MODE: ...]` prompts; Handler layer
  directly executes emergency actions without LLM involvement.
- **WorldMemory**: spatial memory keyed by `MemoryPos`, chunk-indexed for O(1)
  nearby queries, rendered as a 64-block radius each turn.
- **Mindcraft philosophy**: tools are atomic; the LLM plans multi-step tasks.
  See `AGENTS.md` section 9-bis.
- **Mock container tests**: pure-function state machine models validate tool
  decision logic without needing a live game server.

For architecture details, see [`../../ARCHITECTURE.md`](../../ARCHITECTURE.md).
