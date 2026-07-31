# Craft-Agent

通用游戏 Agent 框架（首个落地场景：Minecraft Java 版）。

当前唯一运行路径：
- **azalea-bot（活跃）**：Rust 全栈客户端 bot，直连普通 MC 服务器（含局域网），原生支持 MC 26.2，内置 Baritone 级 pathfinder。
> 旧 `mod-bridge`（Fabric mod TCP 桥接）与 `real`（VLM 截图 + enigo 键鼠）路线已从源码删除。

## 决策内核

- **感知**：结构化游戏状态 + 可选 VLM 视觉补充。
- **决策**：LLM 基于世界状态生成工具调用（OpenAI 兼容，多后端可配）。
- **执行**：azalea 客户端协议包（与真人玩家等价）/ 或旧 mod 主线程精确执行。

完整设计见 [`docs/design/refactor-azalea-client-route.md`](./docs/design/refactor-azalea-client-route.md)，架构概览见 [`ARCHITECTURE.md`](./ARCHITECTURE.md)。

## 工程结构

```
Craft-Agent/
├── Cargo.toml                 # workspace 根
├── Cargo.lock                # 单一锁文件
├── crates/
│   ├── craft-agent/          # 核心：GameAdapter / Agent 主循环 / Session
│   ├── craft-agent-model/    # VLM/LLM 客户端
│   ├── craft-agent-minecraft/# MC 适配器与工具集（azalea 路线）
│   └── craft-agent-viewer/   # 运行可视化
├── config/
│   └── agent.toml            # 多后端配置
├── references/               # 参考项目源码（不参与主工程构建）
└── docs/                     # 开发者文档与教程
```

## 快速开始

```bash
# 构建全部成员（azalea-bot 特性按需开启）
cargo build --workspace

# 主入口：LLM 驱动 bot + Web 仪表盘（连入本地 MC 局域网服，端口 4444）
cargo run -p craft-agent-viewer -- --goal "收集木头做工作台" --steps 40 --port 8080
# 浏览器打开 http://127.0.0.1:8080 查看实时对话与启停控制
```

### 配置后端

编辑 `config/agent.toml`，切换 `[llm]` / `[vlm]` 后端即可。

## Crate 文档

| Crate | 说明 |
|---|---|
| [`crates/craft-agent/`](./crates/craft-agent/) | 核心运行时：Agent 主循环、工具系统、会话管理 |
| [`crates/craft-agent-minecraft/`](./crates/craft-agent-minecraft/) | Minecraft 适配器与工具集 |
| [`crates/craft-agent-model/`](./crates/craft-agent-model/) | LLM/VLM 客户端与配置 |
| [`crates/craft-agent-viewer/`](./crates/craft-agent-viewer/) | Web 仪表盘（Axum + SSE） |

## 文档导航

- [`docs/README.md`](./docs/README.md)：文档总览（含教程、设计归档、crate 文档链接）
- [`docs/tutorials/INDEX.md`](./docs/tutorials/INDEX.md)：教程索引（推荐阅读顺序）
- [`ARCHITECTURE.md`](./ARCHITECTURE.md)：分层架构、13 步主循环、44 工具、P56-P58 治理
- [`PLAN.md`](./PLAN.md)：项目计划（azalea-bot 路线 + 通关路径 + 自动化测试协议）
- [`AGENTS.md`](./AGENTS.md)：全自动工作流手册（含 9-bis 节 Mindcraft 哲学约束）
- [`docs/CHANGELOG.md`](./docs/CHANGELOG.md)：变更日志（P55-P58 最新修复）
- [`docs/adr.md`](./docs/adr.md)：架构决策记录（ADR-001 azalea-only / ADR-004 LLM-driven tools）
- [`game-agent-design.md`](./game-agent-design.md)：⚠️ 历史设计文档（部分过时）
- [`CONTRIBUTING.md`](./CONTRIBUTING.md)：贡献指南
- [`CODE_OF_CONDUCT.md`](./CODE_OF_CONDUCT.md)：行为准则
- [`SECURITY.md`](./SECURITY.md)：安全策略

## License

[MIT](./LICENSE) © 2026 Craft-Agent contributors
