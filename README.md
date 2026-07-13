# Craft-Agent

通用游戏 Agent 框架（首个落地场景：Minecraft Java 版）。

纯视觉路线：截图 → VLM 语义理解 → LLM 决策 → 键鼠执行，不依赖任何游戏内置 API / 内存读取。

## 决策内核

- **感知（VLM）**：识别画面中的方块 / 生物 / UI 状态，输出自然语言场景描述。
- **决策（LLM）**：基于场景描述生成下一步动作（OpenAI 兼容，多后端可配）。
- **执行（enigo）**：把动作映射成 Windows 键鼠操作，驱动游戏。

完整设计见 [`game-agent-design.md`](./game-agent-design.md)。

## 工程结构（Cargo workspace）

```
Craft-Agent/
├── Cargo.toml              # workspace 根：[workspace] + [workspace.dependencies]
├── Cargo.lock             # 单一锁文件（成员不自带）
├── crates/
│   ├── craft-agent/        # 核心：GameAdapter trait / Agent 主循环 / Action
│   └── craft-agent-model/  # VLM 感知 + LLM 决策（OpenAI 兼容多后端）
├── phase0_verify/
│   └── enigo_mc_test/      # 真机验证脚手架（enigo 驱动 MC 视角 / xcap 截图）
├── config/
│   └── agent.toml          # 多后端配置，active 字段一键切换
└── scripts/
    └── check_structure.sh  # 结构自审（单一 lock / 单一 target / 依赖走 workspace）
```

## 快速开始

```bash
# 构建全部成员
cargo build --workspace --features real

# 跑单元测试（real feature 内的测试均为离线单测，不触网）
cargo test --workspace --features real

# 用真实截图探一下 VLM（需配置对应后端 api key 环境变量）
cargo run -p craft-agent-model --example vlm_probe --features real -- \
    phase0_verify/enigo_mc_test/mc_capture.png --config config/agent.toml

# 探一下 LLM 决策
cargo run -p craft-agent-model --example llm_probe --features real -- --config config/agent.toml
```

### 配置后端

编辑 `config/agent.toml`，改 `[vlm].active` / `[decision].active` 即可在 `minicpm` / `agnes` 等后端间切换，无需改代码。每个后端可设 `max_side`（VLM 输入最长边缩放，省带宽 / token；`0` 表示不缩放）。

## 质量门禁

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `bash scripts/check_structure.sh`（防"结构细菌"复发：单一 lock / 单一 target / 依赖走 workspace）
- `cargo test --workspace --features real`

CI（`.github/workflows/ci.yml`）在 GitHub 上自动跑上述全部；本仓库随附 `git` pre-commit 钩子做本地拦截。

## License

[MIT](./LICENSE) © 2026 Craft-Agent contributors
