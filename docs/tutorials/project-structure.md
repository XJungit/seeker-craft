# Project Structure

This guide explains the main directories and crates in SeekerCraft (Craft-Agent) v1.0.

## Top Level

- `Cargo.toml` — workspace root (nightly-2026-07-21, resolver 3).
- `crates/` — Rust workspace members (5 crates).
- `data/` — runtime data: tasks, profiles, blueprints, actions, DSH preset template, config template.
- `docs/` — architecture, tutorials, ADRs, design archives.
- `scripts/` — one-shot setup/start/stop scripts + probe JSON test scripts + CI helpers.
- `tools/dsh-bridge/` — the DSH bridge plugin (game_state / bot_tool / set_goal + dashboard).
- `vendor/azalea/` — local mirror of the maintained azalea fork (submodule, independent git repo + workspace).
- `sessions/` — session JSONL files (gitignored runtime data).
- `.github/workflows/` — CI (fmt+clippy), tests, security audit, docs deploy.

## Crates

- `craft-agent` — generic game-agent logic library: types, GameTool, ToolRegistry, WorldMemory,
  session archive format, task system, profiles, skills. No I/O; shared by viewer and adapters.
- `craft-agent-minecraft` — Minecraft adapter (azalea protocol): the bot, 49 LLM tools,
  azalea domain modules (`commands.rs` / `handler.rs` / `mod.rs`), WorldMemory scanner.
- `craft-agent-viewer` — Axum + SSE web dashboard + DSH bridge endpoints
  (`/api/connect`, `/api/bot_tool`, `/api/game-state`, `/api/goal`).
- `craft-agent-ctl` — ops console: `status|stop|build|deploy|goal|start|viewer|session|tail|health`.
  All paths derived at runtime from the crate location — no machine-specific hard-coding.

## Data

- `data/config/agent.example.toml` — legacy LLM backend template from the in-bot era
  (kept for reference; not used in DSH mode — the LLM comes from DSH).
- `data/tasks/` — 23 structured task JSONs (tier 1-6), machine-checkable completion conditions.
- `data/profiles/` — prompt templates (`_default.json` + per-provider overrides).
- `data/blueprints/` — build blueprints (JSON).
- `data/actions/` — LLM-defined rhai scripts (JSON-wrapped).
- `data/dsh/craft-bot-preset/` — DSH craft-bot preset **source of truth** (rc.3 directory
  format: `agent.cordis.yml` + `preset.yml`, with `{{PROJECT_ROOT}}` / `{{DSH_PKG_ROOT}}`
  placeholders). On DSH 0.1.5-rc.3, `setup.ps1` expands it into
  `~/.dsh/.agent-presets/craft-bot`.
- `data/dsh/craft-bot-preset-017/` — the same preset as a DSH **0.1.7+ bundle**: `package.json`
  declares `dsh.bundle.patch`, and `cordis.patch.yml` is **generated** from the template above
  by `scripts/gen-craft-bot-preset-017.mjs` (never hand-edit it). Verify with
  `node scripts/verify-craft-bot-preset-017.mjs <repo-root> <dsh-pkg-root>`.

## Scripts

- `scripts/setup.ps1` — one-shot install/configure (idempotent): prerequisites check →
  `cargo build --workspace` → DSH bridge plugin registration → craft-bot preset generation →
  `.env` copy → plugin verification. Flags: `-SkipBuild`, `-SkipDsh`.
- `scripts/start.ps1` — one-shot start: viewer (via `craft-agent-ctl viewer`) → connect bot →
  poll until ready. Params: `-Goal`, `-Steps`, `-Port`, `-Mc`, `-Username`.
- `scripts/stop.ps1` — stop viewer via `craft-agent-ctl stop`.
- `scripts/probe/*.json` — tool-layer live-test scripts (no LLM; seconds instead of minutes).

## Vendor / azalea

- `vendor/azalea/` — local mirror of the maintained fork `XJungit/azalea` (`craft-agent` branch).
  Independent git repository + independent cargo workspace. The manifest declares the fork's
  https source with a pinned rev; a gitignored `.cargo/config.toml` `[patch]` redirects to the
  local mirror during development. Fresh clones compile without the patch.
- Fork-update workflow: see [`../ARCHITECTURE.md`](../ARCHITECTURE.md) → "azalea fork maintenance".

For architecture details, see [`../ARCHITECTURE.md`](../ARCHITECTURE.md).
