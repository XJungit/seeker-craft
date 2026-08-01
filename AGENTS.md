# Craft-Agent

An LLM-driven Minecraft bot using the Azalea Rust client protocol. Goal: **beat the Ender Dragon**.

## Mission

Autonomously optimize and maintain this project. Authority: **full decision power**. User only evaluates final result: can the LLM beat Minecraft?

### Priorities
1. Surpass Mindcraft in framework design and stability
2. LLM bot must know MC knowledge: crafting, tools, armor, smelting, brewing, enchanting, reaching the End
3. Optimize agent harness, Azalea interfaces, automation tool suites throughout

### Problem Solving
- **First**: search the internet for solutions
- **Repeated issues**: MUST search for solutions
- Mindcraft and similar projects are reference implementations

## Build & Run

```bash
# Build (nightly only, stable fails)
cargo build --workspace

# Test
cargo test -p craft-agent --lib
cargo test -p craft-agent-minecraft --features azalea-bot --lib

# Run viewer
# 注意：craft-agent-viewer 包有两个 bin（viewer / viewer-next），必须用 --bin 指定
cargo run -p craft-agent-viewer --bin craft-agent-viewer

# Run azalea bot demo
cargo run -p craft-agent-minecraft --example agent_azalea_demo --features azalea-bot -- --goal="挖矿下探" --steps=20
```

## Architecture (5 crates)

```
craft-agent              Core agent framework (~5000 lines)
  agent/                 run_one_turn (2164 lines), compaction, prompt, modes, session
  core/                  types (MinecraftAction), GameTool trait, ToolRegistry, memory (WorldMemory), skill
  task.rs                Task system: 23 tier1-6 tasks, structured success conditions
  profile.rs             3-layer prompt merging (_default → defaults/{mode} → {individual})

craft-agent-minecraft    MC adapter (azalea protocol, ~12000+ lines)
  azalea/mod.rs          AzaleaBot + tick handler, command queue, 33 BotCommand variants
  azalea/craft.rs        2x2/3x3 crafting, smelting, smithing, stonecutter, brewing, enchanting (4706 lines)
  azalea/gather.rs       Block scanning + tool tier checks + auto-equip (568 lines)
  azalea/auto_craft.rs   Recursive recipe satisfaction + tool block placement (681 lines)
  azalea/place.rs        Block placement + container opening + reach checks (814 lines)
  azalea/recipes.rs      Recipe knowledge base (427 lines)
  azalea/perception.rs   Position reading
  azalea/actions.rs      Basic bot actions (goto/mine/chat)
  adapter_azalea.rs      GameAdapter impl, perceive format, execute
  tools_azalea.rs        44 LLM tools (3866 lines)
  action_lib.rs          LLM-defined rhai scripts (338 lines)
  blueprint.rs           Blueprint library (310 lines)

craft-agent-model        LLM client (OpenAI-compatible)
  decision.rs            chat_tools(), fold_tool_history(), parse_chat_tools_response()
  config.rs              Multi-backend config loader
  vision.rs              VLM client

craft-agent-viewer       Web dashboard (Axum + SSE)
  agent_loop.rs          Main loop: agent.step() → chat drain → idle loop

craft-agent-autopilot    Autonomous test loop (build/test → anomaly → RCA → commit)
```

## 44 LLM Tools

| Category | Tools |
|---|---|
| Perception | `perceive`, `memory`, `search_wiki` |
| Movement | `goto`, `mine_below`, `mine_above`, `pickup`, `follow`, `stop_follow` |
| Mining | `mine`, `make_obsidian` |
| Interaction | `interact_block`, `interact_entity`, `attack`, `defend` |
| Crafting | `craft`, `craft_3x3`, `smelt`, `auto_craft`, `enchant` |
| Gathering | `gather` |
| Placement | `place`, `build`, `build_blueprint`, `list_blueprints` |
| Container | `open`, `chest_view`, `chest_withdraw`, `chest_deposit` |
| Inventory | `equip`, `discard`, `consume` |
| NPC / Social | `trade`, `give` |
| Meta | `chat`, `set_goal`, `run_plan`, `run_script`, `new_action`, `list_actions`, `pause_goal`, `resume_goal`, `task_complete`, `task_retry` |

## Agent Loop (run_one_turn, 13 steps)

1. `drain_queues()` — steering/follow_up messages
2. Compaction — messages ≥ 10000 or token over budget → compact()
3. Strip ephemeral messages — perceive, memory, goal from last turn
4. Auto-perceive — inject structured state snapshot
5. Modes — check_modes() → [MODE: ...] text hint
6. SelfPrompter — inject [当前目标]
7. Dynamic context — WorldInfo + Skill + Few-shot + obs warnings
8. WorldMemory — render nearby memories (radius 64)
9. LLM call — with retry (exponential backoff)
10. Text-only check — if no tool_calls, inject nudge + return (continue)
11. Dead-loop check — 4+ same normalized signature → nudge
12. Execute tools — grouped by effect (READ parallel, WRITE sequential)
13. Skill extraction — learn from non-obs tool calls

**Nudge rule:** Must inject AFTER all tool results, never between `assistant(tool_calls)` and `tool` (triggers 400).

## Project Files

```
tasks/                   23 task JSONs (tier1-6: crafting_table → netherite → ender_dragon → elytra)
profiles/                3-layer prompt templates
  _default.json          Base prompt
  defaults/{mode}.json   Mode overrides
  {individual}.json      Individual overrides
blueprints/              4 blueprint JSONs (farm_plot, small_shelter, storage_corner, torch_pillar)
actions/                 LLM-defined rhai scripts (*.rhai.json)
sessions/                Runtime session JSONL files
scripts/                 Debug/diagnostic shell scripts
.github/workflows/ci.yml CI config
data/config/agent.toml   Multi-backend LLM/VLM config
```

## Task System (task.rs)

23 structured tasks with completion conditions:
- `InventoryHas { item, count }` — backpack has item ≥ count
- `AtPosition { x, y, z, radius }` — bot within radius of position
- `BelowY { y }` — below Y coordinate
- `InDimension { dimension }` — currently in the specified dimension
- `PortalActive` — an active Nether portal is present in scan range
- `Killed { entity_kind, count }` — server-reported cumulative entity kills
- `All/Any { conditions }` — composite conditions
- Tasks loaded from `tasks/*.json`, sorted by tier, explicit order, then id

## Profile System (profile.rs)

3-layer prompt merging (field-level, not full replacement):
1. `profiles/_default.json` — baseline (required)
2. `profiles/defaults/{mode}.json` — mode overrides (optional)
3. `profiles/{individual}.json` — individual overrides (optional)

Placeholders: `$NAME`, `$SELF_PROMPT`, `$MEMORY`, `$STATS`, `$INVENTORY`, `$COMMAND_DOCS`, `$EXAMPLES`

## Modes System (agent/modes.rs)

10 reactive modes, tick-level, independent of LLM:
- `self_preservation` — fire/lava escape
- `self_defense` — attack hostile mobs
- `unstuck` — break free when stuck
- `cowardice` — flee from hostiles
- `hunting` — hunt animals for food
- `item_collecting` — pick up drops
- `torch_placing` — place torches in dark
- `elbow_room` — clear cramped spaces
- `idle_staring` — look around when idle
- `cheat` — creative mode cheats

## WorldMemory (core/memory.rs)

Spatial memory with chunk-based indexing:
- 7 memory kinds: Resource, Structure, Container, Entity, Hazard, Portal, Note
- `chunk_key` = (x>>4, y>>4, z>>4) for O(1) nearby queries
- Named anchors: "home", "nether_portal", etc.
- Forgetting: TTL-based + explicit `forget_*`

## Azalea Integration (Command Queue Pattern)

```
LLM tool call → tools_azalea.rs → adapter_azalea.rs::execute()
  → AzaleaBot::push_cmd(BotCommand) → cmd_queue
  → handler tick drains queue → executes with bot API
  → BotEvent::Chat("[采集] ...") sent back via event channel
```

**Key constraint:** Azalea `Client` is only accessible inside handler closure. External code uses fire-and-forget command queue. Results come back asynchronously via `BotEvent`.

## System Prompt Byte Stability

DeepSeek prefix cache requires system prompt to be identical every call.
- **Static content** → system prompt (identity, role_desc, jailbreak, knowledge)
- **Dynamic content** → user messages, injected each turn and stripped the next:
  - `【当前游戏状态（自动注入）】` — perceive state
  - `【邻近世界记忆】` — WorldMemory nearby
  - `[当前目标]` — self_prompt goal
  - `【参考示例】` — few-shot examples
  - `【工具执行】...` — folded tool history

**Regression test:** `regression_system_prompt_byte_stable_across_obs_streak`

## Tool Calling

- All tools use OpenAI function calling (JSON `tool_calls`), never text commands
- `fold_tool_history` in `decision.rs` strips tool_calls for DeepSeek compat
- Converts `tool_calls` + `role:tool` → `【工具执行】name(args) → result` text
- Tools return sync results

## Code Execution

- **`run_script`** → rhai engine (embedded, Rust-native, safe)
  - Functions: goto, mine, mine_below, gather, craft, place, open, chat, attack, smelt, interact, sleep, print
- **`run_plan`** → JSON multi-step plan (sequential, synchronous)
- **`new_action`** → save named rhai script to `actions/<name>.rhai.json`, reusable across sessions

## Perceive Format (adapter_azalea.rs)

```
位置: (-489.0, 86.0, -169.0)
生命: 20/20  饱食: 20/20  等级: 0
主手: dirt
装备: [头盔: 无, 胸甲: 无, 护腿: 无, 靴子: 无]
附近:
  oak_log @(x,y,z) 3.2m
  stone @(x,y,z) 5.1m
背包:
  oak_log x16, stick x4, cobblestone x32
记忆:
  工作台 @(10,64,-20) [结构]
  箱子(32 oak_log) @(12,64,-20) [容器]
卡住: 0轮
```

Block names use `BlockKind` (not `BlockState` Debug format).

## Critical Constraints

### Don't modify vendor/azalea
- `vendor/azalea` is a separate git repo + workspace
- All changes must use azalea's public API only
- To modify: commit in vendor → update SHA in both `.cargo/config.toml` [patch] and `Cargo.toml`

### Cargo network
- Uses `rsproxy.cn` mirror (sparse index)
- `NO_PROXY` env var includes rsproxy.cn, mirrors.ustc.edu.cn, etc.
- azalea is git dependency with local vendor patch — offline builds work

### Git safety
- ❌ Never `git checkout -- <file>`, `git checkout .`, `git restore`, `git reset --hard`, `git clean -fd`
- ❌ Never `git stash` without immediate pop plan
- ✅ Use SearchReplace to revert code (visible, line-by-line)
- ✅ `git add -A && git commit --no-verify -m "wip: checkpoint"` before risky experiments

### Tool Name Discipline
- Tool names must be stable — changing them breaks LLM prompt compatibility
- When adding tools, add to BOTH `tools_azalea.rs` AND `core/types.rs::MinecraftAction`
- Tool descriptions are part of the prompt — keep them concise and action-oriented

## Common Build Errors

| Error | Fix |
|-------|-----|
| `cannot find trait LlmProvider` | `use crate::agent::LlmProvider` |
| `cannot find module azalea_block` | Use `azalea::block::BlockState` |
| `mismatched closing delimiter` | Check for duplicate code blocks from merge errors |
| `failed to get anyhow as dependency` | rsproxy.cn down, temporarily rename `.cargo/config.toml` |
| `tried to attack entity which isn't in EntityIdIndex` | Self-defense mode needs entity existence check |
| `nightly-only feature` | Don't switch to stable, azalea requires nightly |
| LSP ConfigInvalidError | `lsp` must be `true`, `false`, or object — not array |

## Testing

```bash
# All tests
cargo test --workspace

# Specific crate
cargo test -p craft-agent --lib
cargo test -p craft-agent-minecraft --features azalea-bot --lib

# Specific test
cargo test task_manager_lifecycle
cargo test regression_all_tasks_dir_json_loads

# After changing system prompt
cargo test regression_system_prompt_byte_stable
```

## Reference Projects

- **Mindcraft** (mindcraft-bots/mindcraft): JS + mineflayer, LLM bot framework. Reference for tasks, profiles, modes.
- **Azalea** (azalea-rs/azalea): Rust Minecraft client protocol. This project builds on top.
- **Numen**: Structured survival automation (SurvivalJournal, FailureType classification).

## Goal: Beat Minecraft

6-stage completion path:
1. Tier 1-2: Wood/stone/iron tools, crafting table, furnace
2. Tier 3-4: Iron armor, diamond gear, mine to bedrock
3. Tier 5: Enchanting, brewing, nether portal
4. Tier 6: Netherite, shulker boxes, elytra
5. Reach the End
6. Defeat the Ender Dragon

Each stage requires the LLM to know:
- MC crafting recipes (all tiers)
- Tool/armor tier progression
- Smelting and brewing recipes
- Enchantment strategies
- Nether and End dimension mechanics
