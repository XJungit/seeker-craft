# Changelog

All notable changes to **SeekerCraft (Craft-Agent)** are documented here.

Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) conventions
with [Semantic Versioning](https://semver.org/spec/v2.0.0.html). The project is
currently in active development as a single-maintainer project; `v1.0.0` is the
first tagged **1.0 release** (DSH bridge mode is the only supported usage).

## [1.1.0] - 2026-08-15

### Added

- **P156 semantic memory (`remember` tool, 53→54 tools)** — cross-session semantic
  long-term memory persisted to `data/memory/agent.jsonl`: `remember` tool
  (save/forget/list, kind=tactics/strategy/insight/preference, scope-aware) +
  `SemanticMemoryTool` wired into the DSH bridge factory
  (`create_mc_azalea_tools_full_with_semantic`); nearby spatial WorldMemory rendered
  into the perceive scene (`记忆: [已知世界记忆·邻近]` with resource/hazard/structure
  entries + `__self__` anchor). Live-verified via viewer `/api/bot_tool`.

### Changed

- **Tool count 53 → 54** across docs — README bilingual, ARCHITECTURE, crate READMEs,
  benchmarks, dsh-bridge README, ADR; `ALL_TOOL_NAMES` (authoritative) and the
  dsh-bridge `TOOL_NAMES` mirror both list 54, verified by `verify-in-harness.mjs`.

## [1.0.0] - 2026-08-15

### Added

- **1.0 one-shot scripts** — `scripts/setup.ps1` (prereq check → build → DSH bridge
  plugin → craft-bot preset → verify), `scripts/start.ps1` (viewer + connect bot),
  `scripts/stop.ps1` (stop viewer/autopilot). Fresh-clone friendly.
- **DSH craft-bot preset** — `data/dsh/craft-bot-preset/` template; `setup.ps1` generates
  `~/.dsh/.agent-presets/craft-bot` from it (substituting `{{PROJECT_ROOT}}` / `{{DSH_PKG_ROOT}}`).
- **azalea fork dependency** — azalea moved to the maintained fork
  `XJungit/azalea` (`craft-agent` branch), https source + pinned rev `e384e70`; a fresh
  clone compiles with **no local patch**. Fork-update workflow documented in
  `ARCHITECTURE.md` → "azalea fork maintenance".
- **Portable `craft-agent-ctl`** — all machine-specific paths (`VIEWER_EXE` /
  `AUTOPILOT_EXE` / `SESSION` / `WORKSPACE` / `LOG_DIR`) replaced with runtime derivation
  from `CARGO_MANIFEST_DIR` (clone anywhere; `SEEKER_LOG_DIR` overrides the log dir).

### Changed

- **Workspace version → 1.0.0** (was 0.1.0); `craft-agent-viewer` bumped to match.
- **Repo URL corrected** — `Cargo.toml` `repository` → `https://github.com/XJungit/seeker-craft`
  (was stale `anomalyco/craft-agent`).
- **Hard-coded paths removed** — `scripts/llm/switch_llm.js` / `revert_llm.js` derive the
  repo root at runtime; `tools/dsh-bridge/scripts/verify-in-harness.mjs` auto-detects the DSH
  install root (env `DSH_NPX_ROOT` override); `tools/dsh-bridge/README.md` install docs point
  to `setup.ps1` with generic `<repo-root>` placeholders.
- **Obsolete deploy scripts removed** — `scripts/deploy/*` (hard-coded machine paths,
  superseded by `craft-agent-ctl` + `scripts/start.ps1`).
- **Docs rewritten for 1.0** — bilingual README Quick Start → DSH bridge mode
  (setup/start/stop + craft-bot preset + three tools); tutorials (getting-started /
  project-structure / INDEX / README) updated; ARCHITECTURE azalea fork section added.

## [0.2.0] - 2026-08-10

### Added

- **P136 hard-coded version-data sweep** — ore Y-layer knowledge driven by `y_range_hint`
  (diamond −64~16 densest −59, emerald mountains-only −16~320, iron −64~384, coal −64~256,
  etc., per MC 26.2); live-probe-verified: out-of-range hints for diamond/emerald, no false
  positives in-range for iron/coal.
- **Tier 4 live milestone** — full iron armor + diamond sword + shield equipped; bot
  followed Y-layer hints (mine_below to Y≤16), descended to the diamond layer (Y≈−59), and
  located diamond_ore via `search_for_block` (live LLM session).
- **P126 Mindcraft learning batch (4 items, user-driven)** — prompt/tooling parity with Mindcraft:
  - **P126a tool knowledge grouping table** — `to_knowledge_string` rebuilt around the actual
    53 registered tool names in 12 real groups (previously referenced obsolete Mindcraft names,
    45/53 tools fell into `## Other Tools`); `ALL_TOOL_NAMES` completed (4 missing tools).
  - **P126b item-id plural fallback** — `normalize_item_id` maps singular forms
    (`oak_plank` → `oak_planks`, `wheat_seed` → `wheat_seeds`, Mindcraft parity) and all three
    recipe lookups normalize query input before matching (probe-driven fix).
  - **P126c LAST_GOALS task recap** — TaskManager keeps the last 4 completed/failed tasks and
    injects them into the dynamic context as a transient `【任务回顾】` user message (system-prompt
    byte-stable, prefix-cache safe).
  - **P126d perceive current-action label** — game state exposes the pending bot command and the
    perceive scene shows `当前动作: …` (or 空闲), matching Mindcraft `$ACTION`.
- **Documentation / repo hygiene** — README overhaul (badges, architecture diagram,
  6-stage path, 49-tool table), root-level `CHANGELOG.md`, `CITATION.cff`, `AUTHORS`;
  README bilingual progress section (live-verified tiers 1–3 complete, tier 4 in progress).
- **Engineering benchmark layer** — CI coverage job (`cargo llvm-cov` → lcov artifact),
  `docs/benchmarks.md` (410-test baseline, 52 probe scripts, cache hit rates, Ender-Dragon
  progress), and `scripts/bench/` one-shot probe runner + optional Docker repro.
- **P119 `shoot` tool** — bow combat (ReleaseUseItem) for ranged Ender Dragon phase.
- **P120 `mine_above` survivability** — when the bot has no pickaxe, dig a soft-soil
  escape column instead of hard-refusing; bare-hand fallback path added (P120, P120b).
- **P117/P117b end-game recipe fixes** — flint_and_steel and blaze_powder/plank variants
  added to the handwritten 2x2 table, fixing the broken ender-eye crafting chain.

### Changed

- **P135 recipe + Y-hint fixes** — mushroom_stew reverted to three ingredients
  (Wiki-verified, P130 correction); gather ore Y-hint corrected (removed stale 1.16 static
  data, now driven by `y_range_hint`).
- **P134 equip/discard deadlocks + tier4 bedrock task condition** — equip no longer throws
  away single pieces (keeps pickaxe), discard walks away per-slot; `BelowY(-60)` → `BelowY(-58)`
  (station on bedrock top satisfies it).
- **Repo scope** — agent work-info files (AGENTS.md, `.agents/`, `.zcode/`) no longer shipped;
  kept locally, gitignored; doc links cleaned up.
- **Security audit CI fixed** — event-listener 5.4.1 → 5.4.2 (RUSTSEC-2026-0221 unsound,
  patch available); paste / ttf-parser (unmaintained, no patch) added to ignore list with
  reachability rationale — scheduled audit no longer fails daily.

## [0.1.0] - 2026-08-03

Initial stable baseline. All work prior to the date-based changelog move is recorded
in the dated sections below.

### Added

- **P2 structural evolution (stability-first)** — architecture refactors with zero behavior change (all 395 tests green):
  - **P2.1** `run_one_turn` split into `execute_batch` (batch grouping / READ parallel / WRITE serial / slow-tool probe) + `finalize_abort` (P89/P90/P94/P99 four branches converge into `AbortDecision::{Reroute, Handoff}`).
  - **P2.2** `azalea/mod.rs` (6340 lines) split into `azalea/commands.rs` (BotCommand 33 variants + QueuedCommand + parse_chat_command + chat_parser tests) and `azalea/handler.rs` (BotState + tick handler + helpers); mod.rs down to 1995 lines, re-exported via `pub use` with zero external churn.
  - **P2.3** `craft-agent-model` boundary: only depends on `craft_agent::core::{message,types}`; CI quality job enforces with `cargo check -p craft-agent-model --no-default-features`.

### Changed

- **P100 till_and_sow auto-approach** — force_block interactions silently fail beyond ~2.9m (server rejects); till now auto-walks within 2m before interacting (probe-verified).
- **P101 mine air-target auto-correction + P57 false-report fix** — LLM was blind-guessing coordinates and mining air 15+ times (each new coordinate bypasses dead-loop detection). Dispatch now auto-corrects air targets to the nearest solid block; done-polling and feedback are based on the actual mined target (`last_mine_eff`), with three scenarios: solid-mined / air-corrected-mined / air-no-solid suggestion. Probe-verified; live-verified in real LLM sessions.
- **P102 till_and_sow target correction** — LLM tilled air 4 times in a row; non-tillable targets now auto-correct to the nearest tillable block (radius 4, y±1) and continue tilling+sowing with an explicit correction notice; distance checks use the corrected position. Probe-verified.

## 2026-08-02

### Added

- **P81 unstuck enhancement** — 3+ consecutive failed/invalid tool calls (goto timeout / air mining / gather no-resource) trigger mode_id=7 guidance (mine_above to surface / change direction / return to base); 5+ forces a re-prompt.
- **P82 hotbar cache fallback** — `find_hotbar_slot` hit but `set_selected_hotbar_slot` leaves the wrong item in hand (local slot cache lags the server) → `force_hold_in_hotbar` shift-clicks back and retries; wired into do_equip/do_place.
- **P83 perception + knowledge injection** — `overhead_solid` (contiguous solid blocks above head) in BotEvent::State and perceive; UNDERGROUND & CAVE SURVIVAL knowledge section in the default prompt (mushroom stew recipe, keep seeds, mine_above escape, no poison food).
- **P84 tillAndSow farming** — new `till_and_sow` tool (validate dirt/grass/farmland → auto-employ → hoe till → seed sow → idempotent); probe-verified full path including the "A Seedy Place" achievement.
- **P85 sleep** — new `sleep` tool: find bed → approach ≤2m → empty main hand → right-click → verify SleepingPos → wait to wake. Two bugs fixed during testing (absolute hotbar slot panic; sleeping check used fox metadata instead of player SleepingPos).
- **P86 harvest** — new `harvest` tool: scan 32m for mature crops (age 7 / nether_wart 3) → approach → mine → pick up; probe-verified (immature skipped, mature harvested +1 wheat).
- **P87 pvp strafing + bare-hand attack fix** — self_defense strafes around the target (radial 1.8m + tangential 2.0m) with a 40-tick cooldown; critical fix: the old code unconditionally `continue`d without a weapon.
- **P88 raw-state channel + melee overhaul** — (1) `RawState` bot command dumps raw azalea state (`RAW|` prefix, no LLM exposure) to cross-validate the perception renderer; (2) attack() beyond 3.2m always misses → high-priority goto approach to 2m; (3) attack check interval 5s→1s; (4) low-HP counter-attack within 3.2m; (5) skip attacks when approach conditions aren't met. Live-verified.
- **P89 in-turn failure re-planning** — when a WRITE (side-effect) tool fails: abort remaining batch, fill `【已中止】` placeholder tool messages, inject a re-planning nudge with cause + suggestion, re-call the LLM in the same turn (reroute_max=2, read-only failures don't roll back).
- **P90 steering interrupt** — steering goals abort remaining batches and re-route the same turn (thread safe, placeholder-gap 400s eliminated).
- **P91 incremental summary rebuild** — second compaction round reuses `previous_summary` via `<previous-summary>` XML block + UPDATE_SUMMARIZATION_PROMPT instead of a full re-summarize.
- **P92 unified failure prefix** — `Message::to_chatml()` prefixes tool results with `失败` when `is_error=true`.
- **P93 progress events** — `BotEvent::Progress { command, detail }` every 20 ticks, displayed in demo/probe; default-noop in adapter.
- **P94 tool budget guard** — hard cap of 20 tool calls per turn; excess aborted with a convergence cue (no more 25-call placeholder floods).
- **P95 cancel API** — `AzaleaBot.cancel_commands()`: drains queue + notifies waiters + atomic cancel_flag; used by steering/sleep.
- **P96 background pre-fetch compaction** — when estimated tokens exceed 40% of budget, prefetch the summary in a background thread (needs 60% to trigger); `compact()` picks it up immediately.
- **P97 semantic memory (runtime-port)** — `remember` tool (Agent::new auto-registers), top-4 injection as 【近期记忆】 user message, tag-based ranking + recency decay, JSONL persistence, scope (global vs per-server), 5-turn injection cooldown. Live-verified: prompt cache hit >93%.
- **P98 context-management overhaul** — few-shot real message pairs; unified transient-message stripping (single registry); memory injection cooldown; compact task progress rendering; staged knowledge (6 tiers); configurable jailbreak; knowledge string caching (byte-stable prefix for DeepSeek).
- **P99 slow-tool single-action turns** — `GameTool::is_slow()` + 12 slow tools; a batch containing a slow tool executes it then immediately aborts remaining predicted calls with `【已中止】` placeholders. Plus P89b: fixed an UTF-8 boundary panic in the re-planning nudge.

## 2026-07-27

### Changed

- **P55 gather partial success returns Ok** — `gather.rs` now returns `Ok` for partial success (e.g. 14/16 collected) instead of `Err`.
- **P56 plain-text reply governance** — `is_premature_completion` extended with keywords; profile prompt adds rule #4 ("禁止中间宣告完成").
- **P57 smelt batch smelting** — `do_smelt` caps single-batch smelting at 8 items (95s < 120s timeout); the return tells the LLM to re-call `smelt` for the rest.
- **P58 intercept set_goal("") bypass** — refusing `stop_goal()` while text declares "task complete ✅" and forcing a perceive-verification nudge.

### Added

- Mock container integration tests — `MockInventory` / `MockFurnace` pure-function models + `smelt_decide` / `craft_3x3_decide`; 43 tests cover Mindcraft `skills.js` boundary conditions without a server.
- **RecipeBook integration (P48)** — `craft_3x3` looks up vanilla 26.2 recipes first, falling back to handwritten `SHAPED_RECIPES`.
- **smelt takeOutput polling (P49)** — polls result slot at 1s intervals with 11s no-output timeout.
- **Documentation overhaul** — rewrote PLAN.md, ARCHITECTURE.md, crate READMEs, tutorials, ADR.

## 2026-07-23

- **Removed mod-bridge & real routes** — Fabric mod TCP bridge and `real` VLM+enigo path removed. Only supported route is **azalea-bot** (Rust client).
- Viewer rewired to the azalea adapter; VLM screenshots removed.

## 2026-07-18

- **DeepSeek cache optimization** — jailbreak variables moved into dynamic user messages; system prompt byte-identical across turns → >94% prefix cache hit.
- **Crate-level READMEs** and `docs/` tutorials; outdated doc markers.

## 2026-07-16

- Added `docs/` with tutorials and design archive.
- Added `McAgentBuilder` for unified mod-bridge and real paths.
- Made tool implementations `Send + Sync`; cleaned compiler warnings.

## 2026-07-15/14

- Upgraded tool coverage toward Mindcraft parity; session compaction and retry improvements.