# Craft-Agent 全面重构计划

## 核心原则

**LLM 写步骤计划，Mod 执行，Rust 编排。**

```
┌─────────────────────────────────────────────────┐
│               LLM (策略层)                        │
│  生成步骤计划（JSON 数组，支持条件/循环）          │
│  例: [nav_to, if(has_enemy) attack, else mine]  │
└─────────────────────┬───────────────────────────┘
                      │ 一次提交整个计划
┌─────────────────────▼───────────────────────────┐
│           Rust (执行引擎)                         │
│                                                    │
│  接收计划 → 逐条解释执行                            │
│  支持: 顺序 / if-then-else / for-loop / wait       │
│  每条步: 检查条件 → 发 TCP → 等结果 → 下一条        │
│  失败: 返回失败步骤 + 已成功步骤                     │
└─────────────────────┬───────────────────────────┘
                      │ TCP command (单条)
┌─────────────────────▼───────────────────────────┐
│          Java Mod (执行层)                        │
│  原子工具 + 复合工具状态机                         │
│  直接调 Minecraft 原版 API                        │
│  destroyBlock / attack / PathNavigation          │
└─────────────────────────────────────────────────┘
```

**对比 Mindcraft**：
- Mindcraft: LLM 写 JS → `eval()` 执行 → 能写任意代码但也可能写出死循环/恶意代码
- 我们: LLM 写 JSON 计划 → 执行引擎解释执行 → 只有预定义工具和条件，安全可控
- **效果等价**（LLM 都能组合多步 + 条件分支），但**我们更安全**

---

## 步骤引擎设计

### LLM 生成的计划格式

```json
[
  {"tool": "nav_to", "args": {"x": 12, "y": 64, "z": 8}},

  {"if": {"state": "health", "lt": 8},
   "then": [
     {"tool": "moveAway", "args": {"distance": 10}}
   ],
   "else": [
     {"tool": "combat", "args": {"mode": "melee", "ticks": 100}}
   ]},

  {"loop": {"times": 5},
   "do": [
     {"tool": "collect", "args": {"target": "stone", "count": 1}}
   ]},

  {"wait": {"seconds": 3}}
]
```

### 支持的条件表达式

```rust
// 可检查的状态值
state.health         // 血量
state.hunger         // 饱食度
state.has_item("iron_ore")  // 背包是否有某物品
state.has_entity("zombie")  // 附近是否有某实体
state.distance_to(x, y, z)  // 到某点的距离
state.time_of_day           // 游戏时间
state.inventory_full        // 背包是否满

// 比较操作
lt / lte / gt / gte / eq / neq
```

### 支持的循环

```json
// 固定次数
{"loop": {"times": 10}, "do": [...]}

// 条件循环（while）
{"loop": {"while": {"state": "hunger", "lt": 15}}, "do": [
  {"tool": "consume", "args": {"item": "cooked_beef"}}
]}

// 遍历（foreach）
{"loop": {"foreach": "entity", "filter": "type=cow"}, "do": [
  {"tool": "attack", "args": {"ticks": 30}}
]}
```

### 执行引擎行为

```
Rust 执行引擎:
  1. 接收 LLM 的计划 JSON
  2. 逐条解释：
     - tool: 发 TCP 到 Mod，等待结果
     - if: 评估条件（Rust 侧有上一次 reload 的缓存状态）
     - loop: 重复执行子步骤
     - wait: sleep N 秒
  3. 任何一步失败 → 立即停止，返回：
     {
       "plan_status": "failed_at_step_3",
       "successful_steps": 2,
       "error": "nav_to failed: no path",
       "partial_results": [...]
     }
  4. 全部成功 → 返回合并结果
```

---

## 全部工具清单及改进方案

### 1. 核心原子工具（必须修好）

#### 1.1 `collect` — 采集
**当前问题**：Rust 侧自己找方块→导航→挖→检查，流程复杂且易断。
**正确做法**：全部移到 Java Mod 侧。Mod 收到 `collect(target, count)` 后：
1. 用 `BuiltInRegistries.BLOCK` 找最近方块
2. 用 `VanillaPathfinder` 导航到方块
3. 用 `PlayerInteractionManager.destroyBlock()` 破坏方块
4. 重复直到 count 满足或没方块了
5. 返回 `{collected: N, blocks_broken: N, time_ms: T}`
**实现**：新建 `CollectController.java`

#### 1.2 `combat` — 战斗
**当前问题**：自定义瞄准+攻击循环，不如原版精准。
**正确做法**：用 `ServerPlayer.attack(targetEntity)` — 原版自带的命中判定、击退、无敌帧。
**实现**：简化 `CombatController.java`

#### 1.3 `place` — 放置
**当前问题**：基本正常，但失败诊断不够精准。
**改进**：补全 face 参数（北/南/东/西/上/下）

#### 1.4 `nav_to` — 导航
**当前问题**：已用 VanillaPathfinder，需要调参
**改进**：优化跨水/悬崖/栅栏。废弃 `move_to`

#### 1.5 `digDown` — 向下挖
**当前问题**：坐标对齐有问题
**改进**：用 `player.getBlockPositionBelow()`

#### 1.6 `pillar_up` — 垫脚
**改进**：整合到 nav_to 的 auto-pillar 里

#### 1.7 `consume` / `eat_item` — 进食
**改进**：统一到 autoSurvive 层

#### 1.8 `craft` — 合成
**当前问题**：Rust 侧硬编码配方，2×2 网格限制
**改进**：移到 Mod 侧，利用 `CraftingMenu`

---

### 2. 复合工具（新增）

每个复合工具 = Java Mod 侧状态机，一次调用完成多步操作。

#### 2.1 `hunt_food()` — 打猎
```
Mod 内部: 找动物 → nav_to → attack → 捡掉落
输出: {killed: "cow", food_got: 3}
```

#### 2.2 `gather_wood(N)` — 伐木
```
Mod 内部: 找树 → nav_to → destroyBlock × N → 捡掉落
```

#### 2.3 `gather_stone(N)` — 采石
```
Mod 内部: 装备镐 → 找石头 → destroyBlock × N
```

#### 2.4 `craft_tools(type)` — 工具合成链
```
Mod 内部: 检查材料 → gather 缺的 → 合成中间产物 → 合成目标 → equip
```

#### 2.5 `build_shelter()` — 建避难所
```
Mod 内部: 检查材料 → 垫平 → 3×3 小屋 → 火把
```

#### 2.6 `explore_cave()` — 探洞
```
Mod 内部: 找入口 → 下洞 → 插火把 → 扫矿 → 回地面
```

---

### 3. 现有工具分类 & 改进成本

| 类别 | 工具数 | 当前状态 | 改进方案 | 工作量 |
|------|-------|---------|---------|-------|
| **导航** | 6 | 中等 | 统一到 VanillaPathfinder，废弃 move_to | 小 |
| **采集** | 1 | 差 | 移到 Mod 侧状态机 | 大 |
| **战斗** | 8 | 中差 | 用 player.attack 替代自定义瞄准 | 中 |
| **建造** | 6 | 中等 | place 加 face 参数，build 不变 | 小 |
| **物品** | 15 | 中差 | craft 移到 Mod，collect 重写 | 大 |
| **玩家交互** | 12 | 中等 | 基本不变，加错误处理 | 小 |
| **感知** | 3 | 好 | 不变 | 无 |
| **生存** | 5 | 中等 | 增强 autoSurvive | 中 |
| **载具** | 2 | 中等 | 基本不变 | 小 |
| **记忆** | 3 | 好 | 不变 | 无 |
| **元工具** | 2 | 好 | 不变 | 无 |
| **★ 执行引擎** | 1 | 新增 | 步骤引擎 + 条件/循环解释器 | 大 |

---

### 4. 实施阶段

#### Phase 0: 基础设施（2 天）
- [ ] 统一 `VanillaPathfinder` 调参
- [ ] 加 `autoSurvive`（auto-eat + auto-equip）
- [ ] 废弃 `move_to`，全用 `nav_to`
- [ ] 原子工具错误信息标准化

#### Phase 1: 核心原子工具重修（3 天）
- [ ] `collect` → Mod 侧 `CollectController`
- [ ] `combat` → 用 `player.attack(entity)`
- [ ] `craft` → Mod 侧完整配方表
- [ ] `digDown` → 修复坐标对齐
- [ ] `place` → 加 face 参数

#### Phase 2: 步骤执行引擎（3 天）
- [ ] Rust 侧：计划格式定义 + 解释器（顺序/if/loop/wait）
- [ ] Rust 侧：条件评估（状态缓存 + 比较）
- [ ] Rust 侧：错误处理（失败返回 + 部分结果）
- [ ] LLM 侧：system prompt 告诉 LLM 用计划格式

#### Phase 3: 复合工具 + 测试（3 天）
- [ ] `hunt_food()` / `gather_wood(N)` / `gather_stone(N)`
- [ ] `craft_tools(type)` / `build_shelter()` / `explore_cave()`
- [ ] 用步骤引擎重写现有工具的 LLM 使用方式
- [ ] smoke test 覆盖全部工具 + 步骤引擎

#### Phase 4: 打磨（持续）
- [ ] 超时/重试/错误信息精确化
- [ ] 性能优化
- [ ] 更多复合工具

---

### 5. 关于"底层在哪"的决策

**执行引擎在 Rust 侧**（解释计划、评估条件、循环控制）
**原子工具在 Java Mod 侧**（直接调原版 API）

```
Rust 执行引擎:
  计划解释 → 条件评估 → 循环控制 → 错误处理
          ↓ TCP
  Java Mod:
  原子工具实现 (destroyBlock / attack / nav_to / ...)
```

**为什么执行引擎在 Rust 不在 Mod**：
- 计划解释是纯逻辑，不需要游戏 tick
- Rust 更适合做流程控制（Result/Error 处理）
- 条件评估只需要缓存的状态，不需要实时游戏数据
- 解耦：换引擎实现不用改 Mod

---

### 6. 时间线

| 阶段 | 内容 | 预估工时 |
|------|------|---------|
| Phase 0 | 基础设施 | 2 天 |
| Phase 1 | 原子工具重修 | 3 天 |
| Phase 2 | 步骤执行引擎 | 3 天 |
| Phase 3 | 复合工具 + 测试 | 3 天 |
| Phase 4 | 打磨 | 持续 |

**总计**: 11 天。**效果超越 Mindcraft**，因为：
1. 步骤引擎 = LLM 写代码的灵活性（条件/循环/组合）
2. Mod 侧原版 API = 比 Mineflayer 更可靠的底层
3. 安全可控 = 没有 eval() 风险
4. 复合工具 = 常见场景一步到位