# Craft-Agent 重构设计：基于 Numen 哲学 + Baritone 寻路底座

> 状态：设计评审稿（不落地代码）
> 作者：opencode
> 关联：AGENTS.md / 24 条问题清单 / #10 MovementController 神类 / #19 AStar Java/Rust 重复

---

## 0. 背景与原则

### 0.1 我们卡在哪
当前所有 bug（`give_player` 死锁、采矿不入包、寻路超时）都是**执行层实现缺陷**，不是架构选错。但执行层里最痛的"寻路"部分，是自研 A*（`pathing/` 约 1100 行 + `MovementController` 1135 行），它在复杂地形下经常卡死、超时、永远 `isActive()`。

### 0.2 三个项目的真实关系（澄清）
| 项目 | 本质 | 寻路底座 | 对世界的控制深度 |
|---|---|---|---|
| **Mineflayer / Mindcraft** | Node.js 协议层 bot + LLM 胶水 | mineflayer-pathfinder（成熟） | 浅：只能发协议包，不能改游戏内部逻辑 |
| **Baritone** | Fabric/Forge 原生 mod | 自研 A* + 真实物理移动（Mineflayer 同级） | 深：运行在服务端/客户端，直接操作方块实体 |
| **Numen (DeepMind)** | 研究项目，非开源引擎 | 自研 policy + MC 原生能力 | 最深：服务端 Mod 环境（MineDojo）全状态控制 |
| **Craft-Agent（我们）** | Rust LLM ↔ Fabric mod ↔ MC | 自研 A*（弱） | 深：服务端 FakePlayer + GoalEngine 分解执行 |

**核心结论**：我们和 Numen 同一条路线（服务端 Mod 层深度控制），和 Mindcraft 不同路线。Mindcraft 的"寻路好"是 Mineflayer 的功劳，但 Mineflayer 路线**放弃了我们架构最有价值的部分**——服务端深度执行（GoalEngine 直接分解、debug 注入、改世界）。

**不要为了寻路投奔 Mineflayer/azalea 客户端路线**，那等于放弃差异化优势、变成 Mindcraft 复刻、且引入 Rust↔Node↔MC 三层复杂度。

### 0.3 正确形态
> 保留 Fabric mod 深度控制优势，只把**寻路/移动执行底座**换成 Baritone（Fabric 原生、Mineflayer 同级、深度可控）。

这就是"基于 Numen 哲学重构"的可落地含义：**Numen 给方向（服务端深度控制），Baritone 给手脚（好寻路），GoalEngine 继续做大脑。**

---

## 1. 现状盘点（重构前的真实规模）

### 1.1 自研寻路相关代码（应被 Baritone 替代）
```
pathing/
  AStarSearch.java            112 行   ← 删（换 Baritone A*）
  BinaryHeapOpenSet.java       46 行   ← 删
  Movement.java                37 行   ← 删（Baritone 的 Movement 体系）
  Moves.java                  148 行   ← 删
  NavContext.java             203 行   ← 删
  NavGoal.java                114 行   ← 删（换 Baritone Goal）
  Path.java                    28 行   ← 删
  PathCaches.java              69 行   ← 删
  PathNode.java                24 行   ← 删
  PlayerNav.java              116 行   ← 删
  PlayerNavManager.java        70 行   ← 删（换 BaritoneAPI 调用）
  PlayerPathExecutor.java     229 行   ← 删（Baritone 自己执行）
  VanillaPathfinder.java       79 行   ← 删/保留参考
```
**合计约 1265 行可删除。**

### 1.2 MovementController.java（1135 行，#10 神类）
承担：路径跟随、战斗、采集、跟随、搭柱（pillarUp）、丢物。重构后：
- 移动/导航 → **委托 Baritone**
- 战斗/采集/跟随/搭柱 → 拆成独立 Controller（已部分完成：CombatController/CollectController 已独立）

### 1.3 GoalEngine.java（477 行，保留并强化）
目标分解、progressStack 可观测性、#3/#5 仲裁 —— **全部保留**。它是"Numen 哲学"的体现：LLM 只发高层目标，服务端自动分解。

---

## 2. 目标架构

```
┌─────────────────────────────────────────────────────────┐
│  Rust LLM Agent (craft-agent / craft-agent-model)        │
│   - LLM 决策：只发高层目标/计划                           │
│   - compaction / modes / session                         │
└───────────────┬─────────────────────────────────────────┘
                │ TCP (ModCommand / StateBuilder)  ← 已稳定
┌───────────────▼─────────────────────────────────────────┐
│  Fabric Mod (craft-agent-bridge)                         │
│  ┌──────────────────────────────────────────────────┐   │
│  │  MetaController / GoalEngine  → 目标分解 (大脑)   │   │
│  │  CollectController / CombatController / ...       │   │
│  └───────────────┬──────────────────────────────────┘   │
│                   │  "走到 (x,y,z)" / "挖 block" / "挖矿" │
│  ┌───────────────▼──────────────────────────────────┐   │
│  │  NavigationAdapter  ← 新增薄封装层               │   │
│  │  - 把 GoalEngine 的意图翻译成 BaritoneAPI 调用    │   │
│  │  - 监听 Baritone 完成/失败回调                    │   │
│  └───────────────┬──────────────────────────────────┘   │
│                   │                                       │
│  ┌───────────────▼──────────────────────────────────┐   │
│  │  Baritone (baritone-api-fabric)  ← 寻路底座      │   │
│  │  - A* + 真实物理移动 + 自动挖/建/绕路           │   │
│  │  - GoalBlock / GoalNear / GoalGetToBlock         │   │
│  └───────────────┬──────────────────────────────────┘   │
│                   │                                       │
│           FakePlayer (ServerPlayer)  ↔  MC 服务端世界    │
└─────────────────────────────────────────────────────────┘
```

**关键变化**：`pathing/` 整目录消失，`MovementController` 降级为"意图翻译 + 非移动类动作"，新增 `NavigationAdapter` 作为 Baritone 的唯一入口。

---

## 3. Baritone 集成方案

### 3.1 可行性（已查证）
- **API 成熟**：`baritone-api-fabric` 依赖 + `BaritoneAPI` 静态入口，可嵌入自定义 Fabric mod。官方 example：`wagyourtail/baritone-api-fabric-example`（CC0）。
- **版本风险（重要）**：Baritone 目前最高支持到 **MC 1.21.x**（v1.13.1 对应 1.21.6，v1.14.0 对应 1.21.5+）。**我们的 MC 是 26.2，Baritone 尚无对应构建。**
  → 这是本方案最大的阻塞点，见 §6 风险。
- **License**：Baritone 使用 **LGPL-3.0 / 部分 GPL**。作为库依赖（非修改源码）嵌入，对闭源/内部项目通常可接受，但需法务确认分发条款。**建议：仅依赖 `baritone-api-fabric`（API 接口，稳定且小），运行时由用户自行放置完整 Baritone jar**，规避源码 GPL 传染。

### 3.2 集成方式（推荐：API-only 解耦）
1. `build.gradle` 依赖 `baritone-api-fabric`（编译期接口，体积小、稳定）。
2. 运行时要求 MC 的 `mods/` 目录下存在完整 Baritone mod jar（用户自行下载，规避 license 分发）。
3. `NavigationAdapter` 在 `onInitialize` 时探测 `BaritoneAPI.getProvider().getBaritoneForPlayer(fakePlayer)`，拿到该 FakePlayer 的 Baritone 实例。
4. 所有移动意图转为 Baritone 的 `PathingControlManager` 指令 + `Goal` 对象。

### 3.3 NavigationAdapter 接口设计（草案）
```java
public class NavigationAdapter {
    // 走到某方块旁边（替代 CollectController.standoff + PlayerNavManager）
    static void gotoBlock(ServerPlayer p, int x, int y, int z);
    // 走到坐标（替代 MovementController.performMoveTo）
    static void gotoXYZ(ServerPlayer p, double x, double y, double z);
    // 采矿（替代 CollectController 的 findBlock + destroyBlock）
    static void mineBlock(ServerPlayer p, String blockId, int count);
    // 查询状态（替代 PlayerNavManager.isActive / statusString）
    static NavState status(ServerPlayer p); // {IDLE, RUNNING, ARRIVED, FAILED}
    // 取消
    static void cancel(ServerPlayer p);
}
```
GoalEngine / CollectController 只调 `NavigationAdapter`，完全不感知 Baritone 内部。

---

## 4. 各模块重构动作

### 4.1 删除
- `pathing/` 整目录（~1265 行）
- `MovementController` 中的路径跟随/导航代码（保留战斗/采集/跟随/搭柱/丢物的**意图翻译**部分）
- `CollectController` 中的 `findBlock` / `standoff` / `PlayerNavManager` 调用（保留"采集意图 → NavigationAdapter.mineBlock"的编排）

### 4.2 新增
- `NavigationAdapter.java`：Baritone 唯一入口 + 完成/失败回调（用 `PathingEvent` 或轮询 `Baritone.getPathingBehavior().isPathing()`）
- 掉落物入包逻辑（本次已修，保留）：`CollectController` 挖掉方块后直接吸入背包，不依赖掉落物

### 4.3 保留不动
- `GoalEngine`（目标分解、progressStack、#3/#5 仲裁）—— 这是 Numen 哲学核心
- `CraftAgentBridge` 的 dispatch 统一调度层（#7 已修，稳定）
- `DebugController` / `ContainerController` / `CraftingHelper` / `MetaController`
- Rust 侧全部代码（`adapter_mod` / `bridge` / examples / tests）—— 协议层不变

### 4.4 #10 / #19 同时解决
- **#10 MovementController 神类**：导航职责剥离到 NavigationAdapter 后，MovementController 只剩"非移动动作编排"，可进一步拆。
- **#19 AStar Java/Rust 重复**：Java 侧 A* 删除；Rust 侧若另有路径逻辑也一并删除（需核查 `craft-agent-minecraft` 是否含 A* —— 据现有代码，Rust 侧无 A*，仅 Java 侧有，故 #19 主要是 Java 侧清理）。

---

## 5. 迁移路线图（渐进，不推倒重来）

**Phase 0（当前，先止血）**：收口执行层真实 bug（give_player 死锁已修、采矿入包已修待验证、寻路超时待 Baritone）。
**Phase 1**：接入 `baritone-api-fabric`，`NavigationAdapter.gotoXYZ` 跑通（先替换 `performMoveTo`）。e2e_smoke 的 nav_to 改走 Baritone。
**Phase 2**：`NavigationAdapter.mineBlock` 替换 `CollectController` 的 findBlock+standoff+destroy。mine_iron 测试跑绿。
**Phase 3**：`gotoBlock` 替换所有"走到方块旁"逻辑（chest/container/follow/combat 接近）。
**Phase 4**：删除 `pathing/` 整目录 + MovementController 导航代码。完成 #10/#19。
**Phase 5**：补充 Baritone 失败兜底（timeout、不可达 → GoalEngine 重试/放弃）。

每个 Phase 独立可验证、可回滚（git 提交粒度）。

---

## 6. 风险与待决策

| 风险 | 严重度 | 说明 / 缓解 |
|---|---|---|
| **MC 版本不匹配** | 高 | Baritone 最高 1.21.x，我们是 26.2。需等 Baritone 出 26.2 构建，或**降级 MC 到 1.21.x 以匹配 Baritone**（影响面大，需决策）。 |
| License 传染 | 中 | 仅依赖 API、运行时外挂完整 Baritone jar，规避 GPL 源码传染。需法务确认。 |
| Baritone 对 FakePlayer 支持 | 中 | Baritone 通常绑定真实 client player。FakePlayer（服务端）能否挂载 Baritone 实例需 POC 验证（Phase 1 第一个实验）。 |
| 战斗/搭柱等特例移动 Baritone 不覆盖 | 低 | pillarUp、kite 等仍需自研，保留在 MovementController。 |
| 迁移期双寻路并存 | 低 | Phase 1-3 期间自研 A* 与 Baritone 并存，靠 NavigationAdapter 开关切换。 |

**最高优先级待决策**：
1. **MC 版本**：是否接受降级到 1.21.x 以解锁 Baritone？还是等 Baritone 适配 26.2（时间未知）？
2. **License**：法务是否接受"API 依赖 + 运行时外挂"模式？

---

## 7. 结论

"基于 Numen 重构"的正确落地 = **保留服务端深度控制架构 + 用 Baritone 替换自研寻路底座**，而非投奔 Mineflayer 客户端路线。这既补齐了 Mindcraft 同级的寻路能力，又不放弃我们相对 Mindcraft 的真正优势。

最大不确定性是 **MC 26.2 与 Baritone 的版本错位**，需先做 Phase 1 的 POC（FakePlayer 能否挂 Baritone）和版本决策，再全面铺开。
