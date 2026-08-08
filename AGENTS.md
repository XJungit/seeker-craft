# Craft-Agent

基于 Azalea Rust 客户端协议的 LLM 驱动 Minecraft bot。目标：**击败末影龙**。

## 使命（Mission）

自主优化与维护本项目。权限：**完全决策权**。用户只评估最终结果：LLM 能否通关 Minecraft？
优先级：1) 框架设计与稳定性超越 Mindcraft；2) LLM 掌握 MC 知识（合成/工具/盔甲/熔炼/酿造/附魔/抵末地）；
3) 持续优化 agent 框架、Azalea 接口、自动化工具套件。

## 问题解决（Problem Solving）

- **首先**：上网搜索解决方案；**反复出现的问题**必须搜索（Mindcraft/Azalea/MC Wiki 是参考实现）
- 完整迭代方法（差距分析先行 + 证据驱动循环 + 迭代纪律）见工作流技能 `.agents/skills/workflow/SKILL.md`

## 迭代工作流（必须遵守，2026-08-01 起）

> 本质定位：**工作流 = 持续优化项目本身**（差距分析 → 修复 → 回填）。
> bot 运行（viewer/autopilot）只是**按需观测手段**，观测完即停（`craft-agent-ctl stop`）；
> 工具层验证优先用 probe（秒级），LLM 实机观测只在需要确认策略/规划行为时按工作单元启动。
> 完整迭代循环（差距分析先行 → 实机观测 → 修复 → 提交 → 回填）与迭代纪律
> **见工作流技能 `.agents/skills/workflow/SKILL.md`**——禁止跳过差距分析直接修 bug
> （被动修补 P57-P76 模式是流程缺陷；主动差距分析 P77 起是 Mission 第一优先级的正确执行方式）。

## 汇报纪律（2026-08-06 起，用户指令）

- **工作单元完成 ≠ goal 完成**：`[goal:complete]` 标记只能用于整个使命达成（超越 Mindcraft + 通关末地），绝不因单轮工作单元（如 P116 差距表清零）标注 goal 完成。
- goal 是长期 active 状态；每轮只汇报当前工作单元成果，goal 保持 active 直至使命真正达成并经用户确认。

## Git 推送策略（2026-08-01 起，用户指令）

- **本地提交**：每个工作单元完成后 `git add -A && git commit`（必须，保障可回滚）
- **推送 GitHub**：仅当某一功能/方面**确切完善**（实机验证通过、CI 门槛全绿、gap 回填完成）才 `git push origin main`
- **推送前必须实测验证实际效果（2026-08-06 起，用户指令）**：功能类工作单元推送前必须 probe 实机验证（`scripts/probe/*.json`，覆盖主场景 + 边界）或 LLM 实机观测确认；**编译通过/单测通过 ≠ 实际效果已验证**——未做实测的功能只本地提交，绝不推送。提交前同样自查无严重 bug（probe 发现的工具层 bug 优先修，如 P67/P100 先例）。
- 中间过程的小修复/实验/未验证改动：只本地提交，不推送（避免 CI 噪音与历史垃圾）
- 推送前自检（**必须全部通过才 push**，2026-08-02 起，P85-P87 漏跑 fmt/clippy 致 CI 红两次）：
  1. `git log origin/main..HEAD --oneline` 确认待推提交都是"完善"级
  2. `cargo fmt --all -- --check`（CI quality job 门槛）
  3. `cargo clippy --workspace --all-targets --features craft-agent-minecraft/azalea-bot -- -D warnings`（CI quality job 门槛）
  4. `cargo test --workspace`（CI test job 门槛）

## 构建与运行（Build & Run）

```bash
# 构建（仅 nightly，stable 会失败）
cargo build --workspace
# 测试（azalea-bot feature 必须显式）
cargo test -p craft-agent --lib
cargo test -p craft-agent-minecraft --features azalea-bot --lib
# 运行 viewer（只有一个 bin，直接运行）与 azalea demo
cargo run -p craft-agent-viewer --bin craft-agent-viewer
cargo run -p craft-agent-minecraft --example agent_azalea_demo --features azalea-bot -- --goal="挖矿下探" --steps=20
```

## 项目维护与迭代准则（总纲，2026-08-02 起）

> 本质：本项目代码将由 AI 代理长期维护。**一切改动**（功能、修复、重构、实验）
> 服从同一套准则：**稳定、可靠、准确、方便长期维护**。优先级：稳定性 > 工作量。
> LLM 兼容性是不可破坏的契约（工具名、消息格式、prompt 文本、配置 schema）。
> 通用迭代准则（先测试锁定/行为不变/单提交/全量门槛/回滚/文档回填/双点同步）
> 已下沉至工作流技能 `.agents/skills/workflow/SKILL.md`。本文件只保留项目级硬契约。
>
> **文档同步纪律（2026-08-08 起，用户指令）**：文档必须随时跟随代码更新——改代码
> 的同一轮工作完成时同步改文档，文档与代码不一致（过时数字/工具名/表格结构）视为缺陷。
> **README.md（英文）与 README.zh-CN.md（中文）必须逐节对齐一致**；工具数量与工具表以
> `tools_azalea.rs::ALL_TOOL_NAMES`（权威清单，现 53 个）为准，增删工具时同步更新所有
> 引用工具数的文档（README 双语、AGENTS.md 工具表、ARCHITECTURE.md、crate README、
> docs/benchmarks.md 等）。史实型文档（如 PLAN.md）保留当时快照，不追改。

- 架构演进路线图：P1/P2 已完成（ctl 日志统一/action_for 集中映射/run_one_turn 拆分/azalea mod 拆分，细节见 git log）；**P3 按需（不设 deadline）**：`craft.rs` 按域拆分、`tools_azalea.rs` 按域分组、`agent_loop.rs` 抽 helper

### 新增能力纪律（工具/动作/消息格式的双点同步）

工具名必须稳定（LLM prompt 兼容性）。新增工具需同步 5 处：
1. `tools_azalea.rs` 注册 GameTool + `ALL_TOOL_NAMES`
2. `core/types.rs::MinecraftAction` 变体
3. `adapter_azalea.rs` 映射表 `action_for()`（或 execute match）
4. `azalea/mod.rs::parse_chat_command`（probe 驱动）
5. 文档同步：README 双语工具表、AGENTS.md 工具表、ARCHITECTURE.md、crate README、
   所有出现工具计数处（以 `ALL_TOOL_NAMES` 为权威，见上文文档同步纪律）
防线：`regression_every_registered_tool_maps_to_action`（漏一处即测试红）。

### force_block 交互贴脸纪律（P100 教训）

`block_interact`（force_block）类交互在 ~2.9m 外会被服务端静默拒收（无错误返回，
验证时读到旧状态）。**任何 block_interact/交互类工具必须自动靠近 ≤2.5m 再交互**
（参考 till.rs P100 修复：>2m 自动 start_goto + 60×100ms 等待），仅距离检查不可靠。
已覆盖：till_and_sow（P100）、place（P29 距离已收紧）。新增交互类工具默认带靠近逻辑。

### mine 目标修正纪律（P101 教训）

LLM 会盲猜坐标连续 mine 空气格（每次换坐标绕过死循环检测）。**mine 工具必须在
派发时自动修正**：目标格是空气 → `nearest_solid_block`（半径 4）自动改挖最近实心方块，
修正通知走事件流（首帧一次，不消费 result_tx）。**done 判定与反馈必须基于实际挖掘目标**
（`BotState.last_mine_eff`），绝不能用原目标判空气——原目标本就是空气时 done 立即成立、
修正挖掘被终结；挖掘成功后目标是空气属正常（报"Mined block at"），误报"该位置已是空气"
会让 LLM 反复挖同一格（P57 根源）。三场景：实心挖掉→成功 / 空气修正挖掉→修正成功 /
空气无实心→建议提示。

### 交互类目标修正纪律（P102 教训）

LLM 坐标记忆/感知偏差不限于 mine：till_and_sow 曾连续 4 次对空气格犁地（L55/L104/L106/L148），
每次换坐标绕过死循环检测。**坐标型交互工具（till_and_sow 等）必须在派发时自动修正**：
目标格类型不符 → 附近（半径 4，y±1）找最近合法方块（可犁=草方块/泥土/已耕地，且上方无阻挡），
**修正后继续执行原动作**（不是只报错让 LLM 重试），成功消息明确告知修正
（"原目标 X 是 Air，已自动修正犁最近可犁方块 Y 并完成"）；附近无合法目标才报错并给建议。
距离检查/自动靠近必须基于修正后坐标。place 已有同思路的 P5/P11 自动重定位。新增坐标型
交互工具默认带此修正逻辑。

## 架构（6 个 crate）

```
craft-agent              核心 agent 框架：run_one_turn、compaction、prompt、modes、session、
                          core（types/GameTool/ToolRegistry/WorldMemory）、task.rs（23 任务）、profile.rs
craft-agent-minecraft    MC 适配器（azalea 协议）：azalea/ 域模块 + adapter_azalea.rs + tools_azalea.rs（53 工具）
craft-agent-model        LLM 客户端（OpenAI 兼容）：decision/config/vision
craft-agent-viewer       Web 仪表盘（Axum + SSE）：agent_loop.rs 主循环
craft-agent-autopilot    自主测试循环：build/test → anomaly → RCA → commit
craft-agent-ctl          运维控制台：status/stop/build/goal/session/tail/viewer
```

权威分层架构、13 步循环与模块布局见 `ARCHITECTURE.md`；仓库文件树见 `README.md`。

## LLM 工具

- **53 个**，权威清单 `tools_azalea.rs::ALL_TOOL_NAMES`（与 `create_mc_azalea_tools_full` vec 一一对应）；分类表见 README.md / ARCHITECTURE.md
- 新增工具按「新增能力纪律」同步 5 处（第 5 处 = 文档同步）

## Agent 主循环（13 步）

- 完整 13 步（drain_queues → 压缩 → 剔瞬态 → 自动感知 → 模式 → SelfPrompter → 动态上下文 →
  WorldMemory → LLM 调用重试 → 纯文本检查 → 死循环检查 → 执行分组 → 技能抽取）见 `ARCHITECTURE.md`
- **Nudge 规则（硬约束）**：必须在所有工具结果之后注入，绝不能插在 `assistant(tool_calls)` 与 `tool` 之间（会触发 400）

## 系统提示字节稳定性

DeepSeek 前缀缓存要求每次调用的系统提示完全相同。
- **静态内容** → 系统提示（identity、role_desc、jailbreak、knowledge）
- **动态内容** → 用户消息，每轮注入并在下一轮剔除：
  `【当前游戏状态（自动注入）】`（perceive 状态）/ `【邻近世界记忆】` / `[当前目标]`（self_prompt）/ `【参考示例】`（few-shot）

**缓存规则**（DeepSeek kv_cache 文档）：命中需**完整匹配缓存前缀单元**。因此历史消息必须
append-only、绝不重写早期消息（折叠/原地修改会碎前缀）；system + 早期历史稳定 = 前缀命中。
命中率由 usage 的 `prompt_cache_hit_tokens`/`prompt_cache_miss_tokens` 观测（decision.rs 解析）。

**回归测试：** `regression_system_prompt_byte_stable_across_obs_streak`

## 工具调用（Tool Calling）

- 所有工具使用 OpenAI function calling（JSON `tool_calls`），绝不用文本命令
- **原生多轮协议**（官方 DeepSeek 文档即此格式）：`assistant` 消息带 `tool_calls` 数组，
  结果用 `role:"tool"` + `tool_call_id` 配对，历史逐轮 append
- **思考模式约束**（官方文档）：携带 `tools` 的请求必须完整回传 `reasoning_content`，否则 400——
  `to_chatml` 独立字段回传（message.rs）
- **不做折叠**：曾把 `tool_calls` + `role:tool` 折成 `【工具执行】name(args) → result` 文本，
  导致 LLM 模仿伪调用（decision.rs:347 注释），已移除
- 工具返回同步结果

## 代码执行（Code Execution）

- `run_script` → rhai 引擎（内嵌、Rust 原生、安全；函数清单见代码）；`run_plan` → JSON 多步计划（顺序执行、同步）；
  `new_action` → 命名 rhai 脚本存 `actions/<name>.rhai.json`，跨会话复用。实现见 `action_lib.rs`

## Perceive 格式（adapter_azalea.rs）

```
位置: (-489.0, 86.0, -169.0)
生命: 20/20  饱食: 20/20  等级: 0
主手: dirt
装备: [头盔: 无, 胸甲: 无, 护腿: 无, 靴子: 无]
附近:
  oak_log @(x,y,z) 3.2m
  stone @(x,y,z) 5.1m
背包:
  oak_log x16, stick x4, cobblestone x32
记忆:
  工作台 @(10,64,-20) [结构]
  箱子(32 oak_log) @(12,64,-20) [容器]
卡住: 0轮
```

方块名称使用 `BlockKind`（而非 `BlockState` 的 Debug 格式）。此格式是 LLM prompt 契约的一部分——
改格式必须按文档同步纪律更新 README 双语。

## 关键约束

### 不要修改 vendor/azalea

- `vendor/azalea` 是独立的 git 仓库 + workspace（需先设 local 身份：`git -C vendor/azalea config user.name/email`）
- 所有改动只能使用 azalea 的公开 API
- 若要修改（P114 实测流程 2026-08-06）：
  1. 在 vendor/azalea 改代码 → `git commit`（新 commit）
  2. **只更新** `.cargo/config.toml` [patch] 条目的 rev 为新 SHA；`craft-agent-minecraft/Cargo.toml` 声明的 rev 保持不动（必须是 github 上存在的 commit，lock 更新时 cargo 会 fetch https 源它；patch 条目 rev 与声明 rev 可以不同）
  3. 清缓存：`Remove-Item -Recurse "$env:USERPROFILE\.cargo\git\checkouts" -Force`
  4. `cargo check -p craft-agent-minecraft --features azalea-bot` 验证（输出显示 `file:///D:/Craft-Agent/vendor/azalea?rev=<新SHA>` 即生效）
  5. 父仓库提交时把 vendor 的 gitlink 更新一并提交（`git add vendor/azalea`）

### Cargo 网络

- 使用 `rsproxy.cn` 镜像（sparse index）；`NO_PROXY` 环境变量包含 rsproxy.cn、mirrors.ustc.edu.cn 等
- azalea 是带本地 vendor patch 的 git 依赖——离线构建可用

### Git 安全

- ❌ 绝不 `git checkout -- <file>`、`git checkout .`、`git restore`、`git reset --hard`、`git clean -fd`
- ❌ 没有立即 pop 计划绝不 `git stash`
- ✅ 用 SearchReplace 回滚代码（可见、逐行）
- ✅ 冒险实验前 `git add -A && git commit --no-verify -m "wip: checkpoint"`

### 工具命名纪律

- 工具名必须稳定——改名会破坏 LLM prompt 兼容性
- 新增工具时，同时加到 `tools_azalea.rs` 和 `core/types.rs::MinecraftAction`
- 工具描述是 prompt 的一部分——保持简洁、面向行动

## 常见构建错误

| 错误 | 修复 |
|-------|-----|
| `cannot find trait LlmProvider` | `use crate::agent::LlmProvider` |
| `cannot find module azalea_block` | 使用 `azalea::block::BlockState` |
| `mismatched closing delimiter` | 检查合并错误导致的重复代码块 |
| `failed to get anyhow as dependency` | rsproxy.cn 宕机，临时重命名 `.cargo/config.toml` |
| `tried to attack entity which isn't in EntityIdIndex` | 自卫模式需要实体存在性检查 |
| `nightly-only feature` | 不要切回 stable，azalea 需要 nightly |
| LSP ConfigInvalidError | `lsp` 必须是 `true`、`false` 或对象——不能是数组 |

## 测试

```bash
cargo test --workspace
cargo test -p craft-agent --lib
cargo test -p craft-agent-minecraft --features azalea-bot --lib
# 关键回归：task_manager_lifecycle / regression_all_tasks_dir_json_loads /
# regression_system_prompt_byte_stable（修改系统提示后必跑）
```

## Probe 模式（不开 LLM 的工具层测试）

LLM 实机测试慢（每回合 30-60s+），**工具层行为验证一律用 probe**，秒级完成，无需 viewer/agent/LLM：

```bash
cargo run -p craft-agent-minecraft --example azalea_probe --features azalea-bot -- 4444 --cmd "equip iron_helmet helmet"
cargo run -p craft-agent-minecraft --example azalea_probe --features azalea-bot -- 4444 --script scripts\probe\smoke.json
```

脚本格式：`steps` 数组，每项 `{"cmd": "..."}` / `{"wait_ms": N}` / `{"state": true}`（打印状态快照）。
probe bot 名 `craftbot_probe`，与 agent bot 共存不冲突。支持命令全文见
`azalea/commands.rs::parse_chat_command`（probe 驱动——新增工具命令必须同步更新，否则 probe 无法驱动）。
需要 LLM 决策的测试（策略/规划/目标分解）才开 viewer+agent。
probe 发现的工具层 bug 优先修（例：P67 goto 到达判定 1.5→2.5m，60s 超时空等）。

## craft-agent-ctl 运维控制台

**重要**：不要用 shell 的 `Start-Process`/长 `Start-Sleep` 管理进程——管道句柄问题会导致
opencode 工具层 terminated。一律用 ctl（Rust 实现，stdout/stderr 文件重定向，命令快速返回）：

```bash
cargo run -p craft-agent-ctl -- status|stop|build|goal "<g>"|session 10|tail <log> <N>
```

**启动 viewer 只用 `ctl viewer`（2026-08-03 教训）**：PowerShell `Start-Process -ArgumentList`
会把参数数组 join 成单字符串，含空格/中文的 goal 被拆分导致 clap 解析失败、进程静默退出
（反复出现"viewer 没起来"的根因）。`ctl viewer` 用 Rust `Command::args` 逐参传递，无引号问题：

```bash
cargo run -p craft-agent-ctl -- viewer "goal 文本" <steps 默认40>
# 随后 cargo run -p craft-agent-ctl -- start  # agent 不会自动 start
# 日志在 C:\Windows\TEMP\opencode\viewer_run.log(.err)
```

部署流程（ctl deploy 的 build 阶段可能超时，分步执行更稳）：
1. `craft-agent-ctl stop` → 2. `craft-agent-ctl build` → 3. `craft-agent-ctl viewer "goal 文本" 0`
（steps=0 无限循环；只起 viewer 不起 autopilot）→ 4. `craft-agent-ctl start`（必须手动）→
5. `craft-agent-ctl status` 验证 running=true

## 参考项目

- **Mindcraft** (mindcraft-bots/mindcraft)：JS + mineflayer，LLM bot 框架。任务、profiles、modes 的参考。
- **Azalea** (azalea-rs/azalea)：Rust Minecraft 客户端协议。本项目构建于其上。
- **Numen**：结构化生存自动化（SurvivalJournal、FailureType 分类）。

## 目标：通关 Minecraft

6 阶段：Tier 1-2 木/石/铁工具 → Tier 3-4 铁甲/钻石/挖至基岩 → Tier 5 附魔/酿造/下界传送门 →
Tier 6 下界合金/潜影盒/鞘翅 → 抵达末地 → 击败末影龙。每阶段要求 LLM 掌握对应合成配方、
工具/盔甲进阶、熔炼与酿造、附魔策略、下界与末地机制。任务清单见 `tasks/*.json`（tier1-6）。
