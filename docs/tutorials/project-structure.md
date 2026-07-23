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

- `mods/` -- (removed) the old Fabric mod bridge; azalea route needs no mod.

## References

- `pi_agent_rust_study/` -- upstream reference project.

For architecture details, see `../ARCHITECTURE.md`.
