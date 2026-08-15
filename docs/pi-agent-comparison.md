# pi-agent 全面对比（Craft-Agent vs pi_agent_rust）

> 日期：2026-08-02 ｜ 分析对象：Dicklesworthstone/pi_agent_rust（main 分支，earendil pi TypeScript 版的 Rust 移植）
> 用途：作为 harness 架构的参考系，输出可借鉴项（候选 P 项）。
> 注：本项目的 `core/tool.rs`、`core/session.rs`、`core/message.rs` 已直接借鉴 pi 的
> `tools.rs` 副作用位掩码（L36-155）、`agent.rs:417 plan_tool_effect_batches`、session 持久化与 per-message token 估算。

## 1. 定位差异（最根本）

| | pi_agent_rust | Craft-Agent |
|---|---|---|
| 领域 | 编码代理（文件/终端环境） | 游戏代理（Minecraft，Azalea 协议层） |
| 环境 | 本地文件系统 + shell，副作用可回滚/可审批 | 实时世界，动作执行时间不可控（goto 5s vs 挖矿 60s） |
| 轮次模型 | 一次 run = 多轮 LLM 循环（工具迭代） | 一次 run_one_turn = 1 次 LLM 调用 + 一批工具执行 |
| 外部时钟 | 无实时世界，时间由工具调用本身推进 | 世界 tick 持续推进，bot 行为（move/mine）是异步长时动作 |
| 核心约束 | 工具结果立即可得（< 秒级） | 工具结果经命令队列异步返回（秒~分钟级） |

**结论**：pi 的"同轮内循环 50 次 LLM"在编码环境可行，因为工具调用快、结果即时、
环境静止；MC 中每轮工具执行本身要等世界推进，单轮内做几十次 LLM 循环会浪费 token
且实时世界已变化。**每轮单发 LLM + 分层 harness（模式系统/自触发）在 MC 场景是正确架构**，
P89 的"WRITE 失败 → 同轮重规划"就是按需折中，而非全盘循环。

## 2. pi 九个核心系统逐一对比

### 2.1 工具系统（tools.rs）

pi：`Tool` trait（name/label/description/parameters JSON Schema/async execute/effects），
内置仅 8 工具（read/bash/edit/write/grep/find/ls/hashline_edit），`ToolRegistry` = Vec 线性查找。
`ToolEffects` 位标志（READ/WRITE/APPEND/NETWORK/PROCESS），`parallel_safe()` 判定并发安全。

我们：54 工具（GameTool trait + ToolRegistry + MinecraftAction 枚举，2026-08-02 当时为 44，
后经 P 系列扩展至 54），`plan_tool_effect_batches` 同源实现。**已借鉴，无差距。**

### 2.2 主循环（agent.rs）

pi：双层循环——外层 drain steering/follow_up，内层 `while has_more_tool_calls`：
LLM → extract → execute（分批）→ 结果入历史 → 再 LLM。上限 `MAX_TOOL_ITERATIONS=50`，
80% 时注入"软交棒"警告，超限报错并剥离悬挂 tool_calls。

我们：run_one_turn 13 步，单 LLM 调用；dead-loop check（4 次同签名 → nudge）对应 pi 的
迭代上限警告。**差异点：pi 在轮内反复调 LLM（无工具调用才停），我们每轮只调一次。**

### 2.3 失败处理（关键差异）

pi：**单工具失败不中止批次**——失败转 `ToolOutput{is_error:true}` 结构化结果原样入历史，
下轮 LLM 自行判断。批次中断只由 abort/steering 触发。无工具级自动重试，无显式同轮重规划；
只有 provider 请求级幂等重试（`run_continue_with_abort`：只重发失败的 LLM 请求，不重放工具）。

我们：READ 失败不中止、WRITE 失败中止批次（P89 起）→ 【已中止】占位 + 【工具失败重规划】
nudge → 同轮重调 LLM（reroute_max=2）。**P89 实际比 pi 更进了一步**：pi 的重规划是隐式的
（错误进历史 → 下一轮迭代自然纠错），我们是显式的（中止 + 提示 + 定向重调）。

pi 的 steering 中断批次（agent.rs:2785-2789）→ 我们的 steer 消息在轮间注入，轮内不中断。
**候选 P 项：steering 消息到达时中断当前轮剩余批次。**

### 2.4 会话与压缩（session.rs / compaction.rs）

pi：JSONL + 树分支 + 文件锁 + 原子写；**LLM 迭代摘要压缩**（上一轮摘要作为 `<previous-summary>`
增量更新），后台异步压缩（compaction_worker 两阶段配额制）。`CHARS_PER_TOKEN_ESTIMATE=3`，
图片 1200 token，128K 窗口 / 8% 保留 / 10% keep_recent。

我们：JSONL 会话 + compaction（阈值 10000 消息/token 超预算触发）。
**候选 P 项：压缩质量升级为 LLM 增量摘要（增量续写比从头摘要省 token 且保真）。**

### 2.5 错误提示（error_hints.rs）

pi：给用户的 11 类错误 hint 映射（绝不建议破坏性操作，具体可执行）。

我们：bot 错误直接以文本结果回传。**对我们价值低**（LLM 自己就是消费者），不做。

### 2.6 模型路由（model_routing.rs）

pi：**明确 advisory**——健康指标（p95/错误率/成本）只做展示证据，不自动切换。
多模型 = 配置驱动手动选择。

我们：config.rs 多后端配置。**差距极小，不做自动故障转移**（MC 场景低价值高风险）。

### 2.7 调度器（scheduler.rs）

pi 的 scheduler 是 JS 扩展运行时（PiJS）的宏任务调度器，**与 agent 工具调度无关**。跳过。

### 2.8 连接器 / conformance / 扩展

- connectors：仅 HTTP 一个 Connector，给扩展做能力门控。
- conformance：TS oracle vs Rust 的语义 diff 测试框架（扩展契约回归）。
- extensions：JS（QuickJS）/Rust/WASM 三宿主扩展，工具执行前后 hook（dispatch_tool_call_hook
  可 fail-closed 拦截）。

我们：rhai 脚本（run_script/new_action）+ action_lib，无 hook 拦截层。
**候选 P 项（低优先级）：工具执行前 hook 拦截层**——未来若需"危险操作审批/规则拦截"可参考，
当前 bot 场景靠 prompt 约束 + 模式系统已够。

## 3. 逐项对照表

| 维度 | pi_agent_rust | Craft-Agent | 差距 / 动作 |
|---|---|---|---|
| 每轮 LLM 次数 | 多次（≤50 迭代） | 1 次/轮 | 设计使然，保持 |
| 工具分组 | 副作用位掩码分批，读并行写串行 | 同源实现 | ✅ 已借鉴 |
| 工具失败 | is_error 回传，不中止批次 | WRITE 失败中止+同轮重规划（P89） | ✅ P89 更优 |
| 批次级 steering 中断 | 有（agent.rs:2785） | 无（轮间注入） | 🔶 候选 P 项 |
| 迭代预算警告 | 80% 软交棒 + 硬上限 | dead-loop 4 连 nudge | ✅ 等价 |
| LLM 请求重试 | 幂等重发，不重放工具 | 指数退避重试 | ✅ 等价 |
| 压缩 | LLM 增量摘要 + 后台异步 | 阈值触发全文压缩 | 🔶 候选 P 项 |
| 错误 hint | 用户侧 11 类 | 不需要 | ❌ 不做 |
| 模型路由 | advisory 展示层 | 配置驱动 | ❌ 不做 |
| 扩展 hook 拦截 | 有（fail-closed 可配） | 无 | 🔶 低优先级候选 |
| 会话持久化 | JSONL+树+锁+原子写 | JSONL | ✅ 已借鉴 |
| 事件流 | 全生命周期 AgentEvent | SSE 事件通道 | ✅ 等价 |

## 4. 结论

1. **架构方向正确**：MC 需要"每轮单发 + 分层 harness"，不是 pi 式轮内循环。
   P89 的失败重规划就是这个判断的最佳折中，且比 pi 的隐式纠错更显式、更省 token。
2. **已借鉴到位**：副作用分批、session、token 估算三大机制已进代码库。
3. **两个候选 P 项**（按收益排序）：
   - P90：steering 消息到达 → 中断当前轮剩余批次（省 token + 响应性，改动小）
   - P91：压缩升级为 LLM 增量摘要（保真度 + 省 token，改动中等）
