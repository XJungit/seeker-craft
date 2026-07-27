# Craft-Agent 项目计划

> 当前路线：**azalea-bot**（Rust 全栈客户端 bot，直连 MC 服务器，原生支持 MC 26.2）。
> 旧 Java Mod 路线（GoalEngine 在 Java 侧自动分解目标）已废弃——见 [docs/adr.md](./docs/adr.md) ADR-004。
> 当前架构：LLM 通过 37 个工具直接控制 bot，bot 工具只做原子动作，做不了的返回 Err 让 LLM 决策（Mindcraft 哲学）。

---

## 终极目标

LLM 大模型控制 bot 通过 Minecraft（抵达末地 + 击败末影龙）。

达成路径（按依赖顺序）：

1. **早期生存**：砍树 → 合成木镐 → 挖石 → 合成石镐 → 建庇护所
2. **中期工业化**：挖铁 → 熔铁 → 铁镐/铁装 → 挖钻石 → 钻石镐/钻装
3. **下界**：建传送门 → 探索 → 找堡垒 → 杀烈焰人 → 得烈焰棒
4. **末影珍珠**：杀末影人 → 得末影珍珠
5. **末地**：合成末影之眼 → 找要塞 → 激活传送门 → 进末地
6. **击败末影龙**：破坏水晶 + 床炸/剑砍

---

## 当前状态（2026-07-27）

### 已完成

- **架构落地**：三层 crate（craft-agent / craft-agent-minecraft / craft-agent-model / craft-agent-viewer）
- **37 个 LLM 工具**：覆盖感知/记忆/移动/挖掘/战斗/合成/熔炼/采集/放置/容器/装备/交易/建造/计划/脚本/自定义动作/知识搜索
- **Agent 主循环 13 步**：drain_queues → 压缩检查 → 易变注入清理 → auto_perceive → modes 反应 → SelfPrompter → 动态上下文 → WorldMemory → LLM → 纯文字检测 → 死循环检测 → 并行执行 → 技能抽取
- **两层 modes 反应系统**：Agent 层注入提示 + Handler 层直接执行（火/岩浆脱困、自动反击）
- **WorldMemory 空间记忆**：坐标主键 + 分块索引 + 6 种记忆类型 + 每 20 tick 扫描
- **配方知识库（双层）**：RecipeBook（vanilla 26.2 全量）+ 手写 SHAPED_RECIPES fallback
- **蓝图系统**：可复用建筑模板
- **自定义动作**：rhai 嵌入式脚本引擎
- **Web 仪表盘**：Axum + SSE 实时观察 bot 状态
- **全自动化测试工具链**：cargo test 234 个测试 + PowerShell 诊断脚本 + LLM 实机测试 + scan/diag 报告

### P55-P58 最新修复（2026-07-27）

- **P55**：gather 部分成功返回 Ok 而非 Err（修复 100% 失败率）
- **P56**：plain_text_reply 治理（9 个关键词 + profile 规则禁止宣告完成）
- **P57**：smelt 分批熔炼（单次上限 8 个，避免 120s 工具超时）
- **P58**：拦截 set_goal("") 绕过 P56 检测

### 系统性重构（9.3 节方向 A-D 全部完成）

- **方向 A**：mock 容器集成测试（43 个测试覆盖 mindcraft 边界条件）
- **方向 B**：逐函数对齐 mindcraft skills.js（placedTable 回收、takeOutput 循环、燃料 fallback）
- **方向 C**：RecipeBook 替代手写 SHAPED_RECIPES
- **方向 D**：smelt takeOutput 动态轮询（1s 间隔 + 11s 无产物超时）

---

## 下一阶段目标

### 改进点 2：容器同步 / BlockEntity 延迟实测验证（MEDIUM）

P49 takeOutput 循环依赖 `furnace.outputItem()` 真实读取，azalea 的 ContainerHandleRef 状态同步可能有 1-2 tick 延迟。需在 LLM 实机测试中观察是否出现"产物已烧好但 shift_click 取不到"现象。

### 改进点 3：azalea 插件化产物自动收集（LOW）

把"产物自动收集""容器状态同步"抽成 Bevy ECS 插件。P49 的轮询+left_click 兜底已能工作，插件化是优化项不是必需项。触发条件：改进点 2 实测发现同步延迟问题。

### 通关路径推进

当前 bot 已能完成早期+中期生存（砍树/挖矿/合成/熔炼）。下一步需推进到：

- **下界传送门**：需要 lava + obsidian（或水浇岩浆）+ flint_and_steel
- **末地传送门**：需要 ender_pearl + blaze_powder → ender_eye + 找要塞
- **击败末影龙**：需要 bow + bed 爆炸战术

---

## 自动化测试协议

### 测试命令

```bash
# 全量编译
cargo build --workspace

# 全量测试（234 个）
cargo test --workspace --no-fail-fast

# 核心 crate 测试
cargo test -p craft-agent --lib                  # 122 个
cargo test -p craft-agent-minecraft --features azalea-bot --lib  # 118 个
cargo test -p craft-agent-model --lib            # 23 个

# 端到端 LLM 测试
cargo run -p craft-agent-viewer -- --goal "..." --steps 40 --port 8080
```

### 全自动化工具链

```
cargo test (234) → 编译 → 端到端 LLM 测试 → scan_run.ps1 分析 → auto_diag.ps1 诊断
     ↑                                                              ↓
     └────── 修复代码 ←──── 分析报告 ←──────────────────────────────┘
```

测试失败时的决策树见 [AGENTS.md](./AGENTS.md) 第 2.1 节。

---

## 关键架构约束

### System Prompt 字节稳定性

DeepSeek prefix cache 要求 system prompt 每次调用字节完全一致。
- 严禁把动态变量塞进 system prompt
- 动态内容通过 `build_dynamic_instructions_msg()` 返回 user message
- 有回归测试 `regression_system_prompt_byte_stable_across_obs_streak`

### Mindcraft 哲学（9.1-9.5 节）

bot 工具只做能做的，做不了就 return Err 让 LLM 决策。
- 不自动合成工具方块（furnace/pickaxe 等）
- 不自动满足原料依赖
- 错误消息必须列出完整解决步骤

详见 [AGENTS.md](./AGENTS.md) 第九-bis 节。

### 工具名规范

实际工具名禁止用假名：
- `go`（不是 goto/move_to/walk_to）
- `gather`（不是 collect/pickup_item）
- `mine`（不是 dig/break）
- `attack`（不是 combat/fight）
- `craft`（不是 make/create）
- `place`（不是 put/set）

---

## 时间线

| 阶段 | 内容 | 状态 |
|------|------|------|
| Phase 0 | 架构落地（三层 crate + 37 工具 + 13 步主循环） | ✅ 完成 |
| Phase 1 | 早期+中期生存（砍树/挖矿/合成/熔炼） | ✅ 完成 |
| Phase 2 | P54-P58 系统性重构（mock 测试 + Mindcraft 对齐） | ✅ 完成 |
| Phase 3 | 下界传送门 + 末影珍珠 | 🔲 进行中 |
| Phase 4 | 末地传送门 + 击败末影龙 | 🔲 待启动 |

---

## 结论

不再追求"LLM 只发目标，Mod 全自动执行"的旧设计（已废弃，见 ADR-004）。
当前架构：**LLM 通过 37 个工具直接控制 bot，bot 工具是原子操作，做不了的返回 Err 让 LLM 决策**。

这是超越 Mindcraft 的路径——azalea 协议层 + Rust 全栈 + Mindcraft 哲学对齐 + 全自动化测试工具链。
