# Craft-Agent AGENTS.md

## Build

**Java mod:** `$env:JAVA_HOME = 'C:\Users\xj\AppData\Roaming\.minecraft\runtime\java-runtime-epsilon'; .\gradlew.bat build` in `mods/craft-agent-bridge/`

**Rust:** `cargo build` or `cargo test --workspace` (edition 2024, MSRV 1.97.1)

**Smoke test (requires MC + mod loaded):**
`cargo run -p craft-agent-minecraft --example smoke_test --features mod-bridge`

**Viewer dashboard:** `cargo run -p craft-agent-viewer` → http://127.0.0.1:8080

## Project layout

```
D:\Craft-Agent/
├── crates/
│   ├── craft-agent/          — Core agent runtime (LLM loop, compaction, modes, session)
│   ├── craft-agent-model/    — LLM/VLM client layer, multi-backend config
│   ├── craft-agent-minecraft/— Minecraft adapter (62 tools, TCP bridge, survival)
│   └── craft-agent-viewer/   — Axum web dashboard, SSE events, agent control loop
├── mods/craft-agent-bridge/  — Java Fabric mod (MC 26.2, JDK 25, Mojang mappings)
│   └── src/main/java/com/craftagent/bridge/  — 30+ source files
├── config/agent.toml         — Multi-backend LLM/VLM config (deepseek/agnes/stepfun)
└── docs/design/              — Architecture docs, parity analysis
```

## Java mod structure

**Not a thin bridge!** Java side has significant game logic:

| Component | Lines | Role |
|---|---|---|
| `AStar.java` + 13 `pathing/` | ~800 | A* search + VanillaPathfinder + PlayerPathExecutor |
| `MovementController` | 1146 | Pathfinding, combat, collect, follow, pillarUp |
| `CombatController` | 149 | Combat state machine (melee/kite/retreat) |
| `CollectController` | 202 | Block collection automaton |
| `GoalEngine` | 479 | Autonomous goal decomposition (craft/get/smelt/hunt/build/explore/defend) |
| `CraftAgentBridge` | 1004 | TCP server, dispatch, movement ticks, autoSurvive |
| Other controllers | ~3000 | Interaction, container, entity, debug, state |

**Architectural conflict:** GoalEngine (Java) runs independently of Rust LLM loop. Two decision bodies may fight over the player. `performCombat` (MovementController) and `CombatController.tick()` are duplicate combat systems. GoalEngine's craft/smelt logic duplicates Rust-side CraftingHelper.

## MC 26.2 API gotchas

- **`ResourceLocation` → `Identifier`** (`net.minecraft.resources.Identifier`): `Identifier.fromNamespaceAndPath("minecraft", "oak_log")`, no `new ResourceLocation()`.
- **`Registry.get(id)` returns `Optional<Holder.Reference<T>>`**: use `.get().value()`.
- **`EntityType.create(Level, EntitySpawnReason)`**: no `EntityType.create(Level)` overload.
- **Time API removed**: no `setDayTime()`/`setTime()` on ServerLevel/MinecraftServer, use world clock API.
- **No Yarn mappings**: Mojang official, jar is unobfuscated. Use `javap` on `minecraft-merged.jar` to verify signatures.

API reference: NeoForged 26.2 Migration Primer > Fabric 26.2 Announcement > Minecraft Wiki 26.2 > `javap`

## Critical git rules

- **Commit after every logical change** (`git add -A && git commit --no-verify`). No batching.
- **Never use PowerShell `Set-Content`/`Out-File` for Java/Rust source files.** Use Edit tool. Reason: one `Set-Content -NoNewline` destroyed `CraftAgentBridge.java` (never committed → unrecoverable).

## Debug commands (smoke test only, not exposed to LLM)

`debug_spawn(entity,num)` / `debug_give(item,num)` / `debug_place(block,x,y,z)` / `debug_heal` / `debug_damage(amount)` / `debug_clear` / `debug_food(level)` / `debug_teleport_bot{x,z}` / `debug_teleport_player{x,z}`

**Critical:** `debug_teleport_bot` must call `teleportTo` directly inside `performAction` (already on server thread). Do NOT wrap in `onServer()` — `executeIfPossible` queue inside same task deadlocks (30s timeout).

## Smoke test bot landing

`createFakePlayer` spawns at (0.5, 64, 0.5). `build_platform()` places a 9×9 dirt platform at y=63/64. **If MC world isn't fully loaded**, `debug_place` silently fails → bot falls through to real ground (y≈44) → all tools fail. Wait for world load before smoke test.

## Compaction

Uses dedicated Agnes-2.0-flash model (512K context). Set `AGNES_API_KEY` env var. Falls back to primary model if Agnes fails, then to `hard_truncate()` (drops old messages without summary). Compaction triggers when `estimate_tokens() > context_window - reserve`. **Note:** `estimate_tokens()` sums `usage.total_tokens` from each response = overcounts (175 messages → 1.9M estimate).
