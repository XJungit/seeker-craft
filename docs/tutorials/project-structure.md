# Project Structure

This guide explains the main directories and crates in Craft-Agent.

## Top Level

- `crates/` -- Rust workspace members.
- `mods/` -- Minecraft mods used at runtime.
- `config/` -- backend and agent configuration files.
- `docs/` -- developer tutorials and design archives.
- `references/` -- external reference repos not built by the workspace.

## Crates

- `craft-agent` -- generic game agent runtime.
- `craft-agent-model` -- vision and LLM clients.
- `craft-agent-minecraft` -- Minecraft adapters and tools.
- `craft-agent-viewer` -- session visualizer.

## Mods

- `craft-agent-bridge/` -- main Fabric mod bridge.
- `craft-agent-bridge-1.21/` -- MC 1.21 compatibility branch.

## References

- `pi_agent_rust_study/` -- upstream reference project.

For architecture details, see `../ARCHITECTURE.md`.
