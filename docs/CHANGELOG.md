# Changelog

This file tracks notable changes to Craft-Agent.

## 2026-07-27

- **P55: gather partial success returns Ok** — `gather.rs` now returns `Ok` for
  partial success (e.g. 14/16 collected) instead of `Err`. Aligns with the
  Mindcraft philosophy that tools return what they can do; the LLM decides
  next steps. Fixed a 100% failure rate observed in scan_20260727_205138.md.
- **P56: plain-text reply governance** — `is_premature_completion` in
  `agent/mod.rs` extended with 9+ keywords (✅, 任务完成, 已验证, 最终确认,
  smelt/craft/gather/mine 任务). Profile system prompt adds rule #4:
  "禁止中间宣告完成". Prevents the LLM from wasting turns declaring success
  without tool calls.
- **P57: smelt batch smelting** — `do_smelt` now caps single-batch smelting
  at 8 items (95s < 120s tool timeout). Previously `smelt(count=15)` timed
  out at 120s (15 × 10s = 150s). The return message tells the LLM to call
  `smelt` again for the remaining items.
- **P58: intercept set_goal("") bypass** — When the LLM calls
  `set_goal(goal="")` while its text declares "task complete ✅", the agent
  refuses `stop_goal()` and injects a mandatory perceive-verification nudge.
  Closes a bypass of P56 detected in real sessions.
- **Mock container integration tests** — `craft.rs::tests` now includes
  `MockInventory` / `MockFurnace` pure-function models and `smelt_decide` /
  `craft_3x3_decide` decision functions. 43 tests cover all Mindcraft
  `skills.js` boundary conditions without needing a Minecraft server.
- **RecipeBook integration (P48)** — `craft_3x3` now looks up recipes from
  `recipe_book.rs` (vanilla 26.2 full recipe book) first, falling back to
  handwritten `SHAPED_RECIPES`.
- **smelt takeOutput polling (P49)** — `do_smelt` now polls the result slot
  at 1s intervals (was: fixed 30s wait) with 11s no-output timeout. Aligns
  with Mindcraft's `takeOutput` loop.
- **AGENTS.md section 9-bis: Mindcraft philosophy** — Added four-铁律 rules:
  no auto-crafting tool blocks, no auto-crafting tools, no auto-satisfying
  material deps, error messages must list complete resolution steps.
- **Documentation overhaul** — Rewrote `PLAN.md`, `ARCHITECTURE.md`,
  `crates/craft-agent-minecraft/README.md`, `crates/craft-agent-viewer/README.md`,
  `docs/tutorials/adding-tools.md`, `docs/tutorials/adding-adapters.md`,
  `docs/tutorials/configuration.md`, `docs/adr.md`. Updated `agent-loop.md`,
  `troubleshooting.md`, `project-structure.md`, `session-and-compaction.md`,
  `new-contributor-guide.md`, `getting-started.md`, `docs/design/README.md`,
  `crates/craft-agent/README.md`, root `README.md`.

## 2026-07-23

- **Removed mod-bridge & real routes from source**: The Fabric mod TCP bridge
  (`craft-agent-bridge` Java mod) and the `real` VLM+enigo path were deleted.
  The only supported route is now **azalea-bot** (Rust client bot, `azalea-bot` feature).
- Viewer (`craft-agent-viewer`) rewired to the azalea adapter; VLM screenshots removed.

## 2026-07-18

- **DeepSeek cache optimization**: Moved jailbreak variables (obs_streak, bootstrap)
  out of system prompt into dynamic user messages. System prompt now byte-identical
  across all turns for 94%+ prefix cache hit rate.
- **Crate-level READMEs**: Added `README.md` to all 4 crates (`craft-agent`,
  `craft-agent-minecraft`, `craft-agent-model`, `craft-agent-viewer`) and
  `mods/craft-agent-bridge/`.
- **Doc updates**: RELEASING.md, SECURITY.md, troubleshooting guide, session
  & compaction doc updated with DeepSeek cache guidance.
- **Outdated doc markers**: `game-agent-design.md` and `docs/design/mod-bridge.md`
  tagged with ⚠️ warnings pointing to current docs.

## 2026-07-16

- Added `docs/` with tutorials and design archive.
- Added `McAgentBuilder` for unified mod-bridge and real paths.
- Made tool implementations `Send + Sync`.
- Cleaned compiler warnings in `craft-agent-minecraft`.

## 2026-07-15

- Upgraded tool coverage toward Mindcraft parity.
- Added survival checks and inventory helpers.

## 2026-07-14

- Audited existing implementation gaps vs pi reference.
- Added session compaction and retry improvements.
