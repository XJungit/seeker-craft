# 能力现状 vs Mindcraft（2026-07-22 更新）

> 此前文档提出"集成 Baritone"，实际方向为**自研 A* + pathing 子系统**，
> 已落地并验收，本文更新为当前状态。

## 一、寻路

### Mindcraft
mineflayer-pathfinder（A* + 动态避障 + 水域/门/跳跃处理）

### Craft-Agent — 实际架构

```
MovementController.performMoveTo()
  └─ AStar.search()       → 路径网格（8方向 + stepUp + fallGuard）
  └─ smoothPath()         → 去除共线中间点
  └─ PlayerPathExecutor.driveToward()  → 逐帧推进
       ├─ autoDig()       → 前方挡路方块自动破坏
       ├─ isCliffEdge()   → 坠落保护（>3 块自动绕行）
       └─ stuck detection → 120 tick 超时判定
```
Fallback: `VanillaPathfinder`（用 MC 原生的 Zombie 寻路做兜底）

### 评估

| 维度 | 状态 | 差距 |
|---|---|---|
| A* 搜索 | ✅ 自研 10 方向 | 无动态避障（移动实体） |
| 路径执行 | ✅ autoDig + 悬崖检测 | 无水域游泳路径 |
| 原路兜底 | ✅ VanillaPathfinder | 门/栅栏高度判断不准 |
| 复杂度 | 标准平坦/山坡地形通过 | 洞穴/繁茂洞穴可能 FAIL |

## 二、建造

### Mindcraft
JSON 蓝图（3D 数组 blocks[y][z][x]）+ 逐层放置 + 缺料收集续建

### Craft-Agent
`placeAt(x,y,z,item)` 精确单块放置，无蓝图系统

### 差距
- 无层叠/行列批量放置
- 无缺料自动补充
- 无对称/镜像/旋转辅助

## 三、战斗

### Mindcraft
mineflayer-pvp + 硬编码走位（creeper 不贴脸、濒死撤退、风筝）

### Craft-Agent — 实际架构

两套系统并存（需统一）：
1. `MovementController.performCombat()` — 内联战斗（melee/kite/retreat + 苦力怕规避）
2. `CombatController` — tick 驱动状态机（单例，`start()`/`tick()`/`stop()`）

### 评估
- 可打可风筝可撤退
- 但两套系统选择混乱
- 无经验训练/自改进

## 四、规划

### Mindcraft
LLM CoT + SelfPrompter + Modes 反应系统 + 记忆总结

### Craft-Agent — 实际架构

Rust 侧：
- SelfPrompter（持续目标注入）
- Modes（self-preservation / self-defense / unstuck / hunger）
- Compaction（上下文压缩，Agnes LLM）
- Skill Library（历史经验摘要）

Java 侧：
- GoalEngine（自主目标分解：craft/get/smelt/hunt/build/explore/defend）
- CollectController（自主采集状态机）

### 差距
- Rust 与 Java 两个决策体可能冲突
- GoalEngine 未与 Rust Agent 循环集成
- 无记忆总结复用机制

## 五、结论

核心能力（寻路/战斗/规划）**已接近或达到 Mindcraft 水平**，但：
1. **需要统一 Rust/Java 指挥链**（P0）
2. 建造系统缺蓝图（P1）
3. 寻路缺动态避障 + 水域路径（P1）
4. 战斗两套并一套（P1）

不再需要 Baritone 集成。
