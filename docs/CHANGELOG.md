# Changelog

This file tracks notable changes to Craft-Agent.

## 2026-08-03

- **P2 structural evolution (stability-first)** — architecture refactors with zero behavior change (all 395 tests green):
  - **P2.1** `run_one_turn` split into `execute_batch` (batch grouping / READ parallel / WRITE serial / slow-tool probe) + `finalize_abort` (P89/P90/P94/P99 four branches converge into `AbortDecision::{Reroute, Handoff}`).
  - **P2.2** `azalea/mod.rs` (6340 lines) split into `azalea/commands.rs` (BotCommand 33 variants + QueuedCommand + parse_chat_command + chat_parser tests) and `azalea/handler.rs` (BotState + tick handler + helpers); mod.rs down to 1995 lines, re-exported via `pub use` with zero external churn.
  - **P2.3** `craft-agent-model` boundary: only depends on `craft_agent::core::{message,types}`; CI quality job enforces with `cargo check -p craft-agent-model --no-default-features`.
- **P100 till_and_sow auto-approach** — force_block interactions silently fail beyond ~2.9m (server rejects); till now auto-walks within 2m before interacting (probe-verified).
- **P101 mine air-target auto-correction + P57 false-report fix** — LLM was blind-guessing coordinates and mining air 15+ times (each new coordinate bypasses dead-loop detection). Dispatch now auto-corrects air targets to the nearest solid block; done-polling and feedback are based on the actual mined target (`last_mine_eff`), with three scenarios: solid-mined / air-corrected-mined / air-no-solid suggestion. Probe-verified; live-verified in real LLM sessions.
- **P102 till_and_sow target correction** — LLM tilled air 4 times in a row; non-tillable targets now auto-correct to the nearest tillable block (radius 4, y±1) and continue tilling+sowing with an explicit correction notice; distance checks use the corrected position. Probe-verified.

## 2026-08-02

- **P81 unstuck enhancement** — 3+ consecutive failed/invalid tool calls (goto timeout / air mining / gather no-resource) trigger mode_id=7 guidance (mine_above to surface / change direction / return to base); 5+ forces a re-prompt.
- **P82 hotbar cache fallback** — `find_hotbar_slot` hit but `set_selected_hotbar_slot` leaves the wrong item in hand (local slot cache lags the server) → `force_hold_in_hotbar` shift-clicks back and retries; wired into do_equip/do_place.
- **P83 perception + knowledge injection** — `overhead_solid` (contiguous solid blocks above head) in BotEvent::State and perceive; UNDERGROUND & CAVE SURVIVAL knowledge section in the default prompt (mushroom stew recipe, keep seeds, mine_above escape, no poison food).
- **P84 tillAndSow farming** — new `till_and_sow` tool (validate dirt/grass/farmland → auto-approach → hoe till → seed sow → idempotent); probe-verified full path including "A Seedy Place" achievement.
- **P85 sleep** — new `sleep` tool: find bed → approach ≤2m → empty main hand → right-click → verify SleepingPos → wait to wake. Two bugs fixed during testing (absolute hotbar slot panic; sleeping check used fox metadata instead of player SleepingPos).
- **P86 harvest** — new `harvest` tool: scan 32m for mature crops (age 7 / nether_wart 3) → approach → mine → pick up; probe-verified (immature skipped, mature harvested +1 wheat).
- **P87 pvp strafing + bare-hand attack fix** — self_defense strafes around the target (radial 1.8m + tangential 2.0m) with a 40-tick cooldown; critical fix: the old code unconditionally `continue`d without a weapon (bot stood still getting bitten, never retaliating) — now attacks bare-handed.
- **P88 raw-state channel + melee overhaul** — (1) `RawState` bot command dumps raw azalea state (`RAW|` prefix, no LLM exposure) to cross-validate the perception renderer (conclusion: renderer is bug-free); (2) P88-approach: attack() beyond 3.2m always misses → high-priority goto approach to 2m first; (3) P88-b: attack check interval 5s→1s; (4) P88-c: low-HP counter-attack when the enemy is within 3.2m (no more flee-while-bitten); (5) P88-d: skip attacks when approach conditions aren't met (no more 7-miss creeper volleys). Live-verified.
- **P88-e ghost player mystery solved** — "3 players online" was the probe + the host player (Jun, idling underground) + the bot itself; not a bug.
- **P89 in-turn failure re-planning** — when a WRITE (side-effect) tool fails: abort remaining batch, fill `【已中止】` placeholder tool messages (OpenAI requires a response per tool_call or 400), inject a re-planning nudge with cause + suggestion, re-call the LLM in the same turn (reroute_max=2, read-only failures don't roll back).
- **P90 steering interrupt** — steering goals abort remaining batches and re-route the same turn (thread-safe injection queue, no more placeholder-gap 400s).
- **P91 incremental summary rebuild** — second compaction round reuses `previous_summary` via `<previous-summary>` XML block + UPDATE_SUMMARIZATION_PROMPT instead of a full re-summarize.
- **P92 unified failure prefix** — `Message::to_chatml()` prefixes tool results with `失败` when `is_error=true` (fail/err/no/缺/未/失败/超时/拒/无效).
- **P93 progress events** — `BotEvent::Progress { command, detail }` every 20 ticks (goto = remaining distance, mine = remaining Y), displayed in demo/probe; default-noop in adapter.
- **P94 tool budget guard** — hard cap of 20 tool calls per turn; excess aborted with a convergence nudge (no more 25-call placeholder floods).
- **P95 cancel API** — `AzaleaBot.cancel_commands()`: drains queue + notifies waiters + atomic cancel_flag; used by steering/sleep.
- **P96 background pre-fetch compaction** — when estimated tokens exceed 40% of budget, prefetch the summary in a background thread (needs 60% to trigger); `compact()` picks it up immediately.
- **P97 semantic memory (pi-memory port)** — `remember` tool (Agent::new auto-registers), top-4 injection as 【近期记忆】 user message, tag-based ranking + recency decay, JSONL persistence, scope (global vs per-server), 5-turn injection cooldown. Live-verified: prompt cache hit 42624-43584 / miss 2796-3326 (>93% prefix cache hit rate).
- **P97b live-validation fixes** — scope string normalization (`scope_is_global()`), system prompt version drift (1.21.2 → 1.26.2), remember tool guidance in prompt, ctl kill_all safety.
- **P98 context-management overhaul** — A1 few-shot real message pairs (no more text-paste pseudo tool calls, which the LLM imitated); B3 unified transient-message stripping (single registry, ~30 prefixes); B4 memory injection cooldown; B5 compact task progress rendering; A2 staged knowledge (6 tiers, tier-filtered); C7 configurable jailbreak; C8 knowledge string caching (byte-stable prefix for DeepSeek).
- **P99 slow-tool single-action turns** — `GameTool::is_slow()` + 12 slow tools; a batch containing a slow tool executes it then immediately aborts the remaining predicted calls with `【已中止】` placeholders (no re-call — the result is already backfilled; next turn's auto-perceive drives fresh decisions). Plus P89b: fix a UTF-8 boundary panic in the re-planning nudge (`&fmsg[..len.min(160)]` → `chars().take(160)`).

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
