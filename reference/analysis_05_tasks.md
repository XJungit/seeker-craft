# Mindcraft 任务系统与 NPC 蓝图系统分析报告

> 本报告系统化分析 Mindcraft 项目的任务系统（`src/agent/tasks/` + `tasks/*.json`）与 NPC 蓝图系统（`src/agent/npc/`），整理出可移植到 Craft-Agent 的预定义任务设计。
>
> 源码路径：`d:\Craft-Agent\reference\mindcraft\`
> Craft-Agent 对接参考：`d:\Craft-Agent\crates\craft-agent-minecraft\src\tools_azalea.rs`（`BuildTool`）+ `d:\Craft-Agent\crates\craft-agent-minecraft\src\azalea\auto_craft.rs`

---

## 1. Mindcraft 任务系统总览

### 1.1 整体架构

```
tasks/*.json                       src/agent/tasks/
   │  预定义任务（goal/inventory/   │  任务执行/校验逻辑
   │  blueprint/target/timeout）    │
   └──────┬─────────────────────────┘
          │ TaskRunner 读取 JSON
          ▼
   ┌──────────────────────────────────────────┐
   │  Task 实例（src/agent/tasks/tasks.js）   │
   │  - constructor 解析 task_data           │
   │  - initBotTask()：清背包/发物资/传送/对话 │
   │  - isDone()：调 validator + timeout     │
   └──────┬───────────────────────────────────┘
          │
          ▼ 按 type 分派
   ┌────────────┬───────────────┬─────────────────┐
   │construction│ cooking/tech │ debug/其他       │
   │            │ tree         │                  │
   ▼            ▼              ▼
 ConstructionTask   CookingCraftingTask   无 validator
 Validator          Validator
 (Blueprint.check)  (checkItemPresence)
```

### 1.2 任务加载 → 目标注入 → 完成判定 全流程

`Task` 类（`tasks.js:234-302`）的构造逻辑：

1. **载入 task_data**：把整个 JSON 对象存入 `this.data`，`this.task_type = this.data.type`。
2. **构造 goal 文本**：
   - 建造任务且带 `blueprint`：`goal = data.goal + '\n' + blueprint.explain() + '\nmake sure to place the lower levels of the blueprint first'`，**并把 blueprint.explain() 同样拼到 conversation**，让 agent 启动对话时就拿到坐标信息。
   - 其他任务：`goal = data.goal`，`conversation = data.conversation`。
3. **设置 timeout**：`this.taskTimeout = this.data.timeout || 300`（秒）。
4. **选择 validator**：
   - `construction` → `ConstructionTaskValidator(data, agent)`
   - `cooking` / `techtree` → `CookingCraftingTaskValidator(data, agent)`
   - 其他 → `null`（仅靠 timeout 或人工判断）
5. **blocked_actions**：从 `data.blocked_actions[agent.count_id]` 取列表，并在有 goal/conversation 时自动 push `!endGoal` / `!endConversation`（防止 agent 自行结束目标或对话）。
6. **restrict_to_inventory**：`!!this.data.restrict_to_inventory`（限制 agent 只能用初始物资）。
7. **Hells Kitchen 特例**：`task_id.endsWith('hells_kitchen')` 时调用 `hellsKitchenProgressManager.resetTask()`，重置文件级双 agent 进度跟踪器。

`initBotTask()`（`tasks.js:408-509`）执行流程：

```
1. /clear <bot_name> 清空自己背包
2. cooking 任务：new CookingTaskInitiator（agent 0 才会真正调 init()）
3. /tp <self> <other_agent> 把所有 bot 传送到一起（建造任务不随机偏移，其他任务随机 ±5 格）
4. construction 任务：blueprint.autoDelete() 生成 /setblock ... air 命令清空建造区
5. 发放初始物资：data.initial_inventory[agent_id] → /give <bot> <item> <count>
   - 若有 human_count，agent 0 还要给人类玩家发物资
6. cooking initiator.init()：建农场/房子/动物
7. 多 agent 任务：等待 10 秒确认其他 bot 上线，缺失则 killAll
8. 启动对话：!startConversation("<other>", "<conversation>")
9. setAgentGoal()：!goal("<agentGoal>") 写入 SelfPrompter
```

`isDone()`（`tasks.js:365-397`）每轮调用：

```
1. validator.validate() → {valid, score}
2. 若 valid：对所有 available_agents /clear 清空背包，返回 {message:'Task successful', score}
3. 计算 elapsedTime = (now - taskStartTime) / 1000
4. 若 elapsedTime >= 30 且 agent 数量不足：返回 'No other agents found', score 0
5. 若 taskTimeout 且 elapsedTime >= taskTimeout：返回 'Task timeout reached', score = res.score 或 0
6. 否则返回 false（未完成）
```

---

## 2. 任务 JSON Schema 完整定义

### 2.1 字段总表

| 字段 | 类型 | 适用类型 | 含义 |
|---|---|---|---|
| `goal` | `string` \| `{agent_id: string}` \| `array` | 全部 | 任务目标文本。字符串=所有 agent 共享；对象=按 agent_id 分配；数组=hells_kitchen 双 agent 分别对应 target |
| `conversation` | `string` | 多 agent | 启动时由 agent 0 通过 `!startConversation` 发给 agent 1 的开场白 |
| `agent_count` | `int` | 全部 | 预期参与 agent 数量（不含人类） |
| `human_count` | `int` | 可选 | 人类玩家数量 |
| `usernames` | `string[]` | 可选 | 人类玩家名（顺序对应 initial_inventory 的后续 key） |
| `type` | `"construction"` \| `"cooking"` \| `"techtree"` \| `"debug"` | 全部 | 任务类型，决定 validator |
| `target` | `string` \| `string[]` \| `{item: qty}` | techtree/cooking | 目标物品。hells_kitchen 用 2 元素数组，每个 agent 对应一个 |
| `number_of_target` | `int` \| `{item: qty}` | techtree/cooking | 目标数量。`target` 为 dict 时此项忽略 |
| `initial_inventory` | `{agent_id: {item: qty}}` \| `{item: qty}` | 全部 | 初始背包。单 agent 可省略外层 agent_id（如 `construction_house`） |
| `blueprint` | `{materials, levels}` | construction | 建造蓝图（详见第 4 节） |
| `timeout` | `int`（秒） | 全部 | 默认 300 |
| `restrict_to_inventory` | `bool` | 全部 | 限制只用初始物资（不采集/不收礼） |
| `blocked_actions` | `{agent_id: [cmd]}` | 全部 | 禁用的命令（如 `!endGoal`、`!endConversation`） |
| `recipes` | `{item: [step_strings]}` | cooking | 配方步骤文本（仅描述用，不参与校验） |
| `blocked_access_to_recipe` | `string[]` | cooking | 禁用某 agent 对某配方的访问（hells_kitchen） |
| `difficulty` | `"easy"` \| `"medium"` \| `"hard"` | 可选 | 难度标签（评估用，不影响执行） |
| `difficulty_metrics` | `object` | 可选 | 难度量化指标（total_recipe_steps / max_steps_per_recipe / unique_target_items / overall_difficulty_score / difficulty_category） |
| `task_id` | `string` | 全部 | 任务唯一 id（隐式从 JSON key 来）；`hells_kitchen` 后缀触发双 agent 进度文件 |
| `max_depth`/`depth`/`missing_items`/`requires_ctable` | `int`/`int`/`array`/`bool` | techtree 罕见 | 难度分析辅助字段（如 `gather_oak_logs` 中出现） |

### 2.2 基础单 agent 任务示例（原文）

`tasks/basic/single_agent.json`：

```json
{
    "gather_oak_logs": {
      "goal": "Collect at least four logs",
      "initial_inventory": {
        "0": {
          "wooden_axe": 1
        }
      },
      "agent_count": 1,
      "target": "oak_log",
      "number_of_target": 4,
      "type": "techtree",
      "max_depth": 1,
      "depth": 0,
      "timeout": 300,
      "blocked_actions": {
        "0": [],
        "1": []
      },
      "missing_items": [],
      "requires_ctable": false
    }
}
```

### 2.3 多 agent techtree 示例（原文，`example_tasks.json`）

```json
"multiagent_techtree_1_stone_pickaxe": {
    "conversation": "Let's collaborate to build a stone pickaxe",
    "agent_count": 2,
    "initial_inventory": {
        "0": { "wooden_pickaxe": 1 },
        "1": { "wooden_axe": 1 }
    },
    "target": "stone_pickaxe",
    "goal": "Build a stone pickaxe",
    "number_of_target": 1,
    "type": "techtree",
    "timeout": 300
}
```

### 2.4 调试任务示例（`example_tasks.json`）

```json
"debug_inventory_restriction": {
    "goal": "Place 1 oak plank, then place 1 stone brick",
    "initial_inventory": {
        "0" : { "oak_planks": 20 }
    },
    "type": "debug",
    "restrict_to_inventory": true
}
```

### 2.5 Hells Kitchen 烹饪任务示例（原文，`tasks/cooking_tasks/require_collab_test_2_items/2_agent_hells_kitchen.json`）

```json
"multiagent_cooking_bread_golden_apple_hells_kitchen": {
    "conversation": "We need to make bread and golden_apple together. You are supposed to make golden_apple and I am supposed to make bread, but I only have YOUR recipe and you only have access to MY recipe! Let's exchange information and get cooking!",
    "agent_count": 2,
    "target": ["bread", "golden_apple"],
    "type": "cooking",
    "timeout": 300,
    "recipes": {
      "bread": [
        "Step 1: Go to the farm and collect 3 wheat.",
        "Step 2: Go to the crafting table and use the wheat to craft bread."
      ],
      "golden_apple": [
        "Step 1: Get 1 apple and 8 gold ingots from your inventory or other bots.",
        "Step 2: Go to the crafting table and surround the apple with the gold ingots to create a golden apple."
      ]
    },
    "blocked_access_to_recipe": [],
    "goal": {
      "0": "You need to make bread, but you don't have the recipe for it, your partner has it!\n\nYour partner needs to make golden_apple. You have their recipe:\nRecipe for golden_apple:\n...\nYou must communicate effectively to exchange recipe information and complete both dishes. Note: You can only guide your partner with recipe steps. You cannot help with ingredient collection or cooking.",
      "1": "You need to make golden_apple, but you don't have the recipe for it, your partner has it!\n\nYour partner needs to make bread. You have their recipe:\nRecipe for bread:\n...\nYou must communicate effectively..."
    },
    "task_type": "cooking",
    "difficulty_metrics": {
      "total_recipe_steps": 4,
      "max_steps_per_recipe": 2,
      "unique_target_items": 2,
      "overall_difficulty_score": 4,
      "difficulty_category": "medium"
    },
    "difficulty": "medium",
    "initial_inventory": {
      "0": { "gold_ingot": 5 },
      "1": { "gold_ingot": 5, "apple": 1 }
    }
}
```

---

## 3. 三类任务的执行差异

### 3.1 construction（建造任务）

**初始化特殊处理**：
- `Task` 构造时把 `blueprint.explain()` 拼接到 `goal` 和 `conversation`，并追加 `"make sure to place the lower levels of the blueprint first"`。
- `initBotTask()` 调 `blueprint.autoDelete()` 生成 `/setblock x y z air` 命令清空建造区（不随机传送，保证 bot 站在 blueprint 起点）。
- 传送目标为 `levels[0].coordinates`（蓝图第一层起点）。

**Validator**：`ConstructionTaskValidator`
- `validate()` 调 `blueprint.check(bot)`：对每层每个 (zOffset, xOffset) 比较 `bot.blockAt(x,y,z)` 与 `placement[z][x]`。
- 返回 `{mismatches, matches}`，`mismatches.length === 0` 即 `valid`。
- `score = matches / (matches + mismatches) * 100`（完成度百分比）。

**关键文件**：`construction_tasks.js` 的 `Blueprint` 类。

### 3.2 cooking（烹饪任务）

**初始化特殊处理**（仅 agent 0 触发 `CookingTaskInitiator.init()`）：
1. `/fill ... grass_block` 平整 50×50 地面
2. `/fill ... air` 清空 50×50×10 空中
3. **种植 6 类作物**：wheat / beetroots / mushrooms / potatoes / carrots / sugar_cane（×2）/ pumpkins，每块 6×6 或 5×1（pumpkins）
4. **建房子**（10×10×5）：石砖墙 + 玻璃窗 + 橡木门 + 圆石屋顶
5. **内部设施**：crafting_table / furnace（带 64 煤燃料）/ smoker（带 64 煤燃料）/ bed
6. **动物管理**：先 `/kill @e` 清空场内动物 + 掉落物，再 `/summon` 7 种动物（chicken/cow/llama/mooshroom/pig/rabbit/sheep）各 8 只

**Validator**：`CookingCraftingTaskValidator`
- 调 `checkItemPresence(data, agent)` 检查背包是否有目标物品。
- `target` 支持三种格式：
  - `string` → `{[target]: 1}`
  - `array` → 每个 item 数量 1
  - `dict {item: qty}` → 直接用
- `number_of_target` 支持 `int`（所有 target 同量）或 `dict`。
- **Hells Kitchen 特殊路径**：`task_id.endsWith('hells_kitchen')` 且 `target.length === 2` 时，按 `agent.count_id` 取 `target[agentId]` 单独校验，并通过 `hells_kitchen_progress.json` 文件跟踪双 agent 进度，**两 agent 都满足才返回 success**。

### 3.3 techtree（合成任务）

**初始化特殊处理**：无（不建农场不建房子，仅发放物资 + 传送）。

**Validator**：与 cooking 共用 `CookingCraftingTaskValidator`，校验背包有 `target × number_of_target`。

**与 cooking 的区别**：techtree 不需要烹饪世界环境（农场/动物/熔炉），通常只考合成链；cooking 强依赖 initiator 搭的农场+动物+已装燃料的熔炉。

### 3.4 三类对比表

| 维度 | construction | cooking | techtree |
|---|---|---|---|
| Validator | ConstructionTaskValidator | CookingCraftingTaskValidator | CookingCraftingTaskValidator |
| 校验对象 | 世界方块（blueprint.check） | 背包物品 | 背包物品 |
| 特殊初始化 | autoDelete 清空蓝图区 | CookingTaskInitiator 搭农场+动物+房子 | 无 |
| 传送策略 | 固定到 blueprint.levels[0].coordinates | 随机 ±5 偏移 | 随机 ±5 偏移 |
| Goal 文本 | goal + blueprint.explain() | goal（可分 agent_id） | goal |
| 多 agent 协作 | 分物资建同一蓝图 | hells_kitchen 互递配方 | 分物资合成同一物品 |
| Score | 完成度百分比（0-100） | 0 或 1 | 0 或 1 |

---

## 4. NPC 蓝图系统详解

> 注意：Mindcraft 有**两套**蓝图 schema：
> - **任务蓝图**（`tasks/*.json` 中的 `blueprint` 字段）：`{materials, levels:[{level, coordinates, placement}]}`，配合 `Blueprint` 类用于校验。
> - **NPC 蓝图**（`src/agent/npc/construction/*.json`）：`{name, offset, blocks}`，3D 数组配合 `BuildGoal` 用于 NPC 自主建造。
>
> 两套不通用！本节聚焦 NPC 蓝图。

### 4.1 NPC 蓝图 JSON Schema

```json
{
  "name": "string",          // 蓝图名（与文件名一致）
  "offset": int,            // Y 偏移（通常为负数，表示从脚下多少格开始算 blocks[0]）
  "blocks": [               // 3D 数组 [y][z][x]
    [                       // blocks[0] = 第 0 层（最底）
      ["block_id", ""],     // blocks[0][z][x]，"" 表示跳过（不操作），"air" 表示清空
      ...
    ],
    ...
  ]
}
```

**关键约定**：
- `blocks[y][z][x]`：第一维是 Y（高度），第二维是 Z（南北），第三维是 X（东西）。
- 空字符串 `""` = 跳过该格不操作（与 `"air"` 不同，`"air"` 会主动 setblock air）。
- `offset` 是**世界 Y 坐标偏移**：实际世界 Y = `position.y + y + offset`（其中 position 是 bot 选定的放置原点，见 `build_goal.js`）。负数表示 blocks[0] 在脚下方。
- `name` 字段在 `controller.js` 中作 key，与文件名（去 `.json`）对应。

### 4.2 4 个蓝图对比

| 蓝图 | 尺寸 X×Z×Y | offset | 总方块数（非空） | 主要材料 | 复杂度 |
|---|---|---|---|---|---|
| **dirt_shelter** | 5×6×4 | -2 | ≈40 | dirt / bed / chest / door / torch | ★ 简易避难所，2 层可用空间 |
| **small_wood_house** | 5×7×4 | -1 | ≈80 | planks / log（橡木柱） | ★★ 木屋，4 层带屋顶斜面 |
| **small_stone_house** | 5×7×4 | -1 | ≈90 | cobblestone（外墙）+ planks（地基） | ★★ 石屋，结构同木屋但材料更结实 |
| **large_house** | 11×14×13 | -4 | ≈500+ | cobblestone / planks / log / glass / bookshelf / furnace / crafting_table / bed / torch / door / dirt | ★★★★★ 多层别墅，含地下室/书房/玻璃幕墙/阁楼 |

**设计差异详解**：

#### 4.2.1 dirt_shelter（最小，应急避难）

```json
{
    "name": "dirt_shelter",
    "offset": -2,
    "blocks": [
        [["", "", "", "", ""],
         ["", "dirt", "dirt", "dirt", ""],
         ["", "dirt", "dirt", "dirt", ""],
         ["", "dirt", "dirt", "dirt", ""],
         ["", "", "dirt", "", ""],
         ["", "", "dirt", "", ""]],
        [["dirt", "dirt", "dirt", "dirt", "dirt"],
         ["dirt", "chest", "bed", "air", "dirt"],
         ["dirt", "air", "bed", "air", "dirt"],
         ["dirt", "air", "air", "air", "dirt"],
         ["dirt", "dirt", "door", "dirt", "dirt"],
         ["dirt", "dirt", "air", "dirt", "dirt"]],
        [["dirt", "dirt", "dirt", "dirt", "dirt"],
         ["dirt", "air", "air", "air", "dirt"],
         ["dirt", "torch", "air", "air", "dirt"],
         ["dirt", "air", "air", "air", "dirt"],
         ["dirt", "dirt", "door", "dirt", "dirt"],
         ["air", "air", "air", "air", "air"]],
        [["air", "air", "air", "air", "air"],
         ["dirt", "dirt", "dirt", "dirt", "dirt"],
         ["dirt", "dirt", "dirt", "dirt", "dirt"],
         ["dirt", "dirt", "dirt", "dirt", "dirt"],
         ["air", "air", "air", "air", "air"],
         ["air", "air", "air", "air", "air"]]
    ]
}
```

特点：4 层结构（地下 2 层地基 + 2 层空间），3×3 室内，含 chest+bed+door+torch，全部用 dirt。屋顶用 dirt 留出 3×3 空间。

#### 4.2.2 small_wood_house（标准木屋）

```json
{
    "name": "small_wood_house",
    "offset": -1,
    "blocks": [
        [["", "", "", "", ""],
         ["", "planks", "planks", "planks", ""],
         ["", "planks", "planks", "planks", ""],
         ["", "planks", "planks", "planks", ""],
         ["", "planks", "planks", "planks", ""],
         ["", "", "planks", "", ""],
         ["", "", "", "", ""]],
        [["log", "planks", "planks", "planks", "log"],
         ["planks", "chest", "bed", "air", "planks"],
         ["planks", "air", "bed", "air", "planks"],
         ["planks", "air", "air", "air", "planks"],
         ["planks", "air", "air", "air", "planks"],
         ["log", "planks", "door", "planks", "log"],
         ["", "air", "air", "air", ""]],
        ... // 第 2 层带 4 个 torch 角落
        ... // 第 3 层为金字塔屋顶
    ]
}
```

特点：5×7×4，4 角 log 立柱（结构感强），planks 填充，4 角 torch 照明，2 层 door。

#### 4.2.3 small_stone_house

结构同 small_wood_house，但墙体换成 cobblestone，地基仍为 planks（防石头滑落）。

#### 4.2.4 large_house（最复杂）

13 层结构：
- L0-2：地基（cobblestone 平台 + 入口阶梯）
- L3：地下室墙
- L4-7：主屋（4 层高），含 furnace / crafting_table / chest / bed
- L8：阁楼（带书架）
- L9-10：玻璃幕墙层（glass 包围）
- L11：屋顶（log 框架 + planks）
- L12-13：尖顶

特性：multi-room 布局，11×14 大尺度，含 glass / bookshelf / 多个 door / 多个 torch 等装饰性方块。

### 4.3 NPC 蓝图加载（`controller.js`）

```js
init() {
    // 1. 扫描 src/agent/npc/construction/*.json 全部载入 this.constructions
    for (let file of readdirSync('src/agent/npc/construction')) {
        if (file.endsWith('.json')) {
            this.constructions[file.slice(0, -5)] = JSON.parse(...);
        }
    }
    // 2. 补齐为正方体（避免不同层 size 不一致导致越界）
    for (let name in this.constructions) {
        let sizex = blocks[0][0].length;
        let sizez = blocks[0].length;
        let max_size = Math.max(sizex, sizez);
        // 把每层每行都补齐到 max_size × max_size，缺位填 ""
    }
    // 3. 注册 bot 'idle' 事件：空闲 5 秒后若没有 resume_func 则 executeNext
}
```

---

## 5. build_goal.js 蓝图执行流程

`BuildGoal.executeNext(goal, position=null, orientation=null)`（`build_goal.js:20-78`）：

```
输入: goal = {blocks, offset, name}, position = {x,y,z} 或 null, orientation = 0/1/2/3 或 null
输出: {missing: {block: count}, acted: bool, position, orientation}

1. 解析尺寸
   sizex = blocks[0][0].length    // X 维度
   sizez = blocks[0].length       // Z 维度
   sizey = blocks.length          // Y 维度

2. 选位置（若 position 为 null）
   for x in 0..sizex-1:
     position = getNearestFreeSpace(bot, sizex - x, 16)
     if position: break

3. 选朝向（若 orientation 为 null）
   orientation = random(0..3)   // 随机旋转 0/90/180/270

4. 三层循环按 (y, z, x) 顺序遍历
   for y in offset..sizey+offset-1:
     for z in 0..sizez-1:
       for x in 0..sizex-1:
         // 4.1 旋转坐标
         [rx, rz] = rotateXZ(x, z, orientation, sizex, sizez)
         ry = y - offset
         block_name = blocks[ry][rz][rx]

         // 4.2 跳过空字符串
         if block_name === null || '': continue

         // 4.3 计算世界坐标
         world_pos = (position.x + x, position.y + y, position.z + z)
         current_block = bot.blockAt(world_pos)

         // 4.4 不匹配则行动
         if !blockSatisfied(block_name, current_block):
           acted = true
           // 4.4.1 当前不是 air：先破坏
           if current_block.name !== 'air':
             breakBlockAt(world_pos)
             if interrupted: return {missing, acted, position, orientation}
           // 4.4.2 目标不是 air：放置
           if block_name !== 'air':
             block_typed = getTypeOfGeneric(bot, block_name)  // 木/床自动选种类
             if inventory[block_typed] > 0:
               placeBlock(block_typed, world_pos)
               if interrupted: return ...
             else:
               missing[block_typed]++

5. 返回 {missing, acted, position, orientation}
```

### 5.1 坐标旋转（`utils.js:121-126`）

```js
function rotateXZ(x, z, orientation, sizex, sizez) {
    if (orientation === 0) return [x, z];                          // 原向
    if (orientation === 1) return [z, sizex-x-1];                 // 90° 顺时针
    if (orientation === 2) return [sizex-x-1, sizez-z-1];         // 180°
    if (orientation === 3) return [sizez-z-1, x];                 // 270°
}
```

### 5.2 方块匹配宽松规则（`utils.js:73-84`）

`blockSatisfied(target, block)` 不做严格字符串比较：
- `dirt` 同时接受 `dirt` / `grass_block`（草地翻土后变 dirt）
- `planks` / `log` 等 `MATCHING_WOOD_BLOCKS` 接受任意木种（`oak_planks` / `birch_planks` 等都算满足）
- `bed` 接受任意颜色 bed
- `torch` 接受任意 torch 变体
- 其他方块严格名匹配

### 5.3 泛型方块类型推断（`utils.js:5-70`）

`getTypeOfGeneric(bot, block_name)`：把蓝图里的抽象名解析为具体 id：
- 木制方块：选**背包最多**的木种，其次选**最近原木**种类，最后 fallback `oak_*`
- bed：选背包最多颜色的羊毛，fallback `white_bed`
- 其他：原样返回

### 5.4 NPC 控制器主循环（`controller.js:105-206`）

```
executeNext():
  1. moveAway 2 格（避免卡墙）
  2. if 白天 (timeOfDay < 13000):
       a. 若当前在 home 内：useDoor 出门 + moveAway
       b. executeGoal()
  3. else 夜晚:
       a. reset curr_goal
       b. 若不在 home：useDoor 进 home
       c. goToBed

executeGoal():
  goals = temp_goals + data.goals + (curr_goal ? [curr_goal] : [])
  temp_goals = []
  for goal in goals:
    if goal.name 不是 construction (是物品目标):
      if !itemSatisfied(bot, goal.name, goal.quantity):
        item_goal.executeNext(goal.name, goal.quantity)   // 委派给 ItemGoal
        break
    else (是 construction):
      if data.built 已有 goal.name:
        build_goal.executeNext(constructions[name], built.position, built.orientation)  // 续建
      else:
        res = build_goal.executeNext(constructions[name])  // 新建，随机选 position+orientation
        data.built[name] = {name, position: res.position, orientation: res.orientation}
      
      // 把缺料 push 为 temp_goals（让 ItemGoal 后续去收集）
      for block_name in res.missing:
        temp_goals.push({name: block_name, quantity: res.missing[block_name]})
      
      if res.acted: break   // 一次只推进一个动作

  if 全部 goals 已满足且 do_set_goal:
    setGoal()   // LLM 决策下一个目标
```

**关键模式**：建造与采料**交替进行**。`temp_goals` 是临时收集队列，建一块发现缺料就先去采，采完再回来续建。`data.built[name]` 持久化已选 position+orientation，下次续建不会重选。

---

## 6. item_goal.js 物品目标分解流程

`ItemGoal.executeNext(item_name, item_quantity=1)`（`item_goal.js:303-354`）：

```
1. 若 nodes[item_name] 不存在：
   new ItemWrapper(this, null, item_name)   // 自动展开方法树
   存入 nodes[item_name]

2. goal = nodes[item_name]

3. next_info = goal.getNext(item_quantity)
   // DFS 找到下一个 ready 的叶子节点
   if !next_info: return false  // 无可行路径

4. next = next_info.node, quantity = next_info.quantity

5. 防御：附近没有该方块/动物 → fails++ 并 explore (moveAway 8)
   if block 类型且 !getNearbyBlockTypes().includes(source): fails++, explore
   if hunt 类型且 !getNearbyEntityTypes().includes(source): fails++, explore

6. await next.execute(quantity)
   按 type 分派：
   - 'block'  → skills.collectBlock(bot, source, quantity, exclude_positions)   // 排除已建房屋
   - 'smelt'  → skills.smeltItem(bot, to_smelt_name, min(quantity, inventory[to_smelt]))
   - 'hunt'   → 循环 quantity 次: skills.attackNearest(bot, source)
   - 'craft'  → skills.craftRecipe(bot, name, quantity)

7. 检查 final_quantity > init_quantity 判定成功
```

### 6.1 ItemWrapper 方法树构建（`item_goal.js:178-292`）

`ItemWrapper.createChildren()` 自动为每个物品生成多种获取方法：

1. **合成方法**（`mc.getItemCraftingRecipes`）：
   - 每个配方生成 `ItemNode.setRecipe(recipe)`
   - **若配方 size > 4**（即 3×3 工作台合成）：自动添加 `crafting_table` 为 prereq
2. **采集方法**（`mc.getItemBlockSources`）：
   - 每个方块来源生成 `ItemNode.setCollectable(block_source, tool)`
   - `grass_block` 跳过（dirt 节点会顺带处理）
   - 不收集已放置的 torch / bed
3. **熔炼方法**（`mc.getItemSmeltingIngredient`）：
   - 生成 `ItemNode.setSmeltable(source_item)`
   - 自动添加 `furnace` + `coal` 为 prereq + recipe
4. **狩猎方法**（`mc.getItemAnimalSource`）：
   - 生成 `ItemNode.setHuntable(animal_source)`

### 6.2 黑名单（`item_goal.js:7-20`）

```js
const blacklist = [
    'coal_block', 'iron_block', 'gold_block', 'diamond_block',
    'deepslate', 'blackstone', 'netherite',
    '_wood', 'stripped_', 'crimson', 'warped', 'dye'
]
```

命中黑名单的物品不展开子节点（避免无限递归或难获取物品的爆炸式展开）。

### 6.3 最优方法选择（`item_goal.js:256-267`）

```js
getBestMethod(q=1) {
    for method in methods:
        cost = method.getDepth(q) + method.getFails(q)   // 深度 + 失败次数
        if cost < best_cost: best_method = method
    return best_method
}
```

**深度** = 树中最深叶子到根的距离（获取链长度）；
**失败次数** = 子树累计 `fails` 计数。
二者相加作为成本，**优先选最浅且最少失败的方法**。

### 6.4 防循环依赖（`item_goal.js:245-254`）

`containsCircularDependency()`：沿 `parent` 链向上找同名节点，命中即跳过子节点展开（防止 `stick → planks → stick` 死循环）。

### 6.5 工具升级宽松匹配（`utils.js:87-118`）

`itemSatisfied(bot, item, quantity)`：工具类物品接受**同级或更高级**满足：
- `wooden_pickaxe` 满足条件接受 `stone/iron/gold/diamond_pickaxe`
- `stone_pickaxe` 接受 `iron/gold/diamond`
- `iron_pickaxe` 接受 `gold/diamond`
- `gold_pickaxe` 接受 `diamond`

---

## 7. 任务完成判定逻辑

### 7.1 construction 完成判定（`construction_tasks.js:142-158`）

```js
check(bot) {
    const levels = this.data.levels;
    const mismatches = [];
    const matches = [];
    for (let i = 0; i < levels.length; i++) {
        const result = this.checkLevel(bot, i);
        mismatches.push(...result.mismatches);
        matches.push(...result.matches);
    }
    return { mismatches, matches };
}

checkLevel(bot, levelNum) {
    const {coordinates, placement} = this.data.levels[levelNum];
    for (let zOffset = 0; zOffset < placement.length; zOffset++) {
        for (let xOffset = 0; xOffset < row.length; xOffset++) {
            const blockName = row[xOffset];
            const x = startCoords[0] + xOffset;
            const y = startCoords[1];       // 注意：Y 不加 offset，每层 coordinates 已含 Y
            const z = startCoords[2] + zOffset;
            const actualBlockName = bot.blockAt(...).name;
            // 双 air 跳过
            if (blockName === "air" && actualBlockName === "air") continue;
            if (actualBlockName !== blockName) {
                mismatches.push({level, coordinates:[x,y,z], expected, actual});
            } else {
                matches.push({...});
            }
        }
    }
}
```

**关键点**：
- **严格字符串比较**（注意：NPC 蓝图用 `blockSatisfied` 宽松匹配，**任务蓝图用严格 `===`**）。
- 双 air 跳过（不计算入总数，否则 score 会被大量 air 拉高）。
- score = `matches / (matches + mismatches) * 100`，valid 当 `mismatches.length === 0`。

### 7.2 techtree/cooking 完成判定（`tasks.js:76-212`）

`checkItemPresence(data, agent)`：

1. **target 归一化**：
   - `string` → `{[target]: 1}`
   - `array` → 每个 item 数量 1
   - `dict` → 直接用
2. **quantity 归一化**：
   - 若 target 本身是带数量的 dict → 用 target 的值
   - 否则按 `number_of_target`：`int` → 所有同量；`dict` → 直接用；`undefined` → 默认 1
3. **遍历背包**：累计每项 count
4. **逐项比较**：缺哪项 push 到 `missingItems`，全满足则 `success=true`

**Hells Kitchen 路径**（`tasks.js:80-114`）：
```js
if (task_id.endsWith('hells_kitchen') && Array.isArray(target) && target.length === 2) {
    const agentId = agent.count_id;
    const targetForThisAgent = data.target[agentId];   // agent 0 查 target[0]，agent 1 查 target[1]
    const agentResult = checkItemForSingleAgent({...data, target: targetForThisAgent}, agent);
    // 写入文件级进度
    const progress = hellsKitchenProgressManager.updateAgentProgress(task_id, agentId, agentResult.success);
    // 两 agent 都满足才返回 success
    return {
        success: progress.agent0Complete && progress.agent1Complete,
        missingItems: agentResult.missingItems,
        agentComplete: agentResult.success
    };
}
```

**为什么需要文件级进度跟踪**：每个 agent 独立运行进程，互相看不到对方背包状态。文件作为共享黑板，两个 agent 各自校验自己的物品后写入文件，**任一 agent 调 isDone() 时读文件判断双方是否都完成**。

### 7.3 超时与失败判定（`tasks.js:365-397`）

```js
isDone() {
    let res = validator ? validator.validate() : null;
    if (res && res.valid) {
        // 清空所有 agent 背包
        for (agent of available_agents) bot.chat(`/clear ${agent}`);
        return {message: 'Task successful', score: res.score};
    }
    let elapsedTime = (Date.now() - taskStartTime) / 1000;
    
    // 30 秒后仍找不到足够 agent → 失败
    if (elapsedTime >= 30 && available_agents.length !== data.agent_count) {
        return {message: 'No other agents found', score: 0};
    }
    
    // 超时
    if (taskTimeout && elapsedTime >= taskTimeout) {
        return {message: 'Task timeout reached', score: res ? res.score : 0};
    }
    return false;  // 未完成未超时
}
```

---

## 8. Craft-Agent 移植建议

### 8.1 当前 Craft-Agent 现状

**BuildTool**（`tools_azalea.rs:1499-1567`）：
- 接受 JSON 字符串：`{"blocks":[{"x":int,"y":int,"z":int,"block":"id"}, ...]}`
- 流程：解析 JSON → 循环每个 block → `MinecraftAction::Goto` → `MinecraftAction::Place`
- **不区分 air / 不带旋转 / 不带校验 / 不带缺料反馈**

**AutoCraftTool**（`tools_azalea.rs:713-768`）：
- 接受 `(item, count)`
- 调 `do_auto_craft(bot, item, count)` → `ensure(item, count)` 递归满足原料
- 配方图在 `recipes.rs`（手写静态 `&[Recipe]`）

**当前没有**：
- 任务系统（无 `tasks/` 目录，无 TaskRunner）
- 蓝图校验（BuildTool 只放不查）
- 多 agent 协调（单 bot）
- SelfPrompter goal 注入已有（`set_goal` 工具），但无任务级 goal 持久化

### 8.2 任务系统引入方案

#### 8.2.1 目录结构建议

```
crates/craft-agent-minecraft/
├── src/
│   ├── tasks/                       ← 新增
│   │   ├── mod.rs                   — TaskRunner + Task trait
│   │   ├── task.rs                  — Task 结构 + Validator trait
│   │   ├── construction_validator.rs— Blueprint 校验
│   │   ├── item_validator.rs        — 背包物品校验
│   │   └── blueprint.rs             — Blueprint 解析 + auto_delete
│   └── ...
└── tasks/                          ← 新增（数据目录）
    ├── basic/
    │   └── single_agent.json
    ├── crafting/
    │   ├── gather_wood.json
    │   └── craft_iron_pickaxe.json
    ├── construction/
    │   ├── dirt_shelter.json
    │   └── small_wood_house.json
    └── cooking/
        └── bake_bread.json
```

#### 8.2.2 Rust 数据结构设计建议

```rust
// crates/craft-agent-minecraft/src/tasks/task.rs

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskDef {
    pub task_id: String,
    pub goal: GoalSpec,
    #[serde(default)]
    pub conversation: Option<String>,
    #[serde(default = "default_one")]
    pub agent_count: u32,
    #[serde(default)]
    pub task_type: TaskType,
    #[serde(default = "default_timeout")]
    pub timeout: u64,    // 秒
    #[serde(default)]
    pub initial_inventory: InventorySpec,
    #[serde(default)]
    pub target: Option<TargetSpec>,
    #[serde(default)]
    pub number_of_target: Option<QuantitySpec>,
    #[serde(default)]
    pub blueprint: Option<BlueprintDef>,
    #[serde(default)]
    pub restrict_to_inventory: bool,
    #[serde(default)]
    pub blocked_actions: HashMap<String, Vec<String>>,
    #[serde(default)]
    pub recipes: HashMap<String, Vec<String>>,
    #[serde(default)]
    pub difficulty: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum GoalSpec {
    Shared(String),                              // 所有 agent 共享
    PerAgent(HashMap<String, String>),           // {agent_id: goal}
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TaskType {
    Debug,
    Construction,
    Cooking,
    Techtree,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum InventorySpec {
    Empty,
    PerAgent(HashMap<String, HashMap<String, u32>>),  // {agent_id: {item: count}}
    SingleAgent(HashMap<String, u32>),                // {item: count}
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TargetSpec {
    Single(String),                          // 单物品
    Multi(Vec<String>),                      // hells_kitchen 多 agent 各一
    WithQuantities(HashMap<String, u32>),    // {item: qty}
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum QuantitySpec {
    Uniform(u32),
    PerItem(HashMap<String, u32>),
}

fn default_one() -> u32 { 1 }
fn default_timeout() -> u64 { 300 }
```

#### 8.2.3 蓝图数据结构建议

Mindcraft 有两套蓝图 schema，Craft-Agent 可统一为带 `coordinates` 的版本（更易序列化为 BuildTool 的 blocks 数组）：

```rust
// 任务蓝图（与 tasks/*.json 的 blueprint 字段对应）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlueprintDef {
    #[serde(default)]
    pub materials: HashMap<String, u32>,    // 可选，仅作统计
    pub levels: Vec<BlueprintLevel>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlueprintLevel {
    pub level: u32,
    pub coordinates: [i32; 3],               // [x, y, z] 世界坐标
    pub placement: Vec<Vec<String>>,         // [z][x] → block_id
}

// NPC 蓝图（与 npc/construction/*.json 对应，可选）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NpcBlueprint {
    pub name: String,
    pub offset: i32,
    pub blocks: Vec<Vec<Vec<String>>>,       // [y][z][x]
}
```

#### 8.2.4 TaskRunner 主循环建议

```rust
pub struct TaskRunner {
    task: TaskDef,
    start_time: Instant,
    validator: Box<dyn TaskValidator>,
}

impl TaskRunner {
    pub fn new(task: TaskDef, bot: &Client) -> Self {
        let validator: Box<dyn TaskValidator> = match task.task_type {
            TaskType::Construction => Box::new(ConstructionValidator::new(task.blueprint.clone().unwrap())),
            TaskType::Cooking | TaskType::Techtree => Box::new(ItemValidator::new(task.target.clone().unwrap(), task.number_of_target.clone())),
            TaskType::Debug => Box::new(NoOpValidator),
        };
        Self { task, start_time: Instant::now(), validator }
    }

    pub async fn init(&self, bot: &Client) -> Result<()> {
        // 1. /clear self
        // 2. 发放 initial_inventory
        // 3. construction: blueprint.auto_delete()
        // 4. 注入 goal 到 SelfPrompter（调 set_goal 工具）
        Ok(())
    }

    pub fn is_done(&self, bot: &Client) -> DoneResult {
        if let Some(r) = self.validator.validate(bot) {
            if r.valid {
                return DoneResult::Success(r.score);
            }
        }
        if self.start_time.elapsed().as_secs() >= self.task.timeout {
            return DoneResult::Timeout;
        }
        DoneResult::Running
    }
}

pub trait TaskValidator {
    fn validate(&self, bot: &Client) -> Option<ValidationResult>;
}
```

### 8.3 蓝图系统对接 BuildTool 的两种方案

#### 方案 A：在 BuildTool 内部支持 Mindcraft 蓝图格式（推荐）

修改 `BuildTool` 让它直接接受 Mindcraft 的 `levels` 格式：

```rust
// 接受两种 blueprint 格式
#[derive(Deserialize)]
#[serde(untagged)]
enum BlueprintInput {
    /// 现有格式（扁平 blocks 数组）
    Flat { blocks: Vec<FlatBlock> },
    /// Mindcraft 格式（levels）
    Mindcraft { levels: Vec<BlueprintLevel> },
}

// 在 execute 内转换：levels → 扁平 blocks，按 level 升序、每层 z 升序、x 升序展开
// 自动跳过 "air" 与 ""（与 NPC 蓝图行为一致）
// 可选：旋转 orientation
```

**优点**：LLM 调一次 build 工具就能造完整蓝图，减少 tool call 数。
**缺点**：现有 build 工具语义变化，需向后兼容（保留 Flat 格式）。

#### 方案 B：独立 `build_blueprint` 工具

新增一个高层工具 `build_blueprint`，参数为蓝图 JSON 字符串 + 可选 `base_x/y/z` + 可选 `orientation`，内部转换后调底层 BuildTool：

```rust
// crates/craft-agent-minecraft/src/tools_azalea.rs
pub struct BuildBlueprintTool { ctx: Arc<AzaleaToolCtx> }

impl GameTool for BuildBlueprintTool {
    fn name(&self) -> &str { "build_blueprint" }
    fn description(&self) -> &str {
        "按 Mindcraft 蓝图建造：JSON 格式 {\"levels\":[{\"level\":0,\"coordinates\":[x,y,z],\"placement\":[[..]]}]}。\
         自动从底层往上建造，跳过 air。可选参数 orientation (0-3) 旋转。"
    }
    fn parameters(&self) -> Value {
        json!({
            "blueprint": {"type": "string", "description": "Mindcraft 蓝图 JSON"},
            "orientation": {"type": "integer", "description": "旋转 0/1/2/3（默认 0）"}
        })
    }
    // execute: 解析 levels → 生成 (x,y,z,block) 序列 → 循环 Goto+Place
}
```

**优点**：保留现有 BuildTool 不变，新工具专门处理蓝图。
**缺点**：工具数 +1（已有 23 个）。

**建议方案 B**：保持工具单一职责，且 BuildBlueprint 可包含缺料检测返回（`missing: {block: count}`），让 LLM 知道要 auto_craft 哪些材料。

### 8.4 item_goal 与 auto_craft 对接

Craft-Agent 的 `auto_craft` 已实现递归满足原料，**与 Mindcraft 的 ItemGoal 概念基本对齐**，但差异：

| 维度 | Mindcraft ItemGoal | Craft-Agent auto_craft |
|---|---|---|
| 配方来源 | `mc.getItemCraftingRecipes`（运行时查 mineflayer 注册表） | `recipes.rs` 静态 `&[Recipe]` |
| 采集方法 | `mc.getItemBlockSources` 自动发现 | `Gather` method 仅占位，需 LLM 调 `gather` 工具 |
| 熔炼 | 自动 setSmeltable + prereq furnace+coal | `Smelt { fuel }`，auto_craft 自动造/放/开熔炉 |
| 狩猎 | `mc.getItemAnimalSource` + attackNearest | **无** |
| 工具升级宽松匹配 | itemSatisfied 接受同级或更高级 | **无** |
| 失败重试 | fails 计数 + explore | **无**（失败直接报错返回） |
| 多方法择优 | getBestMethod: depth + fails | **无**（每产物单一路径） |

**移植建议**：
1. **保持现有 auto_craft 不变**作为 LLM 工具入口。
2. **新增 TaskValidator 层**校验 `target × number_of_target` 是否满足（不依赖 ItemGoal 内部树）。
3. **可选扩展**：在 auto_craft 内增加狩猎分支（hunt 动物获取 mutton/beef/chicken 等），通过 `attack` 工具实现。
4. **可选扩展**：工具宽松匹配——校验 `iron_pickaxe` 时同时接受 `diamond_pickaxe`。
5. **任务级 target 与 auto_craft 解耦**：任务 JSON 只声明 `target` 与 `number_of_target`，由 LLM 决定调 auto_craft 还是分步 craft/smelte/gather。

### 8.5 预定义任务清单（按难度排序）

按实现优先级与难度递增排列，先实现简单的端到端可验证任务：

#### Tier 1 — 单步合成（仅校验背包，无环境依赖）

| 任务 ID | 目标 | 初始物资 | 难度 |
|---|---|---|---|
| `craft_planks` | oak_planks × 4 | oak_log × 2 | ★ |
| `craft_stick` | stick × 4 | oak_planks × 2 | ★ |
| `craft_crafting_table` | crafting_table × 1 | oak_planks × 4 | ★ |
| `craft_torch` | torch × 4 | coal × 1 + stick × 1 | ★ |

#### Tier 2 — 多步合成链（需 auto_craft）

| 任务 ID | 目标 | 初始物资 | 难度 |
|---|---|---|---|
| `craft_wooden_pickaxe` | wooden_pickaxe × 1 | (空) | ★★ |
| `craft_stone_pickaxe` | stone_pickaxe × 1 | wooden_pickaxe × 1 | ★★ |
| `craft_iron_pickaxe` | iron_pickaxe × 1 | stone_pickaxe × 1 + iron_ingot × 3 | ★★ |
| `craft_chest` | chest × 1 | (空，需 auto_craft 满足 8 oak_planks) | ★★ |

#### Tier 3 — 熔炼 + 合成混合

| 任务 ID | 目标 | 初始物资 | 难度 |
|---|---|---|---|
| `smelt_iron_ingot` | iron_ingot × 1 | furnace × 1 + raw_iron × 1 + coal × 1 | ★★★ |
| `smelt_glass` | glass × 6 | furnace × 1 + sand × 6 + coal × 1 | ★★★ |
| `craft_furnace_and_smelt` | iron_ingot × 1 | cobblestone × 8 + raw_iron × 1 + coal × 1（auto_craft 自动造炉） | ★★★ |

#### Tier 4 — 建造任务（需 BuildTool/BuildBlueprintTool + 蓝图校验）

| 任务 ID | 蓝图 | 尺寸 | 难度 |
|---|---|---|---|
| `build_dirt_pillar` | 1×1×3 自定义 | 微 | ★ |
| `build_dirt_shelter` | 移植 NPC dirt_shelter | 5×6×4 | ★★★ |
| `build_small_wood_house` | 移植 NPC small_wood_house | 5×7×4 | ★★★★ |
| `build_small_stone_house` | 移植 NPC small_stone_house | 5×7×4 | ★★★★ |

#### Tier 5 — 综合（采集 + 合成 + 建造）

| 任务 ID | 目标 | 难度 |
|---|---|---|
| `gather_and_build_wood_house` | 从零开始：伐木→合成木板/工具→建 small_wood_house | ★★★★★ |
| `survive_first_day` | 建造 dirt_shelter + 度过夜晚（含床） | ★★★★★ |

#### Tier 6 — 多 agent 协作（Craft-Agent 暂不支持，预留）

| 任务 ID | 目标 | 难度 |
|---|---|---|
| `collab_stone_pickaxe` | 2 agent 分物资合成 stone_pickaxe | ★★★★ |
| `hells_kitchen_bread` | 2 agent 互递配方做面包 | ★★★★★ |

### 8.6 移植路线图

**Phase 1（最小可用，1-2 周）**：
1. 新建 `crates/craft-agent-minecraft/src/tasks/` 目录
2. 定义 `TaskDef` / `BlueprintDef` 数据结构（serde 序列化）
3. 实现 `ItemValidator`（校验背包物品）+ `ConstructionValidator`（校验世界方块）
4. 新增 `BuildBlueprintTool`（方案 B，接受 Mindcraft levels 格式）
5. 写 5-10 个 Tier 1-2 任务 JSON
6. 在 `craft-agent-viewer` 入口加 `--task <path>` 参数加载任务

**Phase 2（蓝图校验 + 任务循环，2-3 周）**：
1. 实现 `TaskRunner.init()` 与 `TaskRunner.is_done()`
2. 集成到 agent 主循环：每轮 `is_done()` 检查，完成时停止
3. 移植 4 个 NPC 蓝图到 `tasks/construction/`
4. 实现 Tier 3-4 任务

**Phase 3（auto_craft 增强，可选）**：
1. auto_craft 增加狩猎分支
2. ItemValidator 增加工具宽松匹配
3. auto_craft 失败重试机制

**Phase 4（多 agent，预留）**：
- Craft-Agent 当前架构为单 bot，多 agent 需要先支持多 bot 实例
- 暂不实现，仅保留任务 JSON schema 兼容性

### 8.7 蓝图格式转换器（一次性工具）

由于 Mindcraft 有两套蓝图 schema（任务蓝图 `levels` + `coordinates` vs NPC 蓝图 `blocks` + `offset`），可以写一个一次性转换函数：

```rust
/// 把 NPC 蓝图（blocks 3D 数组 + offset）转为任务蓝图（levels + coordinates）
pub fn npc_to_task_blueprint(npc: &NpcBlueprint, base_pos: [i32; 3]) -> BlueprintDef {
    let mut levels = Vec::new();
    for (y, layer) in npc.blocks.iter().enumerate() {
        let coordinates = [
            base_pos[0],
            base_pos[1] + y as i32 + npc.offset,
            base_pos[2],
        ];
        levels.push(BlueprintLevel {
            level: y as u32,
            coordinates,
            placement: layer.clone(),
        });
    }
    BlueprintDef { materials: HashMap::new(), levels }
}
```

这样移植 NPC 蓝图时只需调用一次转换，得到统一的 `levels` 格式供 `BuildBlueprintTool` 使用。

### 8.8 关键差异提醒

移植时需注意以下 Mindcraft 与 Craft-Agent 的语义差异：

1. **Y 坐标语义**：
   - Mindcraft 任务蓝图：每层 `coordinates[1]` 是该层实际世界 Y（已含递增）。
   - Mindcraft NPC 蓝图：`offset` 是相对 bot 当前 Y 的偏移，blocks[y] 实际世界 Y = `position.y + y + offset`。
   - Craft-Agent BuildTool：每个 block 的 `y` 是绝对世界坐标。
   - **移植 NPC 蓝图必须先选定 base_pos，按上面转换器公式计算每层 Y**。

2. **空字符串 vs air**：
   - NPC 蓝图：`""` = 跳过（不操作），`"air"` = 主动 setblock air。
   - 任务蓝图校验：双 air 跳过不计入总数，其他不匹配算 mismatch。
   - Craft-Agent BuildTool：当前无 air 处理，需新增分支。

3. **方块匹配严格性**：
   - 任务蓝图校验用严格 `===` 比较（`oak_planks` ≠ `birch_planks`）。
   - NPC 蓝图 build_goal 用 `blockSatisfied` 宽松匹配（任意木种都满足 `planks`）。
   - **移植 BuildBlueprintTool 时建议采用宽松匹配**（避免 LLM 给了 oak_planks 但蓝图写的是抽象 planks 导致失败）。

4. **任务蓝图不带 offset**：每层 `coordinates` 已含完整世界坐标，不需要额外偏移。

5. **Hells Kitchen 文件级进度**：Craft-Agent 单进程多 bot 时可直接用内存共享，无需文件。但若未来跨进程，可参考 Mindcraft 的 `hells_kitchen_progress.json` 模式。

---

## 附录 A：核心源码文件路径速查

| 文件 | 路径 | 作用 |
|---|---|---|
| 任务主逻辑 | `d:\Craft-Agent\reference\mindcraft\src\agent\tasks\tasks.js` | Task 类、isDone、initBotTask、checkItemPresence |
| 建造任务 | `d:\Craft-Agent\reference\mindcraft\src\agent\tasks\construction_tasks.js` | Blueprint 类、ConstructionTaskValidator、proceduralGeneration、worldToBlueprint、blueprintToTask |
| 烹饪任务 | `d:\Craft-Agent\reference\mindcraft\src\agent\tasks\cooking_tasks.js` | CookingTaskInitiator（建农场+动物+房子） |
| NPC 控制器 | `d:\Craft-Agent\reference\mindcraft\src\agent\npc\controller.js` | NPCContoller 主循环、goals 调度、building 检测 |
| 建造目标 | `d:\Craft-Agent\reference\mindcraft\src\agent\npc\build_goal.js` | BuildGoal.executeNext（蓝图→place 动作序列） |
| 物品目标 | `d:\Craft-Agent\reference\mindcraft\src\agent\npc\item_goal.js` | ItemGoal / ItemWrapper / ItemNode（方法树 + DFS 择优） |
| NPC 数据 | `d:\Craft-Agent\reference\mindcraft\src\agent\npc\data.js` | NPCData 持久化（goals/built/home） |
| 工具函数 | `d:\Craft-Agent\reference\mindcraft\src\agent\npc\utils.js` | blockSatisfied、itemSatisfied、rotateXZ、getTypeOfGeneric |
| 蓝图 dirt_shelter | `d:\Craft-Agent\reference\mindcraft\src\agent\npc\construction\dirt_shelter.json` | 5×6×4 dirt 避难所 |
| 蓝图 small_wood_house | `d:\Craft-Agent\reference\mindcraft\src\agent\npc\construction\small_wood_house.json` | 5×7×4 木屋 |
| 蓝图 small_stone_house | `d:\Craft-Agent\reference\mindcraft\src\agent\npc\construction\small_stone_house.json` | 5×7×4 石屋 |
| 蓝图 large_house | `d:\Craft-Agent\reference\mindcraft\src\agent\npc\construction\large_house.json` | 11×14×13 多层别墅 |
| 任务示例 | `d:\Craft-Agent\reference\mindcraft\tasks\example_tasks.json` | 21 个混合示例任务（debug/construction/techtree/cooking） |
| 单 agent 基础 | `d:\Craft-Agent\reference\mindcraft\tasks\basic\single_agent.json` | gather_oak_logs 简单示例 |
| 合成训练 | `d:\Craft-Agent\reference\mindcraft\tasks\single_agent\crafting_train.json` | 20 个 techtree 合成训练任务 |
| 金字塔任务 | `d:\Craft-Agent\reference\mindcraft\tasks\construction_tasks\custom\pyramid.json` | 5 层 10×10 多材质金字塔 |
| Hells Kitchen | `d:\Craft-Agent\reference\mindcraft\tasks\cooking_tasks\require_collab_test_2_items\2_agent_hells_kitchen.json` | 双 agent 互递配方示例 |
| README | `d:\Craft-Agent\reference\mindcraft\tasks\construction_tasks\README_ConstructionTasks.md` | 建造任务生成说明 |

## 附录 B：Craft-Agent 对接文件

| 文件 | 路径 | 作用 |
|---|---|---|
| BuildTool | `d:\Craft-Agent\crates\craft-agent-minecraft\src\tools_azalea.rs:1499-1567` | 现有 build 工具（扁平 blocks 数组） |
| AutoCraftTool | `d:\Craft-Agent\crates\craft-agent-minecraft\src\tools_azalea.rs:713-768` | 现有 auto_craft 工具入口 |
| 工具注册 | `d:\Craft-Agent\crates\craft-agent-minecraft\src\tools_azalea.rs:1570-1599` | create_mc_azalea_tools 工厂 |
| auto_craft 实现 | `d:\Craft-Agent\crates\craft-agent-minecraft\src\azalea\auto_craft.rs:366-377` | do_auto_craft → ensure 递归 |
| 配方图 | `d:\Craft-Agent\crates\craft-agent-minecraft\src\azalea\recipes.rs` | 静态 RECIPES 数组 + Recipe 结构 |

## 附录 C：任务 JSON 字段速查（按 type 区分必填）

```
debug:           goal, type
techtree:        goal, type, target, number_of_target, agent_count, initial_inventory, timeout
cooking:         goal, type, target, agent_count, timeout, (recipes, conversation, difficulty)
construction:    goal, type, blueprint, agent_count, initial_inventory, timeout, (conversation)
hells_kitchen:   goal(object), type=cooking, target(array len=2), agent_count=2, task_id 后缀 hells_kitchen, recipes, conversation
multi-agent:     上述 + agent_count>=2 + conversation
```
