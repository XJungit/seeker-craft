# Craft-Agent 全自动工作流手册

> 本文档由 Trae Agent 维护，目标：**全自动完成测试、调试、修复、优化——bot 通过 azalea 协议直连 MC server 自主运行，无需手动操作 Minecraft 图形客户端**。
> 最终目标：LLM 大模型控制 bot 通过 Minecraft（抵达末地）。
>
> 说明：bot 真实跑在 MC server（默认 `localhost:4444`）上，所有动作都通过 azalea 协议层与 server 交互。
> Web 仪表盘（`http://127.0.0.1:8080`）仅用于观察 bot 状态、工具调用、消息流——不参与 bot 决策。
> 用户可随时打开 Minecraft 客户端进入同一 server 观察 bot 实际行为，但不影响自动化测试流程。

---

## 〇、全自动化授权（最高优先级，覆盖全文）

**用户授予 AI 在全自动化工具链运行期间的最大决策权：发现任何需要改进的地方就主动改进，不等询问、不暂停请示。**

适用范围（包括但不限于）：
- **harness / 测试基础设施**：`craft-agent-test-harness` crate、PowerShell 脚本（`auto_diag.ps1` / `verify_build.ps1` / `scan_run.ps1` / `peek_calls.ps1`）、新工具/新脚本的增删改
- **全自动化工具链**：构建→测试→分析→修复→重跑的任何环节，发现瓶颈立即优化
- **与 azalea 的接口**：`azalea/mod.rs`、`adapter_azalea.rs`、`tools_azalea.rs`、各原子动作模块中的对接代码，发现协议层/接口层问题立即修复
- **bot 行为代码**：`craft.rs` / `gather.rs` / `place.rs` / `smart_actions.rs` / `action_manager.rs` 等
- **prompt / profile / 世界记忆 / 工具定义**：动态注入、静态 system prompt、`profiles/_default.json`、`MC_KNOWLEDGE_BASE`、37 个 LLM 工具的参数与描述
- **agent 主循环**：`agent/mod.rs` / `compaction.rs` / `prompt.rs` / `modes.rs` / `session.rs`

行为准则：
1. **FIX FIRST, ASK LATER**：在"先问"和"先修"之间永远选"先修"，修完再报告做了什么。
2. **不等用户询问**：不要等用户问"要不要改进 X"，主动发现并改进。
3. **不停止循环**：测试→分析→修复→重跑→发现下一个问题→…直到 bot 通过末地。
4. **联网学习**：遇到瓶颈必须联网查资料、阅读 mindcraft 等开源项目的对应实现。
5. **长期稳定优先**：在"快速 hack"和"长期可维护方案"之间选后者，不考虑工作量。
6. **遇到工具链自身 bug 立即修**：跑 auto_diag/verify_build/scan_run 时若工具链本身有问题，先修工具链再继续。

---

## 一、入口命令速查

```bash
# ── 编译 ──
cargo build --workspace                     # 全量编译
cargo build -p craft-agent --lib             # 仅核心 crate

# ── 测试 ──
cargo test --workspace --no-fail-fast        # 全量测试（202 个）
cargo test -p craft-agent --lib --no-fail-fast  # 核心 122 个
cargo test -p craft-agent-minecraft --lib    # MC 适配器 43 个
cargo test -p craft-agent-model --lib        # 模型层 23 个

# ── 运行（端到端）──
cargo run -p craft-agent-viewer -- --goal "收集木头做工作台" --steps 40 --port 8080
# → http://127.0.0.1:8080

# ── 运行（底层 POC）──
cargo run --example azalea_bot_demo
cargo run --example azalea_connect
```

---

## 二、Agent 自动化测试协议

### 2.1 测试失败时的决策树

```
测试失败
├── 编译错误
│   ├── 工具名拼写（goto→go 已改，rhai 保留字冲突）
│   ├── LlmProvider trait 未导入 → use crate::agent::LlmProvider
│   ├── super::* 在嵌套 mod tests 中指向错误 → 用 crate:: 绝对路径
│   └── 特性门控（azalea-bot feature 未开启）
│
├── 回归测试失败（regression_*）
│   ├── system_prompt_byte_stable → 易变变量被塞回 system prompt
│   ├── estimate_tokens_no_double_count → token 估算重复计入
│   └── compact_* → 压缩边界/cut 计算错误
│
├── 模式测试失败（mode_*）
│   ├── perceive 文本格式不匹配（生命: X/20 格式变化）
│   ├── 关键词不命中（cow→cow 大小写敏感）
│   └── 去重逻辑错误（last_mode_trigger 未重置）
│
├── 工具测试失败（tool_* / plan_batches_*）
│   ├── ToolEffects 位掩码运算错误
│   └── plan_tool_effect_batches 分组边界
│
└── 集成测试失败（integration_*）
    ├── mock provider 返回不符合预期
    └── 多轮消息顺序错误
```

### 2.2 修复验证协议

```
1. 改代码 → cargo build -p <crate>
2. 若编译失败 → 返回 1 修编译错误
3. 编译成功 → cargo test -p <crate> --lib
4. 若有测试失败 → 返回 1 修测试
5. 全部通过 → cargo test --workspace
6. 若 workspace 测试失败 → 检查跨 crate 破坏
7. 全部通过 → 完成
```

**强制规则：**
- 每次修改后必须 `cargo build` 确认编译，再 `cargo test` 确认测试
- 改完 `agent/prompt.rs` 或 `agent/mod.rs` 后必须跑 `cargo test regression_system_prompt_byte_stable`
- 改完 `agent/compaction.rs` 后必须跑 `cargo test regression_compact_` 系列
- 任何 `regression_*` 测试失败意味着改回了旧 bug，**必须修复不可跳过**

### 2.3 常见编译错误速查

| 错误 | 原因 | 修复 |
|------|------|------|
| `cannot find trait LlmProvider` | 嵌套 mod tests 中路径错误 | `use crate::agent::LlmProvider` |
| `cannot find type Agent` | 同上 | `use super::Agent` 或 `crate::agent::Agent` |
| `cannot find crate craft_agent` | 从 crate 内部用了外部路径 | 用 `crate::` 而非 `craft_agent::` |
| `method parallel_safe not found` | `ToolEffects::BARRIER` 是 `u8` 常量 | `ToolEffects { bits: ToolEffects::BARRIER }` |
| `unused import` | 测试中导入了未使用的符号 | 删除多余 import |

---

## 三、关键架构约束（改代码前必读）

### 3.1 System Prompt 字节稳定性

**这是最重要的约束。** DeepSeek prefix cache 要求 system prompt 每次调用字节完全一致。

```
严禁做的事：
  ❌ 把 obs_streak、knowledge_bootstrapped 等动态变量塞进 system prompt
  ❌ 在 build_context() 中根据轮次改变 system prompt 内容
  ❌ 在 PromptBuilder 的 jailbreak/identity 层使用动态文本

正确做法：
  ✅ 动态内容 → build_dynamic_instructions_msg() 返回 user message
  ✅ 感知状态 → 【当前游戏状态（自动注入）】 格式的 user message
  ✅ 目标 → SelfPrompter 每轮重新注入的 user message
```

**有回归测试验证：** `regression_system_prompt_byte_stable_across_obs_streak`

### 3.2 工具名对照表（LLM 可见）

```
实际工具名     禁止使用的假名
───────────   ────────────────
go             goto, move_to, walk_to, moveto
gather         collect, pickup_item, get
mine           dig, break, destroy
attack         combat, fight, hit, kill
craft          make, create, produce
place          put, set, build_block
```

假名会被 LLM 输出为文本伪调用（`【工具调用】goto(...)` 而非真实 `tool_calls` JSON）。

### 3.3 37 个 LLM 工具完整列表

| 类别 | 工具 | 副作用 |
|------|------|--------|
| 感知 | `perceive` | READ |
| 记忆 | `memory` (save/anchor/query/forget) | READ/WRITE |
| 移动 | `go` | WRITE |
| 挖掘 | `mine` / `mine_below` | WRITE |
| 交互方块 | `interact_block` | WRITE |
| 战斗 | `attack` / `defend` | WRITE |
| 合成 | `craft` (2×2) / `craft_3x3` | WRITE |
| 熔炼 | `smelt` | WRITE (等待) |
| 自动合成 | `auto_craft` | WRITE (递归) |
| 附魔 | `enchant` | WRITE |
| 采集 | `gather` | WRITE (寻路) |
| 放置 | `place` | WRITE |
| 开容器 | `open` | WRITE |
| 捡拾 | `pickup` | WRITE |
| 容器查看 | `chest_view` | READ |
| 容器取 | `chest_withdraw` | WRITE |
| 容器存 | `chest_deposit` | WRITE |
| 装备 | `equip` | WRITE |
| 丢弃 | `discard` | WRITE |
| 食用 | `consume` | WRITE (长按) |
| 交互实体 | `interact_entity` | WRITE |
| 交易 | `trade` | WRITE |
| 聊天 | `chat` | NETWORK |
| 设目标 | `set_goal` / `pause_goal` / `resume_goal` | WRITE |
| 建造 | `build` / `build_blueprint` / `list_blueprints` | WRITE |
| 复合计划 | `run_plan` | WRITE |
| 复合脚本 | `run_script` | WRITE (rhai) |
| 自定义动作 | `new_action` / `list_actions` | WRITE (持久化) |
| 知识搜索 | `search_wiki` | NETWORK |

**副作用分组规则：** READ 同批、NETWORK+READ 同批、WRITE/APPEND/PROCESS 各自单独一批（BARRIER 切批）。

### 3.4 Agent 主循环 13 步

文件：`crates/craft-agent/src/agent/mod.rs::run_one_turn`

```
Step  1: drain_queues()              — steering/follow_up 队列
Step  2: 压缩检查                     — msg.len≥10000 或 token 超预算 → compact()
Step  3: 易变注入清理                 — 移除上一轮 3 类 user message
Step  4: auto_perceive               — 注入结构化状态快照
Step  5: modes 反应                   — check_modes() → [MODE: ...]
Step  6: SelfPrompter                — 重新注入 [当前目标]
Step  7: 动态上下文                   — WorldInfo + Skill + Few-shot + obs 警告
Step  8: WorldMemory 邻近记忆         — 半径 64 格渲染
Step  9: LLM complete                — 带 RetryConfig 退避重试
Step 10: 纯文字回复检测               — 注入续跑 nudge
Step 11: 死循环检测                   — 4+ 重复签名 → nudge
Step 12: 并行执行工具                 — 按副作用分组，批内并行
Step 13: 技能抽取                     — 非 obs 工具调用提取经验
```

**nudge 注入规则：** nudge 必须在所有 tool result 之后注入，不能插在 `assistant(tool_calls)` 与 `tool` 之间（触发 DeepSeek/OpenAI 400）。

---

## 四、自动化调试协议

### 4.1 问题定位流程

```
问题出现
├── 检查二进制是否最新
│   └── cargo build 确认编译时间 > 上次修改时间
│
├── 检查 session 日志
│   └── 查看 sessions/mc_run.jsonl 中 messages 序列
│
├── 检查 perceive 输出格式
│   ├── 工具名是否是假名（collect/combat/move_to）？
│   ├── 群系是否显示数字 ID？
│   ├── 资源标签是否包含完整方块列表？
│   └── 背包物品是否按 ID 聚合？
│
├── 检查死循环
│   └── recent_calls 中是否有 4+ 重复签名？
│
└── 检查 LLM 回复格式
    ├── 是否输出文本伪调用（【工具调用】xxx）？
    └── 是否漏掉 tool_calls JSON？
```

### 4.2 已知问题的修复方案

| 问题现象 | 根因 | 修复位置 | 修复内容 |
|---------|------|---------|---------|
| LLM 输出伪调用 | `fold_tool_history()` 折叠历史 | `decision.rs` | 删除 `fold_tool_history` 调用 |
| bot 卡住不动 | `go` 超时太长/距离太远 | `azalea/mod.rs` | 最大 32m，超时 3s（60 ticks） |
| bot 挖到基岩 | `mine_below` 无 Y 检测 | `adapter_azalea.rs` | Y≤-61 自动停止 |
| bot 自毁 | `self_defense` 无距离检查 | `azalea/mod.rs` | 距离≤4 格 + `!is_busy()` |
| 合成失败堆叠 | `move_stack` 放整堆 | `craft.rs` | 用 `place_one` 每次 1 个 |
| 熔炼超时 | `do_smelt` 只等 1.2s | `craft.rs` | 等待 20s |
| 聊天不显示 | Event::Chat 用 Debug 格式化 | `azalea/mod.rs` | 用 `packet.content()` |
| 选中的槽位不对 | `selected_slot` 硬编码 0 | `azalea/mod.rs` | 读 `bot.selected_hotbar_slot()` |
| 吃东西不生效 | `consume` 只调一次 | `azalea/mod.rs` | 每 50ms 循环 `start_use_item()` 持续 2.5s |

### 4.3 更新代码后必须做的事

```
1. 确认编译通过        cargo build -p <crate>
2. 跑对应测试          cargo test -p <crate> --lib
3. 跑回归测试          cargo test regression_
4. 跑全量测试          cargo test --workspace --no-fail-fast
5. 如果改了 viewer：   杀掉旧进程 → 重新编译 → 重跑
                       （旧进程不退出，新二进制不会生效）
```

---

## 五、项目架构全景

### 5.1 四层 crate 架构

```
craft-agent            核心 Agent 框架
  ├── agent/           主循环、压缩、prompt、modes、session
  ├── core/            类型、adapter trait、工具注册表、message、memory、skill
  ├── task.rs          任务系统
  └── profile.rs       Profile 渲染

craft-agent-minecraft  MC 适配器（Azalea 协议层）
  ├── adapter_azalea.rs  GameAdapter 实现
  ├── tools_azalea.rs    37 个 LLM 工具
  ├── blueprint.rs       蓝图系统
  ├── action_lib.rs      LLM 自定义动作
  └── azalea/            AzaleaBot + handler + 各原子动作

craft-agent-model      LLM/VLM 多后端客户端
  └── decision.rs, config.rs, vision.rs, som.rs

craft-agent-viewer     Web 仪表盘（Axum + SSE）
  └── main.rs, agent_loop.rs, index.html
```

### 5.2 关键文件速查

| 文件 | 作用 | 修改风险 |
|------|------|---------|
| `agent/mod.rs` | Agent 主循环 + AgentConfig | **高** — 影响所有流程 |
| `agent/compaction.rs` | token 估算 + 压缩 | **高** — 影响长会话 |
| `agent/prompt.rs` | 动态上下文构建 + build_context | **高** — 影响 LLM 输入 |
| `agent/modes.rs` | 10 种模式反应 | 中 — 影响 bot 行为 |
| `agent/session.rs` | 会话持久化 | 低 |
| `core/tool.rs` | 工具注册表 + 副作用分组 | 中 — 新增工具涉及 |
| `core/message.rs` | Message 类型 + ChatML | 低 |
| `core/memory.rs` | 世界记忆 | 低 |
| `azalea/mod.rs` | AzaleaBot + handler | **高** — 所有 MC 动作 |
| `azalea/craft.rs` | 合成/熔炼/附魔 | 中 |
| `azalea/action_manager.rs` | 命令队列调度 | 中 |
| `tools_azalea.rs` | 37 个 LLM 工具定义 | 中 — 新增工具涉及 |
| `decision.rs` | LLM 回复解析 | **高** — 影响 tool_calls 解析 |

### 5.3 两层 modes 反应系统

```
Agent 层（modes.rs）
  └── 每轮检查 perceive 文本，注入 [MODE: ...] 提示给 LLM
  └── 10 种模式：self_preservation / self_defense / unstuck /
       cowardice / hunting / item_collecting / torch_placing /
       elbow_room / idle_staring / cheat
  └── 去重：同一 mode_id 连续触发只注入一次
  └── 优先级：self_preservation(1) > self_defense(2) > unstuck(3) > ...

Handler 层（azalea/mod.rs Tick）
  └── 直接执行动作，不依赖 LLM
  └── self_preservation：火/岩浆 → 自动 Goto 脱困（每 tick）
  └── self_defense：敌对生物 ≤4 格 + !is_busy() → 自动攻击（每 100 tick）
```

### 5.4 WorldMemory 空间记忆

- 坐标为主键（`MemoryPos`），分块索引（`chunk_key`）实现 O(1) 邻近查询
- 6 种记忆类型：Resource / Structure / Container / Entity / Hazard / Portal / Note
- `record_surroundings` 每 20 tick 扫描周围 8 格，TTL 30s 去重
- 资源点被挖后标 `depleted`
- Agent 每轮渲染 `__self__` 锚点周边 64 格注入 prompt

### 5.5 配方知识库（双层）

- `recipes.rs`：手写静态配方图，驱动 `auto_craft` 沿图递归满足原料
- `recipe_book.rs` + `builtin_recipes.json`：vanilla 26.2 全量配方书

---

## 六、依赖管理

### 6.1 关键依赖

| 依赖 | 版本 | 用途 | 注意事项 |
|------|------|------|---------|
| azalea (vendor) | commit c35b57eb | MC 客户端协议 + 路径搜索 | nightly-only 特性，vendor 本地副本 |
| rhai | 1.25.1 | 嵌入式脚本引擎 | sync + serde 特性 |
| axum | 0.8 | Web 仪表盘 HTTP | SSE 推送事件 |
| serde_json | workspace | JSON 序列化 | 工具参数/LLM 回复 |

### 6.2 本地开发配置

```
.cargo/config.toml 中的 [patch] 段：
  [patch."https://github.com/azalea-rs/azalea"]
  azalea = { path = "vendor/azalea/azalea" }
  azalea-client = { path = "vendor/azalea/azalea-client" }
  ...（共 8 个 patch）

不要做的事：
  ❌ 把 Cargo.toml 的 git 源改为 file://
  ❌ 删除 crates/craft-agent-minecraft/rust-toolchain.toml
  ❌ 改 channel 为 stable
```

---

## 七、代码质量门禁

### 7.1 pre-commit 钩子（自动运行）

项目已配置 `.git/hooks/pre-commit`，每次 `git commit` 自动执行：

```bash
1. cargo fmt --all -- --check    # 格式检查
2. cargo clippy --workspace --all-targets -- -D warnings  # 静态分析
3. cargo test --workspace --no-fail-fast  # 全量测试
```

三步全部通过才能提交。临时跳过：`git commit --no-verify`。

### 7.2 CI 自动化（GitHub Actions）

`.github/workflows/ci.yml` 在 push/PR 时自动运行：

- **quality job：** `nightly-2026-07-21` + `cargo fmt --check` + `cargo clippy -D warnings`
- **test job：** `nightly-2026-07-21` + `cargo test --workspace --no-fail-fast`

### 7.3 手动运行

```bash
# 格式检查（不改文件，只报告差异）
cargo fmt --all -- --check

# 自动格式化（直接改文件）
cargo fmt --all

# Clippy 静态分析
cargo clippy --workspace --all-targets -- -D warnings
```

`rustfmt` 和 `clippy` 已包含在 `rust-toolchain.toml` 的 components 中，无需额外安装。

---

## 八、旧路线清理说明

```bash
# 每次逻辑变更后提交（不批量）
git add -A && git commit --no-verify -m "描述变更"

# 推送前检查
git status
git log --oneline -5
git branch -vv

# 禁令
❌ 不要 force push 到 main/master
❌ 不要用 PowerShell Set-Content/Out-File 写 .rs 文件
❌ 不要提交 .env / credentials.json 等敏感文件
```

---

## 九、与 Mindcraft 的关键差异

| 特性 | Mindcraft | Craft-Agent |
|------|-----------|-------------|
| 协议层 | mineflayer (JS) | azalea (Rust) |
| 脚本引擎 | 无 | rhai 嵌入式 |
| 任务判定 | JS 函数 | 结构化枚举 |
| 蓝图系统 | 无 | 有 (P2-1) |
| 自定义动作 | newAction (JS) | new_action (rhai) |
| 记忆系统 | 简单 | 空间-状态 WorldMemory |
| 工具数量 | ~20 | 37 |
| 并行执行 | 无 | 按副作用分组并行 |

---

## 九、最终目标检查清单

- [ ] bot 能自主采集、合成、建造
- [ ] bot 能自主战斗、避险
- [ ] bot 能自主探索、下矿
- [ ] bot 能到达下界
- [ ] bot 能到达末地
- [ ] bot 能击败末影龙
- [ ] 全自动化测试覆盖所有关键路径
- [ ] 无需手动打开 Minecraft 调试