# Craft-Agent AGENTS.md

## Build

**Rust (azalea client — 唯一路线):** `cargo build` / `cargo test --workspace` (edition 2024, nightly pinned via rust-toolchain.toml)
- `rust-toolchain.toml` 锁 `nightly-2026-07-21`。**必须用 nightly**：azalea (vendor, rev c35b57eb) 依赖 nightly-only 特性（`generic_const_exprs` / `min_specialization` / `type_changing_struct_update`），stable 1.97.1 编不过。cargo 自动读该文件选工具链，**不要手动改 channel 为 stable**。
- azalea 依赖在 `Cargo.toml` 里**永久写 https 源**（可移植）。本地离线开发靠 `.cargo/config.toml` 的 `[patch."https://github.com/azalea-rs/azalea"]` 重定向到 `vendor/azalea`（file git 源）。**不要再手动把 Cargo.toml 改 file:// 来回切换**（旧工作流已废弃）。
- azalea bot demo: `cargo run -p craft-agent-minecraft --example agent_azalea_demo --features azalea-bot`
- 其他 azalea 示例：agent_azalea_demo / azalea_adapter_demo / azalea_bot_demo / azalea_connect / azalea_place_demo

> 旧路线 `mod-bridge`（Fabric mod TCP 桥接）与 `real`（VLM 截图 + enigo 键鼠）已从源码删除，仅保留 azalea 客户端协议层。

**Viewer dashboard:** `cargo run -p craft-agent-viewer` → http://127.0.0.1:8080

## Project layout

```
D:\Craft-Agent/
├── crates/
│   ├── craft-agent/          — Core agent runtime (LLM loop, compaction, modes, session)
│   ├── craft-agent-model/    — LLM/VLM client layer, multi-backend config
│   ├── craft-agent-minecraft/— Minecraft adapter (azalea bot 路线)
│   └── craft-agent-viewer/   — Axum web dashboard, SSE events, agent control loop
├── config/agent.toml         — Multi-backend LLM/VLM config (deepseek/agnes/stepfun)
└── docs/design/              — Architecture docs, parity analysis
```

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
