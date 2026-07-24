# Craft-Agent AGENTS.md

## Build

**Rust (azalea — 唯一路线):** `cargo build` / `cargo test --workspace` (edition 2024, nightly pinned via rust-toolchain.toml)

- `rust-toolchain.toml` 锁 `nightly-2026-07-21`。azalea (vendor commit c35b57eb) 依赖 nightly-only 特性（`generic_const_exprs` / `min_specialization` / `type_changing_struct_update`），stable 编不过。cargo 自动读该文件选工具链，**不要手动改 channel 为 stable**。
- azalea 依赖在 `Cargo.toml` 里写 `file:///D:/Craft-Agent/vendor/azalea` git 源（本地 vendor）。`.cargo/config.toml` 的 `[patch]` 段可重定向到 https 源（联网编译用）。**不要手动改 Cargo.toml 里的源路径**。
- azalea bot demo: `cargo run -p craft-agent-minecraft --example agent_azalea_demo --features azalea-bot`
- 其他示例：`agent_azalea_demo` / `azalea_adapter_demo` / `azalea_bot_demo` / `azalea_connect` / `azalea_place_demo` / `calibrate_4444`
- **Viewer dashboard:** `cargo run -p craft-agent-viewer` → http://127.0.0.1:8080

> 旧路线 `mod-bridge`（Fabric mod TCP 桥接）与 `real`（VLM 截图 + enigo 键鼠）已从源码删除，仅保留 azalea 客户端协议层。

## Project layout

```
crates/
├── craft-agent/              — 核心：Agent 主循环（LLM 决策 + 工具编排 + 压缩）
│   ├── agent/mod.rs          — Agent::run_one_turn() 主循环
│   ├── agent/compaction.rs   — token 估算 + 上下文压缩（Agnes 专用模型）
│   ├── agent/prompt.rs       — 提示词构建（WorldInfo / MC 知识注入）
│   ├── agent/modes.rs        — 模式响应系统
│   ├── agent/session.rs      — 会话持久化
│   └── core/                 — 跨游戏抽象（types/adapter/tool/message/session/skill）
├── craft-agent-model/        — LLM/VLM 多后端客户端（OpenAI 兼容）
├── craft-agent-minecraft/    — MC 适配器（Azalea 客户端协议层）
│   └── src/azalea/           — 命令队列模式 bot + 合成/采集/放置/配方
│   └── src/tools_azalea.rs   — 15 个 LLM 工具定义
└── craft-agent-viewer/       — Axum Web 仪表盘，SSE 事件流
config/agent.toml             — 多后端 LLM/VLM 配置 + 压缩模型配置
```

## Core architecture

**Agent 循环** (`crates/craft-agent/src/agent/mod.rs`): `run_one_turn()` → 压缩检查 → 自动感知注入 → 模式检查 → LLM complete (带重试) → 文字回复检测 → 死循环检测 → 并行执行工具 → 技能抽取。

**Azalea 集成** (`crates/craft-agent-minecraft/src/azalea/`): 使用命令队列模式。Azalea 的 `Client` 只能在 handler 闭包内使用，外部无法持有，所以 `AzaleaBot` 把动作指令 push 进 `BotState.cmd_queue`，handler 每 tick drain 并执行。**所有 `AzaleaBot` 方法都是 fire-and-forget**，结果通过 `BotEvent` 事件通道异步回传。

**15 个 LLM 工具** (`crates/craft-agent-minecraft/src/tools_azalea.rs`): perceive / goto / mine / mine_below / interact_block / attack / chat / craft / craft_3x3 / smelt / gather / place / open / auto_craft / enchant。全部通过 `GameAdapter.execute(Action::Minecraft(...))` 代理到 `AzaleaBot`。

**配方知识库** (`crates/craft-agent-minecraft/src/azalea/recipes.rs`): 手写静态配方表。azalea 未暴露配方查询 API，所以这里手动维护 26.2 常见配方（2×2 / 3×3 / 熔炼 / 采集）。`auto_craft` 沿配方图递归满足原料。新增配方只需加一行。

## Compaction

Uses dedicated Agnes-2.0-flash model (512K context). Set `AGNES_API_KEY` env var. Falls back to primary model if Agnes fails, then to `hard_truncate()` (drops old messages without summary). Compaction triggers when `estimate_tokens() > context_window - reserve`. **Note:** `estimate_tokens()` sums `usage.total_tokens` from each response = overcounts (175 messages → 1.9M estimate).

## Critical git rules

- **Commit after every logical change** (`git add -A && git commit --no-verify`). No batching.
- **Never use PowerShell `Set-Content`/`Out-File` for Rust source files.** Use Edit tool. Reason: one `Set-Content -NoNewline` destroyed `CraftAgentBridge.java` (never committed → unrecoverable).

## Dependencies

- Azalea 依赖在 `vendor/azalea/`（git submodule，commit c35b57eb）。如果本地编译报 git 源错误，确认 `.cargo/config.toml` 的 `[patch]` 段指向正确路径，且 `[protocol] file.allow = "always"` 已设置。
- Windows 专用：`windows-sys` 用于前台窗口接管（仅在 `azalea-bot` 特性启用时编译，实际 azalea 路线不依赖此功能，保留供未来用）。
- 工作区 resolver = "3"，所有公共依赖在 `Cargo.toml` 的 `[workspace.dependencies]` 单点声明。