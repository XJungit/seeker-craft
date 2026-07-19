# Craft-Agent 能力升级计划：对齐 Mindcraft

> 日期：2026-07-15
> 目标：寻路、建造、战斗、规划四个维度达到或超过 Mindcraft 水平
> 方案：集成 Baritone (A* 寻路) + 自研蓝图建造 + 战斗 AI + 规划系统

## 一、现状差距分析

### 1. 寻路
- **Mindcraft**: mineflayer-pathfinder (A* + 动态避障 + 水域/门/跳跃处理)
- **Craft-Agent**: moveToward (朝目标走 + 遇墙跳/绕，无 A*)
- **差距**: 巨大。无路径规划，遇复杂地形必卡

### 2. 建造
- **Mindcraft**: 自研 JSON 蓝图 (3D 数组 blocks[y][z][x]) + 逐层放置 + 缺料收集续建
- **Craft-Agent**: place (准星处放置，无坐标指定，无蓝图)
- **差距**: 巨大。无法精确建造结构

### 3. 战斗
- **Mindcraft**: mineflayer-pvp + 硬编码走位 (creeper 不贴脸、濒死撤退、风筝)
- **Craft-Agent**: attack (按住左键，无走位)
- **差距**: 中等。能打但不会走位，遇苦力怕必死

### 4. 规划
- **Mindcraft**: LLM CoT + SelfPrompter (持续目标注入) + Modes 反应系统 + 记忆总结
- **Craft-Agent**: 基础循环 + nudge，无持续目标注入，无反应系统
- **差距**: 中等

## 二、技术方案

### 核心策略：集成 Baritone

Baritone 是 Java Fabric mod，与我们架构完全同构。直接作为依赖加入 `craft-agent-bridge`，通过 `BaritoneAPI` 调用其 A* 寻路、建造、挖矿能力。

**优势**：
- A* 寻路立刻达到/超过 Mindcraft 水平（Baritone 能挖/放方块开路，mineflayer-pathfinder 不能）
- 建造系统自带 BuildProcess
- 成熟稳定（4277 commits，支持到 MC 1.21.8）
- 不需要重写任何底层

### 架构变更

```
┌─────────────────────────────────────────┐
│  Rust (craft-agent)                     │
│  ├── LLM 决策循环                        │
│  ├── 规划系统 (SelfPrompter + Modes)     │
│  ├── 蓝图系统 (JSON 格式)                │
│  └── 工具层 (命令 → mod)                 │
└──────────────┬──────────────────────────┘
               │ TCP (端口 25567)
┌──────────────▼──────────────────────────┐
│  Java Fabric Mod (craft-agent-bridge)   │
│  ├── Baritone 集成                       │
│  │   ├── path_to(x,y,z)  → A* 寻路      │
│  │   ├── build(blueprint) → 蓝图建造    │
│  │   └── mine(target)    → 自动挖矿     │
│  ├── 战斗 AI (走位/风筝/撤退)            │
│  ├── place_at(x,y,z,item) → 精确放置    │
│  └── dig_at(x,y,z) → 精确破坏           │
└─────────────────────────────────────────┘
```

## 三、详细实施计划

### Phase 1: 寻路 — 集成 Baritone (P0)

#### 1.1 Mod 侧：添加 Baritone 依赖

**文件**: `mods/craft-agent-bridge/build.gradle`
- 添加 Baritone Fabric 依赖 (maven repo: `https://maven.example.com/baritone`)

**文件**: `mods/craft-agent-bridge/src/main/java/com/craftagent/bridge/CraftAgentBridge.java`
- 新增命令处理:
  - `path_to {x, y, z}`: 调用 `BaritoneAPI.getProvider().getPrimaryBaritone().getCustomGoalProcess().setGoalAndPath(new GoalBlock(x,y,z))`
  - `path_stop`: `BaritoneAPI.getProvider().getPrimaryBaritone().getPathingBehavior().cancelIfSafe()`
  - `path_status`: 查询路径状态 (arrived/in_progress/failed/calc_failed)
- 返回结构: `{reached: bool, final_dist: f64, ticks_used: u32, path_status: string}`

#### 1.2 Rust 侧：新工具

**文件**: `crates/craft-agent-minecraft/src/tools_mod.rs`
- 重构 `ModMoveToTool` → 使用 `path_to` 命令（替换原 moveToward）
- 新增 `ModPathStatusTool` (可选，查询寻路状态)

**文件**: `crates/craft-agent-minecraft/src/bridge.rs`
- `ModCommand` 枚举新增 `PathTo { x, y, z }` 变体

#### 1.3 验收标准
- [ ] `path_to(100, 64, 200)` 能绕过障碍物到达目标
- [ ] 遇水会绕行或游泳
- [ ] 遇高墙会跳跃或绕行
- [ ] 返回准确的 reached/final_dist
- [ ] 寻路超时有降级处理

---

### Phase 2: 建造 — 蓝图系统 + 精确放置 (P0)

#### 2.1 Mod 侧：精确放置/破坏命令

**文件**: `CraftAgentBridge.java`
- 新增 `place_at {x, y, z, item}`: 
  - 找到背包中的 item
  - 用 `mc.gameMode.useItemOn(player, itemStack, blockHitResult)` 直接在指定坐标放置
  - 构造 BlockHitResult: `new BlockHitResult(Vec3.atCenterOf(pos), Direction.UP, pos, false)`
  - 返回 `{placed: bool, x, y, z}`
- 新增 `dig_at {x, y, z}`:
  - `mc.gameMode.destroyBlock(pos)` 或模拟左键
  - 返回 `{broken: bool, block_id: string}`

#### 2.2 Rust 侧：蓝图系统

**文件**: `crates/craft-agent-minecraft/src/blueprint.rs` (新建)
```rust
pub struct Blueprint {
    pub name: String,
    pub offset: i32,           // y 偏移（地基层为负）
    pub blocks: Vec<Vec<Vec<String>>>,  // [y][z][x] = block_name
}

impl Blueprint {
    pub fn from_json(path: &str) -> Result<Self>;
    pub fn build_at(&self, x: i32, y: i32, z: i32, orientation: u32) -> Vec<BuildStep>;
    pub fn materials_needed(&self) -> HashMap<String, u32>;
}

pub struct BuildStep {
    pub x: i32, pub y: i32, pub z: i32,
    pub action: BuildAction,  // Place(item) or Dig
}
```

**文件**: `crates/craft-agent-minecraft/src/tools_mod.rs`
- 新增 `ModBuildTool`:
  - 参数: `blueprint` (名称), `x, y, z` (位置), `orientation` (0-3)
  - 执行: 加载蓝图 → 按层生成 BuildStep → 逐步调用 place_at/dig_at
  - 缺料时返回 `missing` 列表，让 LLM 先去采集
- 新增 `ModBlueprintsTool`:
  - 列出可用蓝图 (dirt_shelter, wood_house, stone_house 等)

#### 2.3 内置蓝图

**文件**: `crates/craft-agent-minecraft/blueprints/*.json`
- `dirt_shelter.json`: 3x3 简易泥土庇护所 (参考 Mindcraft 格式)
- `wood_house.json`: 5x5 木屋 (含门、工作台、床)
- `stone_house.json`: 5x5 石屋
- `wall_3x3.json`: 3x3 墙壁片段

#### 2.4 验收标准
- [ ] `build("dirt_shelter", x, y, z, 0)` 能建出完整庇护所
- [ ] 缺料时返回缺失列表
- [ ] 已有方块不重复放置
- [ ] 非 air 方块先挖再放
- [ ] 建造中途可中断续建

---

### Phase 3: 战斗 — 走位 AI (P1)

#### 3.1 Mod 侧：战斗模式

**文件**: `CraftAgentBridge.java`
- 重构 `attack` 命令为 `combat {mode, target_id?}`:
  - `mode: "melee"`: 近战走位 (接近→攻击→后撤→循环)
  - `mode: "retreat"`: 撤退 (远离最近的敌对实体)
  - `mode: "kite"`: 风筝 (保持 3-4m 距离)
- 战斗逻辑 (mod 侧自主执行，不依赖 Rust 逐 tick 控制):
  ```
  while (target alive && self alive && !cancelled):
    dist = distance to target
    if target is creeper and dist < 5:
      move away 3 blocks
      wait for explosion cooldown
    elif dist > 4:
      path_to(target)  // 用 Baritone 寻路接近
    elif dist < 3:
      attack
      move back 1 block  // 攻击后撤
    else:
      attack
    sleep(200ms)
  ```
- 自动装备最优武器 (已有)
- 濒死 (hp < 5) 自动撤退

#### 3.2 Rust 侧

**文件**: `tools_mod.rs`
- 重构 `ModAttackTool` → `ModCombatTool`:
  - 参数: `mode` (melee/retreat/kite), `ticks` (持续时间)
  - 调用 `combat` 命令
  - 返回战斗结果 (击杀/撤退/超时)

#### 3.3 验收标准
- [ ] 单只僵尸：能击杀且不掉血或掉血 < 5
- [ ] 苦力怕：检测后撤，不被炸死
- [ ] 濒死 (hp<5)：自动撤退
- [ ] 多目标：优先打最近的

---

### Phase 4: 规划 — SelfPrompter + Modes 反应 (P1)

#### 4.1 SelfPrompter (持续目标注入)

**文件**: `crates/craft-agent/src/agent.rs`
- 新增 `SelfPrompter` 机制:
  - LLM 可调用 `set_goal(goal_text)` 工具设定长期目标
  - 每步注入: `你正在执行目标: '{goal}'. 下一步必须调用工具.`
  - 连续 3 步无工具调用 → 停止 self-prompting
  - 目标完成时调用 `end_goal()`

#### 4.2 Modes 反应系统

**文件**: `crates/craft-agent/src/modes.rs` (新建)
- tick 级反应模式 (参考 Mindcraft modes.js):
  - `self_preservation`: 生命值低 → 撤退/吃食物
  - `self_defense`: 附近有敌对实体 → 自动战斗
  - `unstuck`: 卡住超过 20s → moveAway
  - `hunger`: 饥饿值低 → 吃食物
- 每步 perceive 后检查 modes，必要时注入紧急消息打断当前计划

#### 4.3 记忆总结

**文件**: `crates/craft-agent/src/agent.rs`
- 新增 `summarize_memory` 工具:
  - 当历史超过阈值时，让 LLM 总结"重要事实/技巧/长期提醒"
  - 存入 WorldInfo (已有机制)
  - 后续注入 prompt

#### 4.4 Prompt 工程改进

**文件**: `crates/craft-agent/src/agent.rs` (MC_KNOWLEDGE)
- 添加建造指南: "使用 build(blueprint) 建造庇护所，不要手动逐块放置"
- 添加战斗指南: "遇到苦力怕用 combat(mode=retreat)，不要近战"
- 添加规划指南: "复杂任务先用 set_goal 分解，再逐步执行"

#### 4.5 验收标准
- [ ] 设定目标后 agent 持续执行直到完成
- [ ] 生命值低时自动撤退
- [ ] 附近有僵尸时自动战斗
- [ ] 卡住时自动脱困

## 四、实施顺序

| 顺序 | Phase | 依赖 | 工作量 |
|------|-------|------|--------|
| 1 | Phase 1 寻路 | 无 | 中（Baritone 集成 + 命令封装） |
| 2 | Phase 2 建造 | Phase 1 (用 path_to 接近建造点) | 大（蓝图系统 + place_at） |
| 3 | Phase 3 战斗 | Phase 1 (用 path_to 接近/撤退) | 中（战斗 AI 逻辑） |
| 4 | Phase 4 规划 | 无 | 中（prompt + modes 系统） |

Phase 4 可与 Phase 1-3 并行开发。

## 五、风险与对策

| 风险 | 对策 |
|------|------|
| Baritone 版本与 MC 版本不兼容 | 先确认 MC 版本，选对应 Baritone 版本 |
| Baritone 依赖冲突 | 用 jar-in-jar 或 shadow 方式打包 |
| place_at 在多人模式被服务器拒绝 | 当前单机模式无此问题；多人模式后续处理 |
| 战斗 AI TCP 延迟 | 战斗逻辑全在 mod 侧自主执行，Rust 只发命令 |
| 蓝图格式复杂 | 直接采用 Mindcraft 的 JSON 格式，兼容性好 |

## 六、参考资源

- Baritone 仓库: https://github.com/cabaletta/baritone
- Baritone API 文档: https://baritone.leijurv.com/
- Mindcraft 仓库: https://github.com/kolbytn/mindcraft
- Mindcraft 蓝图格式: `src/agent/npc/construction/*.json` (blocks[y][z][x])
- Mindcraft 战斗逻辑: `src/agent/library/skills.js` (attackEntity/defendSelf)
- Mindcraft 规划系统: `src/agent/self_prompter.js` + `src/agent/modes.js`
