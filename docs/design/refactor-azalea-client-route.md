# Craft-Agent 架构转向：Fabric Mod → Azalea 客户端协议层

> 状态：设计评审稿（不落地代码）
> 作者：opencode
> 决策来源：用户确认"转向 Azalea 客户端协议层路线"
> 关联：refactor-numen-philosophy-baritone-base.md（前稿，已作废结论）、24 条问题清单

---

## 0. 决策变更说明

前一份设计稿（Baritone 底座）的结论是"保留 Fabric mod 深度控制 + 换 Baritone"。但核查后发现：
- **Baritone 最高仅支持 MC 1.21.x，不支持我们的 26.2**。
- **Azalea 原生支持 MC 26.2**，且其 pathfinder **明确基于 Baritone 移植**（`Much of the pathfinder's code is based on Baritone`），即获得 Mineflayer 同级寻路。
- Azalea 是 **MIT 协议**（无 Baritone 的 LGPL/GPL 传染风险）。

因此用户决策转向 **Azalea 客户端协议层路线**：放弃 Fabric 服务端 mod 深度控制，改为 Rust 全栈客户端 bot（与 Mindcraft/Mineflayer 同形态，但用 Rust + 原生 26.2 + Baritone 级寻路）。

> 注：这是与 Numen（服务端深度控制）**不同的路线**。取舍是明确的——用"深度控制"换"版本匹配 + 成熟寻路 + 无 license 风险 + Rust 全栈统一"。

---

## 1. 两条路线对比（终态）

| 维度 | 旧：Fabric Mod（Numen 路线） | 新：Azalea 客户端（Mindcraft 路线） |
|---|---|---|
| bot 形态 | 服务端 FakePlayer（ServerPlayer） | 客户端 Client（连同一个普通 MC 服务器） |
| 世界控制深度 | 深：改方块/注入/服务端逻辑 | 浅：只能发协议包（与真人玩家等价） |
| 寻路底座 | 自研 A*（弱，~1265 行 pathing/） | **Azalea pathfinder（Baritone 移植，强）** |
| MC 版本 | 26.2（Fabric mod） | **26.2（Azalea 原生支持）** |
| 运行方式 | MC 服务端 + 加载 mod + TCP 桥 | 独立 Rust 二进制连 MC 服务器 |
| License 风险 | Baritone GPL 传染（若走前稿） | **MIT，无风险** |
| LLM 大脑 | craft-agent（保留） | craft-agent（保留，不变） |

**不变的部分（核心资产）**：
- `craft-agent`（agent 循环 / compaction / modes / session / 决策）
- `craft-agent-model`（LLM/VLM 多后端）
- `craft-agent-viewer`（可视化，仍可用）
- `tools_mod/*` 的**高层动作语义**（mine/move/place/craft/combat/container...）——只是翻译目标从 `ModCommand` 换成 Azalea `Client` API。

**被替代/删除的部分**：
- `mods/craft-agent-bridge/`（整个 Java Fabric mod，~8000 行）→ 删除
- `craft-agent-minecraft` 的 `bridge.rs` / `adapter_mod.rs`（TCP 协议层）→ 重写/删除
- `pathing/` 相关（本就在 Java 侧，随 mod 删除）

---

## 2. 新架构

```
┌──────────────────────────────────────────────────────────┐
│  Rust LLM Agent (craft-agent / craft-agent-model)         │
│   决策层：WorldState → Action（与传输无关，完全保留）      │
└───────────────────────┬──────────────────────────────────┘
                         │ Action / WorldState（core::adapter 接口）
┌───────────────────────▼──────────────────────────────────┐
│  craft-agent-minecraft  (重写适配器层)                    │
│  ┌────────────────────────────────────────────────────┐  │
│  │  adapter_azalea.rs  ← 实现 GameAdapter 接口        │  │
│  │   - perceive(): 读 Azalea Client 世界状态          │  │
│  │     构建 WorldState（实体/方块/背包/坐标/朝向）     │  │
│  │   - execute(Action): 翻译成 Azalea Client API       │  │
│  │     move→Pathfinder, mine→Client::mine,            │  │
│  │     place→block_interact, attack→Client::attack...  │  │
│  └────────────────────────────────────────────────────┘  │
└───────────────────────┬──────────────────────────────────┘
                         │ azalea crate API 调用
┌───────────────────────▼──────────────────────────────────┐
│  Azalea (Rust MC 客户端库, MIT)                           │
│   - 物理 / Pathfinder / 破块 / 建造 / 背包 / 攻击         │
│   - 连入普通 MC 服务器（Fabric/Forge/Vanilla 均可）       │
└───────────────────────┬──────────────────────────────────┘
                         │ MC 协议 (TCP)
                    ┌────▼─────┐
                    │ MC 服务器 │（你已有的 PCL2 世界）
                    └──────────┘
```

**运行时形态变化**：不再需要"MC 服务端加载 craft-agent-bridge mod + 另起 Rust 连 TCP"。改为：**启动一个普通 MC 服务器（或连你现有的），Rust 二进制以 bot 身份连入**。bot 就是服务器里的一个玩家。

---

## 3. 改造范围（精确）

### 3.1 完全删除
- `mods/craft-agent-bridge/` 整个目录（Java Fabric mod，~8000 行，含 pathing/、MovementController、GoalEngine、所有 Controller）。
- `crates/craft-agent-minecraft/src/bridge.rs`（TCP 协议 `ModCommand`/`ModState`/`McBridge`）。
- `crates/craft-agent-minecraft/src/adapter_mod.rs`（TCP 适配器）。

### 3.2 重写
- **新增 `adapter_azalea.rs`**：实现 `core::adapter::GameAdapter` 接口，内部用 azalea crate。
  - `perceive`：从 `azalea::Client` 读 `world`、`inventory`、`entity`、自身 `position`/`rotation`。
  - `execute`：
    - `move` → `azalea::pathfinder` 的 `goto(GoalBlock/GoalNear)`（替代自研导航）
    - `mine(block)` → `Client::mine(BlockPos)`（Azalea 自带破块 + 掉落物拾取）
    - `place` → `Client::block_interact` + 手持物品
    - `attack` → `Client::attack`
    - `craft` → 仍走 MC 原生配方（需开合成台 GUI 或调用配方 API；Azalea 支持 inventory 操作）
    - `look`/`look_at` → 设 `Client` 旋转
- `lib.rs`：feature flag 从 `mod-bridge` 改为 `azalea`（或默认启用）。
- `builder.rs`：构造 `MinecraftModAdapter` → 改为构造 `MinecraftAzaleaAdapter`（带账号/服务器地址）。

### 3.3 保留不动
- `craft-agent/src/**`（agent 循环、compaction、modes、session、decisions）
- `craft-agent-model/src/**`
- `craft-agent-viewer/src/**`
- `tools_mod/*` 的**动作语义定义**（Action 枚举、tool 描述）——它们与传输无关，只是当前在 `adapter_mod` 里被翻译成 ModCommand。改造后改在 `adapter_azalea` 里翻译成 Azalea API。
- `survival.rs` / `survival_decisions.rs`（生存决策逻辑，依赖 WorldState，不依赖传输）

### 3.4 #10 / #19 自然消解
- **#10 MovementController 神类**：随 Java mod 删除而消失。
- **#19 AStar Java/Rust 重复**：Java A* 随 mod 删除；Rust 侧本无 A*（只有 `pathfind` 字符串匹配），改用 Azalea pathfinder。
- 之前清单里所有"Java mod 内部"的 bug（#1/#2/#3/#5/#6/#7 等）**随 mod 删除而一并消失**——这是转向的最大红利：**之前修的那些死锁/线程/同步 bug 全部不再存在**，因为根本不再有"服务端线程 + TCP 读线程 + FakePlayer"这套复杂模型。

---

## 4. 具体动作 → Azalea API 映射（草案）

| 当前 ModCommand | Azalea 替代 |
|---|---|
| `nav_to(x,y,z)` | `client.pathfinder().goto(GoalNear::new(x,y,z,1))` |
| `mine(block)` | `client.mine(block_pos)` + Pathfinder 接近 |
| `place(item,x,y,z)` | `client.block_interact()` 或设置后 `client.set_block` |
| `attack(entity)` | `client.attack(entity_id)` |
| `craft(item)` | inventory + 配方（Azalea `ContainerClientExt`） |
| `give_player` | 走近 + 丢物（`client.drop` / `player_inventory`） |
| `chest/transfer` | 开 GUI + inventory 操作 |
| `goal_*`(GoalEngine) | **由 Rust 侧 survival/goal 决策层承担**（因为服务端 GoalEngine 没了）；或保留为 Rust 侧 goal 规划 |
| `debug_*` | 部分不可用（debug 是 mod 专属）；改为游戏内指令或测试 fixture 重构 |

**重要**：Java 侧的 `GoalEngine`（目标分解：log→planks→stick→iron_pickaxe）是"服务端深度执行"的产物。转向后，**这部分逻辑应上移到 Rust 侧**（`tools_mod/goal.rs` 已有雏形），让 Rust agent 自己规划"造铁镐需要先挖矿/烧炼/合成"的步骤序列。这其实更契合"LLM 发目标、Rust 规划、Azalea 执行"的清晰分层。

---

## 5. 迁移路线图（渐进）

**Phase 0（止血，可选）**：若暂时不想立刻切，先保留现有 mod 跑通当前 bug 修复。但既然已决策转向，**建议直接进 Phase 1**，不再投入 mod bug 修复。

**Phase 1 — 骨架连通**
- 新建 `adapter_azalea.rs`，实现最小 `GameAdapter`：连服 + perceive（读坐标/背包）+ execute（`move` 走 Azalea pathfinder）。
- 用现有 e2e 的 `nav_to` 场景验证：bot 真的走到指定坐标。
- 删除 `bridge.rs` / `adapter_mod.rs` 的 TCP 依赖，或 feature-gate 隔离。

**Phase 2 — 动作覆盖**
- 逐个实现 `mine` / `place` / `attack` / `craft`，对应现有 tools_mod 动作。
- 复跑 `e2e_smoke` 的 collect / pillar_up / place 场景（改用 Azalea 原生破块/建造）。

**Phase 3 — 目标规划上移**
- 把 Java GoalEngine 的分解逻辑（craft 链、smelt、hunt）迁移到 Rust `tools_mod/goal.rs`。
- 复跑 `give_armor` / `gear_to_chest` / 新建的 `mine_iron`（此时 Azalea pathfinder + 原生破块 + 掉落物自动拾取，采矿链路应天然跑通）。

**Phase 4 — 清理**
- 删除 `mods/craft-agent-bridge/` 整个目录。
- 删除死代码、旧 feature flag。
- 更新 AGENTS.md（MC 版本、运行方式、依赖变化）。

**Phase 5 — Viewer / 回归**
- 确认 `craft-agent-viewer` 仍能对接新适配器（状态来源变了，但 WorldState 结构不变）。
- 补齐 Rust 侧单元测试（之前 Java 侧无单测，转向后正好补）。

---

## 6. 风险与待决策

| 风险 | 严重度 | 说明 / 缓解 |
|---|---|---|
| **Azalea 成熟度** | 中 | 官方自认"many parts unfinished, breaking changes"。选稳定 tag（如 `0.16.0+mc26.1`），锁定版本，避免追 main。 |
| **craft 配方 API 完整度** | 中 | Azalea 的 inventory/配方操作不如服务端直接。需 POC 验证铁镐等合成能否程序化触发。 |
| **GoalEngine 上移工作量** | 中 | Java GoalEngine ~477 行分解逻辑要重写成 Rust。但逻辑清晰，可渐进。 |
| **debug_* 能力丢失** | 低 | mod 专属调试指令（debug_give/place/spawn）在纯客户端不可用。测试 fixture 需改用"游戏内给物品"或单独测试服。 |
| **反作弊** | 低 | Azalea 目标"don't trigger anti-cheats"，物理真实。但私有服/本地无影响。 |
| **多 bot / 服务端控制** | 低 | 失去"服务端直接改世界"能力；若未来需要，只能走数据包/指令。 |

**最高优先级待决策**：
1. **服务器形态**：连你已有的 PCL2 本地服？还是单独起一个 vanilla/forge 服给 bot？Azalea 连服需要服务器地址+账号（离线/正版）。
2. **GoalEngine 上移范围**：是否接受把"目标分解"从 Java 服务端移到 Rust 侧（推荐，更清晰）？还是暂时用简单脚本式目标？
3. **旧 mod 代码保留策略**：立即删除 vs 保留一段时间对照？建议 feature-gate 隔离旧 `mod-bridge`，确认新链路稳后再删。

---

## 7. 结论

转向 Azalea 是**用架构简化换成熟度**的正确取舍：
- 消除整个 Java Fabric mod 的复杂线程/死锁/同步模型（之前清单里过半 bug 根源）。
- 获得 Baritone 级寻路（Azalea pathfinder 移植自 Baritone），MC 26.2 原生支持。
- MIT 协议，Rust 全栈统一，无 license 风险。

代价是失去服务端深度控制（debug 注入、直接改世界），且 GoalEngine 分解逻辑需上移到 Rust。整体工作量集中在"重写适配器 + 目标规划上移"，大脑层（craft-agent / model）零改动。
