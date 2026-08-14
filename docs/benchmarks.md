# SeekerCraft Benchmarks & Evidence

> 本页公开 SeekerCraft（Craft-Agent）的可复现量化数据：自动化测试、probe 工具层验证、
> LLM 实机观测、系统提示缓存命中率、末地路径进度。所有数字均可由仓库内脚本复现。

## 1. 自动化测试基线

| 维度 | 数值 | 复现命令 |
|---|---|---|
| 单元/集成测试总数 | 全绿（2026-08-15 v1.0.0 实测） | `cargo test --workspace` |
| 核心 agent 测试 | 全绿 | `cargo test -p craft-agent --lib` |
| MC 适配器测试（azalea-bot） | 178 全绿 | `cargo test -p craft-agent-minecraft --features azalea-bot --lib` |
| Mock 容器决策测试 | 43 | `craft.rs::tests`（无服务器） |
| 工具↔动作映射回归 | 1 | `regression_every_registered_tool_maps_to_action` |
| CI 门槛 | fmt + clippy `-D warnings` + 全部测试 | `.github/workflows/ci.yml` |

> 系统提示字节稳定性回归测试（`regression_system_prompt_byte_stable_across_obs_streak`）
> 已随 in-bot 主循环删除（2026-08-14，DSH 桥接模式起）；字节稳定性由 DSH 大脑负责。

## 2. Probe 工具层验证（无 LLM，秒级）

工具层行为验证不依赖 LLM 回合（每回合 30-60s+），由 `azalea_probe` 驱动，
63 个 JSON 脚本覆盖全部工具主场景 + 边界：

```bash
cargo run -p craft-agent-minecraft --example azalea_probe --features azalea-bot -- 4444 --script scripts/probe/smoke.json
```

| 领域 | 代表性脚本 | 覆盖点 |
|---|---|---|
| 移动 | p110_goto_anchor / p111_goto_player / p113_move_away | anchor 导航、实体跟随、远离 |
| 挖掘 | p101_mine_auto / p105 / p120* | 空气目标自动修正、徒手逃生、软土柱 |
| 农业 | till_and_sow / p102 / p104 / harvest_test / p80 | 自动靠近、修正、播种、收获、拾取 |
| 战斗 | p87_* / p88_* / strafe_test / p114 / p119 | 走位、反击、近战判定、弓射 |
| 睡眠 | sleep_test1-5 | 找床→上床→醒来 |
| 合成 | p117_* / p118_use_item | 末影之眼链路配方、投掷 |
| 运维 | state_check / raw_compare | 状态渲染器交叉验证 |

完整清单：`scripts/probe/`（63 个）。

## 3. 系统提示字节稳定性（DeepSeek 前缀缓存）

| 指标 | 实测值 | 说明 |
|---|---|---|
| 前缀缓存命中率 | >93%（P97 实机） | 动态状态走用户消息，system 字节稳定 |
| 命中样例 | 42624-43584 tokens hit / 2796-3326 miss | P97 实机观测 |

工程做法：静态内容（identity/role/jailbreak/knowledge）→ system；动态内容
（perceive/记忆/目标/示例）→ 每轮注入并在下一轮剔除的用户消息。

## 4. 末地路径进度（P 系列里程碑）

| 里程碑 | 状态 | 证据 |
|---|---|---|
| P84 农耕 till_and_sow | ✅ probe 实机 | "A Seedy Place" 成就路径 |
| P85 sleep | ✅ probe 实机 | 床→SleepingPos→醒来 |
| P87 战斗走位 + 徒手反击 | ✅ 实机 | 不再原地挨打 |
| P88 近战判定 overhaul | ✅ 实机 | 3.2m 判定 / 1s 检查 / 低血反击 |
| P89/90/94/99 失败收敛 | ✅ 测试 | AbortDecision 枚举 + 预算守卫 |
| P97 语义记忆 | ✅ 实机 + 缓存证据 | 记忆注入 + >93% 缓存命中 |
| P117/P117b 末影之眼链路配方 | ✅ probe | flint_and_steel / blaze_powder 2×2 |
| P118 use_item 投掷末影之眼 | ✅ probe | 要塞定位链路 |
| P119 shoot 弓射（ReleaseUseItem） | ✅ probe | 龙战远程 |
| P120/P120b 无镐逃生 | ✅ probe | 徒手/软土柱自动绕 |
| P121 状态渲染交叉验证 | ✅ raw_compare | renderer 无 bug 结论 |

进度跟踪：`docs/mindcraft-gap.md`（Mindcraft 对位审计 + 优先级队列，每工作单元更新）。

## 5. 规模参数

| 维度 | 数值 |
|---|---|
| crate 数 | 6（agent / minecraft / model / viewer / autopilot / ctl） |
| LLM 工具数 | 54（`ALL_TOOL_NAMES` 权威登记） |
| 结构化任务 | 23（tier1-6，机器可判定 JSON） |
| 反应式模式 | 10（tick 级，无 LLM 延迟） |
| WorldMemory 记忆类型 | 7（资源/结构/容器/实体/危险/传送门/笔记） |
| 12 个月 commit | 408+ |

## 复现声明

以上全部数据由仓库内代码/脚本生成：probe 脚本（`scripts/probe/`）、测试
（`crates/*`）、CI（`.github/workflows/`）。服务器环境：vanilla MC 26.2 局域网
服务器 + 协议级 Azalea 客户端；LLM 后端 OpenAI 兼容（DeepSeek 系列默认）。
