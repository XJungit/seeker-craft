# Craft-Agent

通用游戏 Agent 框架（首个落地场景：Minecraft Java 版）。

当前支持两条运行路径：
- **mod-bridge**：通过 Fabric mod 结构化感知与执行，适合后台 / 精确控制。
- **real**：保留截图 + VLM + 键鼠执行，适合真机可视化场景。

## 决策内核

- **感知**：结构化游戏状态 + 可选 VLM 视觉补充。
- **决策**：LLM 基于世界状态生成工具调用（OpenAI 兼容，多后端可配）。
- **执行**：mod 主线程精确执行 / 或键鼠执行。

完整设计见 [`game-agent-design.md`](./game-agent-design.md)，架构概览见 [`ARCHITECTURE.md`](./ARCHITECTURE.md)。

## 工程结构

```
Craft-Agent/
├── Cargo.toml                 # workspace 根
├── Cargo.lock                # 单一锁文件
├── crates/
│   ├── craft-agent/          # 核心：GameAdapter / Agent 主循环 / Session
│   ├── craft-agent-model/    # VLM/LLM 客户端
│   ├── craft-agent-minecraft/# MC 适配器与工具集
│   └── craft-agent-viewer/   # 运行可视化
├── mods/
│   ├── craft-agent-bridge/           # MC Fabric mod（Java）
│   └── craft-agent-bridge-1.21/      # MC 1.21 兼容分支
├── config/
│   └── agent.toml            # 多后端配置
├── references/               # 参考项目源码（不参与主工程构建）
└── docs/                     # 开发者文档与教程
```

## 快速开始

```bash
# 构建全部成员
cargo build --workspace

# mod-bridge 示例
cargo run -p craft-agent-minecraft --example agent_multi_step_mod --features mod-bridge \
  -- --steps=40 --goal="收集木头做工作台" --session=sessions/mc_run_mod.jsonl

# 真机路径
cargo run -p craft-agent-minecraft --example agent_multi_step_mod --features real \
  -- --steps=40 --goal="收集木头做工作台" --session=sessions/mc_run_real.jsonl
```

### 配置后端

编辑 `config/agent.toml`，切换 `[llm]` / `[vlm]` 后端即可。

## Crate 文档

| Crate | 说明 |
|---|---|
| [`crates/craft-agent/`](./crates/craft-agent/) | 核心运行时：Agent 主循环、工具系统、会话管理 |
| [`crates/craft-agent-minecraft/`](./crates/craft-agent-minecraft/) | Minecraft 适配器与工具集 |
| [`crates/craft-agent-model/`](./crates/craft-agent-model/) | LLM/VLM 客户端与配置 |
| [`crates/craft-agent-viewer/`](./crates/craft-agent-viewer/) | TUI 运行仪表盘 |

## 文档导航

- [`docs/README.md`](./docs/README.md)：文档总览（含教程、设计归档、crate 文档链接）
- [`docs/tutorials/INDEX.md`](./docs/tutorials/INDEX.md)：教程索引（推荐阅读顺序）
- [`ARCHITECTURE.md`](./ARCHITECTURE.md)：分层架构与运行时流程
- [`game-agent-design.md`](./game-agent-design.md)：⚠️ 历史设计文档（部分过时）
- [`CONTRIBUTING.md`](./CONTRIBUTING.md)：贡献指南
- [`CODE_OF_CONDUCT.md`](./CODE_OF_CONDUCT.md)：行为准则
- [`SECURITY.md`](./SECURITY.md)：安全策略

## License

[MIT](./LICENSE) © 2026 Craft-Agent contributors
