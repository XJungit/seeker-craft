# SeekerCraft (Craft-Agent)

**[English](README.md) | [中文](README.zh-CN.md)**

[![CI: fmt + clippy](https://img.shields.io/github/actions/workflow/status/XJungit/seeker-craft/ci.yml?label=CI&logo=github)](https://github.com/XJungit/seeker-craft/actions/workflows/ci.yml)
[![CI: tests](https://img.shields.io/github/actions/workflow/status/XJungit/seeker-craft/ci.yml?label=ci&logo=github&label=tests)](https://github.com/XJungit/seeker-craft/actions/workflows/ci.yml)
[![Security audit](https://img.shields.io/github/actions/workflow/status/XJungit/seeker-craft/audit.yml?label=cargo-audit&logo=github)](https://github.com/XJungit/seeker-craft/actions/workflows/audit.yml)
[![Docs](https://img.shields.io/github/actions/workflow/status/XJungit/seeker-craft/deploy-docs.yml?label=docs&logo=github)](https://github.com/XJungit/seeker-craft/actions/workflows/deploy-docs.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust: nightly-2026-07-21](https://img.shields.io/badge/rust-nightly--2026--07--21-orange.svg)](rust-toolchain.toml)

**一个由 LLM 驱动的 Minecraft 机器人，目标是击败末影龙。Rust + Azalea 协议客户端，
无 mod、无截图——通过类型化工具观察、规划、执行的真正的协议级玩家。**

| | |
|---|---|
| **核心问题** | LLM 能否从一无所有开始自主生存、制造并击败末影龙？ |
| **运行时** | 纯 Rust 客户端，基于 [Azalea](https://github.com/azalea-rs/azalea)（MC 26.2），无需服务端 mod |
| **大脑** | 任意 OpenAI 兼容 LLM（DeepSeek 前缀缓存优化）；2026-08-14 起为 DSH（DeepSeek Harness）桥接模式 |
| **规模** | 6 个 crate、53 个 LLM 工具、23 个结构化任务、10 个反应式模式、空间记忆 |
| **开发循环** | 自主：差距分析 → 修复 → probe 验证 → 提交（工作流笔记仅存本地，不随仓库发布） |

> **项目性质声明。** 本项目完全由 AI 辅助编程（vibe coding）生成，属个人实验项目，
> 不作为工程最佳实践、学习资料或生产参考。项目目的在于探索 AI Agent 与
> vibe coding 方法论；实现过程中学习借鉴了
> [Mindcraft](https://github.com/mindcraft-bots/mindcraft)、
> [Azalea](https://github.com/azalea-rs/azalea) 等开源项目，并刻意采用作者此前
> 完全未接触过的 Rust 技术栈，追求全 Rust 技术栈的 vibe coding。

---

## 亮点

- **真实协议客户端** — 通过 Azalea Rust 客户端（MC 26.2）以普通玩家身份连接，内置寻路；无 mod、无截图。
- **53 个类型化 LLM 工具** — 感知、移动、挖矿、合成（2x2/3x3/熔炼/附魔/酿造）、放置、建造、容器、交易、战斗及元工具。
- **10 个反应式模式** — 自卫、狩猎、自动拾取、插火把、脱困、清理拥挤空间等，tick 级运行，不受 LLM 延迟影响（bot 端；LLM 姿态经 `set_mode` 切换）。
- **结构化任务系统** — 23 个分层任务（木头 → 石头 → 铁 → 钻石 → 下界合金 → 末影龙），带机器可校验的完成条件。
- **空间 WorldMemory** — 按区块索引的记忆（资源/建筑/容器/危险/传送门），带 TTL 遗忘与命名锚点。
- **DSH 桥接模式** — 2026-08-14 起 in-bot LLM 循环已移除，DSH（DeepSeek Harness）成为唯一大脑，经 viewer 桥（`/api/connect` + `/api/bot_tool` + `/api/game-state` + `/api/goal`）驱动 bot。
- **Probe 模式** — 无 LLM 的工具层测试框架，秒级验证工具行为（而非分钟的 LLM 运行时）。
- **运维控制台（craft-agent-ctl）** — 进程生命周期、目标注入、会话检查。
- **Autopilot** — 运维监督器（10s 轮询）：拉起 viewer + 连接 bot、停滞 steering、崩溃恢复、异常检测（无改代码逻辑）。

## 架构

```
seeker-craft/
├── Cargo.toml                     # workspace 根（nightly-2026-07-21）
├── crates/
│   ├── craft-agent/               # 纯逻辑库：types/GameTool/ToolRegistry/WorldMemory/session/task/profile/skill
│   ├── craft-agent-minecraft/     # Azalea 适配器：bot + 53 个工具
│   ├── craft-agent-model/         # LLM/VLM 客户端（in-bot 时代，保留兼容；现 LLM 由 DSH 提供）
│   ├── craft-agent-viewer/        # Web 仪表盘（Axum + SSE）+ DSH 桥（connect/bot_tool/game-state/goal）
│   ├── craft-agent-autopilot/     # 运维监督器（10s 轮询：viewer+连接、停滞 steering、崩溃恢复）
│   └── craft-agent-ctl/           # 运维控制台
├── data/
│   ├── config/agent.example.toml  # LLM 后端配置模板（复制为 agent.toml）
│   ├── tasks/                     # 23 个任务 JSON（tier 1-6）
│   ├── profiles/                  # 3 层提示词模板
│   ├── blueprints/                # 建造蓝图
│   └── actions/                   # LLM 定义的 rhai 脚本
└── vendor/azalea/                 # 固定版本的 Azalea 源码（submodule，官方上游）
```

### DSH 桥接运行时（2026-08-14 起）

```
DSH（DeepSeek Harness）大脑 ──HTTP──► craft-agent-viewer 桥
  │  /api/connect    → azalea 客户端加入 MC（账号 CraftAgent）
  │  /api/bot_tool   → 派发 53 工具之一（GameTool::execute）
  │  /api/game-state → 实时 BotState 快照（perceive 格式）
  │  /api/goal       → 更新运营目标
  ▼
craft-agent-minecraft（53 工具 + WorldMemory 每 20 tick 扫描 + handler.rs 反应式模式）
  ▼
azalea (vendor) ──► MC server (TCP)
```

> **in-bot 13 步主循环已移除**（2026-08-14，阶段3 清理）：`run_one_turn`、auto-perceive、
> SelfPrompter、execute_batch、每轮动态上下文注入在 Rust 侧已不存在。大脑（DSH）现在负责
> 决策/规划/上下文注入/系统提示稳定性；Rust 只经 viewer 桥暴露 bot 实时能力。
> 详见 [ARCHITECTURE.md](ARCHITECTURE.md)。

## 击败末影龙的 6 阶段

| 阶段 | 关卡 | 任务 |
|---|---|---|
| 1 | **木头与工具** | 采集木头、合成工作台、木镐、石镐 |
| 2 | **铁器时代** | 熔炉、铁镐 |
| 3 | **生存装备** | 面包、铁甲、铁剑、盾牌 |
| 4 | **钻石时代** | 钻石镐/剑/甲、挖至基岩 |
| 5 | **下界与魔法** | 附魔台、附魔剑、酿造台、下界传送门 |
| 6 | **终局** | 下界合金锭/镐、潜影盒、鞘翅、末影龙 |

23 个任务（6 层）全部以机器可判定 JSON 形式随仓库发布（[`data/tasks/`](data/tasks/)，任务系统说明见 [`docs/ARCHITECTURE.md`](ARCHITECTURE.md)）。

## 当前进度（2026-08-08）

**已实机端到端验证（真实服务器、无 mod）：**

| 阶段 | 状态 | 证据 |
|---|---|---|
| Tier 1–2：木 → 石 → 铁镐链路 | ✅ 实机 | bot 自主采集木头、合成木板/木棍/镐，并经由附近工作台合成出 iron_pickaxe |
| Tier 3：生存装备 | ✅ 实机 | 全套铁甲 + 钻石剑 + 盾牌；生命/饱食回满 |
| Tier 4：钻石时代 | 🔄 进行中 | bot 遵循 Y 层提示（mine_below 至 Y≤16），下挖到钻石层（Y=-59），并用 `search_for_block` 定位到 diamond_ore |
| Tier 5：下界与魔法 | ⬜ 下一步 | 下界传送门 / 附魔 / 酿造尚未端到端验证 |
| Tier 6：终局 | ⬜ 待办 | 下界合金 / 潜影盒 / 鞘翅 / 末影龙 |

**近期已推送的 P 系列里程碑：**

- **P135** — mushroom_stew 配方回退三原料（Wiki 验证）；gather 矿石 Y 提示纠错（移除 1.16 静态数据，改为 `y_range_hint` 动态驱动）
- **P136** — 版本写死内容全面排查：矿石 Y 层知识库（钻石 −64~16 最密 −59、铁 −64~384、绿宝石仅山地等）、版本号规范（MC 26.2）

**验证纪律：** 所有工具层行为推送前均经 probe 实机验证（见 `scripts/probe/*.json`）；Y 提示正确性已 probe 验证——钻石（越界提示）、绿宝石（群系提示）、铁/煤（范围内不误报）。完整里程碑表见 [`docs/benchmarks.md`](docs/benchmarks.md)。

## 53 个 LLM 工具

| 类别 | 工具 |
|---|---|
| 感知 | `perceive`, `memory`, `search_wiki`, `search_for_block` |
| 移动 | `goto`, `goto_player`, `move_away`, `mine_below`, `mine_above`, `pickup`, `follow`, `stop_follow` |
| 模式 | `set_mode` |
| 挖掘 | `mine`, `make_obsidian` |
| 交互 | `interact_block`, `interact_entity`, `attack`, `defend`, `use_item`, `shoot`, `sleep` |
| 合成 | `craft`, `craft_3x3`, `smelt`, `auto_craft`, `enchant` |
| 采集 | `gather`, `till_and_sow`, `harvest` |
| 放置 | `place`, `build`, `build_blueprint`, `list_blueprints` |
| 容器 | `open`, `chest_view`, `chest_withdraw`, `chest_deposit` |
| 背包 | `equip`, `discard`, `consume` |
| NPC/社交 | `trade`, `give` |
| 元操作 | `chat`, `set_goal`, `run_plan`, `run_script`, `new_action`, `list_actions`, `pause_goal`, `resume_goal`, `task_complete`, `task_retry` |

## 快速开始

### 前置条件

- Rust **nightly**（见 `rust-toolchain.toml`；stable 会失败——azalea 需要 nightly）
- 一个机器人能加入的 Minecraft Java 服务器（任意 vanilla 1.20.4+ / MC 26.2 服务端，局域网也行）

### 构建与测试

```bash
cargo build --workspace
cargo test -p craft-agent --lib
cargo test -p craft-agent-minecraft --features azalea-bot --lib
```

### 配置 LLM 后端

```bash
cp data/config/agent.example.toml data/config/agent.toml
# 编辑 data/config/agent.toml — 填入你的 API key（或用 api_key_env + 环境变量）
```

任何 OpenAI 兼容端点都可以（DeepSeek、OpenAI、本地网关等）。
Key 永远不会被提交：`agent.toml` 已被 gitignore。

### 运行 bot

```bash
# Web 仪表盘 + agent（LLM 驱动）
cargo run -p craft-agent-viewer --bin craft-agent-viewer \
  -- --goal "挖矿下探" --steps 0 --port 8080 --mc localhost:4444 --username CraftAgent
# 打开 http://127.0.0.1:8080

# Probe 模式 — 不经过 LLM 测试工具层（秒级，而非分钟级）
cargo run -p craft-agent-minecraft --example azalea_probe --features azalea-bot -- 4444 --cmd "equip iron_helmet helmet"
```

### Probe 脚本

```bash
# 功能/端到端验证（见 scripts/probe/*.json）
cargo run -p craft-agent-minecraft --example azalea_probe --features azalea-bot -- 4444 --script scripts\probe\smoke.json
```

## 文档

| 文档 | 内容 |
|---|---|
| [ARCHITECTURE.md](ARCHITECTURE.md) | 分层架构、DSH 桥接运行时、模块布局 |
| [docs/mindcraft-gap.md](docs/mindcraft-gap.md) | Mindcraft 差距审计 + 优先级队列 |
| [docs/benchmarks.md](docs/benchmarks.md) | 测试基线、运行探测覆盖、缓存命中率、末影龙进度 |
| [docs/adr.md](docs/adr.md) | 架构决策记录 |
| [docs/README.md](docs/README.md) | 完整文档索引（教程、设计归档） |
| [CHANGELOG.md](CHANGELOG.md) | 版本化变更日志 |
| [SECURITY.md](SECURITY.md) | 安全策略与漏洞上报 |
| [CONTRIBUTING.md](CONTRIBUTING.md) | 如何开发、测试提交修复 |

文档同时发布到 **GitHub Pages**（rustdoc + docs）——见
[docs workflow](https://github.com/XJungit/seeker-craft/actions/workflows/deploy-docs.yml)。

## 相关项目

- [Mindcraft](https://github.com/mindcraft-bots/mindcraft) — JS + mineflayer LLM bot；任务/配置/模式参考实现
- [Azalea](https://github.com/azalea-rs/azalea) — Rust Minecraft 客户端协议

## License

[MIT](LICENSE) — 维护者见 [AUTHORS](AUTHORS)。引用请注明 [CITATION.cff](CITATION.cff)。