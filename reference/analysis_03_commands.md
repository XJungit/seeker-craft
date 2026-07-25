# Mindcraft 命令系统分析报告

> 源码路径：`d:\Craft-Agent\reference\mindcraft\`
> 分析对象：`src/agent/commands/{actions,queries,index}.js`、`src/agent/coder.js`、`src/agent/conversation.js`、`bots/{execTemplate,lintTemplate}.js`
> 对照对象：Craft-Agent `crates/craft-agent-minecraft/src/tools_azalea.rs`（23 个 LLM 工具）

---

## 1. Mindcraft 命令系统总览

### 1.1 架构

```
                  ┌──────────────────────────┐
                  │      LLM (GPT/Claude)    │
                  │  生成自然语言+!cmd(...)    │
                  └────────────┬─────────────┘
                               │ 文本响应（非结构化 tool_call）
                               ▼
            ┌────────────────────────────────────────┐
            │  agent.js::handleMessage / requestAction │
            │  ① containsCommand()  正则探测          │
            │  ② truncCommandMessage()  截断尾随内容  │
            └────────────┬───────────────────────────┘
                         ▼
            ┌────────────────────────────────────────┐
            │  commands/index.js::executeCommand      │
            │  ① parseCommandMessage()                │
            │     - commandRegex 匹配 !name(args)     │
            │     - argRegex 提取参数（数字/bool/串） │
            │     - 按声明 params 做 type/domain 校验 │
            │  ② commandMap[name].perform(agent,...)  │
            └────────────┬───────────────────────────┘
                         │
        ┌────────────────┴──────────────────┐
        ▼                                     ▼
┌──────────────────┐               ┌──────────────────┐
│  Action 命令     │               │  Query 命令      │
│  runAsAction()   │               │  直接同步返回    │
│  包裹到          │               │  pad(res) 字符串 │
│  ActionManager   │               └──────────────────┘
│  .runAction()    │
│  - 超时（10min） │
│  - 中断          │
│  - resume        │
└────────┬─────────┘
         │ await actionFn()
         ▼
┌────────────────────────────────────────┐
│  library/skills.js  (38 个原子技能)    │
│  goToPlayer / collectBlock / craft...  │
└────────┬───────────────────────────────┘
         │  mineflayer API
         ▼
┌────────────────────────────────────────┐
│       mineflayer Bot (JS)              │
│       pathfinder / inventory / dig ... │
└────────────────────────────────────────┘
```

### 1.2 核心特点

- **协议是文本**：LLM 输出形如 `!collectBlocks("oak_log", 5)` 的字符串，由正则解析。**不是 OpenAI tool_call JSON**。
- **每轮只执行一条命令**：`truncCommandMessage()` 把命令后的文本全部丢弃；`getCommandDocs()` 显式声明 "Only use one command in each response"。
- **Action / Query 二分**：Query 同步返回字符串，不动世界；Action 走 `ActionManager`，带超时/中断/resume。
- **命令是声明式 schema**：每条命令是 `{name, description, params, perform}`；`params` 是有序对象，每个参数 `{type, description, domain?}`。`index.js` 把它折成 `getCommandDocs()` 文本拼进 prompt。
- **安全沙箱**：`!newAction` 走 `coder.js`，用 SES `Compartment` 隔离 LLM 生成的 JS 代码，只注入 `skills/world/Vec3/log`。
- **unblockable 命令**：`!stop / !stats / !inventory / !goal` 不可被 `blacklistCommands` 屏蔽。
- **快循环保护**：`ActionManager` 检测到 20ms 内连续 3+ 次动作或 5+ 次会 `cleanKill`，防 LLM 死循环。
- **resume 动作**：`!followPlayer` 用 `resume=true`，每轮 idle 后自动续跑，直到被打断或新命令。

### 1.3 命令解析协议（`index.js`）

调用语法（来自 `getCommandDocs()`）：

```
!commandName
!commandName("arg1", 1.2, true)
```

- 字符串必须用双引号；不能用代码块
- 数字：`-?\d+(\.\d+)?`
- 布尔：`true|false`（解析时还接受 `t/f/0/1/on/off`）
- 每次响应只能一条命令，多余内容被 `truncCommandMessage` 截断

正则（原文）：

```js
const commandRegex = /!(\w+)(?:\(((?:-?\d+(?:\.\d+)?|true|false|"[^"]*")(?:\s*,\s*(?:-?\d+(?:\.\d+)?|true|false|"[^"]*"))*)\))?/
const argRegex = /-?\d+(?:\.\d+)?|true|false|"[^"]*"/g;
```

参数类型校验（`parseCommandMessage`）：
- `int` / `float`：转 Number；NaN 报错 `Param 'X' must be of type Y.`
- `boolean`：`parseBoolean` 不区分大小写
- `BlockName`：通过 `getBlockId(arg)` 校验是否合法方块；自动给 `oak_plank` → `oak_planks`（同样 `seed` → `seeds`）
- `ItemName`：通过 `getItemId(arg)` 校验
- `BlockOrItemName`：二者任一合法即可
- 数值 `domain`：`[lower, upper, endpointType]`，默认 `[)`；区间外报错 `Param 'X' must be an element of [a, b).`
- 参数数量必须严格匹配（无默认值，无 optional 除了 `!getCraftingPlan` 的 `quantity`）

错误反馈给 LLM：解析失败/参数非法/类型错误都返回**纯字符串错误**，原样进入对话历史，下一轮 LLM 看到错误后自行重试。无重试次数上限（除 `!newAction` 限 5 次）。

### 1.4 Action 执行模型（`action_manager.js`）

```js
async runAction(actionLabel, actionFn, { timeout, resume = false } = {})
```

- `timeout`：**分钟**单位；`-1` 或 `0` 表示无超时（默认 10 分钟）
- 串行执行：新 action 进来时 `await this.stop()` 中断当前；`stop()` 10 秒内不退出则 `cleanKill` 整个进程
- `bot.interrupt_code` 标志位让生成的 JS 在每行 `;` 后自动检查并退出
- 返回 `{success, message, interrupted, timedout}`；`runAsAction` 把 `message`（`getBotOutputSummary` 截断到 500 字符）回给 LLM
- `resume=true`：动作结束后挂起 `resume_func`，bot idle 时自动续跑（用于 `!followPlayer`）

输出截断（`getBotOutputSummary`）：

```
若 output.length > 500:
  "Action output is very long (N chars) and has been shortened.
   First outputs: <前 250 字>
   ...skipping many lines.
   Final outputs: <后 250 字>"
否则:
  "Action output:\n<output>"
```

---

## 2. 所有 Action 命令清单

共 **39 个** action 命令。表格列：name / 参数 / 对应 skill / 超时 / 失败处理。

| # | name | params (类型 / domain) | 对应 skill / 调用 | 超时 | 失败/备注 |
|---|------|------------------------|-------------------|------|-----------|
| 1 | `!newAction` | `prompt: string` | `coder.generateCode(history)` | `settings.code_timeout_mins` | 受 `allow_insecure_coding` 开关控制；5 次重试 |
| 2 | `!stop` | — | `agent.actions.stop()` | — | 强制中断所有动作，触发 `bot.emit('idle')` |
| 3 | `!stfu` | — | `agent.shutUp()` | — | 停止聊天/self-prompting，但保留当前动作 |
| 4 | `!restart` | — | `agent.cleanKill()` | — | 重启进程，无返回 |
| 5 | `!clearChat` | — | `history.clear()` | — | 返回 "chat history was cleared" |
| 6 | `!goToPlayer` | `player_name: string`; `closeness: float [0,∞]` | `skills.goToPlayer` | 默认 10min | — |
| 7 | `!followPlayer` | `player_name: string`; `follow_dist: float [0,∞]` | `skills.followPlayer` | **resume=true** | 持续跟随直到被打断 |
| 8 | `!goToCoordinates` | `x: float`; `y: float [-64,320]`; `z: float`; `closeness: float [0,∞]` | `skills.goToPosition` | 默认 | — |
| 9 | `!searchForBlock` | `type: BlockName`; `search_range: float [10,512]` | `skills.goToNearestBlock(bot, type, 4, range)` | 默认 | range<32 自动抬到 32 |
| 10 | `!searchForEntity` | `type: string`; `search_range: float [32,512]` | `skills.goToNearestEntity` | 默认 | — |
| 11 | `!moveAway` | `distance: float [0,∞]` | `skills.moveAway` | 默认 | — |
| 12 | `!rememberHere` | `name: string` | `memory_bank.rememberPlace` | — | 不走 runAction，直接返回 `"Location saved as X."` |
| 13 | `!goToRememberedPlace` | `name: string` | `memory_bank.recallPlace` → `skills.goToPosition` | 默认 | 名字不存在时 `skills.log` 提示并 return |
| 14 | `!givePlayer` | `player_name: string`; `item_name: ItemName`; `num: int [1,MAX]` | `skills.giveToPlayer` | 默认 | — |
| 15 | `!consume` | `item_name: ItemName` | `skills.consume` | 默认 | 吃/喝 |
| 16 | `!equip` | `item_name: ItemName` | `skills.equip` | 默认 | — |
| 17 | `!putInChest` | `item_name: ItemName`; `num: int [1,MAX]` | `skills.putInChest` | 默认 | — |
| 18 | `!takeFromChest` | `item_name: ItemName`; `num: int [1,MAX]` | `skills.takeFromChest` | 默认 | — |
| 19 | `!viewChest` | — | `skills.viewChest` | 默认 | 仅查看最近箱子内容 |
| 20 | `!discard` | `item_name: ItemName`; `num: int [1,MAX]` | `moveAway(5)` → `discard` → `goToPosition(原位)` | 默认 | 丢弃前先走开 5 格避免捡回 |
| 21 | `!collectBlocks` | `type: BlockName`; `num: int [1,MAX]` | `skills.collectBlock` | **10 分钟** | 寻路+挖掘+拾取 |
| 22 | `!craftRecipe` | `recipe_name: ItemName`; `num: int [1,MAX]` | `skills.craftRecipe` | 默认 | num 是配方次数而非产物数 |
| 23 | `!smeltItem` | `item_name: ItemName`; `num: int [1,MAX]` | `skills.smeltItem` | 默认 | 成功后 500ms 触发 `cleanKill` 重启以刷新背包 |
| 24 | `!clearFurnace` | — | `skills.clearNearestFurnace` | 默认 | 取出最近熔炉所有物品 |
| 25 | `!placeHere` | `type: BlockOrItemName` | `skills.placeBlock(bot, type, pos.x, pos.y, pos.z)` | 默认 | 用 bot 当前坐标；说明明确不用于建结构 |
| 26 | `!attack` | `type: string` | `skills.attackNearest(bot, type, true)` | 默认 | 持续攻击直到目标死亡 |
| 27 | `!attackPlayer` | `player_name: string` | `skills.attackEntity(bot, player, true)` | 默认 | 找不到玩家时 `skills.log` + return false |
| 28 | `!goToBed` | — | `skills.goToBed` | 默认 | — |
| 29 | `!stay` | `type: int [-1,MAX]`（秒） | `skills.stay` | 默认 | -1 = 永远；暂停所有 modes |
| 30 | `!setMode` | `mode_name: string`; `on: boolean` | `bot.modes.setOn` | — | mode 不存在时返回 mode 列表 docs |
| 31 | `!goal` | `selfPrompt: string` | `self_prompter.start(prompt)` 或 `setPromptPaused` | — | 启动 self-prompting 持续目标循环 |
| 32 | `!endGoal` | — | `self_prompter.stop()` | — | 返回 "Self-prompting stopped." |
| 33 | `!showVillagerTrades` | `id: int` | `skills.showVillagerTrades` | 默认 | — |
| 34 | `!tradeWithVillager` | `id: int`; `index: int [1,MAX]`; `count: int [1,MAX]` | `skills.tradeWithVillager` | 默认 | index 是 1-indexed |
| 35 | `!startConversation` | `player_name: string`; `message: string` | `convoManager.startConversation` | — | 仅对其他 bot；已在对话则不重发 |
| 36 | `!endConversation` | `player_name: string` | `convoManager.endConversation` | — | 未在对话时返回提示 |
| 37 | `!lookAtPlayer` | `player_name: string`; `direction: string ("at"/"with")` | `vision_interpreter.lookAtPlayer` | 默认 | 用 VLM，azalea 路线不适用 |
| 38 | `!lookAtPosition` | `x: int`; `y: int`; `z: int` | `vision_interpreter.lookAtPosition` | 默认 | 用 VLM |
| 39 | `!digDown` | `distance: int [1,MAX]` | `skills.digDown` | 默认 | 遇岩浆/水/≥4 格落差自动停 |
| 40 | `!goToSurface` | — | `skills.goToSurface` | 默认 | 走到头顶最高方块 |
| 41 | `!useOn` | `tool_name: string`; `target: string` | `skills.useToolOn` | 默认 | 对最近目标使用工具（"hand"=空手，"nothing"=无目标） |

> 注：实际共 41 行（含 `!stop`/`!stfu`/`!restart`/`!clearChat` 等元命令）。原文 actions.js 数组就是这么多条。

---

## 3. 所有 Query 命令清单

共 **14 个** query 命令，全部同步返回字符串（用 `pad()` 包裹前后换行），不动世界。

| # | name | params | 返回内容 | 备注 |
|---|------|--------|----------|------|
| 1 | `!stats` | — | `STATS`：位置(2 位小数) / 游戏模式 / 生命 / 饥饿 / 生物群系 / 天气 / 时段 / 当前动作 / 附近玩家(人类+bot) / modes 简文档 | 还会列出 `agent.actions.currentActionLabel` 或 "Idle" |
| 2 | `!inventory` | — | `INVENTORY`：物品名:数量；`WEARING`：头/胸/腿/脚；创造模式提示无限物品 | 空背包显示 "Nothing" |
| 3 | `!nearbyBlocks` | — | `NEARBY_BLOCKS`：去重方块列表；水/岩浆标注 source/flowing；头顶第一实体方块 | — |
| 4 | `!craftable` | — | `CRAFTABLE_ITEMS`：当前背包可合成物品列表 | 用 `world.getCraftableItems` |
| 5 | `!entities` | — | `NEARBY_ENTITIES`：人类玩家 / bot 玩家 / 实体计数；村民列出 `(id:profession)` 与 baby IDs | 区分成年村民(可交易)与幼年村民(不可交易) |
| 6 | `!modes` | — | `bot.modes.getDocs()` | 所有 mode 文档 + 开关状态 |
| 7 | `!savedPlaces` | — | `"Saved place names: " + memory_bank.getKeys()` | — |
| 8 | `!checkBlueprintLevel` | `levelNum: int [0,MAX]` | `checkLevelBlueprint(agent, levelNum)` 输出 | 建筑任务用 |
| 9 | `!checkBlueprint` | — | `checkBlueprint(agent)` 输出 | 当前蓝图未放置方块 |
| 10 | `!getBlueprint` | — | `agent.task.blueprint.explain()` | 完整蓝图说明 |
| 11 | `!getBlueprintLevel` | `levelNum: int [0,MAX]` | `blueprint.explainLevel(levelNum)` | 指定层蓝图 |
| 12 | `!getCraftingPlan` | `targetItem: string`; `quantity: int [1,∞) optional default=1` | `mc.getDetailedCraftingPlan` 输出 + 缺料分析 | 唯一带 `optional`+`default` 的命令；已有物品会先减去并提示 |
| 13 | `!searchWiki` | `query: string` | `https://minecraft.wiki/w/{query}` 抓取 HTML，去 navbox 表后取 `mw-parser-output` 文本 | 404 提示调整关键词；异常返回错误串 |
| 14 | `!help` | — | `getCommandDocs(agent)`：所有可用命令文档（自动跳过 `agent.blocked_actions` 中的命令） | — |

---

## 4. 命令 schema 原文摘录

以下 7 个关键命令的 `params` 字段为源码原文（含 domain 区间和类型别名）。

### 4.1 `!goToCoordinates`（标准坐标移动）

```js
{
    name: '!goToCoordinates',
    description: 'Go to the given x, y, z location.',
    params: {
        'x': {type: 'float', description: 'The x coordinate.', domain: [-Infinity, Infinity]},
        'y': {type: 'float', description: 'The y coordinate.', domain: [-64, 320]},
        'z': {type: 'float', description: 'The z coordinate.', domain: [-Infinity, Infinity]},
        'closeness': {type: 'float', description: 'How close to get to the location.', domain: [0, Infinity]}
    },
    perform: runAsAction(async (agent, x, y, z, closeness) => {
        await skills.goToPosition(agent.bot, x, y, z, closeness);
    })
}
```

### 4.2 `!searchForBlock`（BlockName 类型 + range domain）

```js
{
    name: '!searchForBlock',
    description: 'Find and go to the nearest block of a given type in a given range.',
    params: {
        'type': { type: 'BlockName', description: 'The block type to go to.' },
        'search_range': { type: 'float', description: 'The range to search for the block. Minimum 32.', domain: [10, 512] }
    },
    perform: runAsAction(async (agent, block_type, range) => {
        if (range < 32) {
            skills.log(agent.bot, `Minimum search range is 32.`);
            range = 32;
        }
        await skills.goToNearestBlock(agent.bot, block_type, 4, range);
    })
}
```

### 4.3 `!collectBlocks`（自定义 10 分钟超时）

```js
{
    name: '!collectBlocks',
    description: 'Collect the nearest blocks of a given type.',
    params: {
        'type': { type: 'BlockName', description: 'The block type to collect.' },
        'num': { type: 'int', description: 'The number of blocks to collect.', domain: [1, Number.MAX_SAFE_INTEGER] }
    },
    perform: runAsAction(async (agent, type, num) => {
        await skills.collectBlock(agent.bot, type, num);
    }, false, 10) // 10 minute timeout
}
```

### 4.4 `!tradeWithVillager`（多 int 参数 + 1-indexed）

```js
{
    name: '!tradeWithVillager',
    description: 'Trade with a specified villager.',
    params: {
        'id': { type: 'int', description: 'The id number of the villager that you want to trade with.' },
        'index': { type: 'int', description: 'The index of the trade you want executed (1-indexed).', domain: [1, Number.MAX_SAFE_INTEGER] },
        'count': { type: 'int', description: 'How many times that trade should be executed.', domain: [1, Number.MAX_SAFE_INTEGER] },
    },
    perform: runAsAction(async (agent, id, index, count) => {
        await skills.tradeWithVillager(agent.bot, id, index, count);
    })
}
```

### 4.5 `!setMode`（boolean 参数）

```js
{
    name: '!setMode',
    description: 'Set a mode to on or off. A mode is an automatic behavior that constantly checks and responds to the environment.',
    params: {
        'mode_name': { type: 'string', description: 'The name of the mode to enable.' },
        'on': { type: 'boolean', description: 'Whether to enable or disable the mode.' }
    },
    perform: async function (agent, mode_name, on) {
        const modes = agent.bot.modes;
        if (!modes.exists(mode_name))
            return `Mode ${mode_name} does not exist.` + modes.getDocs();
        if (modes.isOn(mode_name) === on)
            return `Mode ${mode_name} is already ${on ? 'on' : 'off'}.`;
        modes.setOn(mode_name, on);
        return `Mode ${mode_name} is now ${on ? 'on' : 'off'}.`;
    }
}
```

### 4.6 `!getCraftingPlan`（唯一带 optional + default + 半开区间）

```js
{
    name: '!getCraftingPlan',
    description: "Provides a comprehensive crafting plan for a specified item. This includes a breakdown of required ingredients, the exact quantities needed, and an analysis of missing ingredients or extra items needed based on the bot's current inventory.",
    params: {
        targetItem: { 
            type: 'string', 
            description: 'The item that we are trying to craft' 
        },
        quantity: { 
            type: 'int',
            description: 'The quantity of the item that we are trying to craft',
            optional: true,
            domain: [1, Infinity, '[)'], // Quantity must be at least 1,
            default: 1
        }
    },
    perform: function (agent, targetItem, quantity = 1) {
        /* ...getDetailedCraftingPlan... */
    },
}
```

### 4.7 `!newAction`（动态代码生成入口）

```js
{
    name: '!newAction',
    description: 'Perform new and unknown custom behaviors that are not available as a command.', 
    params: {
        'prompt': { type: 'string', description: 'A natural language prompt to guide code generation. Make a detailed step-by-step plan.' }
    },
    perform: async function(agent, prompt) {
        if (!settings.allow_insecure_coding) { 
            agent.openChat('newAction is disabled. Enable with allow_insecure_coding=true in settings.js');
            return "newAction not allowed! Code writing is disabled in settings. Notify the user.";
        }
        let result = "";
        const actionFn = async () => {
            try {
                result = await agent.coder.generateCode(agent.history);
            } catch (e) {
                result = 'Error generating code: ' + e.toString();
            }
        };
        await agent.actions.runAction('action:newAction', actionFn, {timeout: settings.code_timeout_mins});
        return result;
    }
}
```

### 4.8 `getCommandDocs()` 输出格式（LLM 看到的文档）

```
*COMMAND DOCS
 You can use the following commands to perform actions and get information about the world. 
    Use the commands with the syntax: !commandName or !commandName("arg1", 1.2, ...) if the command takes arguments.
    Do not use codeblocks. Use double quotes for strings. Only use one command in each response, trailing commands and comments will be ignored.

!goToCoordinates: Go to the given x, y, z location.
Params:
x: (number) The x coordinate.
y: (number) The y coordinate.
z: (number) The z coordinate.
closeness: (number) How close to get to the location.
...
*
```

类型映射（`typeTranslations`）：
- `float` / `int` → `number`
- `BlockName` / `ItemName` / `BlockOrItemName` → `string`
- `boolean` → `bool`

---

## 5. coder.js 代码执行机制详解

### 5.1 总体流程

```
!newAction(prompt)
    │
    ▼
agent.actions.runAction('action:newAction', actionFn, {timeout: settings.code_timeout_mins})
    │
    ▼
Coder.generateCode(history)
    │
    ├── 1. lockdown()  ── SES 全局锁定（仅一次）
    ├── 2. messages = history.getHistory() + 系统提示 "Code generation started..."
    ├── 3. for i in 0..MAX_ATTEMPTS(=5):
    │       ├── 若 bot.interrupt_code → return null
    │       ├── prompter.promptCoding(messages_copy)  → LLM 响应 res
    │       ├── 若无 ```code block```:
    │       │   ├── 若 res 含 !newAction → 截断后继续（轮次消耗）
    │       │   ├── no_code_failures >= 3 → return "Action failed, agent would not write code."
    │       │   └── 否则注入 "Error: no code provided..." 重试
    │       ├── code = res.substring(indexOf('```')+3, lastIndexOf('```'))
    │       ├── _stageCode(code):
    │       │   ├── _sanitizeCode：trim，去掉开头的 `Javascript/javascript/js`
    │       │   ├── replaceAll('console.log(', 'log(bot,')
    │       │   ├── replaceAll('log("', 'log(bot,"')
    │       │   ├── **每行后注入中断检查**：
    │       │   │   `code.replaceAll(';\n', '; if(bot.interrupt_code) {log(bot, "Code interrupted.");return;}\n')`
    │       │   ├── 缩进 4 空格套入 execTemplate.js：
    │       │   │   ```js
    │       │   │   (async (bot) => {
    │       │   │       /* CODE HERE */
    │       │   │       log(bot, 'Code finished.');
    │       │   │   })
    │       │   │   ```
    │       │   ├── 同时套入 lintTemplate.js（带 import skills/world/Vec3）做 lint
    │       │   ├── 写文件到 /bots/{agent.name}/action-code/{N}.js（保留历史）
    │       │   └── makeCompartment({skills, log, world, Vec3}).evaluate(src) → mainFn
    │       ├── _lintCode(src_lint_copy):
    │       │   ├── 用 skillRegex /((?:skills|world)\.(.*?))\(/g 提取所有调用
    │       │   ├── 与 skill_library.getAllSkillDocs() 对比，未知函数报 "These functions do not exist"
    │       │   └── ESLint.lintText：列出每个 error 的 message/line/column/Related Code Line
    │       ├── lint 失败 → 注入 "Error: Code lint error:..." 重试
    │       ├── executionModule.main(bot) 执行
    │       │   ├── 成功 → return "Agent wrote this code: \n```<sanitized>\nCode Output:\n<output>"
    │       │   └── 异常 → 注入 assistant res + system "Code Output:...\nCODE EXECUTION THREW ERROR: ...\nPlease try again:"，重试
    │       └── bot.interrupt_code → return null
    └── 4. 全部失败 → "Code generation failed after 5 attempts."
```

### 5.2 沙箱（SES Compartment）

`lockdown.js` 调用 `ses` 包的 `lockdown()`，全局冻结原型链，然后 `makeCompartment`：

```js
export const makeCompartment = (endowments = {}) => {
  return new Compartment({
    Math,
    Date,
    ...endowments   // skills, log, world, Vec3
  });
}
```

**LLM 生成的代码能访问的 API**（白名单）：
- `skills.*`（38 个函数：goToPlayer / collectBlock / craftRecipe / smeltItem / attackNearest / placeBlock / discard / equip / consume / giveToPlayer / putInChest / takeFromChest / viewChest / goToPosition / goToNearestBlock / goToNearestEntity / moveAway / stay / goToBed / tillAndSow / activateNearestBlock / showVillagerTrades / tradeWithVillager / digDown / goToSurface / useToolOn / useDoor / pickupNearbyItems / breakBlockAt / wait / log / ...）
- `world.*`（getInventoryCounts / getNearestBlocks / getSurroundingBlocks / getFirstBlockAboveHead / getNearbyPlayerNames / getNearbyEntities / getBiomeName / getVillagerProfession / getCraftableItems / ...）
- `Vec3`（vec3 库）
- `log(bot, msg)`（= `skills.log`）
- `Math` / `Date`（untamed）
- JS 内置（Array/Object/Promise/...）

**不能访问**：`process` / `require` / `fs` / `fetch` / `globalThis` 原始引用 / 任何未注入的模块。

**安全限制**：
- `evalTaming: 'unsafeEval'`：允许 mineflayer 依赖 protodef 用 eval，但 compartment 内仍受限
- `consoleTaming: 'unsafe'` / `errorTaming: 'unsafe'`：保留原 console 与错误堆栈便于调试
- 代码每行 `;` 后自动插入 `if(bot.interrupt_code) {log(bot, "Code interrupted.");return;}` —— 但注释说"may cause problems in callback functions"，即回调里的 `;` 不会被打断
- `console.log` 被替换为 `log(bot, ...)`，所有输出进 `bot.output` 供 `getBotOutputSummary` 截断后回传 LLM

### 5.3 Lint 检查

两层：

1. **函数白名单**：扫描代码（去注释后）所有 `skills.xxx(` / `world.xxx(` 调用，与 `skill_library.getAllSkillDocs()` 第一行对比。未知函数直接报 "These functions do not exist:\n<list>"。
2. **ESLint 静态分析**：`ESLint.lintText(code)`，列出每个 message 的 line/column/related code line。

Lint 失败时不执行，把错误回灌给 LLM 重试。

### 5.4 执行模板（`bots/execTemplate.js` 原文，6 行）

```js
(async (bot) => {

/* CODE HERE */
log(bot, 'Code finished.');

})
```

LLM 代码被替换 `/* CODE HERE */` 后整体作为 IIFE 表达式，`compartment.evaluate(src)` 返回这个异步函数，然后外部 `await mainFn(bot)` 调用。

### 5.5 Lint 模板（`bots/lintTemplate.js` 原文，10 行）

```js
import * as skills from '../../../src/agent/library/skills.js';
import * as world from '../../../src/agent/library/world.js';
import Vec3 from 'vec3';

const log = skills.log;

export async function main(bot) {
    /* CODE HERE */
    log(bot, 'Code finished.');
}
```

这是 ESLint 看到的"完整模块"——把 LLM 代码放进一个带正确 import 的 `main` 函数里 lint，所以 LLM 写的 `skills.xxx` / `world.xxx` 引用能被正确解析。

### 5.6 重试与中断

- `MAX_ATTEMPTS = 5`：总尝试 5 次
- `MAX_NO_CODE = 3`：连续 3 次不写代码就放弃
- 中断：`bot.interrupt_code` 标志位（由 `!stop` 或 `ActionManager.stop()` 设置），代码每行 `;` 后检查
- 超时：`settings.code_timeout_mins`（默认 10 分钟，由 `ActionManager._startTimeout` 触发，超时后 `agent.history.add('system', 'Code execution timed out...')` 并 `await this.stop()`）

### 5.7 对话管理中的命令路由（`conversation.js`）

bot↔bot 对话时，`ConversationManager` 维护单条 active conversation：
- `startConversation(send_to, message)` → 暂停 self_prompter，JSON `{message, start, end}` 经 `sendBotChatToServer` 发送
- `receiveFromBot(sender, received)` → 标签 `(FROM OTHER BOT)` 前缀后 `agent.handleMessage`
- `_scheduleProcessInMessage`：根据双方 busy 状态决定响应延迟
  - 两方都忙 → 不响应（除非当前动作在 `talkOverActions = ['stay', 'followPlayer', 'mode:']` 中，则 200ms 响应）
  - 对方忙、自己闲 → 5000ms 延迟
  - 自己忙、对方闲 → 若可 talk-over 则 200ms，否则 LLM 决定是否响应
  - 两方都闲 → 200ms
- 对方掉线 10 秒后自动 endConversation

---

## 6. Craft-Agent 移植建议

### 6.1 命令对照表（Mindcraft → Craft-Agent 已有工具）

| Mindcraft 命令 | Craft-Agent 工具 | 状态 | 备注 |
|----------------|------------------|------|------|
| `!goToCoordinates` | `goto` | ✅ 已有 | CA 用 integer，MC 用 float+domain |
| `!goToPlayer` | — | ❌ 缺失 | CA 无玩家寻路（单机场景可缺） |
| `!followPlayer` | — | ❌ 缺失 | resume 模式，CA 无对应概念 |
| `!searchForBlock` | `gather` 间接覆盖 | ⚠️ 部分 | CA `gather(item,count)` 自动寻路最近方块；MC 拆为 search+goto 两步 |
| `!searchForEntity` | `interact_entity` 间接覆盖 | ⚠️ 部分 | CA 没有纯"走到实体旁"工具 |
| `!moveAway` | — | ❌ 缺失 | CA 无"远离"动作 |
| `!rememberHere` / `!goToRememberedPlace` | `memory(action=anchor)` / `goto` | ✅ 已有 | CA 用 `memory(anchor,name=...)` + `goto`，但缺"按名召回"单步工具 |
| `!savedPlaces` | `memory(action=query,by_anchor=...)` | ⚠️ 部分 | CA 要按名查，不能一次列全部 |
| `!givePlayer` | — | ❌ 缺失 | 单机不重要 |
| `!consume` | — | ❌ 缺失 | CA 无吃东西工具（无饥饿系统对接） |
| `!equip` | — | ❌ 缺失 | CA 无装备切换工具 |
| `!putInChest` / `!takeFromChest` / `!viewChest` | `open` 仅打开 | ❌ 缺失 | CA 只能开容器，不能存取 |
| `!discard` | — | ❌ 缺失 | — |
| `!collectBlocks` | `gather` | ✅ 已有 | CA `gather(item,count)` 等价 |
| `!craftRecipe` | `craft` / `craft_3x3` | ✅ 已有 | CA 拆 2×2/3×3 两个工具 |
| `!smeltItem` | `smelt` | ✅ 已有 | CA 需指定 fuel+output，MC 只指定 input |
| `!clearFurnace` | — | ❌ 缺失 | — |
| `!placeHere` | `place` | ✅ 已有 | CA 需指定坐标，MC 用当前坐标 |
| `!attack` | `attack` | ✅ 已有 | CA 当前忽略 target 总是攻击最近实体 |
| `!attackPlayer` | — | ❌ 缺失 | 单机不重要 |
| `!goToBed` | — | ❌ 缺失 | — |
| `!stay` | — | ❌ 缺失 | CA 用 modes 间接覆盖 |
| `!setMode` | modes 通过 prompt 注入 | ⚠️ 部分 | CA modes 是 prompt-level，无显式开关工具 |
| `!goal` | `set_goal` | ✅ 已有 | CA 等价 |
| `!endGoal` | `set_goal(goal="")` | ✅ 已有 | CA 用空串清空 |
| `!showVillagerTrades` | `interact_entity(kind="villager")` + `perceive` | ⚠️ 部分 | CA 无显式"列报价"工具，靠 perceive 文本 |
| `!tradeWithVillager` | `trade(offer)` | ✅ 已有 | CA 用 0-indexed，MC 用 1-indexed |
| `!startConversation` / `!endConversation` | — | ❌ 不适用 | CA 单 bot，无 bot↔bot 对话 |
| `!lookAtPlayer` / `!lookAtPosition` | — | ❌ 不适用 | VLM 路线，azalea 不用 |
| `!digDown` | `mine_below` | ✅ 已有 | CA 无距离参数，每次挖一格 |
| `!goToSurface` | — | ❌ 缺失 | — |
| `!useOn` | `interact_block` / `interact_entity` | ⚠️ 部分 | CA 拆为两个工具，无"工具+目标"组合 |
| `!newAction` | `run_script` / `run_plan` | ✅ 已有（替代） | CA 用 rhai 替代 JS，安全沙箱由引擎提供 |
| `!stop` / `!stfu` / `!restart` / `!clearChat` | 由 viewer 控制 | ⚠️ 不适用 | CA 由 Web 仪表盘启停，非 LLM 工具 |
| **Query** | | | |
| `!stats` | `perceive` | ✅ 已有 | CA perceive 返回结构化状态 |
| `!inventory` | `perceive` 间接 | ⚠️ 部分 | CA perceive 含背包前 5 格，无完整清单 |
| `!nearbyBlocks` | `perceive` 间接 | ⚠️ 部分 | CA perceive 含周围方块 |
| `!craftable` | — | ❌ 缺失 | CA 无"可合成清单"查询 |
| `!entities` | `perceive` 间接 | ⚠️ 部分 | CA perceive 含附近实体 |
| `!modes` | — | ❌ 缺失 | CA 无显式 mode 文档工具 |
| `!checkBlueprint*` / `!getBlueprint*` | `build` | ⚠️ 部分 | CA build 只执行，无蓝图查询 |
| `!getCraftingPlan` | — | ❌ 缺失 | CA 缺合成计划工具（auto_craft 内部有逻辑但不暴露） |
| `!searchWiki` | `search_wiki` | ✅ 已有 | CA 用中文 wiki.biligame.com，MC 用 minecraft.wiki |
| `!help` | system prompt 注入 | ⚠️ 不适用 | CA 工具 schema 由 OpenAI tool_call 协议自带 |

### 6.2 Craft-Agent 缺失、建议补齐的工具

按优先级排序：

**高优先级**（影响基础生存）：
1. **`equip`**：装备武器/工具/护甲。当前 `attack` / `mine` 无法保证用对工具。
2. **`consume`**：吃东西回血。当前 modes.self_preservation 只能逃离危险，不能主动吃食物。
3. **`chest_deposit` / `chest_withdraw` / `chest_view`**：箱子存取。当前 `open` 只打开，LLM 无法用箱子整理物资。
4. **`go_to_surface`**：地下挖矿后回地表。当前只能靠坐标猜。
5. **`move_away`**：远离危险（爆炸/岩浆/苦力怕）。当前 modes 自卫只能攻击，不能撤退。

**中优先级**（提升效率）：
6. **`list_craftable`**：列出当前可合成物品。让 LLM 不必先 perceive 再脑补配方。
7. **`crafting_plan`**：给出目标物品的合成树与缺料分析。把 `auto_craft` 内部的配方图逻辑暴露给 LLM。
8. **`place_here`**：用 bot 当前坐标放置（不必指定 x,y,z）。简化放置火把/工作台流程。
9. **`dig_down(distance)`**：带距离参数的向下挖。比反复调 `mine_below` 高效。
10. **`search_block(type, range)`**：纯寻路到某类方块，不挖掘。便于侦察。

**低优先级**（多 bot/创意模式才需要）：
11. `go_to_player` / `follow_player` / `give_player` / `attack_player`：多玩家场景。
12. `go_to_bed` / `stay` / `discard` / `clear_furnace` / `use_on`：边缘行为。
13. `start_conversation` / `end_conversation`：bot↔bot 对话，CA 架构暂不支持。

### 6.3 命令 schema 写法差异

| 维度 | Mindcraft | Craft-Agent |
|------|-----------|-------------|
| **协议** | 文本 `!cmd("arg", 1.2)` + 正则解析 | OpenAI tool_call JSON |
| **schema 格式** | 自定义 `{type, description, domain?}` 对象 | 标准 JSON Schema（`{type, properties, required}`） |
| **类型系统** | `int/float/string/boolean/BlockName/ItemName/BlockOrItemName` | `integer/string/boolean/number/array/object` + `enum` |
| **参数校验** | `parseCommandMessage` 手写：类型转换 + domain 区间 + BlockName/ItemName 数据库校验 | LLM 端按 JSON Schema 校验，CA 端 `args.get("x").as_i64()` 手动取值 |
| **范围限制** | `domain: [lower, upper, '[)']` | JSON Schema `minimum/maximum` 或 `enum` |
| **枚举** | 无（用 string + description 描述合法值） | `enum: ["save","anchor","query","forget"]` |
| **可选/默认** | 仅 `!getCraftingPlan` 用 `optional: true, default: 1` | `required` 数组 + 工具内部 `unwrap_or` |
| **文档给 LLM** | `getCommandDocs()` 拼成大段文本进 system prompt | OpenAI 协议自带 `functions`/`tools` 字段，LLM 直接看 schema |
| **多参数顺序** | 必须严格按声明顺序 | JSON 对象，无序 |
| **错误反馈** | 字符串错误进对话历史 | `ToolResult{message, is_error}` 进 tool 消息 |
| **每轮调用数** | 严格 1 条（多余截断） | 多工具并行（按 `ToolEffects` 副作用分组） |
| **超时** | `ActionManager` 分钟级 + `bot.interrupt_code` 行级检查 | `push_cmd_and_wait` 默认 120s + handler 200 tick 强制释放 |
| **快循环保护** | `recent_action_counter` 20ms 内 3+/5+ 次 cleanKill | `recent_calls` 容量 10，4+ 重复注入 nudge |

**移植要点**：
- Mindcraft 的 `domain` 在 CA 可直接用 JSON Schema `minimum/maximum` 替代，无需新机制
- Mindcraft 的 `BlockName/ItemName` 类型校验很有价值，CA 当前 LLM 传错 item id 只能在执行时 azalea 报错。可在 `execute` 入口加一份方块/物品白名单（已有 `azalea-registry` 依赖可直接用）
- CA 已有的 `run_plan` 工具本质上就是"无代码版 newAction"——比 Mindcraft 的 SES 沙箱更安全但也更弱。复杂分支逻辑只能靠 `run_script` 的 rhai

### 6.4 代码执行移植：`run_script` (rhai) vs `coder.js` (JS+SES)

| 维度 | Mindcraft `coder.js` | Craft-Agent `run_script` |
|------|----------------------|--------------------------|
| **引擎** | Node.js + SES `Compartment` | rhai 1.25.1（嵌入式） |
| **语言** | 完整 JavaScript（async/await/Promise） | rhai 脚本（无 async，函数同步执行） |
| **沙箱** | SES lockdown 全局冻结 + Compartment 白名单注入 | rhai 引擎本身隔离，`register_fn` 显式注入 |
| **注入 API** | `skills.*`（38 个 async 函数）/ `world.*` / `Vec3` / `log` / `Math` / `Date` | `go/mine/mine_below/gather/craft/place/open/chat/attack/smelt/interact/sleep/print`（12 个同步函数） |
| **API 数量** | 38+ 个 skill + world 工具 | 12 个动作包装 |
| **异步** | 原生支持 `await` | 不支持，所有动作同步阻塞（`_exec_action` 内部 `exec_mc_sync` 120s 超时） |
| **回调中的中断** | `;` 后插入检查，但回调里失效 | 无中断机制（脚本太长会触发 `max_operations=100_000` / `max_call_levels=20`） |
| **Lint** | ESLint + skill 白名单双校验 | 无 lint，rhai 语法错误运行时报错 |
| **重试** | 5 次重试 + 3 次无代码保护 | 无重试，错误一次性返回 |
| **保留代码文件** | 写到 `/bots/{name}/action-code/{N}.js` 供调试 | 不保留 |
| **保留字问题** | 无 | `goto` 是 rhai 保留字，必须用 `go` 替代 |
| **数据结构** | JS 对象/数组/Map/Set | rhai 对象/数组/Map（无 Set） |
| **生态** | 可调任何 mineflayer 插件 | 只能调 `register_fn` 注册的函数 |
| **性能** | V8 JIT，快 | rhai AST 解释执行，慢但够用 |
| **代码生成** | LLM 直接写 JS（生态熟） | LLM 写 rhai（小众语法，LLM 可能不熟） |

**移植建议**：

1. **保留 `run_script` 作为主入口**：rhai 沙箱比 SES 更严，安全性更高，CA 已有完整实现。缺点是 LLM 对 rhai 语法不熟，错误率会高。建议在 `run_script` 的 description 里多放示例（当前已有一行示例，可扩展为 3-5 个常见模式）。

2. **扩展 `register_fn` 白名单**：当前只有 12 个函数，缺 `enchant/trade/interact_entity/auto_craft/craft_3x3/perceive/memory/set_goal`。把这些也注册进 rhai，让脚本能调用全部 23 个工具。

3. **加 lint 层**（可选）：在 `engine.eval` 前用正则扫一遍 `unknown_func(`，与已注册函数名对比，提前报错。比 rhai 运行时错误信息友好。

4. **加超时**：rhai `Engine::new()` 默认无超时，建议 `engine.set_max_operations(100_000)`（已有）+ 用 `Engine::register_fn("sleep", ...)` 限制总时长。当前 `sleep(ms)` 无上限，恶意脚本可 `sleep(999999999)` 卡死 agent。

5. **加重试**（可选）：当前 `run_script` 错误一次性返回。可参考 `coder.js` 的 5 次重试，把 rhai 错误回灌给 LLM 重写。但 CA 的 `run_script` 错误信息通常已足够 LLM 修正，重试收益不大。

6. **保留代码**（可选）：把每次 `run_script` 的脚本写到日志/会话，便于事后调试。CA 已有 `Session JSONL` 持久化，工具调用参数本就会被记录，无需额外文件。

7. **不移植 SES 沙箱**：rhai 已经是沙箱，再套 SES 没意义。Mindcraft 用 SES 是因为它要执行 LLM 生成的 JS——CA 不执行 LLM 生成的代码，只执行 LLM 写的 rhai 脚本，rhai 引擎本身就是隔离边界。

8. **`run_plan` 是更安全的替代**：对于"按顺序执行 N 个动作"的场景，`run_plan` 比 `run_script` 更安全（只调用已注册工具，不执行任意代码）。建议在 prompt 里引导 LLM 优先用 `run_plan`，仅在需要条件/循环时才用 `run_script`。

### 6.5 错误反馈格式对比

**Mindcraft**：
- 解析错误：`"Command !foo was given 2 args, but requires 3 args."` 纯字符串进对话
- 类型错误：`"Error: Param 'x' must be of type int."` 纯字符串
- 域错误：`"Error: Param 'x' must be an element of [0, 10)."` 纯字符串
- BlockName 错误：`"Invalid block type: oak_wood."` 纯字符串
- 执行错误：`getBotOutputSummary` 截断到 500 字符 + `"!!Code threw exception!!\nError: ...\nStack trace:\n..."`
- LLM 看到错误后自然语言重试，无结构化重试协议

**Craft-Agent**：
- 参数缺失：`anyhow!("缺少 x")` → `ToolResult{message: "缺少 x", is_error: true}`
- 执行错误：`adapter.execute_shared` 返回 `ActionResult{ok: false, detail: ...}` → `ToolResult{message: detail, is_error: !ok}`
- 错误进 OpenAI tool result 消息（`role: "tool"` + `content`），LLM 下一轮看到后重试
- 死循环检测：`recent_calls` 4+ 重复注入 nudge（`"你已连续调用相同工具 4 次，请改用其他策略或 perceive 重新观察"`）

**移植建议**：CA 的结构化错误反馈已优于 Mindcraft。可借鉴 Mindcraft 的两点：
1. **域校验前置**：在 `execute` 入口检查 `count >= 1` / `level in [1,2,3]` 等，提前返回友好错误，而不是传给 azalea 触发底层 panic
2. **BlockName/ItemName 白名单**：用 `azalea-registry` 在入口校验方块/物品 id，错误信息 `"Invalid block type: oak_wood. Did you mean oak_log?"` 比 azalea 的协议错误友好

### 6.6 总结

Mindcraft 命令系统的核心价值在于：
1. **声明式 schema + 文本协议**：简单直接，LLM 易学，但每轮只能一条命令、无并行
2. **Action/Query 二分**：清晰区分副作用与纯查询
3. **`!newAction` 代码生成**：兜底任何预定义命令覆盖不到的场景，SES 沙箱保证安全
4. **快循环保护**：防 LLM 死循环消耗 token

Craft-Agent 已有的 23 个工具覆盖了 Mindcraft 约 60% 的命令，且在协议层（OpenAI tool_call vs 文本正则）、并行层（ToolEffects 分组 vs 严格串行）、安全层（rhai 引擎 vs SES+JS）都更现代。主要缺口在**装备/消耗/箱子操作/地表寻路**等基础生存工具，以及**可合成清单/合成计划**等查询工具。`run_script` 是 `!newAction` 的安全替代，但 API 白名单只有 12 个函数（vs Mindcraft 38 个 skill），建议扩展到全 23 个工具以提升表达力。

---

## 附录：源文件清单

| 文件 | 行数 | 作用 |
|------|------|------|
| `src/agent/commands/actions.js` | 502 | 41 个 action 命令定义 |
| `src/agent/commands/queries.js` | 347 | 14 个 query 命令定义 |
| `src/agent/commands/index.js` | 259 | 命令注册/解析/执行/docs 生成 |
| `src/agent/coder.js` | 228 | LLM 代码生成 + SES 沙箱 + ESLint |
| `src/agent/conversation.js` | 353 | bot↔bot 对话管理 + 命令路由延迟策略 |
| `src/agent/action_manager.js` | 177 | Action 执行 + 超时 + 中断 + 快循环保护 |
| `src/agent/library/lockdown.js` | 32 | SES lockdown + Compartment 工厂 |
| `src/agent/library/skills.js` | ~2000 | 38 个原子 mineflayer 技能 |
| `bots/execTemplate.js` | 6 | 代码执行 IIFE 模板 |
| `bots/lintTemplate.js` | 10 | ESLint 检查模板（带 import） |
