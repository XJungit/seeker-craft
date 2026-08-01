# SeekerCraft

**[English](README.md) | [中文](README.zh-CN.md)**

**一个由 LLM 驱动的 Minecraft 机器人，目标是击败末影龙。**

SeekerCraft 是用 Rust 实现的自主 Minecraft Agent：LLM 观察结构化游戏状态、
规划多步生存策略，并通过基于
[Azalea](https://github.com/azalea-rs/azalea) 客户端协议的 44 个类型化工具执行——
无需 mod、无需截图，是真正的协议级玩家。

目标很简单：从一无所有开始，生存、制造，最终击败末影龙。

## 亮点

- **真实协议客户端** — 通过 Azalea Rust 客户端（MC 26.2）以普通玩家身份连接，内置寻路。
- **44 个 LLM 工具** — 感知、移动、挖矿、合成（2x2/3x3/熔炼/附魔/酿造）、放置、建造、容器、交易及元工具。
- **10 个反应式模式** — 自卫、狩猎、自动拾取、自动穿甲、插火把、脱困等，tick 级运行，不依赖 LLM 延迟。
- **结构化任务系统** — 23 个分层任务（木头 → 下界合金 → 末影龙），带机器可校验的完成条件。
- **空间 WorldMemory** — 按区块索引的记忆（资源/建筑/容器/危险/传送门），带 TTL 遗忘。
- **字节稳定的系统提示** — 为 DeepSeek 风格前缀缓存设计；动态状态以用户消息注入。
- **Probe 模式** — 无 LLM 的工具层测试框架，秒级验证工具行为。
- **运维控制台（craft-agent-ctl）** — 进程生命周期、目标注入、会话检查。
- **Autopilot** — 自动循环：构建、测试、异常分类、根因分析并提交。

## 架构

```
seeker-craft/
├── Cargo.toml                    # workspace 根
├── crates/
│   ├── craft-agent/              # 核心 agent：run_one_turn 循环、模式、压缩、技能、WorldMemory
│   ├── craft-agent-minecraft/    # Azalea 适配器：bot、44 工具、合成/熔炼/附魔
│   ├── craft-agent-model/        # LLM/VLM 客户端（OpenAI 兼容，多后端）
│   ├── craft-agent-viewer/       # Web 仪表盘（Axum + SSE）
│   ├── craft-agent-autopilot/    # 自动开发循环
│   └── craft-agent-ctl/          # 运维控制台
├── data/config/agent.example.toml  # LLM 后端配置模板（复制为 agent.toml）
├── tasks/                        # 23 个任务 JSON（tier 1-6）
├── profiles/                     # 3 层提示词模板
├── blueprints/                   # 建造蓝图
├── actions/                      # LLM 定义的 rhai 脚本
└── vendor/azalea/                # 固定版本的 Azalea 源码（submodule，官方上游）
```

详见 [ARCHITECTURE.md](ARCHITECTURE.md)（13 步 Agent 循环）与
[AGENTS.md](AGENTS.md)（开发工作流）。

## 快速开始

### 前置条件

- Rust **nightly**（见 `rust-toolchain.toml`；stable 会失败——azalea 需要 nightly）
- 一个机器人能加入的 Minecraft Java 服务器（任意 vanilla 1.20.4+ / MC 26.2 服务端，局域网也行）

### 构建

```bash
cargo build --workspace
```

### 配置 LLM 后端

```bash
cp data/config/agent.example.toml data/config/agent.toml
# 编辑 data/config/agent.toml — 填入你的 API key（或用 api_key_env + 环境变量）
```

任何 OpenAI 兼容端点都可以（DeepSeek、OpenAI、本地网关等）。
Key 永远不会被提交：`agent.toml` 已被 gitignore。

### 运行

```bash
# Web 仪表盘 + agent（LLM 驱动）
cargo run -p craft-agent-viewer --bin craft-agent-viewer \
  -- --goal "挖矿下探" --steps 0 --port 8080 --mc localhost:4444 --username CraftAgent
# 打开 http://127.0.0.1:8080

# Probe 模式 — 不经过 LLM 测试工具层（秒级，而非分钟级）
cargo run -p craft-agent-minecraft --example azalea_probe --features azalea-bot -- 4444 --cmd "equip iron_helmet helmet"
```

### 测试

```bash
cargo test -p craft-agent --lib
cargo test -p craft-agent-minecraft --features azalea-bot --lib
```

## 文档

- [ARCHITECTURE.md](ARCHITECTURE.md) — 分层架构、13 步循环、44 工具
- [AGENTS.md](AGENTS.md) — 自主开发工作流（差距分析 → 修复 → 验证）
- [docs/mindcraft-gap.md](docs/mindcraft-gap.md) — Mindcraft 差距审计 + 优先级队列
- [docs/CHANGELOG.md](docs/CHANGELOG.md) — 变更日志
- [docs/adr.md](docs/adr.md) — 架构决策记录
- [docs/README.md](docs/README.md) — 完整文档索引

## 相关项目

- [Mindcraft](https://github.com/mindcraft-bots/mindcraft) — JS + mineflayer LLM bot；任务/配置/模式参考实现
- [Azalea](https://github.com/azalea-rs/azalea) — Rust Minecraft 客户端协议

## License

[MIT](LICENSE)
