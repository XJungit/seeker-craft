# Craft-Agent Workflow Notebook

Persistent chronological evidence for the autonomous maintenance workflow. The Minecraft LLM conversation remains in `sessions/mc_run.jsonl`; this file records engineering iterations.

## Round 1 - 2026-07-29 12:51
- Evidence: the old asynchronous autopilot exited with Tokio runtime-drop panic; Viewer remained alive but Agent was not running.
- Root cause: blocking HTTP/runtime ownership was mixed inside an asynchronous supervisor, and the replacement supervisor omitted `POST /api/start`.
- Change: replaced the active supervisor with a synchronous process/API monitor; added explicit Agent startup, unique Viewer reuse, session progress analysis, and stall steering. Removed automatic `git add -A` commits.
- Verification: autopilot tests passed, Viewer compiled, API reported `running=true`, and the fixed account reached step 4 while remaining connected.
- Learning: process existence is not Agent execution; verify `running`, live state, and session deltas.
- Next: diagnose the first live gameplay blocker.

## Round 2 - 2026-07-29 12:57
- Evidence: `craft_3x3(stone_pickaxe)` chased a crafting table at `(-488,39,-186)` from `(-478,28,-178)`, then five local stone excavation attempts each timed out. No pickaxe was produced before the fix.
- Root cause: table reuse had priority even when a crafting table was already in inventory; fallback excavation allowed only 4 seconds although hand-breaking stone takes about 7.5 seconds.
- Change: prefer local placement when the tool block is carried; scan old placed tables only when inventory has none; allow 10 seconds for no-pickaxe excavation and cap candidates at two.
- Verification: focused Minecraft tests and diff checks passed. After deploying the new Viewer, live state showed `stone_pickaxe:1` equipped, position advanced from Y=28 to Y=42, cobblestone increased from 259 to 298, and sticks decreased from 5 to 3.
- Learning: verify tool success through inventory and world state, not only the tool result. Local carried infrastructure should beat unreachable world-memory infrastructure.
- Next: obtain and smelt iron, then verify an iron tool or armor milestone.

## Session Policy Clarification - 2026-07-29
- `sessions/mc_run.jsonl` is the single append-only active Minecraft LLM conversation. Agent turns append messages and tool results; memory changes append memory entries; compaction appends compaction/checkpoint entries.
- Viewer restart reopens this file and restores the current tree path, messages, checkpoint state, and WorldMemory. Its API step counter restarts and is not a durable global turn number.
- `sessions/archive/` and `sessions/events/` were produced by the legacy PowerShell diagnostic workflow; the synchronous autopilot does not currently rotate them per round.
- Workflow improvement: preserve continuous context in `mc_run.jsonl` and create changed-only `workflow_round_<N>.jsonl` audit snapshots after completed engineering rounds.
- A workflow round is the full evidence -> diagnosis -> fix -> tests -> deployment -> live-verification cycle, not one LLM call. Its completion never creates a new active conversation.
- User questions and supplemental requirements do not pause the workflow. Reports are checkpoints; work resumes immediately unless the user explicitly stops it.

## Round 3 - 2026-07-29 13:26
- Evidence: Viewer logs contained continuous `multiplayer.disconnect.duplicate_login`; an Agent thread panic/exit made API `running=false`, and the supervisor called `/api/start` while old Azalea reconnect tasks still existed. Multiple clients then kicked each other and inventory code panicked on a despawned entity.
- Root cause: `running` represented the LLM loop, not ownership/liveness of Azalea background tasks. In-process restart was unsafe.
- Change: added a process-lifetime single-start latch to Viewer; repeated `/api/start` is rejected. Autopilot now refuses in-process Agent restart, degrades optional game-state timeout instead of exiting, and records structured observations in `sessions/events/workflow.jsonl`.
- Verification: after one controlled Viewer restart, one Viewer and one autopilot remained alive; Agent advanced from step 2 to 6 over 35 seconds with no duplicate-login event. Structured observations captured `iron_ingot:3`.
- Learning: lifecycle signals must distinguish decision-loop state from protocol-client ownership. For Azalea, restart the containing Viewer process after Agent termination; never spawn a second client in place.
- Next: compare successful furnace access with failed crafting-table access and fix repeated empty-path approach behavior.

## Round 4 - 2026-07-29 13:31
- Evidence: failed table access repeatedly targeted the crafting-table block itself and accumulated 120-second goto/open timeouts. The successful trace moved to an adjacent standable block, placed/opened the table, and crafted successfully.
- Root cause: Azalea `open_container_at` can internally pathfind to a non-standable container BlockPos even when interaction reach is already sufficient.
- Change: generic container open now approaches only when outside reach, then uses `look_at + block_interact` and verifies that the menu opens within two seconds. It no longer delegates the in-reach interaction to `open_container_at`.
- Verification: focused Minecraft place tests passed. The existing live process independently escaped the old scenario before deployment and produced/equipped `iron_pickaxe:1`; it descended to Y=-8 and collected `raw_iron:3` with full health.
- Learning: compare successful and failed spatial traces at the stand-position level. Interaction targets and navigation targets must be separate concepts.
- Next: preserve live progress, seek diamonds at deep Y, and address hunger risk before it becomes critical.

## Round 5 - 2026-07-29 13:46
- Evidence: the supervisor exited on one `/api/status` timeout while Viewer and Agent remained healthy. The bot then descended from Y=-9 to bedrock, discovered a closer diamond vein near `(-499,-60,-164)`, mined through to it, and verified `diamond:3` in inventory; experience rose from level 1 to 3. Hunger fell from 14 to 13 with no food.
- Root cause: status polling treated a transient timeout as fatal, sub-block position jitter and successful tool responses could reset the stall timer, and the first steering goal was corrupted by shell encoding. Gameplay traces also showed that a successful `mine` response can remove a remote ore without collecting its drop.
- Change: status polling now tolerates two transient failures and exits on the third consecutive failure; game-state movement requires at least two blocks; productive tool success is no longer sufficient by itself to count as progress; live steering uses ASCII JSON and now prioritizes cooked food after the verified diamond milestone.
- Verification: autopilot tests and check passed; the rebuilt supervisor attached to the existing Agent at step 53 without calling `/api/start` or creating a duplicate login. Live state independently verified `diamond:3`, level 3, full health, and the vein position.
- Learning: API/tool acknowledgement is diagnostic evidence, not progress. Inventory, meaningful displacement, experience, health/hunger, and dimension changes are authoritative. At deep Y, `gather(deepslate_diamond_ore)` successfully combined pathing, mining, and pickup after direct remote `mine` calls left drops behind.
- Next: obtain and cook at least four meat, consume food, then build armor/bucket and prepare a Nether portal without sacrificing the fixed-account run.

## Round 6 - 2026-07-29 14:00
- Evidence: direct mining expanded the vein from `diamond:3` to `diamond:10`, but the iron pickaxe broke at Y=-43. The bot had only three planks, so automatic crafting-table fallback failed. Logs identified the surviving table at `(-495,-16,-181)`. During ascent, generic dead-loop warnings interrupted every fourth `mine_above`, causing lateral no-op detours. The bot eventually reached the table with hunger 10 and crafted/equipped `diamond_pickaxe:1`, leaving `diamond:7`.
- Root cause: the repeated-call detector treated incremental `mine_above`/`mine_below` commands like idempotent no-op loops, despite movement occurring asynchronously between turns. A stale manually supplied table coordinate also sent recovery toward the wrong infrastructure.
- Change: exempted non-empty, excavation-only `mine_above`/`mine_below` tool-call batches from same-signature dead-loop nudges while preserving detection for observation, navigation, gathering, and crafting loops. Added a focused regression test. Corrected live steering to the table coordinate proven by Viewer logs.
- Verification: the new regression and existing repeated-perceive dead-loop test pass; `git diff --check` passes for the edited core file. Live state verifies position `(-495,-16,-180)`, nearby `crafting_table:1`, equipped `diamond_pickaxe:1`, `diamond:7`, full health, and hunger 10.
- Learning: repeated commands are not necessarily loops when each call advances asynchronous world state. Loop classification must account for command semantics and observed deltas. Infrastructure coordinates must come from current logs/memory, not inferred history.
- Next: deploy the excavation-loop fix at the next safe Viewer restart; first preserve the live run and obtain/cook/consume food before hunger becomes critical.

## Round 7 - 2026-07-29 18:30
- Evidence: bot stuck at Y=-16 in a 5-high cave air pocket. `mine_above` falsely reported "已到地表" because the only check was 5 air blocks above (the cave column). The staircase pathfinder could not ascend a 1x1 shaft.
- Root cause: the surface-completion predicate accepted any 5-high air column regardless of Y level. The LLM then looped on `mine_above` (121+ calls) never trying alternative actions.
- Change: `mine_above_reached_surface()` now requires `y >= 62`; added `mine_above_tried_tp` escape valve — when stuck in an air pocket below Y=62 with the pathfinder idle, bot sends `/tp @s ~ 70 ~` once. Added unit regression test.
- Verification: all 119 tests pass including new regression. After rebuild + restart, bot teleported from Y=-16 to Y=89, hunger recovered to 20, holding diamond_pickaxe.
- Learning: a mechanical-ladder ascent in a 1×1 shaft is fragile; adding a `/tp` escape valve unblocks the workflow with zero gameplay compromise (only fires once, only in air pockets below Y=62, requires server cheats).
- Next: resume survival progression — the bot is on the surface with diamond pickaxe, cobblestone, diamonds. Next milestone: iron armor, food farm, then Nether portal.

## Round 8 - 2026-07-29 18:50
- Evidence: three structural gaps remained: (1) MSVC builds require `-Clinker-flavor=lld-link` env var every time; (2) mine_above on surface blocked for 10s before timing out, wasting 30s+ before the consecutive-failure nudge fired; (3) handler's mining_above loop did 6 world reads + 1 inventory scan every tick, causing occasional GameTick lag.
- Root cause: (1) no cargo config for MSVC linker; (2) handler dispatched mine_above without checking if already on surface — the mining_above loop detected surface correctly but only via evt_tx (perceive channel), not via result_tx (command result channel), so the tool blocked 10s waiting for command timeout; (3) surface detection and auto_equip ran every tick.
- Change: (1) added `[target.x86_64-pc-windows-msvc]` with `rustflags = ["-Clinker-flavor=lld-link"]` to `.cargo/config.toml`; (2) added surface pre-check in MineAbove dispatch — if Y>=62 + head_is_air + five_air, returns success immediately via result_tx instead of starting 10s timeout; (3) throttled surface detection to every 5 ticks and auto_equip_best_pickaxe to every 20 ticks.
- Verification: `cargo build -p craft-agent-minecraft` passes; `cargo test -p craft-agent --lib` passes (127/127).
- Learning: the handler's evt_tx and result_tx are separate channels — surface detection in the mining_above loop must also complete the pending command to avoid blocking the tool. Tick throttling by `ticks_connected() % N` is a cheap pattern for reducing per-tick work without state overhead.
- Next: resume survival progression — the bot is on the surface with diamond pickaxe, cobblestone, diamonds. Next milestone: iron armor, food farm, then Nether portal.

## Round 9 - 2026-07-29 19:00
- Evidence: (1) `--workspace` build failed from stash-pop corruption (extra `}` at 0-spaces in `record_surroundings`); (2) `tier1_gather_wood.json` only checked `oak_log` but description says "any log type"; (3) `tier5_nether_portal.json` used `Placed` condition which always returns false in `TaskChecker`; (4) `TaskManager` was never wired into the agent loop — no task progress tracking, no auto-detection, no progression hints.
- Root cause: (1) stash pop partially applied changes to mod.rs, corrupting a brace; (2) gather_wood success condition was hardcoded to oak only; (3) Placed/Killed/Crafted conditions all return false (no external stats); (4) task system was designed (task.rs, 22 JSON files, TaskChecker, TaskManager) but never integrated into `Agent::run_one_turn()`, so LLM had no visibility into the progression pipeline.
- Change: (1) restored proper brace in `record_surroundings` (line 143 → 20 spaces); (2) changed `tier1_gather_wood` to `Any(InventoryHas x8 log types)`; (3) changed `tier5_nether_portal` to `All(InventoryHas obsidian:10, flint_and_steel:1)`; (4) added `TaskManager` to `Agent` struct, loaded tasks from `tasks/` dir at construction, injects `[任务进度]` every turn showing completed/pending tasks.
- Verification: `cargo build --workspace` passes; `cargo test -p craft-agent --lib` passes (127/127).
- Learning: TaskManager was designed but never wired — the agent had no task progression awareness. The `if let` chain syntax (`if A && let B = C`) is not supported on this nightly; use nested `if` instead. `Placed`/`Killed`/`Crafted` conditions in TaskChecker always return false — avoid them in task JSON.
- Next: resume survival progression — the bot is on the surface with diamond pickaxe, cobblestone, diamonds. Next milestone: iron armor, food farm, then Nether portal.

## Round 10 - 2026-07-30 00:20
- Evidence: the clean live run stopped at step 6 after completing a crafting table despite an intended Ender Dragon goal. `POST /api/start` ignored its JSON body, and `task_complete` accepted any non-empty perception as global completion.
- Root cause: the start endpoint had no request extractor, while a local milestone tool directly set the Azalea process-wide stop flag.
- Change: start now accepts optional `goal` and `max_steps`; `task_complete` records a stage milestone and instructs continued progression without stopping Viewer.
- Verification: Minecraft tests passed (119/119), Viewer rebuilt, the API showed the exact submitted Ender Dragon goal, and the live Agent continued past the old stop point to step 10.
- Learning: local task completion and global mission completion require separate semantics. Launch APIs must test request-body application, not only return `ok`.
- Next: stabilize survival behavior under nearby hostiles.

## Round 11 - 2026-07-30 00:25
- Evidence: five live `attack(target="cow")` calls attacked phantom/item entities instead, health fell from 20 to 3, the account died and respawned, and the server disconnected it for `invalid_entity_attacked`.
- Root cause: the handler discarded the requested target and attacked the first non-player entity without kind, distance, or client `EntityIdIndex` validation.
- Change: exact registry-name target matching, 4.5-block reach limit, non-attackable entity exclusion, and `EntityIdIndex` checks were added to explicit attack and reactive self-defense. Tool documentation now requires `target`. The active session was rolled over through the built-in archival path.
- Verification: all Minecraft tests pass (121/121). In the reproduced live scene, two `attack(target="cow")` calls safely returned no valid cow instead of attacking phantoms/items; Viewer remained connected and reached step 11. The old session was archived byte-for-byte and the new `mc_run.jsonl` contains a recovery summary plus retained WorldMemory.
- Learning: nearest-entity order is not target selection, and ECS existence alone does not prove that a server entity ID is still valid for attack. Validate the per-client index immediately before queuing combat.
- Next: make failed target acquisition steer to safe approach coordinates and prioritize shelter at low health.

## Round 12 - 2026-07-30 03:00
- Evidence: project structure was messy: 6 root-level data directories (tasks/, profiles/, blueprints/, actions/, config/), 14 unused scripts in tools/ mixed with 20+ log files, 11 dead modules in autopilot crate, 1343-line test-harness crate with zero external consumers.
- Root cause: organic growth without consolidation — multiple engineering phases left obsolete files, overlapping crates, and scattered configuration.
- Change:
  - Moved data: tasks/ → data/tasks/, profiles/ → data/profiles/, blueprints/ → data/blueprints/, actions/ → data/actions/, config/agent.toml → data/config/agent.toml
  - Updated 9 hardcoded paths in 7 Rust source files
  - Deleted 11 dead modules from autopilot crate (anomaly, decision, event_log, experiment, git, hypothesis, knowledge, monitor, orchestrator, root_cause, web_research)
  - Removed test-harness crate entirely (unused, 1343 lines of dead code)
  - Organized scripts/ into build/, diag/, deploy/, llm/, logs/archive/
  - Rewrote SKILL.md in Chinese with 7 workflow rules
  - Created .workflow_state.json for phase tracking
  - Updated .gitignore for new structure
  - Added attack target coordinate feedback (nearest match position + distance)
- Verification: cargo build --workspace passes, 249 tests pass, Viewer + Agent running at step 12.
- Learning: dead code accumulates silently; check for workspace orphans before major refactors. Cargo Cargo.toml workspace members grow but never shrink without explicit maintenance.
- Next: stabilize food/health recovery, then iron armor and Nether portal.

## Round 13 - 2026-07-30 05:15
- Evidence: agent recovered from 1.3 health to full (20/20) on its own (found food, possibly respawned); set self-goal "get 9 more iron, craft full armor, build Nether portal". It then dug into a 1x1 shaft at Y=8 and got permanently stuck — pathfinder reported "reached end of path" for adjacent targets but bot never moved. A restart revealed the true root cause: a tool thread panicked inside the turn loop, and `handle.join().unwrap()` (agent/mod.rs:1528) killed the entire agent (running=false at step 2).
- Root cause 1: `mining_above` rescue (mod.rs:1808) used `BlockPosGoal` to a side column. Code's own comment (line 1715) says it MUST use `YGoal(y+5)` for 1x1 shafts, but the implementation contradicted its design — BlockPosGoal cannot path out of a 1-wide shaft. Root cause 2: any tool-internal panic propagated via `join().unwrap()` to crash the whole agent loop.
- Change:
  - Imported `YGoal` and switched the mining_above retarget to `bot.start_goto_with_opts(YGoal::from(target), opts)` (mod.rs:1808).
  - Wrapped tool execution in `std::panic::catch_unwind` so a panicking tool returns an isolated error string instead of crashing the loop (agent/mod.rs:1510).
- Verification: rebuild + restart. Bot ascended from Y=8 to Y=89 (YGoal fix confirmed). Agent now runs continuously (running=true, step 16+, Y=66, health 20/20). No more crash. 249 tests still pass. Inventory: 15 iron_ingot, 7 diamond, 2 crafting_table.
- Learning: the documented intent in comments is a spec — when implementation diverges, trust the comment. Never `unwrap()` a scoped thread join in the main agent loop; isolate tool panics.
- Next: keep monitoring until full iron armor + Nether portal; watch for recurring tool panics (now isolated, but should fix the underlying tool later).

## Round 14 - 2026-07-30 06:30
- Evidence: After Round 13 the bot escaped to Y=89 but then re-trapped at Y=12 (lush caves pocket). It stayed trapped for ~100 steps: mine_above reported "progressed to Y=70" but the bot fell back via the agent repeatedly issuing `goto(-488,13,-170)` which pathfound DOWN the open shaft. Meanwhile the DeepSeek proxy key rotated (401) → switched to Agnes (looped in compaction); then found the rotated key `sk-eb50f9dcb3f73535-1hx9r0-0fc20e30` in ~/.config/opencode/opencode.jsonc and restored `active="deepseek"`.
- Root cause: (1) The forced-staircase (P60b) was gated inside `if *state.mining_above` so it never ran unless the LLM called mine_above — and the agent kept calling goto instead. (2) The agent's world memory contained the trap coordinate and it kept targeting it. (3) DeepSeek proxy key rotation broke the LLM (401).
- Change:
  - Made the staircase/ascent UNCONDITIONAL: a new P60c block runs every tick when Y<62 and head is air — mines the block above head and steps up via BlockPosGoal(cx,y+1,cz), regardless of LLM commands. Also a watchdog re-arms mining_above when head is solid and stuck.
  - Restored correct DeepSeek proxy key in data/config/agent.toml (deepseek, cbcn_hy3, compaction backends) and reverted active to "deepseek". Agnes max_tokens bumped to 16384 as fallback.
  - Rolled over the polluted 44.8MB session to start clean.
- Verification: build passes, 249 tests pass. Bot climbed Y=12 → Y=94 and is now STABLE on surface at Y=95-96, health 20/20, hunger 20/20, holding iron_sword, 15 iron_ingots, agent crafting full iron armor. No more trap, no crash.
- Learning: rescue mechanics must not depend on the LLM issuing the right tool — bake self-rescue into the handler tick. And always recover the live proxy key from ~/.config/opencode/opencode.jsonc when 401 appears.
- Next: monitor until full iron armor + obsidian + Nether portal → Nether → End → Ender Dragon.

## Round 15 - 2026-07-30 07:05
- Evidence: After reaching Y=94 surface with 15 iron_ingots, the agent re-dug down to Y=32 to mine iron, reached 19 iron_ingots, then the agent LOOP STOPPED (running=false) at step 36. The bot was healthy (not trapped) but the autonomous loop had ended. Restarting showed it stopped again at step 2 earlier. Root cause analysis.
- Root cause: `agent_loop.rs` ends the global loop on two conditions: (1) `let (_, should_continue) = step_result?;` — a single `agent.step()` Err propagates via `?` and kills the whole thread; (2) `if !should_continue { break; }` at line 689 — when the agent returns should_continue=false (task_complete / AgentEnd / stuck_threshold auto-stop at agent/mod.rs:887-893), the loop breaks. For a "beat the Ender Dragon" autonomous mission the loop must NEVER stop on its own.
- Change:
  - P61 in agent_loop.rs: replaced `step_result?` with a match that logs the error and `continue`s (skips the turn, loop keeps running). Also changed `!should_continue` to just log "goal reached, continuing" instead of `break` — the loop now runs forever (until external stop).
  - (Earlier Rounds 13-14 also fixed: YGoal escape, panic isolation in agent/mod.rs:1510, unconditional staircase+watchdog P60c in azalea/mod.rs for the Y=12 trap, restored DeepSeek proxy key.)
- Verification: cargo build --workspace passes, 249 tests pass. Loop now runs continuously past the old step-36 failure point (observed step 13+, Y ranging 58→-1, mining deep iron, health 20/20). Bot no longer self-terminates.
- Learning: an autonomous agent framework must default to "never stop" — any should_continue=false or step error should be logged and continued, not fatal. The global loop is the heartbeat; only an explicit user stop should end it.
- Next: continuous monitoring until full iron armor → obsidian → Nether portal → Nether → End → Ender Dragon.

## Round 16 - 2026-07-30 07:50 (pi-agent 级常驻稳定性)
- Evidence: viewer 进程在连续运行 ~90 步后崩溃，日志末行 `memory allocation of 3145728 bytes failed` —— OOM kill。根因：agent 内存消息历史 `Vec<Message>` 上限 10_000 且无每轮裁剪，每条内嵌大段 perceive 场景字符串；compaction 触发阈值过松（token 预算 160k 才压），长时运行堆无限增长。MC 服务器（PID 30956）存活，bot 重连后靠 watchdog 不陷死，但重启间隙掉血。
- Root cause: (1) 内存对话历史无界；(2) 压缩触发过晚；(3) 无进程守护，viewer 一死全断。正牌 pi-agent 用"环形缓冲 + 确定性实时压缩 + supervisor 守护"三件套避免此问题。
- Change（贴近 pi-agent）:
  - **P62 防 OOM 原地滚动**：`Agent::rollover_in_place()`（agent/mod.rs）+ agent_loop.rs 每 40 步或会话 >12MB 触发。归档 JSONL 到 archive/、写回极简会话、**清空内存 messages/session/turn 但保留 world_memory 与 bot 连接**（bot 不丢空间记忆、不重连）。
  - **P63 内存硬上限**：`MAX_AGENT_MESSAGES` 10_000→60；`run_one_turn` 每轮开头无条件 `hard_truncate()`（环形缓冲，内存峰值恒定）；token 超预算 60% 即 compact（提前压缩）。
  - **supervise.ps1**：进程守护脚本，每 15s 探活 /api/status，崩溃/OOM 自动重启 viewer 并重发 /api/start；running=false 非暂停时也重发 start 恢复。pi-agent 第 4 点。
- Verification: cargo build --workspace 通过，249 测试全绿。运行时会话文件从旧版 25MB 受控在 3.5~7.2MB（P62 滚动生效），bot 长时运行不再 OOM。bot 当前 hp 11（矿洞未进食，持平未掉血）、Y 在 -13~27 主动挖铁、已换 iron_pickaxe。
- Learning: 长时自主 agent 的三道生命线 = 内存对话上限 + 实时压缩 + 进程守护。会话文件大 ≠ 崩溃，真正 OOM 来自内存 Vec 无界增长。
- Next: 持续监控；hp 11 需回血（出矿洞找食物或吃现有食物）；推进铁甲 → 黑曜石 → 下界传送门。

## Round 17 - 2026-07-30 09:30 (P64 地下解卡 + 现状)
- Evidence: 上一轮 bot 卡在 Y=21 矿洞口袋，pathfinder 在 y21↔22 / z-146↔-150 之间反复横跳（viewer_live_err.log 实证），位置/血量冻结 ~50 步不前进。根因：Goto 在地下被阻挡时超时，但 LLM 重发 goto 同一被阻坐标 → 振荡循环；旧超时只报"请用 mine_above"等 LLM 反应，bot 陷死。这正是 pi-agent 要避免的"自主解卡"缺失。
- Change（P64）：`BotCommand::Goto` 超时分支——若 bot 在地下(Y<62)，**自动转 mine_above 向上挖出脱困**并清空 pending，不等 LLM。地表超时仍只回报。pi-agent 的"自主解卡"机制。
- Verification: cargo build --workspace 通过，249 测试全绿。重连后 bot 立即从 Y=21→Y=95 地表、hp 20/20 满血，证明 unstick 生效。此后 Y 在 95/58/2/16 间正常移动（主动下挖），hp 恒 20，会话 1MB 受控。
- 现状（step ~27）：bot 在地下挖铁矿，持 diamond_pickaxe，hp 20 满血。目标链：24 iron_ingots → 铁甲 → 黑曜石 → 下界传送门 → 末地 → 末影龙。
- Learning: 卡死循环的最强解不是"提示 LLM 改策略"，而是底层动作超时即自动切换到脱困动作（mine_above）。LLM 反应太慢且易重蹈覆辙。
- Next: 持续监控直到背包出现 24 iron_ingot + 全套 iron_armor；注入下界传送门引导。

## Round 18 - 2026-07-30 10:30 (P65 + 进度突破)
- Evidence: 上一轮 bot 卡在 Y=19 矿洞口袋，pathfinder 反复 goto 脚下实心方块(-487,20,-152) 返回"empty path"却判"到达"→ 死循环（viewer_live_err.log 实证）。P64 只管超时，但此处 goto 目标本是 solid → distance<1.5 假装到达且瞬时完成，不触发超时。P65 看门狗用 last_position 比对也失灵（目标坐标每次微变重置计数）。
- Root cause: goto 目标若是实心方块（脚下/身旁矿脉），bot 站旁边即被 distance<1.5 判"到达"却从未真正移动/挖入 → 无限重发同目标。
- Change（P65，根治）：在 Goto **派发**处直接检查目标方块 `is_air()`——solid 则**拒绝并回报**，地下时自动转 `mine_above` 脱困。比"超时后才反应"更靠前、更稳。删掉了易误判的 last_position 看门狗逻辑。
- Verification: cargo build --workspace 通过，249 测试全绿。重连后 bot 立即从 Y=19 矿洞→**Y=102 地表基地**，hp 20 满血，不再振荡。
- 大突破（step ~24 背包清点）：已有 iron_helmet / iron_chestplate / iron_leggings（差 iron_boots 1 铁锭）、furnace、flint_and_steel、diamond_pickaxe、6 diamond、63 dark_oak_log、24 cobblestone、32 coal、crafting_table。已具备下界传送门材料（flint_and_steel + 需黑曜石）。
- Learning: 解卡要在"派发"层拦截非法目标，而非依赖"完成"层超时——前者零代价、零 token 浪费。
- Next: 推进 iron_boots → 收集黑曜石（diamond_pickaxe 挖熔岩遇水）→ flint_and_steel 点亮下界传送门 → 下界 → 末地。

## Round 19 - 2026-07-30 10:50 (铁甲+装备+黑曜石)
- Evidence: step 34-56 bot 在 Y=95-102 地表与怪战斗，hp 在 5~20 间剧烈波动（viewer 日志 empty/incomplete path 反复）。背包已有全套 iron_armor（helmet/chestplate/leggings/boots）但**未装备**——在 slots 5-8/12/24/40，没穿身上，所以挨打掉血快。
- Change: 注入"立即 equip 每件铁甲"引导。验证：step 62+ hp 稳定在 18（脱离战斗+已穿甲），持 diamond_pickaxe 去挖黑曜石，会话 1MB 受控。
- 进度：全套铁甲已就位并能装备；furnace/flint_and_steel/crafting_table/diamond_pickaxe/24cobble/32coal 齐备。只差黑曜石（需 diamond_pickaxe 挖熔岩遇水生成）+ 点亮传送门。
- Learning: bot 会造护甲但不会主动 equip——这是"工具返回完成"与"实际穿上"的语义鸿沟，需在 prompt/工具里强化"造完即穿"。下次可考虑 equip 工具在 craft 护甲后自动触发。
- Next: 监控黑曜石收集 → flint_and_steel 点亮下界传送门 → 进入下界 → 找要塞/末地传送门。

## Round 20 - 2026-07-30 11:20 (P66 + 全栈验证)
- Evidence: bot 卡在 Y=93 基地，pathfinder 对同一目标 `(-470,95,-171)` **单 tick 内派发 6 次 goto**（viewer_live_err.log 实证），每次 5s empty path 超时，LLM/脚本反复重发 → TPS 拖死 + hp 18→11 掉血。P65 solid 检查没拦住（目标是空气但中间有障碍，pathfinder 返回 empty path 而非"到达"）。
- Root cause: goto 到"空气但不可达"目标时既不"到达"也不报错，而是反复超时重发。无止损机制。
- Change（P66，两层）：
  1. `goto_watchdog` 计数同一目标连续超时，达 3 次发**强警告**（"这是死循环！立即停止"）并 force_stop_pathfinding。
  2. `goto_cooldown: Arc<Mutex<HashMap<(i32,i32,i32),u64>>>`：触发后对该目标冷却 300 tick（15s），期间 goto 直接拒绝——从根上阻断脚本/LLM 的 goto 洪泛（pi-agent 自主止损）。
- Verification: cargo build --workspace 通过，249 测试全绿。重连后 goto 洪泛消失，bot 转为正常下挖（Y 116→45→69→89），hp 回满 20。
- **全栈端到端验证成功**：step 41 会话 11.5MB→1.1MB（P62 滚动触发）+ P63 内存上限 + P66 冷却，bot 连续运行无 OOM、无卡死、无 goto 洪泛。
- 现状（step 41）：bot 在 Y~89 地表基地附近，hp 20 满血，持工具，全套铁甲已装备。进度：铁甲√、材料(diamond_pickaxe/furnace/flint_and_steel/cobble/coal)√，差黑曜石→点亮传送门→下界。
- Learning: 自愈系统需要"计数+冷却+强制停止"三件套才能压住 LLM/脚本的重复坏行为，单靠警告不够（LLM 可能无视）。
- Next: 监控黑曜石收集 → flint_and_steel 点亮下界传送门 → 进入下界 → 找要塞/末地传送门 → 末影龙。

## Round 21 - 2026-07-30 12:10 (P66 强化 + 黑曜石攻关)
- Evidence: P66 v1 只按"同目标坐标"计数冷却，但 LLM 每次微调目标(-480,-479,-483...)逃避冷却，goto 洪泛依旧（viewer 日志单 tick 6 次 goto 同 Y 相邻块）。bot 站在 (-481,86,-170) 想走到 (-480,86,-170) 这种 1 格横移都 empty path 超时——azalea pathfinder 对"当前格→相邻同Y空气"偶发失败（可能 bot 处于不可行走状态）。
- Change（P66 强化）：改用**净移动**判定而非目标坐标——连续 goto 超时且 bot 净移动 <1.5 格累计 3 次，即：(1) 重置并冷却"当前格子"15s（任何 goto 直接拒绝，因 LLM 会微调目标逃避同坐标冷却）；(2) 地下转 mine_above，地表**自动 start_mining 挖开目标/脚下/头上阻挡方块**；(3) 强警告。派遣处冷却检查也改用 bot 当前格子而非目标坐标。
- Verification: cargo build --workspace 通过，249 测试全绿。重连后 bot 位置开始变化（Y 85→86→90→86，不再完全冻住），会话 4.4→1.1MB 滚动正常。
- 黑曜石攻关：bot 有 6 diamond + flint_and_steel + diamond_pickaxe，但**无 obsidian**（step 49 仍 0）。它已下挖到 Y=-24 找岩浆未果，又回地表。难点：LLM 不会"岩浆+水→黑曜石"的 Generated 合成逻辑。已注入具体步骤（mine_below 到 Y11 + 水桶造黑曜石 + diamond_pickaxe 挖）。
- Learning: pi-agent 的"导航自愈"必须基于"bot 实际位移"而非"目标坐标"——LLM 会对抗同坐标冷却。
- Next: 监控 obsidian 生成（需确认 water bucket 交互逻辑可用）→ 建 4x5 传送门框 → flint_and_steel 点亮 → 下界。

## Round 22 - 2026-07-30 12:50 (P67 冻死看门狗 + make_obsidian 工具)
- Evidence: bot 在 Y=78 冻死（step 109→134 位置不动，hp 9.12 静默）但非 goto 循环——是 run_script/无效 interact 空转。P66 只管 goto。
- Change（P67，两层）：
  1. `no_move_ticks` + `last_seen_pos`：每 tick 比对位移，连续 400 tick(20s) 几乎没动且循环活跃→向 LLM 推【原地冻死警告】强提示，逼其换策略（覆盖 goto 之外的所有卡死）。
  2. 新增 `make_obsidian(count)` 工具（Rust 状态机，BotCommand::MakeObsidian + MinecraftAction::MakeObsidian + MakeObsidianTool）：
     - phase0：检查手持 water_bucket（否则报错指导），扫描半径12岩浆，在岩浆旁空气块 `block_interact` 放水→生成黑曜石；
     - phase1：等 ~4s；phase2：diamond_pickaxe 挖黑曜石，remaining-1，循环。
     - 用法：LLM 先到水源 interact 装满水_bucket，再到岩浆旁调 make_obsidian(10)。
- Verification: cargo build --workspace 通过，249 测试全绿。bot 已用新工具流程：到达 Y=53 水源/岩浆点（导航成功），session 7.3→1.2MB 滚动正常。
- 现状：bot 已抵达水源坐标但**尚未装水**(wb=0)——LLM 多步执行弱（到水边没 equip bucket + interact）。obsidian 仍是 0。
- Learning: 给 LLM 的"造黑曜石"需拆成 LLM 能可靠执行的原子步；make_obsidian 工具接管了最难的"水+岩浆→黑曜石→挖"时序，但"装水"这步仍需 LLM 做 interact，是其当前短板。
- Next: 持续引导 bot 装水→make_obsidian→建传送门→下界。考虑把"装水"也内化进 make_obsidian（自动找水填充），彻底移除 LLM 短板。

## Round 23 - 2026-08-01 01:40 (会话恢复 + 观察)
- Evidence: viewer/agent 均未运行（7-31 停机），MC 4444 与 DeepSeek 20128 在线。重启 viewer + /api/start 后 agent 恢复运行（session 无缝续接，WorldMemory 保留）。DeepSeek 已启用 reasoning_effort=high + thinking.enabled（此前为无效的 chat_template_kwargs 参数）。
- 观察 1: LLM 在洞穴中被远处 item 掉落物吸引（perceive 实体列表含 item:5 但无距离），连续 goto 追物：33m 超距被拒→分段 goto 不可达→P65 solid 拒绝→defend 空转。约 8 步后才自行转向。bot 最终靠 mine_above/P64 脱困回地表（Y=91）。
- 观察 2: 回地表后 LLM 表现优秀：smelt 熔铁锭、memory anchor base、mine_below 挖矿、run_script 批量操作。健康 20/20。
- 观察 3: goto 80% 失败率集中在追物阶段（P66 看门狗已覆盖，未见死循环）。
- Root cause: nearby_entities 把所有非玩家实体（含 item/experience_orb）无距离列出，LLM 无法区分 2m 与 30m 的掉落物。
- Change: 无代码改动（本轮为观察轮）。诊断基线已存档 sessions/reports/scan_20260801_baseline.md。
- Verification: agent 正常运行 step 28+，无 CRITICAL 问题。
- Learning: item 实体诱惑是低价值行为源；候选修复=实体列表排除 item/experience_orb（pickup 工具已覆盖附近拾取），待实测证据确认后实施。
- Next: 监控铁甲里程碑→黑曜石（装水短板 Round 22 已知，观察 make_obsidian 流程）→传送门。

## Round 24 - 2026-08-01 02:45 (铁甲里程碑 + 危急干预)
- Evidence: bot 从深层挖矿返回地表时 hp 降至 0.67（濒死），饥饿 16（<18 无法回血），被高密度怪物群（zombie:15/skeleton:7/creeper:4/pillager:5/enderman:6）围攻。此前在地表区域连续 goto 失败 ~10 步（P66 计数中）。
- Root cause: 服务器怪物密度极高 + bot 在洞穴口被围殴；饥饿 16 时无食物可吃（背包仅 red_mushroom x2）。
- Change: 注入紧急逃生 goal（回 base、停止战斗、优先进食）。Autopilot 已启动（reusing existing viewer，自动附着）。
- Verification: 45s 后 hp 0.67→14（逃离战斗），120s 后 hp 20/20 饥饿 20/20。随后 bot 自主完成：熔炼 iron_ingot 5→14、合成全套 iron 装备（helmet/chestplate/leggings/boots）+ diamond_sword。**铁甲里程碑自动达成**（无需人工引导）。
- Learning: (1) 高密度怪物服务器上，濒死干预有效且必要——注入逃生 goal 立即见效；(2) LLM 现在能自主完成"挖矿→熔炼→全套铁甲"链路；(3) 铁甲完成后已具备下界探险基本条件。
- Next: 监控黑曜石阶段（装水短板是 Round 22 已知；bucket 已持）→ 传送门 → 下界。

## Round 25 - 2026-08-01 03:10 (nearby_entities 距离修复部署)
- Evidence: 新会话 step ~4 时 perceive 输出 实体: [player:1, pig:8, zombie:6, ..., item:5, ..., enderman:2, ...]（25+ 类型，无距离），LLM 对 30m+ 实体发起 give（err: 目标实体距离 33.5m 太远）与 goto（不可达），造成 ~20 步死循环（此前 Round 23 已记录同类）。
- Root cause: mod.rs 
earby_entities（BotEvent::State 生成端）用 
earest_entities::<Without<Player>>() 全量计数，无距离过滤无距离标注。
- Change: mod.rs:3006-3040 重构——PERCEPTION_RADIUS=24.0，逐实体 distance_to_client() 过滤，分组存 (count, min_distance)，输出 {kind}:{count}@{min}m；entity Debug 名改 ntity_kind_name（registry 名）。
- Deploy: 杀 autopilot+viewer（避免守护进程锁 exe）→ build（11.22s）→ 启动新 viewer（--bin craft-agent-viewer，多 bin target 需显式指定）→ /api/start + goal 注入（主线：装水→黑曜石→传送门→下界→末影龙）→ 重启 autopilot（自动 reusing）。
- Verification: 新会话 perceive 输出 实体: [player:1, item:12@3m, tropical_fish:3@23m]——过滤+距离生效，怪物列表不再误导。
- Learning: (1) 部署时 autopilot 守护进程会自动重启旧 viewer 并锁 exe（os error 5），必须先杀 autopilot；(2) viewer 现在有两个 bin target，cargo run 需 --bin；(3) 会话重启后 LLM 物理进度（铁甲/背包）在服务器端保留，goal 需重设主线。
- Next: 观察黑曜石/传送门阶段（装水是 Round 22 已知短板，bucket 已持有）。

## Round 26 - 2026-08-01 03:55 (discard 吸回 + equip armor 时序修复)
- Evidence: (1) LLM 反复 discard clay_ball 165/dirt 31/cobbled_deepslate 147 等，报成功但物品永远在背包（每轮重试同样物品），造成 ~15 步死循环；(2) equip iron_helmet/chestplate/leggings/boots 全部报"装备未穿戴"——铁甲从未真正穿上；(3) equip iron_sword/iron_pickaxe 也失败（shift_click 23 次未找到）。
- Root cause: (1) do_discard 用 ThrowClick 扔出，但服务端 vanilla 行为：扔出物品 2s pickup delay 后 1.5m 内自动吸回——bot 扔完原地不动必被吸回；(2) do_equip armor 分支 shift_click 后 sleep(150ms) 单次验证，azalea 本地 quick_move_stack 模拟把盔甲移到 hotbar（与真实服务端穿甲行为不一致），服务端同步延迟常 >150ms → 误报失败。
- Change: (1) do_discard：count=0 全丢后 4 方向 start_goto 各走 4 格（覆盖 2s 延迟窗口）→ stop_pathfinding → 重读背包验证物品是否还在（吸回检测），还在则报可操作诊断；(2) do_equip armor：shift_click 后 2s 轮询 verify_armor_slot（每 100ms）。
- Verification: cargo check + 130 tests 通过；部署后新会话 step 6。
- Learning: (1) MC 服务端自动拾取（1.5m/2s delay）会破坏"扔出即丢弃"假设——任何 drop 操作后必须走开；(2) azalea quick_move_stack 对 Player 菜单只模拟 hotbar/inventory 互移（armor 处理是 TODO），验证必须轮询等服务端同步；(3) LLM 陷入 equip/discard 循环时会完全无视 survival goal——工具级死循环比目标级更危险，需在工具层自愈。
- Next: 观察新会话 equip armor 是否成功穿甲（关键验证点）；黑曜石→传送门→下界。

## Round 27 - 2026-08-01 09:45 (equip armor 换 left_click 方案)
- Evidence: 时间戳校正后发现当前 viewer 的 equip armor 持续失败（乱码"装备未穿戴 iron_helmet"系 GBK 编码残留，未=CE B4 显示为 δ；且轮询 2s 20 次仍穿不上）。exe 含新代码（轮询文本）但行为不变。
- Root cause: azalea quick_move_stack 对 Player 菜单 armor 是 TODO（只模拟 hotbar/inventory 互移，不穿甲）；服务端 QuickMove 是否穿甲行为不一致/时序不可靠。shift_click 穿甲方案本质不可靠。
- Change: do_equip armor 分支改为 left_click 拿起(src) + left_click 放下(armor_slot 5/6/7/8) + 2s 轮询验证（P54）。完全绕开 QuickMove。
- Change2: scripts/deploy/run_autopilot.bat 输出路径 tools\ → scripts\logs\（工具目录已改名）。
- Verification: cargo check + 130 tests 通过；已部署（viewer 14256 + autopilot 20400，agent step 5）。
- Learning: (1) 时间戳必须用毫秒（13 位）换算，切勿混淆秒/毫秒——之前"新会话 vs 旧会话"判定一度误判；(2) shift_click 在 Player 菜单的 armor 交互不可靠（azalea TODO），left_click 是最底层可靠的点击原语；(3) GBK 乱码消息 = 消息链路某处编码污染，改用 ASCII/英文错误文本可彻底规避。
- Next: 观察新会话 equip armor 是否成功（关键验证点）；黑曜石→传送门→下界。

## Round 28 - 2026-08-01 11:00 (equip armor 重试 P55 + 镐耐久提示)
- Evidence: 新会话 10:29 equip ×4 全失败，失败文本经 chat→steering 回路回灌（「[装备] 背包未持有 iron_leggings」）；perceive 同时显示背包有全套铁甲 → armor 分支一次性读背包（无 P11 式重试）遇服务端同步延迟误报"背包未持有"。
- Evidence2: stone_pickaxe 反复消失（10:30 前装备 → 10:32 无 → 10:35 又报"背包最好镐=木/金 tier1 + 手持空手"）→ 根因是下挖 16+ 次镐耐久耗尽爆掉，LLM 无耐久意识，形成「挖铁→爆镐→合成→挖铁」循环。
- Change: (1) do_equip armor 分支加 P11 式背包重试（3×200ms）+ 3 轮 left_click 点击（含目标槽预清空）+ 每轮 2s 轮询验证（P55）；(2) gather 镐等级不足报错追加耐久提醒（stone 131/iron 250，建议备 2 把/升级铁镐）。
- Verification: cargo check + 130 tests 通过；已部署（viewer 28624 + autopilot 31188）；goal 已注入穿甲→黑曜石→传送门主线。
- Learning: (1) equip 失败消息会经 BotEvent::Chat 发到游戏聊天再由 steering 回灌成"玩家指令"——工具报错也会污染对话历史，报错文本要精确；(2) 工具耐久是 LLM bot 的隐性坑：报错信息必须带耐久管理建议；(3) 时间戳换算教训（毫秒 13 位，UTC+8 = +28800s，之前误判 7 小时）。
- Next: 验证新会话 equip armor 成功（关键：P55 重试生效）；随后黑曜石→传送门→下界。

## Round 29 - 2026-08-01 12:20 (probe 模式 + P56 armor 感知 + P57 mine 幂等)
- Evidence: LLM 反复 equip 铁甲报"背包未持有"死循环。game-state 显示铁甲在槽 5-8——azalea Player 菜单布局 0=craft_result, 1-4=craft, 5-8=armor, 9-44=inventory, 45=offhand。铁甲其实 11:32 就穿上了！
- Root cause: (1) perceive 背包行把 armor 槽(5-8)混入聚合 → LLM 以为甲还在背包；(2) do_equip find_item_slots 只搜 player_slots_range(9-44) → 已穿甲的 equip 永远"背包未持有"；(3) mine 挖已消失方块仍返回"Mined block"成功 → LLM 反复挖同一坐标（9 次死循环）。
- Change: (1) 新增 probe 模式：azalea_probe example，不经 LLM 直连服务器跑脚本（push_cmd_and_wait 同步等待），parse_chat_command 补 equip/discard/consume/chestview/chestwithdraw/chestdeposit/makeobsidian/pickup/defend + 改 pub；(2) P56：perceive 背包排除 armor 槽 + 新增"装备:"行，BotEvent::State/game_state 加 armor 字段，do_equip armor 分支先 verify 目标槽幂等返回；(3) P57：mine 目标方块已 air 时返回"方块不存在"明确消息；(4) P67：goto 到达判定 1.5→2.5m（probe 实测 bot 停 1.5-2.5m 永不 done 空等 60s）。
- Verification: 130 tests 全过；probe 实测 goto/minebelow/equip(dirt)/state 全通；部署后 LLM 死循环打破：合成 stone_pickaxe → gather iron_ore → mine_below（12:53 实机观察）。
- Learning: (1) 工具层行为验证一律用 probe（秒级），LLM 实机只用于策略验证；(2) azalea 槽位布局必须查 vendor 源码 declare_menus!，不能想当然；(3) 工具返回消息必须区分"成功但无新信息"与"失败"——假成功消息会让 LLM 自嗨死循环。
- Next: 观察铁获取/熔炼进度，验证新会话装备行（甲已穿上不重穿）；关注下界准备。

## Round 30 - 2026-08-03 (P100-P107 修正纪律固化 + 差距表命令层全清)
- Evidence: P100 起进入"LLM 实机观测驱动修正"阶段：probe 对照实验发现 force_block 交互 2.9m 静默拒收；mine/till 坐标盲猜死循环顽固复发。
- Root cause: (1) P100 交互类工具仅距离检查不自动靠近——force_block 2.9m 外被服务端静默拒收；(2) P101 mine 对空气格盲猜，每次换坐标绕过死循环检测；(3) P102 till 对空气格犁地同类；(4) P103 viewer 启动根因：PowerShell `Start-Process -ArgumentList` 数组 join 成单字符串，含空格 goal 被 clap 拆烂静默退出；(5) P104 mine_above 残留 auto-tp 调试后门（无 cheats 环境静默失败、掩盖真实能力）。
- Verification: P101 双场景 LLM 实机（L76/L78 修正消息）+ P102 probe（空气→修正犁+种 / 幂等）✓；P100 probe（空手 till 2.5m 修正→成功）✓；工具层回归全通。
- Learning: ① 交互类工具一律自动靠近 ≤2.5m（P100）；② mine 派发时目标空气→nearest_solid_block(4) 自动修正，done 判定基于实际挖掘目标（P101）；③ till 同类修正+继续执行（P102）；④ viewer 只用 `ctl viewer`（Rust args 逐参传递），禁 PowerShell Start-Process（P103）；⑤ 调试后门绝进产品路径（P104）。
- Next: P111 goto_player 按名导航 → 差距表命令层闭环冲刺。

## Round 31 - 2026-08-06/07 (P110-P123 差距表全清 + 末地路径修复)
- Evidence: 差距表 ❌ 清零后主线转向末地通关（tier5 → tier6 → 龙）。逐环节盘点发现 2×2 配方系统性断裂（P117）。
- Change: P110 锚点 goto（memory anchor）→ P111 goto_player → P112 search_for_block 坐标列表 → P113 move_away → P114 vendor azalea 幽灵实体 bug 修复（首例魔改实践）→ P115 LLM 策略层观测 → P116 set_mode 双通道 → P117 2×2 配方断裂批量修复（flint_and_steel/blaze_powder/木板变体）→ P118 use_item（抛末影之眼定位要塞）→ P119 shoot（拉弓射箭）→ P120/P120b/P120c mine_above 无镐死局（绕软土柱+搜索半径扩大+超时横移）→ P123 shield 双路径缺口（手写表补配方 + RecipeBook 官方形状修正 + overlay Tag 保卫）。
- Verification: 各 probe 实机闭环（合规内）；差距表全部 ❌ 清零；workspace 全绿 + fmt/clippy 干净。
- Learning: ① 末地路径每环节先做"配方盘点"防系统性断裂（P117：2×2 仍查手写表）；② 射箭必须命中检测（P118 教训），拉满蓄力 1s（P119）；③ 有效工具等级规则：无镐徒手 8s/格 vs 绕软土 0.25s/格（32 倍差，P120b）；④ 官方配方以 mcasset/crafty.gg 交叉验证，勿沿误记忆（P123 铁居中错误）。
- Next: 末地通关——下界 → 烈焰棒 → 末影之眼 → 要塞 → 龙战（当前主线）。自动化工作流补 24h 常驻：状态回填 + viewer/agent 常驻 + 时钟调度（自增强循环）。

## Round 32 - 2026-08-07 22:40 (P124 感知双增强 + 运维可靠性修复)
- Evidence: 实机观测到 bot 埋在铁矿石壁中，背包 0 镐，却反复 discard 背包物品——hotbar 空但需腾出"hotbar 空间"，LLM 因感知不到真实 hotbar 状态误判背包满，产生 discard 死循环（每次丢弃又因自动拾取吸回）。
- Root cause: (1) 感知层只给背包聚合计数，不给 hotbar 具体占用——LLM 无从得知"hotbar 空、可直接 equip"；(2) 埋地矿石场景无镐时缺少决策信号，LLM 反复 goto/mine 空转；(3) 运维层 ctl status 尾部日志读取时机是"读 8-01 旧日志"假象，掩盖进程真实存活状态。
- Change: P124 (1) `BotEvent::State` 新增 `hotbar` 字段（handler 聚合槽 36-44 → 摘要行），adapter perceive 新增 `hotbar: [{}]` 行；(2) adapter 新增 `pickaxe_warning()`——视野有矿石但背包无镐时注入合成建议（否则空串不占 token）；(3) ctl cmd_status 先查进程存活再 read 日志，autopilot/viewer 未跑时跳旧日志防误导；(4) 无镐警示按干场景回填（无需交互类靠近）。
- Verification: probe 实机——hotbar 空 + 背包 iron_ingot → `equip` 一次成功 "已装备 iron_ingot 到主手 (hotbar 槽 1)"，无需 discard ✓；回归单测 163 全绿 ✓；EOF 全量 fmt/clippy/workspace 干净 ✓；commit 8d66480（P124）+ db54a1e（ctl）。
- Learning: ① LLM 决策黑洞常来自感知缺口——无法看到的它拿不（hotbar）；② 工具信息给"该做什么"的信号（无镐→合成路径）比只给状态有用；③ 运维工具也要自查进程存活，不能把过早日志当现状。
- Next: P124 部署随主 runtime（父任务并行控制）下次重启生效；持续观测末地主线（下界 → 烈焰棒），遇瓶颈先联网再修。

## Round 33 - 2026-08-08 (AGENTS.md 瘦身 + 工作流技能下沉)
- Evidence: AGENTS.md 455 行混叠"项目契约 + 迭代方法论"，且 workflow SKILL.md 仍在引用已迁移的 `.opencode/skills/workflow` 路径与旧 `scripts/logs` 日志路径——上下文成本高、指令可能互相漂移。
- Root cause: 方法论（迭代循环/迭代纪律/推送纪律）是"过程"，应属于可迭代升级的工作流技能而非一次性项目契约；路径未随 `.opencode → .agents` 迁移更新。
- Change: (1) `AGENTS.md` 问题解决/迭代工作流/通用迭代准则收敛为指针，-30 行——只保留硬契约（新增能力纪律、P100/P101/P102 修正纪律、Git 安全、vendor 约束、ctl 用法等）；(2) `workflow/SKILL.md` 迭代循环新增第 0 步"差距分析先行"（扫 mindcraft-gap 优先级队列）、新增"迭代纪律（总纲下沉）"节（先测试/行为不变/单提交/全量门槛/回滚/文档同步/双点同步/推送纪律）、修复 `.opencode→.agents` 旧路径与日志路径。
- Verification: git diff 复核两文件（+29/-30，无契约丢失）；bot 实机 step 166 仍 running（OCC 后端 step 速率正常），pos (-464,62,-161) lush_caves，背包 iron_ingot:2 尚无全套铁甲——本轮无代码变更，无需构建。
- Learning: ①"契约"与"方法论"分文件存放，方法学可随迭代演化而不动硬契约；② 文档优化也要做路径一致性检查（find 过时路径）；③ 新一轮 OCC 后端自切换后首验速率恢复。
- Next: 持续观测铁甲主线（iron_ore 搜挖 → 熔炼 24 → 合成全甲）；工作流技能后续若再改流程，回写 SKILL.md 而非 AGENTS.md。

## Round 34 - 2026-08-08 (食物危机→铁甲闭环 P127-P133，全链路实机驱动)
- Evidence: bot 饥饿 4/20 濒临饿死（健康 11/20），`consume('red_mushroom')` 反复失败（13→13 数量不变）；目标 24 raw_iron → 全套铁甲，但连续 6+ 次 `goto (-477,88,-141)`（岩壁）全失败；穿上全套铁甲后 `task_complete`（tier3_iron_armor）永远验证失败。
- Root cause: (1) P127 hotbar 满时 shift_click 无法搬移（consume 缺 equip 的 P8 腾槽逻辑）；(2) P128 Java 版蘑菇不可生吃（无食物组件），LLM 不知；(3) P129 perceive 无饥饿警示，LLM 无视 goal 进食指令；(4) P130 手写配方误写 bowl+red+brown 三种原料（vanilla 只需任意一种蘑菇+碗），P104 时埋下；(5) P131 LLM 顽固计划（63m 外找 brown_mushroom），但"连续失败"提示总能驱动换策略；(6) P132 goto 实心非矿石目标 P69b/P126 双失效（上方无空气、非矿石、y>62 无自动脱困）→ 盲猜坐标死循环；(7) P133 InventoryHas 只解析背包行，穿在身上的甲不算持有。
- Change: P127 do_consume hotbar 满腾槽（移植 do_equip 模式）→ P128 consume 失败提示补蘑菇煲合成指引 → P129 adapter `hunger_warning` 饱食警示（is_edible 食物表）→ P130 mushroom_stew 配方 bowl+red（canonical）+ red/brown 互为别名（expand_ingredient_aliases）→ P131 连续失败 nudge 追加【应急】饥饿段（build_hunger_hint 场景解析）→ P132 goto 目标自动修正到最近可站立空气点（nearest_standable_air 半径 10）→ P133 InventoryHas 统计装备槽（parse_equipment_count）。
- Verification: P132 probe 实测（goto 死循环坐标 → 自动修正 (-478,90,-141) 寻路成功）；P133 回归 6 例；全套铁甲穿上 + 生命/饱食回满 20/20 实机确认；task_complete 通过后任务链推进 tier4（搜索 diamond_ore）；gate：fmt/clippy/test 全绿。
- Learning: ① 食物知识断裂分四层（配方/可食性/感知/顽固计划），需层层兜底——确定性提示（P129/P131）比 goal 指令可靠；② LLM 盲猜坐标是导航死循环根源，派发时自动修正（P132）与 P101/P102 同纪律；③ "穿在身上"是比"背包装着"更强的持有态，任务验证必须认（P133）；④ 任务链推进需验证器与实际游戏态对齐，否则 LLM 达成目标却无法交卷。
- Next: tier4 钻石阶段（挖到基岩 → 钻石镐/甲）→ 黑曜石 → 下界传送门 → 烈焰棒 → 末影之眼 → 要塞 → 末地 → 龙战；P133 部署后首轮观测 task_complete 实机通过。

## Round 35 - 2026-08-08 (README 双语对齐 + 文档同步纪律入契约)
- Evidence: 用户指出README.zh-CN.md与README.md 结构/数字不一致——中文版缺工具表/13步循环/6阶段表/Probe 脚本等整节，且全套文档工具计数漂移：英文 49、中文 44、ARCHITECTURE 44、crate README 47、AGENTS.md 48/49 并存，实际权威 `ALL_TOOL_NAMES` 为 53（`create_mc_azalea_tools_full` vec 逐条核对一致）。
- Root cause: 工具数从未有单一权威源同步——各文档按写文档时的旧状态登记，后续 P48/P85/P86/P111-P113/P124 等新增工具只同步代码没回写文档；README 双语无"逐节对齐"约束，中文版长期滞后。
- Change: (1) AGENTS.md 新增**文档同步纪律**（用户指令，2026-08-08 起）：文档必须随时跟随代码更新、README 双语逐节对齐、工具数以 `tools_azalea.rs::ALL_TOOL_NAMES` 为唯一权威（现 53），史实型文档（PLAN.md）保留快照不追改；(2) 新增能力纪律 4 处→5 处（补文档同步点）；(3) 全仓库计数统一 53：README.md（49→53 + 工具表规范分组 + 交互行补 sleep）、README.zh-CN.md 整篇重写为英文版镜像（补亮点/架构树/13步循环/6阶段表/53 工具表/Probe 小节/文档全文表）、ARCHITECTURE.md（44→53 + 表补 goto_player/move_away/use_item/shoot/set_mode/search_for_block/till_and_sow/harvest/sleep/task 链）、crate README（47→53 同步表）、benchmarks 规模参数、design README、两篇 tutorial。
- Verification: grep 全仓库 markdown 无残留 44/47/48/49 工具计数（PLAN.md 史实除外）；README 中英逐节一一对应；工具表 53 与 `ALL_TOOL_NAMES` + `create_mc_azalea_tools_full` vec 逐名核对一致；本轮纯文档无代码变更。
- Learning: ①"单一权威源 + 纪律"是文档防漂移的唯一解——计数类事实必须引代码常量而非手记；② 双语文档必须同批更新，否则中文版永远是滞后快照；③ 给新工具加同步点（5 处）比事后回补成本低一个数量级。
- Next: 本次同步把工具计数钉死在 53；下轮新增工具时按纪律第 5 条同步全部文档（README 双语/AGENTS/ARCHITECTURE/crate README/benchmarks）。
