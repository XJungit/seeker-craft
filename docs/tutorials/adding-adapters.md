# Adding Adapters

This guide explains how to add a new game adapter or runtime path.

## GameAdapter trait

Implement `GameAdapter` with:
- `perceive() -> WorldState`
- `execute(action) -> ExecResult`
- optional `capture() -> Screenshot`

## Runtime selection

Use `McAgentBuilder` as a reference for wiring:
- `McAdapter::ModBridge` for structured TCP bridge
- `McAdapter::Real` for screenshot + input

## Steps

1. Add the adapter module under `craft-agent-minecraft/src/`.
2. Implement `GameAdapter`.
3. Add tooling in a new or existing `tools_*` module.
4. Register tools in a factory function.
5. Update `McAgentBuilder` if you want builder support.

## Example

See `adapter_azalea.rs` and `tools_azalea.rs` for the azalea-bot reference implementation.
