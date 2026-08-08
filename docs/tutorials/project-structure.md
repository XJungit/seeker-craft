# Project Structure

This guide explains the main directories and crates in Craft-Agent.

## Top Level

- `crates/` — Rust workspace members.
- `config/` — backend and agent configuration files (`agent.toml`).
- `profiles/` — system prompt and MC knowledge base (`_default.json`).
- `docs/` — developer tutorials and design archives.
- `tools/` — PowerShell automation scripts (`auto_diag.ps1`, `scan_run.ps1`, etc.).
- `sessions/` — session JSONL files and diagnostic reports.
- `vendor/` — vendored azalea crates (independent git repo + workspace).
- `references/` — external reference repos not built by the workspace (mindcraft, pi_agent_rust_study).

## Crates

- `craft-agent` — generic game agent runtime: Agent main loop (13 steps),
  tool registry, session, compaction, prompt assembly, modes, WorldMemory.
- `craft-agent-model` — LLM/VLM clients and config model.
- `craft-agent-minecraft` — Minecraft adapter and 53 LLM tools (azalea-bot route).
- `craft-agent-viewer` — Axum + SSE Web dashboard for runtime visualization.

## Removed Directories

- `mods/` — (removed) the old Fabric mod bridge; azalea route needs no mod.
- `craft-agent-bridge` — (removed) Java mod source; azalea route needs no Java.

## Vendor

- `vendor/azalea/` — vendored copy of the azalea MC client protocol library.
  Independent git repository + independent cargo workspace. See `AGENTS.md`
  section 8.2 for the correct workflow when modifying vendor code.

## References

- `reference/mindcraft/` — Mindcraft reference source (skills.js, etc.).
- `reference/pi_agent_rust_study/` — upstream pi reference project.

For architecture details, see [`../ARCHITECTURE.md`](../ARCHITECTURE.md).
For automation workflow, see [`../../AGENTS.md`](../../AGENTS.md).
