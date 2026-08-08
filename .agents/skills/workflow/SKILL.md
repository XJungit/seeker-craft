---
name: workflow
description: Craft-Agent 持久维护工作流——先读 NOTEBOOK.md，按观察→检测→对比→假设→研究→实验→验证→学习的证据驱动循环迭代
---

# 技能: workflow

# Craft-Agent 工作流

将 `NOTEBOOK.md` 视为持久工程记忆。每次开始或恢复维护前先阅读。每轮证据驱动的迭代完成后追加一条记录。当通用工作流规则改进时更新此技能。

## 使命

持续改进固定账号 `CraftAgent`，直到在生存模式下击败末影龙。在此过程中改进 Agent 框架、Azalea 适配器、自动化工具、Minecraft 知识和运行稳定性。编译、测试、轮次和工具调用次数是证据，不是完成标准。

## 六条科学保证

1. **完整记录**：持久化结构化状态、游戏状态、会话指标、错误、假设、变更和验证结果。不依赖聊天记忆。
2. **异常检测**：维护历史指标窗口，标记实质性偏差，不需要预定义失败标签。
3. **对比分析**：比较成功与失败的执行过程、变更前后的状态，而不是从单次跟踪猜测。
4. **网络搜索**：在解决不熟悉或重复出现的问题之前，搜索 Mindcraft、Azalea、Minecraft Wiki 等相关来源。
5. **实验验证**：提出可证伪的预测，在可行时只改变一个相关变量，通过实机状态加测试验证。
6. **知识沉淀**：将时间顺序证据追加到 `NOTEBOOK.md`；将反复验证的经验升级为稳定规则写入此技能。

使用循环：**观察 → 检测 → 对比 → 假设 → 研究 → 实验 → 验证 → 学习 → 继续**。编译通过不是实验确认；旧失败场景必须在实机状态下改善。

## 迭代循环

0. **差距分析先行**：扫 `docs/mindcraft-gap.md` 优先级队列，找出主线收益最高的缺失项；
   禁止跳过差距分析直接修 bug（被动修补 = 流程缺陷）。当前工作单元收益最高项见
   NOTEBOOK 末尾 "Next"。
1. 工具套件驱动观测：确认 autopilot 在跑（`craft-agent-ctl status`；不在则
   `ctl build` + `ctl viewer "<主线目标>" 0` + `ctl start` 拉起）。
   证据从套件留证读：`ctl tail auto5_out.log 30` + `sessions/events/workflow.jsonl` 尾部
   （observation/anomaly）+ `GET /api/game-state`。不要手工盯日志。
2. 从实机证据确认一个具体失败或缺失能力——工具层用 probe（秒级），区分
   harness bug vs LLM 决策；策略/规划行为才按工作单元开 LLM 实机观测
3. 先搜索互联网解决不熟悉或重复出现的 Minecraft/Azalea/Mindcraft 问题
4. 定位根因：框架、适配器、工具、提示词或策略
5. 在 `vendor/azalea` 之外做最小正确修改
6. 运行聚焦测试 + 相关 crate 测试 + `git diff --check` + `cargo fmt --all -- --check`，全绿后再提交
7. 仅必要时部署。除非新二进制必须加载，否则重用现有 Viewer
8. 验证真实状态增量：库存里程碑、位置/维度、生命/生存恢复、成功生产或末影龙击杀
   （以结构化 game-state 为准，不看 LLM 工具返回文本）
9. 回填 `docs/mindcraft-gap.md` 状态 + 追加到 `NOTEBOOK.md`，报告给用户，然后立即继续

## 迭代纪律（总纲下沉，2026-08-02 起）

- **先测试锁定行为**：任何改动（尤其重构）前，先写/确认针对性测试锁定现有行为；改动后全量验证
- **行为不变原则**：工具名、消息格式、prompt 文本、任务/配置文件 schema 绝不因重构而变
- **单提交单关注点**：一次改动一个提交，回滚粒度 = 单次提交；纯移动/重命名提交的 diff 只有位置变化
- **全量门槛**：每次改动后 `cargo test --workspace` + `cargo fmt --all -- --check` + clippy `-D warnings` 全绿才算完成
- **可回滚**：风险实验前先 `git add -A && git commit --no-verify -m "wip: checkpoint"`；
  绝不 `git checkout`/`reset`/`restore`/`stash` 回滚（用可见、逐行可回滚的方式）
- **文档同步**：每次迭代回填 `docs/mindcraft-gap.md` 状态与 NOTEBOOK.md；流程变迁时回写本技能与 AGENTS.md
- **双点同步防线**：新增/变更工具、动作、消息格式时四处同步（见 AGENTS.md「新增能力纪律」）+ 回归测试兜底
- **推送纪律（2026-08-06 起）**：功能类工作单元推送前必须 probe 实机验证或 LLM 实机观测确认真有效果；
  编译通过/单测通过 ≠ 实际效果已验证——未实测的功能只本地提交，绝不推送。推送前自检：
  `cargo fmt --all -- --check` + `cargo clippy --workspace --all-targets --features craft-agent-minecraft/azalea-bot -- -D warnings` + `cargo test --workspace` 全绿

一轮工程工作流不是一次 LLM 轮次，也不是一次会话边界。它涵盖实机证据、诊断、变更、测试、部署和实机验证。完成或报告一轮工作流不得停止 Agent 或替换其活跃对话。

## 运行时规则

- 保持一个 Viewer 和一个 `CraftAgent` 登录。不轮换用户名。
- 不因监控轮次而重启 Viewer。仅在崩溃、断线或已验证的二进制更新后重启。
- 通过 `POST /api/start` 启动 Agent；进程存在不代表它正在运行。
- 长期观测交给 autopilot（10s 轮询 + 停滞 steering + 崩溃恢复 + 异常留证）；
  人工只做按需 `ctl status`/`ctl tail`/`ctl session` 抽查与 probe 级验证。
- 监控 `/api/status`、`/api/game-state`、`sessions/mc_run.jsonl`、`sessions/events/workflow.jsonl`
  和 Viewer 日志。autopilot 负责其中大部分，人工抽查。
- `sessions/mc_run.jsonl` 是 Minecraft 对话/会话：一个头部加追加树条目。Viewer 重启时重新打开；每轮追加新条目。不用于工程笔记。
- 工程笔记本是 `.agents/skills/workflow/NOTEBOOK.md`。
- 不自动暂存或提交无关工作树变更。仅在被明确要求时提交。
- 不在固定数量的失败假设后停止。记录失败，改变方法，继续，除非用户停止或外部依赖不可用。
- 未经用户明确命令，不得停止或暂停整体维护工作流。Agent 退出、Viewer 失败、测试失败、轮次报告或完成的里程碑必须触发恢复或下一次迭代，而不是终止。
- 进程隔离是强制的：永远不使用 `taskkill /T`、递归子进程终止、宽泛的进程名杀死或端口所有者杀死而不验证可执行文件身份。Viewer 替换只能终止已验证的 `craft-agent-viewer` PID；Minecraft、OpenCode、终端及其进程树是外部基础设施。
- 资源隔离是强制的：永远不并行运行多个 Cargo 构建/测试，不在交互式 OpenCode 运行时触发完整全目标重建。串行运行重型 Rust 工作，使用 `CARGO_BUILD_JOBS=2`，重用正常目标缓存，在构建和部署阶段之间验证 OpenCode/Minecraft 健康。

## 七条额外工作流规则

1. **所有修改除 vendor/azalea 外**：可改范围包括所有 crates（craft-agent、craft-agent-minecraft、craft-agent-model、craft-agent-viewer、craft-agent-autopilot）、profiles、tasks、blueprints、actions、config、scripts、docs、.agents 配置和根目录文件。唯一不能改的是 `vendor/azalea/` 和 `.git/`。

2. **更新旧测试**：每次修改后，不仅要添加新的回归测试，还要检查被改功能是否有旧测试需要更新语义。跑 `cargo test --lib` 全量通过后，人工检查是否有语义上被新行为覆盖但没失败的旧测试。

3. **主动优化提示词、配置、任务 JSON**：当 LLM 连续出现同一种错误模式时，先优化 `_default.json` 提示词和任务 JSON，再考虑改代码。这些不需要 Rust 编译就能生效，优先级高于代码修改。

4. **实机状态来自 Azalea 客户端，而非 LLM 对话**：LLM 工具返回的文本不可靠。验证进度时，必须通过 `api/game-state` 的结构化 JSON（position、health、hunger、inventory）确认，不能只看工具返回文本。BotEvent::State（perceive 注入）是中可靠来源，仅供 LLM 决策用。

5. **工作流状态文件**：项目根目录 `.workflow_state.json` 记录当前阶段。五个阶段：`investigate`（分析根因）、`fixing`（编码修复）、`testing`（测试验证）、`deploying`（部署到实机）、`observing`（观察实机运行）、`blocked`（等待外部依赖）。每次切换阶段时更新。

6. **文件结构**：`data/` 目录存放所有可配置数据（tasks、profiles、blueprints、actions、config），`scripts/` 目录存放辅助脚本，`sessions/` 存放运行时数据。根目录只保留代码和关键配置文件。

7. **目录不存在时回退**：`data/profiles/`、`data/blueprints/`、`data/actions/` 等目录不存在时，代码应静默回退到空状态，而不是崩溃。`ToolRegistry` 和 `BlueprintLibrary` 等已支持空初始化。

## 进度标准

已验证的进度至少包括以下之一：

- 库存变化且通过感知/API 观察到新状态
- 获得持久里程碑：工具、盔甲、食物、钻石、黑曜石、传送门、烈焰棒、末影之眼、末地访问或末影龙击杀
- 位置有意义地向有效目标移动，包括 Y 级或维度变化
- 以前失败的工具在相同复现场景中成功完成
- 崩溃/死锁被移除，实机 Agent 继续通过旧的失败点

以下不独立计为进度：编译成功、经过轮次、Agent 步数、LLM 输出、fire-and-forget 成功消息或未经验证状态变化的工具调用。

## 会话处理

- `Session::open` 恢复现有 JSONL 树和当前叶子
- `Session::save` 追加仅持久化高水位线后的条目
- 头部更新可能重写第一行；压缩/检查点保持可恢复性
- 跨 Viewer 重启保留 `sessions/mc_run.jsonl`，使游戏推理和 WorldMemory 存活
- `sessions/mc_run.jsonl` 是一个连续活跃对话，不是每 Agent 轮次或工程轮次一个文件。Viewer 重启重置 UI 步数计数器，但不重置持久化对话轮次/历史
- 保持活跃会话有界。在受控 Viewer 重启时，使用经过测试的会话 rollover 操作将原始 JSONL 逐字节归档，并创建包含恢复摘要、当前目标、有效 WorldInfo 操作和最新选定分支 WorldMemory 的紧凑活跃 `mc_run.jsonl`
- 如果文件损坏，在允许新会话前归档证据；永远不静默将丢失历史视为成功

## 用户交互

- 用户问题、状态请求和补充要求是带外协作。回答或纳入它们，然后自动继续活跃工作流。
- 不将用户消息解释为暂停。仅在明确的暂停/停止指令或需要用户操作的外部阻塞时暂停。
- 轮次报告是进度检查点，不是交接。报告后立即开始下一次迭代。
- 当连续维护活跃时，永远不为检查点、解释、恢复的失败或完成的轮次发送 `final` 响应。使用简短进度更新并继续执行。仅在用户明确命令停止/暂停或外部阻塞确实需要用户操作时，才允许 `final` 响应。
- 自主做出常规工程、诊断、研究和优先决策。仅在真正不可逆/产品决策无法推断时询问。

## 笔记本格式

每轮追加一个章节：

```markdown
## Round N - YYYY-MM-DD HH:MM
- Evidence: 可观察的失败和游戏状态
- Root cause: 确认的机制
- Change: 更改的文件和行为
- Verification: 测试和实机结果
- Learning: 可复用的规则或拒绝的假设
- Next: 下一个实验
```

将稳定规则保留在此文件中，将时间顺序细节保留在 `NOTEBOOK.md` 中。

## 关键接口

- Viewer 状态: `GET http://127.0.0.1:8080/api/status`
- 游戏状态: `GET http://127.0.0.1:8080/api/game-state`
- 启动 Agent: `POST http://127.0.0.1:8080/api/start`
- 停止 Agent: `POST http://127.0.0.1:8080/api/stop`
- 注入目标: `POST http://127.0.0.1:8080/api/goal` with `{"goal":"..."}`
- 运行时会话: `sessions/mc_run.jsonl`
- Viewer/Autopilot 日志: `craft-agent-ctl` 重定向到 `C:\Windows\TEMP\opencode\viewer_run.log(.err)` 等（`ctl status` / `ctl tail <log> <N>` 查看）

## 工具套件速查（2026-08-08 起：工作流必须用工具套件驱动）

> 定位：**probe = 工具层秒级验证（首选）**；**autopilot = 长期观测/恢复/留证**；
> **viewer = LLM 实机观测（按工作单元启动）**；**ctl = 运维入口**。
> 跑工作流 ≠ 自己盯着日志；启动 autopilot 让它持续观测、检测异常、恢复运行时、注入目标。

### 自动化测试套件（autopilot）——持续迭代的引擎

```bash
cargo run -p craft-agent-ctl -- build     # 编译 viewer+autopilot
cargo run -p craft-agent-ctl -- deploy    # 全流程：stop→build→viewer→autopilot→start
# 分步部署（ctl deploy 的 build 可能超时，更稳）：
#   1) ctl stop → 2) ctl build → 3) ctl viewer "goal 文本" 0（steps=0 无限循环）
#   → 4) ctl start（必须手动）→ 5) ctl status 验证 running=true
cargo run -p craft-agent-ctl -- status    # 进程 + API + game-state + 日志尾
cargo run -p craft-agent-ctl -- tail auto5_out.log 30   # autopilot 日志（观测/恢复/steering 记录）
cargo run -p craft-agent-ctl -- session 20             # 最近会话工具活动
cargo run -p craft-agent-ctl -- goal "<目标>"          # 注入目标（等价 POST /api/goal）
cargo run -p craft-agent-ctl -- health 300             # 健康监控：步数推进观察（5s 轮询）
```

autopilot 启动后**自动**执行：workspace check → 拉起/复用 viewer → start agent →
10s 轮询 `/api/status` + `/api/game-state` + session 分析 → 无进展 240s 判停滞并 steering
注入 → viewer 崩溃/API 连续失败自动恢复运行时（替换 viewer 防 duplicate_login）→
**异常检测**（死亡/重生/装备丢失/濒死恢复，见下）。

### 实机观测证据流（autopilot 自动留证）

- `sessions/events/workflow.jsonl`：逐 10s 追加结构化观察（`type:"observation"`：status、
  position/health/inventory、session 指标）与异常（`type:"anomaly"`：death/respawn/
  armor_loss/near_death，含详情+时间戳）。**迭代证据以此文件为准**，不要只靠聊天记忆。
- `sessions/events/supervisor_state.json`：监督器阶段（Starting/Monitoring/
  RecoverRuntime/SteeringStall）、恢复次数、停滞次数、last_error。
- 卡住诊断顺序：`ctl status`（进程/API）→ `ctl tail auto5_out.log 50`（最近 steering/
  recovery/anomaly）→ `ctl session 20`（LLM 在干什么）→ `GET /api/game-state`（实机状态）。

### 工具层验证（probe，秒级，不开 LLM）

```bash
cargo run -p craft-agent-minecraft --example azalea_probe --features azalea-bot -- 4444 --cmd "equip iron_helmet helmet"
cargo run -p craft-agent-minecraft --example azalea_probe --features azalea-bot -- 4444 --script scripts\probe\smoke.json
```

- 工具层 bug 用 probe 复现/验证（秒级），需要 LLM 决策的策略/规划行为才开 viewer+agent 实机观测。
- 支持命令见 `azalea/commands.rs::parse_chat_command`——**新增工具命令必须同步更新**。
- probe bot 名 `craftbot_probe`，与 agent bot 共存不冲突。
- 推送到 GitHub 前必须 probe 实测（主场景+边界）或 LLM 实机观测确认，仅编译/单测通过不推送。

### 按需观测（viewer，观测完即停）

```bash
cargo run -p craft-agent-ctl -- viewer "goal 文本" 40   # 只起 viewer 不起 autopilot
cargo run -p craft-agent-ctl -- stop                    # 观测完停止
```

### 跑工作流的正确姿势（新 agent 会话恢复时必读）

1. `craft-agent-ctl status`：进程是否活着、agent 是否 running、当前 goal、game-state 摘要
2. `craft-agent-ctl tail auto5_out.log 30`：最近 3 分钟 autopilot 观测/steering 记录
3. `craft-agent-ctl session 20`：LLM 最近在做什么
4. 读 `sessions/events/workflow.jsonl` 尾部：是否刚发生 death/armor_loss 异常（要跟进）
5. 若无 autopilot 在跑：`ctl build` + `ctl viewer "<当前主线目标>" 0` + `ctl start` 重新拉起
6. 差距分析先行（见上文迭代循环第 0 步），修完回填 NOTEBOOK + gap 文档

## 当前里程碑路径

石镐 → 铁和盾牌/盔甲 → 食物和桶 → 钻石 → 黑曜石/下界传送门 → 烈焰棒 → 末影珍珠/末影之眼 → 要塞 → 末地 → 末影龙。

此技能的基础目录: D:\Craft-Agent\.agents\skills\workflow
此技能中的相对路径（如 reference/）相对于此基础目录。NOTEBOOK.md 也在此目录中。