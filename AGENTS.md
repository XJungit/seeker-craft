# Craft-Agent

基于 Azalea Rust 客户端协议的 LLM 驱动 Minecraft bot。目标：**击败末影龙**。

## 使命（Mission）

自主优化与维护本项目。权限：**完全决策权**。用户只评估最终结果：LLM 能否通关 Minecraft？

### 优先级（Priorities）
1. 在框架设计与稳定性上超越 Mindcraft
2. LLM bot 必须掌握 MC 知识：合成、工具、盔甲、熔炼、酿造、附魔、抵达末地
3. 持续优化 agent 框架、Azalea 接口、自动化工具套件

### 问题解决（Problem Solving）
- **首先**：上网搜索解决方案
- **反复出现的问题**：必须搜索解决方案
- Mindcraft 及类似项目是参考实现

### 迭代工作流（必须遵守，2026-08-01 起）
> 本质定位：**工作流 = 持续优化项目本身**（差距分析 → 修复 → 回填）。
> bot 运行（viewer/autopilot）只是**按需观测手段**，不是常驻工作流——
> 每个工作单元按需启动、观测完即停（`craft-agent-ctl stop`）。
> 工具层验证优先用 probe（秒级），LLM 实机观测只在需要确认
> 策略/规划行为时按工作单元启动。

每个工作单元开始前按顺序执行：
1. **差距分析先行**：扫 `docs/mindcraft-gap.md` 优先级队列，找出主线收益最高的缺失项
2. **实机观测**：probe/status/session 确认问题真实存在（区分 harness bug vs LLM 决策）
3. **修复/实现**：改代码 → `cargo test -p craft-agent-minecraft --features azalea-bot --lib` → 提交
4. **回填**：更新 `docs/mindcraft-gap.md` 状态与 AGENTS.md（如有流程变更）
> 禁止跳过第 1 步直接修 bug——被动修补（P57-P76 模式）是流程缺陷，
> 主动差距分析（P77 起）是 Mission 第一优先级的正确执行方式。

### Git 推送策略（2026-08-01 起，用户指令）
- **本地提交**：每个工作单元完成后 `git add -A && git commit`（必须，保障可回滚）
- **推送 GitHub**：仅当某一功能/方面**确切完善**（实机验证通过、CI 门槛全绿、gap 回填完成）才 `git push origin main`
- 中间过程的小修复/实验/未验证改动：只本地提交，不推送（避免 CI 噪音与历史垃圾）
- 推送前自检（**必须全部通过才 push**，2026-08-02 起，P85-P87 推送时因漏跑 fmt/clippy 导致 CI 红两次）：
  1. `git log origin/main..HEAD --oneline` 确认待推提交都是"完善"级
  2. `cargo fmt --all -- --check`（CI quality job 门槛）
  3. `cargo clippy --workspace --all-targets --features craft-agent-minecraft/azalea-bot -- -D warnings`（CI quality job 门槛）
  4. `cargo test --workspace`（CI test job 门槛）

## 构建与运行（Build & Run）

```bash
# 构建（仅 nightly，stable 会失败）
cargo build --workspace

# 测试
cargo test -p craft-agent --lib
cargo test -p craft-agent-minecraft --features azalea-bot --lib

# 运行 viewer
# 注意：craft-agent-viewer 包只有一个 bin，直接运行即可
cargo run -p craft-agent-viewer --bin craft-agent-viewer

# 运行 azalea bot demo
cargo run -p craft-agent-minecraft --example agent_azalea_demo --features azalea-bot -- --goal="挖矿下探" --steps=20
```

## 项目维护与迭代准则（总纲，2026-08-02 起）

> 本质：本项目代码将由 AI 代理长期维护。**一切改动**（功能、修复、重构、实验）
> 服从同一套准则：**稳定、可靠、准确、方便长期维护**。优先级：稳定性 > 工作量。
> LLM 兼容性是不可破坏的契约（工具名、消息格式、prompt 文本、配置 schema）。

### 通用迭代准则（适用于所有工作单元）

1. **先测试锁定行为**：任何改动（尤其重构）前，先写/确认针对性测试锁定现有行为；改动后全量验证
2. **行为不变原则**：工具名、消息格式、prompt 文本、任务/配置文件 schema 绝不因重构而变
3. **单提交单关注点**：一次改动一个提交，回滚粒度 = 单次提交；纯移动/重命名提交的 diff 只有位置变化
4. **全量门槛**：每次改动后 `cargo test --workspace` + `cargo fmt --all -- --check` + clippy `-D warnings` 全绿才算完成
5. **可回滚**：冒险实验前先 `git add -A && git commit --no-verify -m "wip: checkpoint"`；绝不 `git checkout`/`reset`/`restore`/`stash` 回滚（用 SearchReplace 逐行可见回滚）
6. **文档同步**：每次迭代回填 `docs/mindcraft-gap.md` 状态；流程性变更必须回写 AGENTS.md 本准则
7. **双点同步防线**：任何涉及工具/动作/消息格式的新增，按下方"新增能力纪律"四处同步，回归测试兜底

### 架构演进路线图（稳定优先，逐步执行）

- **P1 已完成基线收尾（2026-08-03）**：P1.3 ctl 日志文件名统一 ✓ / P1.2 工具↔MinecraftAction 映射集中（`action_for()` + 47 工具/32 变体登记表）+ 全量回归测试 ✓ / P1.1 瞬态消息统一 `push_transient` helper（debug 断言前缀已登记；顺带修复 P12/P31/P56 nudge 的 `你的目标是:` 公共前缀漏登记 → 此前 nudge 永不剔除混入压缩摘要）✓
- **P2 结构性已完成（2026-08-03）**：
  - P2.1 `run_one_turn` 拆分 ✓：`execute_batch`（批分组/READ 并行/WRITE 串行/slow 探测）+ `finalize_abort`（P89/P90/P94/P99 四分支收敛为 `AbortDecision::{Reroute, Handoff}` 枚举）
  - P2.2 `azalea/mod.rs`（6340 行）拆分 ✓：`azalea/commands.rs`（BotCommand 33 变体 + QueuedCommand + parse_chat_command + chat_parser 测试）+ `azalea/handler.rs`（BotState + tick 主体 handle + 专属 helper），mod.rs 1995 行，经 `pub use` re-export 保持外部引用零改动
  - P2.3 `craft-agent-model` 边界 ✓：Cargo.toml 文档标注只依赖 `craft_agent::core::{message,types}`，CI quality job 用 `cargo check -p craft-agent-model --no-default-features` 验证不渗透上层
- **P3 按需（不设 deadline）**：`craft.rs`（4730 行）按域拆 craft_table/smelt/brew/enchant/smith；`tools_azalea.rs`（4166 行）按域分组文件保留单一 `register_all_tools()`；`agent_loop.rs` 事件推送/会话保存/滚动抽 helper

### 新增能力纪律（工具/动作/消息格式的双点同步）

工具名必须稳定（LLM prompt 兼容性）。新增工具需同步 4 处：
1. `tools_azalea.rs` 注册 GameTool
2. `core/types.rs::MinecraftAction` 变体
3. `adapter_azalea.rs` 映射表 `action_for()`（或 execute match）
4. `azalea/mod.rs::parse_chat_command`（probe 驱动）
防线：`regression_every_registered_tool_maps_to_action`（漏一处即测试红）。

## 架构（5 个 crate）

```
craft-agent              核心 agent 框架（约 5000 行）
  agent/                 run_one_turn（4098 行含测试）、compaction、prompt、modes、session
  core/                  types（MinecraftAction 32 变体）、GameTool trait、ToolRegistry、memory（WorldMemory）、skill
  task.rs                Task 系统：23 个 tier1-6 任务，结构化成功条件
  profile.rs             3 层 prompt 合并（_default → defaults/{mode} → {individual}）

craft-agent-minecraft    MC 适配器（azalea 协议，约 12000+ 行）
  azalea/mod.rs          AzaleaBot + connect/动作 API/背包三件套（1995 行，P2.2 已拆）
  azalea/commands.rs     BotCommand 33 变体 + QueuedCommand + parse_chat_command + chat_parser 测试（P2.2 拆出）
  azalea/handler.rs      BotState + tick 主体 handle + 专属 helper（now_ms/nearby_active_portal/block_memory_meta/record_surroundings/nearby_player_position）（P2.2 拆出）
  azalea/craft.rs        2x2/3x3 合成、熔炼、锻造、切石、酿造、附魔（4730 行）
  azalea/gather.rs       方块扫描 + 工具等级检查 + 自动装备（568 行）
  azalea/auto_craft.rs   递归配方满足 + 工具方块放置（681 行）
  azalea/place.rs        方块放置 + 容器开启 + 触及范围检查（827 行）
  azalea/recipes.rs      配方知识库（427 行）
  azalea/perception.rs   位置读取
  azalea/actions.rs      基础 bot 动作（goto/mine/chat）
  azalea/smart_actions.rs 多工具聚合动作（1319 行）
  adapter_azalea.rs      GameAdapter 实现、perceive 格式、execute、工具↔动作映射
  tools_azalea.rs        47 个 LLM 工具（4166 行）
  action_lib.rs          LLM 定义的 rhai 脚本（338 行）
  blueprint.rs           蓝图库（310 行）

craft-agent-model        LLM 客户端（OpenAI 兼容）
  decision.rs            chat_tools()、fold_tool_history()、parse_chat_tools_response()
  config.rs              多后端配置加载器
  vision.rs              VLM 客户端

craft-agent-viewer       Web 仪表盘（Axum + SSE）
  agent_loop.rs          主循环：agent.step() → chat drain → idle loop

craft-agent-autopilot    自主测试循环（build/test → anomaly → RCA → commit）
```

## 44 个 LLM 工具

| 类别 | 工具 |
|---|---|
| 感知 | `perceive`, `memory`, `search_wiki` |
| 移动 | `goto`, `mine_below`, `mine_above`, `pickup`, `follow`, `stop_follow` |
| 挖掘 | `mine`, `make_obsidian` |
| 交互 | `interact_block`, `interact_entity`, `attack`, `defend` |
| 合成 | `craft`, `craft_3x3`, `smelt`, `auto_craft`, `enchant` |
| 采集 | `gather`, `till_and_sow`, `harvest` |
| 放置 | `place`, `build`, `build_blueprint`, `list_blueprints` |
| 容器 | `open`, `chest_view`, `chest_withdraw`, `chest_deposit` |
| 背包 | `equip`, `discard`, `consume` |
| NPC / 社交 | `trade`, `give` |
| 元操作 | `chat`, `set_goal`, `run_plan`, `run_script`, `new_action`, `list_actions`, `pause_goal`, `resume_goal`, `task_complete`, `task_retry` |

## Agent 主循环（run_one_turn，13 步）

1. `drain_queues()` — 获取 steering/follow_up 消息
2. 压缩（Compaction）— 消息 ≥ 10000 或 token 超预算 → compact()
3. 剔除临时消息 — 上一轮的 perceive、memory、goal
4. 自动感知（Auto-perceive）— 注入结构化状态快照
5. 模式（Modes）— check_modes() → [MODE: ...] 文本提示
6. SelfPrompter — 注入 [当前目标]
7. 动态上下文 — WorldInfo + Skill + Few-shot + 观测警告
8. WorldMemory — 渲染邻近记忆（半径 64）
9. LLM 调用 — 带重试（指数退避）
10. 纯文本检查 — 若无 tool_calls，注入 nudge 并返回（continue）
11. 死循环检查 — 4+ 次相同规范化签名 → nudge
12. 执行工具 — 按效果分组（READ 并行、WRITE 串行）
13. 技能抽取 — 从非观测工具调用中学习

**Nudge 规则：** 必须在所有工具结果之后注入，绝不能插在 `assistant(tool_calls)` 与 `tool` 之间（会触发 400）。

## 项目文件

```
tasks/                   23 个任务 JSON（tier1-6：crafting_table → netherite → ender_dragon → elytra）
profiles/                3 层 prompt 模板
  _default.json          基础 prompt
  defaults/{mode}.json   模式覆盖
  {individual}.json      个体覆盖
blueprints/              4 个蓝图 JSON（farm_plot、small_shelter、storage_corner、torch_pillar）
actions/                 LLM 定义的 rhai 脚本（*.rhai.json）
sessions/                运行时会话 JSONL 文件
scripts/                 调试/诊断 shell 脚本
.github/workflows/ci.yml CI 配置
data/config/agent.toml   多后端 LLM/VLM 配置
```

## 任务系统（task.rs）

23 个带完成条件的结构化任务：
- `InventoryHas { item, count }` — 背包中有物品 ≥ count
- `AtPosition { x, y, z, radius }` — bot 位于位置 radius 范围内
- `BelowY { y }` — 位于 Y 坐标以下
- `InDimension { dimension }` — 当前处于指定维度
- `PortalActive` — 扫描范围内存在激活的下界传送门
- `Killed { entity_kind, count }` — 服务器报告的累计实体击杀数
- `All/Any { conditions }` — 复合条件
- 任务从 `tasks/*.json` 加载，按 tier、显式顺序、id 排序

## Profile 系统（profile.rs）

3 层 prompt 合并（字段级合并，非整体替换）：
1. `profiles/_default.json` — 基线（必需）
2. `profiles/defaults/{mode}.json` — 模式覆盖（可选）
3. `profiles/{individual}.json` — 个体覆盖（可选）

占位符：`$NAME`, `$SELF_PROMPT`, `$MEMORY`, `$STATS`, `$INVENTORY`, `$COMMAND_DOCS`, `$EXAMPLES`

## 模式系统（agent/modes.rs）

10 个反应式模式，tick 级，独立于 LLM：
- `self_preservation` — 火/岩浆脱困
- `self_defense` — 攻击敌对生物
- `unstuck` — 卡住时脱困
- `cowardice` — 逃离敌对生物
- `hunting` — 狩猎动物获取食物
- `item_collecting` — 拾取掉落物
- `torch_placing` — 黑暗处放置火把
- `elbow_room` — 清理拥挤空间
- `idle_staring` — 空闲时环顾四周
- `cheat` — 创造模式作弊

## WorldMemory（core/memory.rs）

基于区块索引的空间记忆：
- 7 种记忆类型：Resource、Structure、Container、Entity、Hazard、Portal、Note
- `chunk_key` = (x>>4, y>>4, z>>4) 实现 O(1) 邻近查询
- 命名锚点："home"、"nether_portal" 等
- 遗忘机制：基于 TTL + 显式 `forget_*`

## Azalea 集成（命令队列模式）

```
LLM 工具调用 → tools_azalea.rs → adapter_azalea.rs::execute()
  → AzaleaBot::push_cmd(BotCommand) → cmd_queue
  → handler tick 排空队列 → 用 bot API 执行
  → BotEvent::Chat("[采集] ...") 通过事件通道回传
```

**关键约束：** Azalea `Client` 只能在 handler 闭包内访问。外部代码使用 fire-and-forget 命令队列。结果通过 `BotEvent` 异步返回。

## 系统提示字节稳定性

DeepSeek 前缀缓存要求每次调用的系统提示完全相同。
- **静态内容** → 系统提示（identity、role_desc、jailbreak、knowledge）
- **动态内容** → 用户消息，每轮注入并在下一轮剔除：
  - `【当前游戏状态（自动注入）】` — perceive 状态
  - `【邻近世界记忆】` — 邻近 WorldMemory
  - `[当前目标]` — self_prompt 目标
  - `【参考示例】` — few-shot 示例

**缓存规则**（DeepSeek kv_cache 文档）：命中需**完整匹配缓存前缀单元**（请求结束位置/公共前缀/固定间隔落盘）。因此历史消息必须 append-only、绝不重写早期消息（折叠/原地修改会碎前缀）；system + 早期历史稳定 = 前缀命中。命中率由 usage 的 `prompt_cache_hit_tokens`/`prompt_cache_miss_tokens` 观测（decision.rs 解析，Usage 字段）。

**回归测试：** `regression_system_prompt_byte_stable_across_obs_streak`

## 工具调用（Tool Calling）

- 所有工具使用 OpenAI function calling（JSON `tool_calls`），绝不用文本命令
- **原生多轮协议**（官方 DeepSeek 文档即此格式）：`assistant` 消息带 `tool_calls` 数组，结果用 `role:"tool"` + `tool_call_id` 配对，历史逐轮 append
- **思考模式约束**（官方文档）：携带 `tools` 的请求必须完整回传 `reasoning_content`，否则 400——`to_chatml` 独立字段回传（message.rs）
- **不做折叠**：曾把 `tool_calls` + `role:tool` 折成 `【工具执行】name(args) → result` 文本，导致 LLM 模仿伪调用（decision.rs:347 注释），已移除
- 工具返回同步结果

## 代码执行（Code Execution）

- **`run_script`** → rhai 引擎（内嵌、Rust 原生、安全）
  - 函数：goto, mine, mine_below, mine_above, gather, craft, craft_3x3, place, open, chest_view, chat, attack, defend, smelt, interact, interact_entity, enchant, auto_craft, trade, equip, discard, consume, pickup, make_obsidian, follow, stop_follow, give, till_and_sow, harvest, sleep, print
- **`run_plan`** → JSON 多步计划（顺序执行、同步）
- **`new_action`** → 将命名 rhai 脚本保存到 `actions/<name>.rhai.json`，跨会话可复用

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

方块名称使用 `BlockKind`（而非 `BlockState` 的 Debug 格式）。

## 关键约束

### 不要修改 vendor/azalea
- `vendor/azalea` 是独立的 git 仓库 + workspace
- 所有改动只能使用 azalea 的公开 API
- 若要修改：在 vendor 中提交 → 同步更新 `.cargo/config.toml` [patch] 与 `Cargo.toml` 中的 SHA

### Cargo 网络
- 使用 `rsproxy.cn` 镜像（sparse index）
- `NO_PROXY` 环境变量包含 rsproxy.cn、mirrors.ustc.edu.cn 等
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
# 全部测试
cargo test --workspace

# 指定 crate
cargo test -p craft-agent --lib
cargo test -p craft-agent-minecraft --features azalea-bot --lib

# 指定测试
cargo test task_manager_lifecycle
cargo test regression_all_tasks_dir_json_loads

# 修改系统提示后
cargo test regression_system_prompt_byte_stable
```

## Probe 模式（不开 LLM 的工具层测试）

LLM 实机测试慢（每回合 30-60s+），**工具层行为验证一律用 probe**，
秒级完成，无需 viewer/agent/LLM：

```bash
# 单条命令（无需 --script）
cargo run -p craft-agent-minecraft --example azalea_probe --features azalea-bot -- 4444 --cmd "equip iron_helmet helmet"
# 脚本（见 scripts/probe/smoke.json 示例）
cargo run -p craft-agent-minecraft --example azalea_probe --features azalea-bot -- 4444 --script scripts\probe\smoke.json
```

脚本格式：`steps` 数组，每项为 `{"cmd": "..."}` / `{"wait_ms": N}` / `{"state": true}`（打印状态快照）。
probe bot 名 `craftbot_probe`，与 agent bot 共存不冲突。命令文本见
`parse_chat_command`（azalea/commands.rs，支持 goto/mine x y z/minebelow/mineabove/attack/gather/
craft/craft3/smelt/autocraft/place/open/enchant/trade/interact/interactblock x y z/tillandsow x y z seed/chat/chat 消息/
follow/give/equip/discard/consume/chestview/chestwithdraw/chestdeposit/makeobsidian/pickup/defend/sleep/harvest）。
需要 LLM 决策的测试（策略/规划/目标分解）才开 viewer+agent。

新增工具命令时必须同时更新 `parse_chat_command`，否则 probe 无法驱动。
probe 发现的工具层 bug 优先修（例：P67 goto 到达判定 1.5→2.5m，
probe 实测 bot 停在 1.5-2.5m 处永不 done → 60s 超时空等）。

## craft-agent-ctl 运维控制台（替代频繁 shell 长命令）

**重要**：不要用 shell 的 `Start-Process`/长 `Start-Sleep` 管理进程——
管道句柄问题会导致 opencode 工具层 terminated。一律用 ctl（Rust 实现，
stdout/stderr 文件重定向，命令快速返回）：

```bash
cargo run -p craft-agent-ctl -- status    # 进程 + API + game-state 摘要 + 日志尾部
cargo run -p craft-agent-ctl -- stop      # taskkill 所有 craft-agent 进程
cargo run -p craft-agent-ctl -- build     # 编译 viewer + autopilot
cargo run -p craft-agent-ctl -- goal "<g>" # 注入 steering goal
cargo run -p craft-agent-ctl -- session 10 # 会话最近 10 个工具结果
cargo run -p craft-agent-ctl -- tail <log> <N>
```

部署流程（ctl deploy 的 build 阶段可能超时，分步执行更稳）：
1. `craft-agent-ctl stop`
2. `craft-agent-ctl build`
3. `Start-Process target\debug\craft-agent-viewer.exe -ArgumentList '--goal','...','--steps','0','--port','8080','--mc','localhost:4444','--username','CraftAgent' -WindowStyle Hidden`（不带重定向参数）
4. `Start-Process target\debug\craft-agent-autopilot.exe -WindowStyle Hidden`（autopilot 会自动 start agent）
5. `craft-agent-ctl status` 验证 running=true

## 参考项目

- **Mindcraft** (mindcraft-bots/mindcraft)：JS + mineflayer，LLM bot 框架。任务、profiles、modes 的参考。
- **Azalea** (azalea-rs/azalea)：Rust Minecraft 客户端协议。本项目构建于其上。
- **Numen**：结构化生存自动化（SurvivalJournal、FailureType 分类）。

## 目标：通关 Minecraft

6 阶段通关路径：
1. Tier 1-2：木/石/铁工具、工作台、熔炉
2. Tier 3-4：铁甲、钻石装备、挖至基岩
3. Tier 5：附魔、酿造、下界传送门
4. Tier 6：下界合金、潜影盒、鞘翅
5. 抵达末地
6. 击败末影龙

每个阶段要求 LLM 掌握：
- MC 合成配方（全等级）
- 工具/盔甲等级进阶
- 熔炼与酿造配方
- 附魔策略
- 下界与末地维度机制
