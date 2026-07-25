# Mindcraft Prompt 体系系统化分析报告

> 源码路径：`d:\Craft-Agent\reference\mindcraft\`
> 分析对象：14 份文件（5 defaults + 3 tasks + 4 model profiles + prompter.js + settings.js）
> 目的：提炼可移植到 Craft-Agent 的 prompt 设计模式与具体可复用片段

---

## 1. Mindcraft Prompt 体系总览

### 1.1 三层 Profile 叠加架构

Mindcraft 的 prompt 体系采用**三层 JSON 字段填充**机制，核心逻辑在 `src/models/prompter.js` 构造函数（行 17-43）：

```javascript
// 第一层：默认 profile（永远加载）
let default_profile = JSON.parse(readFileSync(path.join(defaults_dir, '_default.json'), 'utf8'));

// 第二层：模式 profile（根据 settings.base_profile 选择其一）
if (settings.base_profile.includes('survival'))   base_fp = '.../survival.json';
else if (settings.base_profile.includes('assistant')) base_fp = '.../assistant.json';
else if (settings.base_profile.includes('creative'))  base_fp = '.../creative.json';
else if (settings.base_profile.includes('god_mode'))  base_fp = '.../god_mode.json';
let base_profile = JSON.parse(readFileSync(base_fp, 'utf8'));

// 第三层：个体 profile（用户传入，如 andy.json / qwen.json）
this.profile = profile;

// 字段填充规则：base 覆盖 default，individual 覆盖 base
for (let key in default_profile)
    if (base_profile[key] === undefined) base_profile[key] = default_profile[key];
for (let key in base_profile)
    if (this.profile[key] === undefined) this.profile[key] = base_profile[key];
```

**关键原则**：
- `base overrides default, individual overrides base`（prompter.js 行 44 注释原文）
- 字段级覆盖（不是整体替换）：个体 profile 只需写"想改的字段"，其余自动继承
- `settings.base_profile` 用 `includes()` 子串匹配，容错性好

### 1.2 叠加流程图

```
┌─────────────────────────────────────────────────────────────┐
│                  _default.json (永远加载)                    │
│  conversing / coding / saving_memory / bot_responder /     │
│  image_analysis / modes(10种) / conversation_examples(23) / │
│  coding_examples(6) / cooldown / speak_model                │
└──────────────────────────┬──────────────────────────────────┘
                           │ 字段填充（base 缺失时取 default）
                           ▼
┌─────────────────────────────────────────────────────────────┐
│  模式 profile（4 选 1，由 settings.base_profile 决定）        │
│  survival.json   — modes 全开（生存挑战）                    │
│  assistant.json  — modes 同 default（助手模式）              │
│  creative.json   — modes 大多关（创造模式，无需生存）         │
│  god_mode.json   — cheat=true（上帝模式，可作弊）            │
│  → 4 个文件都只含 "modes" 字段，不改 prompt 模板              │
└──────────────────────────┬──────────────────────────────────┘
                           │ 字段填充（individual 缺失时取 base）
                           ▼
┌─────────────────────────────────────────────────────────────┐
│  个体 profile（用户传入，如 andy.json / qwen.json）           │
│  ┌──────────────────────────────────────────────────────┐  │
│  │ 模型特定 profile：deepseek.json / qwen.json /         │  │
│  │   claude_thinker.json / andy-4-reasoning.json         │  │
│  │ → 只写 model / embedding / cooldown 等字段             │  │
│  └──────────────────────────────────────────────────────┘  │
│  ┌──────────────────────────────────────────────────────┐  │
│  │ 任务 profile（(tasks/*.json)）：                       │  │
│  │   construction_profile / cooking_profile /            │  │
│  │   crafting_profile                                    │  │
│  │ → 改写 conversing prompt + 自定义 conversation_examples│  │
│  └──────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────┘
```

### 1.3 6 种 Prompt 类型

`_default.json` 定义了 6 个 prompt 模板字段（每种用途独立）：

| 字段 | 用途 | 调用入口（prompter.js） |
|---|---|---|
| `conversing` | 主对话/命令模式 system prompt | `promptConvo()` 行 214 |
| `coding` | JS 代码生成模式 system prompt | `promptCoding()` 行 264 |
| `saving_memory` | 记忆压缩 prompt | `promptMemSaving()` 行 280 |
| `bot_responder` | 多 bot 通信决策 prompt | `promptShouldRespondToBot()` 行 293 |
| `image_analysis` | 视觉分析 prompt | `promptVision()` 行 303 |
| `goal_setting` | 目标设置 prompt（**已废弃**，行 311 注释 "deprecated"） | `promptGoalSetting()` 行 310 |

### 1.4 12 个模板变量替换点

`replaceStrings()` 方法（prompter.js 行 137-204）支持以下 `$PLACEHOLDER`：

| 变量 | 含义 | 替换来源 |
|---|---|---|
| `$NAME` | bot 名字 | `this.agent.name` |
| `$SELF_PROMPT` | 当前目标 | SelfPrompter 未停止时填 `YOUR CURRENT ASSIGNED GOAL: "..."` |
| `$MEMORY` | 总结记忆（500 字符） | `this.agent.history.memory` |
| `$STATS` | 状态+实体+邻近方块 | `!stats` + `!entities` + `!nearbyBlocks` 命令输出 |
| `$INVENTORY` | 背包 | `!inventory` 命令输出 |
| `$ACTION` | 当前动作标签 | `this.agent.actions.currentActionLabel` |
| `$COMMAND_DOCS` | 命令文档 | `getCommandDocs(agent)` |
| `$CODE_DOCS` | 相关技能文档 | SkillLibrary embedding 检索 top-N（`relevant_docs_count=5`） |
| `$EXAMPLES` | few-shot 示例 | Examples 类 embedding 检索 top-2（`num_examples=2`） |
| `$TO_SUMMARIZE` | 待总结对话 | `stringifyTurns(to_summarize)` |
| `$CONVO` | 最近对话 | `stringifyTurns(messages)` |
| `$LAST_GOALS` | 最近目标成败 | 遍历 last_goals 字典 |
| `$BLUEPRINTS` | 蓝图列表 | `this.agent.npc.constructions` |

**安全机制**（行 199-202）：替换后扫描剩余 `\$[A-Z_]+` 模式，未识别的占位符会 `console.warn`。

### 1.5 10 种 Modes 布尔开关

```json
"modes": {
    "self_preservation": true,   // 自保（避火/岩浆）
    "unstuck": true,             // 脱困
    "cowardice": false,          // 怯懦（遇敌逃跑）
    "self_defense": true,        // 自卫（被攻击反击）
    "hunting": true,             // 狩猎（主动攻击动物）
    "item_collecting": true,     // 拾物
    "torch_placing": true,       // 插火把
    "elbow_room": true,          // 保持周围空间
    "idle_staring": true,        // 闲置环视
    "cheat": false               // 作弊（创意模式命令）
}
```

通过 `getInitModes()` 返回给 agent 初始化行为模式（prompter.js 行 108-110）。

---

## 2. `_default.json` 完整结构剖析

### 2.1 字段一览（19840 字节，最重要）

| 字段 | 类型 | 用途 |
|---|---|---|
| `cooldown` | number(3000) | LLM 调用冷却（毫秒），`checkCooldown()` 强制等待 |
| `conversing` | string | 主对话/命令 system prompt 模板 |
| `coding` | string | JS 代码生成 system prompt 模板 |
| `saving_memory` | string | 记忆压缩 prompt |
| `bot_responder` | string | 多 bot 通信决策 prompt |
| `image_analysis` | string | 视觉分析 prompt |
| `speak_model` | string("openai/tts-1/echo") | TTS 模型 |
| `modes` | object | 10 种布尔开关 |
| `conversation_examples` | array | 23 个对话 few-shot 示例 |
| `coding_examples` | array | 6 个代码 few-shot 示例 |

### 2.2 `conversing` 模板原文摘录（核心）

```
You are an AI Minecraft bot named $NAME that can converse with players, see, move, mine, build, and interact with the world by using commands.
$SELF_PROMPT Be a friendly, casual, effective, and efficient robot. Be very brief in your responses, don't apologize constantly, don't give instructions or make lists unless asked, and don't refuse requests. Don't pretend to act, use commands immediately when requested. Do NOT say this: 'Sure, I've stopped. *stops*', instead say this: 'Sure, I'll stop. !stop'. Respond only as $NAME, never output '(FROM OTHER BOT)' or pretend to be someone else. If you have nothing to say or do, respond with an just a tab '\t'. This is extremely important to me, take a deep breath and have fun :)
Summarized memory:'$MEMORY'
$STATS
$INVENTORY
$COMMAND_DOCS
$EXAMPLES
Conversation Begin:
```

**结构分析**：
1. **身份行**：`You are an AI Minecraft bot named $NAME that can converse...`（一句话定义能力边界）
2. **目标注入**：`$SELF_PROMPT`（每轮覆盖，避免目标漂移）
3. **行为约束**（5 条）：
   - 友好/随意/高效
   - 回复简短
   - 不要频繁道歉
   - 不要主动列清单
   - 不要拒绝请求
4. **反幻觉规则**：
   - "Don't pretend to act, use commands immediately" — 禁止假装行动
   - 显式正反例：`Do NOT say 'Sure, I've stopped. *stops*', instead say 'Sure, I'll stop. !stop'`
   - 禁止冒充其他 bot：`never output '(FROM OTHER BOT)'`
5. **空响应处理**：`If you have nothing to say or do, respond with an just a tab '\t'`（用 Tab 表示"无话可说"，避免 LLM 强行编造）
6. **情绪暗示**：`This is extremely important to me, take a deep breath and have fun :)`（"深呼吸"暗示 Chain-of-Thought）
7. **上下文块**（按顺序注入）：
   - `Summarized memory:'$MEMORY'` — 长期记忆
   - `$STATS` — 状态/实体/邻近方块
   - `$INVENTORY` — 背包
   - `$COMMAND_DOCS` — 命令文档
   - `$EXAMPLES` — few-shot 示例
8. **结尾标记**：`Conversation Begin:`（明确对话开始）

### 2.3 `coding` 模板原文摘录

```
You are an intelligent mineflayer bot $NAME that plays minecraft by writing javascript codeblocks. Given the conversation, use the provided skills and world functions to write a js codeblock that controls the mineflayer bot ``` // using this syntax ```. The code will be executed and you will receive it's output. If an error occurs, write another codeblock and try to fix the problem. Be maximally efficient, creative, and correct. Be mindful of previous actions. Do not use commands !likeThis, only use codeblocks. The code is asynchronous and MUST USE AWAIT for all async function calls, and must contain at least one await. You have `Vec3`, `skills`, and `world` imported, and the mineflayer `bot` is given. Do not import other libraries. Do not use setTimeout or setInterval. Do not speak conversationally, only use codeblocks. Do any planning in comments. This is extremely important to me, think step-by-step, take a deep breath and good luck!
$SELF_PROMPT
Summarized memory:'$MEMORY'
$STATS
$INVENTORY
$CODE_DOCS
$EXAMPLES
Conversation:
```

**关键约束**：
- 错误处理自愈：`If an error occurs, write another codeblock and try to fix the problem`
- 异步强制：`MUST USE AWAIT for all async function calls, and must contain at least one await`
- 环境边界：`Vec3, skills, world imported, bot given. Do not import other libraries`
- 禁用 API：`Do not use setTimeout or setInterval`
- 规划方式：`Do any planning in comments`（把思考写在代码注释里）

### 2.4 `saving_memory` 模板原文摘录

```
You are a minecraft bot named $NAME that has been talking and playing minecraft by using commands. Update your memory by summarizing the following conversation and your old memory in your next response. Prioritize preserving important facts, things you've learned, useful tips, and long term reminders. Do Not record stats, inventory, or docs! Only save transient information from your chat history. You're limited to 500 characters, so be extremely brief and minimize words. Compress useful information.
Old Memory: '$MEMORY'
Recent conversation:
$TO_SUMMARIZE
Summarize your old memory and recent conversation into a new memory, and respond only with the unwrapped memory text:
```

**核心设计**：
- **优先级**：`Prioritize preserving important facts, things you've learned, useful tips, and long term reminders`
- **禁止项**：`Do Not record stats, inventory, or docs!`（避免重复存储易变状态）
- **硬限制**：`500 characters`（强制压缩）
- **输出格式**：`respond only with the unwrapped memory text`（无包装纯文本，便于直接覆盖 `$MEMORY`）

### 2.5 `bot_responder` 模板原文摘录

```
You are a minecraft bot named $NAME that is currently in conversation with another AI bot. Both of you can take actions with the !command syntax, and actions take time to complete. You are currently busy with the following action: '$ACTION' but have received a new message. Decide whether to 'respond' immediately or 'ignore' it and wait for your current action to finish. Be conservative and only respond when necessary, like when you need to change/stop your action, or convey necessary information.
Example 1: You:Building a house! !newAction('Build a house.').
Other Bot: 'Come here!'
Your decision: ignore
Example 2: You:Collecting dirt !collectBlocks('dirt',10).
Other Bot: 'No, collect some wood instead.'
Your decision: respond
Example 3: You:Coming to you now. !goToPlayer('billy',3).
Other Bot: 'What biome are you in?'
Your decision: respond
Actual Conversation: $TO_SUMMARIZE
Decide by outputting ONLY 'respond' or 'ignore', nothing else. Your decision:
```

**核心设计**：
- **决策二选一**：`ONLY 'respond' or 'ignore'`（强制单 token 输出，便于解析）
- **3 个 few-shot 示例**直接嵌入 prompt（不用 embedding 检索）
- **保守原则**：`Be conservative and only respond when necessary`
- **触发条件**：`change/stop your action, or convey necessary information`
- **决策依据**：`actions take time to complete`（动作执行期间不能被打断）

### 2.6 `image_analysis` 模板原文摘录

```
You are a Minecraft bot named $NAME that has been given a screenshot of your current view. Analyze and summarize the view; describe terrain, blocks, entities, structures, and notable features. Focus on details relevant to the conversation. Note: the sky is always blue regardless of weather or time, dropped items are small pink cubes, and blocks below y=0 do not render. Be extremely concise and correct, respond only with your analysis, not conversationally. $STATS
```

**视觉特化知识**（重要）：
- `the sky is always blue regardless of weather or time`（避免 LLM 误判天气/时间）
- `dropped items are small pink cubes`（掉落物的渲染特征）
- `blocks below y=0 do not render`（深暗之域/下界渲染限制）

### 2.7 `conversation_examples` 23 个 few-shot 示例分类

按场景类型归类：

| 类别 | 示例数 | 典型示例 |
|---|---|---|
| 闲聊问候 | 1 | "Hey! What are you up to?" / "Nothing much..." |
| 多 bot 协作 | 5 | startConversation/endConversation/givePlayer |
| 玩家交互 | 8 | goToPlayer/followPlayer/lookAtPlayer/collectBlocks/attack |
| 工具结果处理 | 4 | Code Output 错误 → retry / searchForBlock 扩大范围 |
| 合成 | 2 | craftRecipe(stick,4) → 16 sticks |
| 建造 | 2 | newAction("Build a cobblestone house") |
| 模式切换 | 1 | setMode("hunting", false) |
| 记忆锚点 | 1 | rememberHere("base") / goToRememberedPlace("base") |
| 目标设定 | 3 | goal("Survive...") / 多步目标计划 |
| 自提示循环 | 2 | "You are self-prompting with the goal: '...'" 多步执行 |

**值得直接复用的示例原文**（自提示循环示例，_default.json 行 192-205）：

```
{"role": "system", "content": "You are self-prompting with the goal: \"Get a wooden pickaxe\". Respond:"}
{"role": "assistant", "content": "What's around me? !nearbyBlocks"}
{"role": "system", "content": "NEARBY_BLOCKS\n- oak_log\n- dirt\n- cobblestone"}
{"role": "assistant", "content": "I see some oak logs, dirt, and cobblestone nearby. My goal is to craft a wooden pickaxe. I'll start by collecting some wood. !collectBlock(\"oak_log\", 10)"}
{"role": "system", "content": "Successfully collected 9 wood."}
{"role": "assistant", "content": "I collected 9 oak logs. I need a pickaxe first, which requires planks and sticks. !craftItem(\"oak_plank\", 9)"}
{"role": "system", "content": "Successfully crafted 9 oak_planks."}
{"role": "assistant", "content": "I have crafted 9 oak planks. Now I need sticks. !craftItem(\"stick\", 2)"}
{"role": "system", "content": "Successfully crafted 2 sticks."}
{"role": "assistant", "content": "I have crafted 2 sticks. Now I can craft a wooden pickaxe. !craftItem(\"wooden_pickaxe\", 1)"}
{"role": "system", "content": "Successfully crafted 1 wooden_pickaxe."}
{"role": "assistant", "content": "Looks like I have a wooden pickaxe now! Goal accomplished. !endGoal"}
```

**错误恢复示例**（_default.json 行 137-145，3 个）：

```
{"role": "system", "content": "Code output: Action failed."}
{"role": "assistant", "content": "Apologies, I made an error. Let me try again. !newAction(\"Try again to complete the task.\")"}

{"role": "system", "content": "Code output: Could not find any oak_log in 20 blocks."}
{"role": "assistant", "content": "I'll have to search a wider area. !searchForBlock(\"oak_log\", 100)"}

{"role": "system", "content": "Code output: Placed oak_planks at (-154, 96, -228)."}
{"role": "assistant", "content": "I've placed an oak plank for the house. Wait, I should use newAction to build complicated structures! !newAction(\"Build a house with oak planks\")"}
```

### 2.8 `coding_examples` 6 个示例分类

| 类别 | 示例数 | 关键模式 |
|---|---|---|
| 单行采集 | 1 | `await skills.collectBlock(bot, 'oak_log', 10);` |
| 单行熔炼 | 1 | `await skills.smeltItem(bot, 'chicken', 8);` |
| 单行移动 | 1 | `await skills.goToPlayer(bot, 'zZZn98');` |
| 错误恢复 | 1 | ReferenceError → 重新查 world.getNearestBlock |
| 循环建造 | 1 | for 循环 placeBlock 建塔 |
| 复杂蓝图 | 1 | 6 层嵌套 for 建房子（front/back/left/right wall + floor + ceiling） |

**复杂蓝图示例原文**（_default.json 行 250-251，可直接复用的建房子模板）：

```javascript
const position = world.getPosition(bot);
const startX = position.x;
const startY = position.y;
const startZ = position.z;
const width = 7;
const depth = 7;
const height = 4;

// Build front wall (z = startZ)
for (let x = startX; x < startX + width; x++) {
  for (let y = startY; y < startY + height; y++) {
    await skills.placeBlock(bot, 'oak_planks', x, y, startZ);
  }
}
// ... 类似建 back/left/right wall + floor + ceiling
```

---

## 3. 4 种模式 Profile 差异

4 个模式 profile 文件**都只含 `modes` 字段**，不改 prompt 模板。差异全在布尔开关组合：

| Mode | assistant.json | creative.json | god_mode.json | survival.json | _default.json |
|---|---|---|---|---|---|
| self_preservation | true | **false** | **false** | true | true |
| unstuck | true | **false** | **false** | true | true |
| cowardice | false | false | false | false | false |
| self_defense | true | **false** | **false** | true | true |
| hunting | false | **false** | **false** | true | true |
| item_collecting | true | **false** | **false** | true | true |
| torch_placing | true | **false** | **false** | true | true |
| elbow_room | true | true | **false** | true | true |
| idle_staring | true | true | true | true | true |
| cheat | false | false | **true** | false | false |

**模式设计哲学**：
- **assistant**（默认）：关 hunting（不主动狩猎），其余生存功能开。适合"被动助手"
- **creative**：除 elbow_room + idle_staring 外全关。创造模式无需生存/自卫/采集
- **god_mode**：除 idle_staring + cheat=true 外全关。上帝模式不死亡不需自保，但允许作弊命令
- **survival**：全开（与 _default 相同）。完整生存体验

**重要发现**：
- 4 个文件**没有任何 prompt 模板字段**，纯粹是 modes 配置
- assistant 与 _default 唯一差异：`hunting: false`（_default 是 true）
- creative/god_mode 通过关闭 self_preservation/unstuck 等关闭"自动行为"，让 LLM 完全主导

---

## 4. 3 种任务 Profile 差异

任务 profile 通过**改写 conversing prompt + 替换 conversation_examples** 实现任务特化。

### 4.1 `construction_profile.json`（建造任务）

**核心改动**：
- `name`: "andy"
- `model`: "gpt-4o-mini"
- `modes`: `cheat: false`（其余继承 base）
- `conversing`: 改写为"协作建造"导向
- `conversation_examples`: 4 个 blueprint 协作示例

**conversing prompt 关键新增片段**：

```
You are a task-focused Minecraft bot named $NAME. You have to collaborate with other agents in the world to complete the current task
Feel free to ask other agents questions and make a plan to achieve the goal. You can request them to give them some of their inventory items if required to complete the goal.
...
Act human-like as if you were a typical Minecraft player, rather than an AI.
...
Share resources and information with other bots!
```

**与 _default 差异**：
- 身份从 "AI Minecraft bot" 改为 "task-focused Minecraft bot"
- 新增 "collaborate with other agents" 协作导向
- 新增 "Act human-like" 人格化要求
- 新增 "Share resources and information with other bots!" 资源共享原则
- 反幻觉正反例扩展：`Do NOT say 'On my way! Give me a moment.', instead say 'On my way! !goToPlayer("playername", 3)'`

**4 个 blueprint 协作示例**（construction_profile.json 行 13-41）核心模式：

示例 1（单 bot 检查 + 放置）：
```
assistant: Alright I have the necessary materials to build, what needs to be done for the first level of the blueprint? !checkBlueprintLevel(0)
system: Level 0 requires the following fixes:\n Place oak_door at X: 144, Y: -60, Z: -179
assistant: I'll start by placing the oak_door at the specified coordinates. !newAction("Place oak_door at X: 144, Y: -60, Z: -179")
assistant: I've placed the oak_door. What's next? !checkBlueprintLevel(0)
system: Level 0 is complete
```

示例 3（多 bot 分工 + 传递材料）：
```
assistant: I need 30 stones to build level 1 of blueprint, but I only have 20. Can you pass me some stones if you have any?
user: philips: (FROM OTHER BOT) Sure, I'll pass you 10 stones. !givePlayer("fujibayashi", "stone", 10)
assistant: I've received the stones, let me start placing them. !newAction("Place stone for level 1")
```

### 4.2 `cooking_profile.json`（烹饪任务）

**核心改动**：
- `model`: "claude-3-5-sonnet-latest"
- `modes`: `hunting: false, item_collecting: true, elbow_room: false`（其余继承 base）
- `conversing`: 加入农场场景知识 + 协作提示
- `saving_memory`: 自定义（强调保留目标相关信息）

**conversing prompt 关键新增片段（场景知识注入）**：

```
General Searching Tips:
- You will be spawned in a farm with many crops and animals nearby. The farm area is extensive - search thoroughly for needed resources (with searchForBlocks parameters like 64,128,256)
 There is a crafting table, fully fueled furnace and fully fueled smoker with coal are also available nearby which you can use to your advantage. On top of this plants like mushrooms, wheat, carrots, beetroots, pumpkins, potatoes are also present nearby.
Collaboration tips - Divide tasks efficiently between agents for faster completion and share inventory items.
- Communicate your plan and progress clearly.
```

**saving_memory 自定义版本**（cooking_profile.json 行 10）：

```
You are a minecraft bot named $NAME that has been talking and playing minecraft by using commands. Update your memory by summarizing the following conversation and your old memory in your next response. Prioritize preserving important facts, things you've learned, useful tips, and long term reminders. Do Not record stats, inventory, or docs! Only save transient information from your chat history. $SELF_PROMPT Make sure to include information relevant to the goal and inventory you have collected. You're limited to 500 characters, so be extremely brief and minimize words. Compress useful information.
```

**与 _default 差异**：新增 `$SELF_PROMPT Make sure to include information relevant to the goal and inventory you have collected.`（强调目标相关记忆）

### 4.3 `crafting_profile.json`（合成任务，few-shot 最丰富）

**核心改动**：
- `model`: "claude-3-5-sonnet-latest"
- `modes`: `hunting: false, elbow_room: false`
- `conversing`: 改写为 "playful" + 禁用 startConversation
- `saving_memory`: 同 cooking
- `conversation_examples`: **6 个多 bot 协作合成示例**（最值得复用）

**conversing prompt 关键新增片段**：

```
You are a playful Minecraft bot named $NAME that can converse with players, see, move, mine, build, and interact with the world by using commands.
...
 Your name is $NAME, do not pretend to be other bots. You are in a conversation by default do not use !startConversation to start a conversation. Conversation Begin:
```

**6 个协作合成示例核心模式**：

示例 1（stone_pickaxe 协作）：
```
user: philips: (FROM OTHER BOT) Hey! let's collaborate to build a stone pickaxe!
assistant: Cool, let me check what is in my inventory !inventory\nI have 10 oak logs. What do you have?
user: philips: (FROM OTHER BOT) Let me check !inventory \nI have 10 stones.
assistant: Let me give you the stones and you can craft the pickaxe. !givePlayer("playername", "stone", 10)
```

示例 2（chain 协作 + getCraftingPlan 反推）：
```
user: yasuke: (FROM OTHER BOT) Lets craft 5 chains together
assistant: Sure, can you let me know what inventory do you have while I check how to craft chains?
user: yasuke: (FROM OTHER BOT) I have 10 iron_nugget and 3 mushroom.
assistant: Thanks for letting me know, let me check how to build a chain, !getCraftingPlan("chain",5)
system: You are missing the following items:\n- 2 iron_ingot\n\nOnce you have these items, here's your crafting plan:\n\nCraft 2 iron_ingot -> 18 iron_nugget\nCraft 10 iron_nugget + 5 iron_ingot -> 5 chain
assistant: Seems like we need 2 iron_ingot, but wait, we need the iron_ingot for getting 10 iron_nugget, since you already have 10 iron_nugget, I don't need to search for 2 iron_ingots in the world, could you give me the 10 iron_nugget ?
```

**这个示例展示了"配方反推"高级推理**：bot 看到 getCraftingPlan 输出"需要 2 iron_ingot"，但推理出"iron_ingot 是用来造 iron_nugget 的，对方已有 iron_nugget，所以不需要再找 iron_ingot"。

示例 4（cyan_bed 三方协作）：
```
assistant: Hmm, seems like we don't have enough to craft a cyan bed. Let me reach out to the third agent and ask for help! !startConversation("jacob", Hey jacob, can you help us craft a cyan bed? We need cyan_dye and oak_planks.)
```

示例 5（ender_chest 三方协作 + 配方分解）：
```
system: To craft 1 ender_chest, you need:\n- 8 obsidian\n- 1 ender_eye\n\nCraft 8 obsidian + 1 ender_eye -> 1 ender_chest
assistant: We need 8 obsidian total and 1 ender_eye. Between us we have 6 obsidian and 1 ender_eye. Let's ask lucas if he has the remaining obsidian!
```

---

## 5. 推理模型 Prompt 特殊处理

### 5.1 `andy-4-reasoning.json`（Ollama 本地推理模型）

**核心改动**：4 个 prompt 字段全部改写，加入"思考"暗示词。

**conversing 关键差异**：
```
Act human-like as if you were a typical Minecraft player, rather than an AI. Be very brief in your responses, don't apologize constantly, don't give instructions or make lists unless asked, and don't refuse requests. Think in high amounts before responding.
...
$EXAMPLES
Reason before responding. Conversation Begin:
```

**与 _default 差异**：
- 新增 `Think in high amounts before responding.`（行内思考暗示）
- 结尾新增 `Reason before responding.`（后置思考暗示）
- **去掉了** _default 的 "Be a friendly, casual, effective, and efficient robot"（去掉人格化形容词，专注推理）

**coding 关键差异**：
```
Think deeply before responding. Do not use setTimeout or setInterval. Do not speak conversationally, only use codeblocks. Do any planning in comments.
```
新增 `Think deeply before responding.`

**saving_memory 关键差异（中文输出！）**：
```
You're limited to 500 characters, so be extremely brief, think about what you will summarize before responding, minimize words, and provide your summarization in Chinese. Compress useful information.
```

**重要发现**：andy-4-reasoning.json 行 10 明确要求 `provide your summarization in Chinese`（用中文输出记忆总结）。这是 Mindcraft 唯一显式指定输出语言的 prompt。

**bot_responder**：与 _default 完全相同（决策类 prompt 不需要推理增强）。

### 5.2 `claude_thinker.json`（Claude thinking 模式）

**极简配置**，**完全不改 prompt 模板**，只配置 model params：

```json
{
    "name": "claude_thinker",
    "model": {
        "model": "claude-sonnet-4-6",
        "params": {
            "thinking": {
                "type": "enabled",
                "budget_tokens": 4000
            }
        }
    },
    "embedding": "openai"
}
```

**设计哲学**：
- Claude 的 thinking 是 API 层原生能力（`thinking.type=enabled`），不需要在 prompt 里暗示"深呼吸"
- `budget_tokens: 4000` 控制思考预算（避免过度推理浪费 token）
- 所有 prompt 字段继承自 _default + base_profile，零修改

**对比 andy-4-reasoning**：
- andy-4 是 Ollama 本地模型，无原生 thinking API，只能靠 prompt 暗示
- Claude 有原生 thinking API，prompt 完全不动
- 这是"模型能力 vs prompt 工程"的典型对比

### 5.3 模型特定 profile 对比

| Profile | 改动内容 | 设计模式 |
|---|---|---|
| `deepseek.json` | 仅 `name + model="deepseek-chat" + embedding="openai"` | 极简，全继承 |
| `qwen.json` | 加 `cooldown: 5000` + 完整 model URL 配置 + embedding 配置 | 服务端配置 |
| `claude_thinker.json` | model params 加 thinking 配置 | 模型能力特化 |
| `andy-4-reasoning.json` | 改写 4 个 prompt 字段加思考暗示 | prompt 工程特化 |

**qwen.json 完整原文**（带 URL 的服务端配置示例）：

```json
{
    "name": "qwen",
    "cooldown": 5000,
    "model": {
        "api": "qwen",
        "url": "https://dashscope-intl.aliyuncs.com/compatible-mode/v1",
        "model": "qwen-max"
    },
    "embedding": {
        "api": "qwen",
        "url": "https://dashscope-intl.aliyuncs.com/compatible-mode/v1",
        "model": "text-embedding-v3"
    }
}
```

---

## 6. Craft-Agent 移植建议

### 6.1 当前 Craft-Agent 现状

**当前 system prompt**（`crates/craft-agent-viewer/src/agent_loop.rs` 行 346-383）：
- **硬编码**在 Rust 源码里，改 prompt 必须重编译
- 单一 system prompt 字符串（约 40 行），无分层
- 已有 17 个 few-shot 示例（`craft-agent/src/agent/prompt.rs` 行 21-182）
- 已有 3 个 modes（self_preservation / self_defense / unstuck）
- 已有 PromptBuilder 五层结构（identity / role_desc / scenario / examples / jailbreak）
- 已有 WorldInfo 关键词触发动态注入
- 已有词重叠 few-shot 检索（无 embedding 依赖）

**对比 Mindcraft 的差距**：
| 维度 | Craft-Agent | Mindcraft | 差距 |
|---|---|---|---|
| Prompt 配置 | 硬编码 Rust | JSON profile 文件 | **缺 profile 文件化** |
| 模式预设 | 无 | 4 种（survival/assistant/creative/god_mode） | **缺模式预设** |
| 任务 profile | 无 | 3 种（construction/cooking/crafting） | **缺任务 profile** |
| Modes 数量 | 3 种 | 10 种 | **缺 7 种** |
| Few-shot 检索 | 词重叠（离线） | embedding（在线） | 各有优劣 |
| 记忆压缩 | compaction 三级回退 | saving_memory prompt + 500 字符 | **缺字符硬限制** |
| 多 bot 通信 | 无 | bot_responder prompt | **完全缺失** |
| 视觉分析 | 无（azalea 路线） | image_analysis prompt | 不需要 |
| 模板变量 | 无（直接字符串拼接） | 12 个 $PLACEHOLDER | **缺变量系统** |

### 6.2 可直接抄的设计（P0 优先级）

#### 6.2.1 三层 Profile 叠加机制（P0）

**移植方案**：在 `config/` 目录新建 `profiles/` 子目录：

```
config/
├── agent.toml          # 现有 LLM 配置
└── profiles/
    ├── _default.json   # 主 prompt 模板（对应 Mindcraft _default.json）
    ├── survival.json   # 模式预设（只含 modes 覆盖）
    ├── assistant.json
    ├── creative.json
    ├── god_mode.json
    └── tasks/
        ├── construction.json
        ├── cooking.json
        └── crafting.json
```

**Rust 实现要点**（参考 prompter.js 行 17-43）：
- 启动时读取 `_default.json` → 根据 `agent.toml` 的 `base_profile` 字段选模式 profile → 读个体 profile
- 字段级覆盖：`individual > base > default`
- 用 `serde_json::Value` + `merge()` 实现字段填充

**AgentConfig 改造**：
```rust
pub struct AgentConfig {
    pub prompt: String,           // 保留（向后兼容）
    pub profile_path: Option<PathBuf>,  // 新增：profile 文件路径
    pub base_profile: String,     // 新增："survival"/"assistant"/...
    // ...
}
```

#### 6.2.2 Modes 10 种布尔开关扩展（P0）

**当前 Craft-Agent 只有 3 种**（`craft-agent/src/agent/modes.rs`），可补齐 Mindcraft 的 7 种：

```rust
pub struct Modes {
    pub self_preservation: bool,  // 已有
    pub unstuck: bool,            // 已有
    pub self_defense: bool,       // 已有
    pub cowardice: bool,          // 新增：遇敌逃跑（与 self_defense 互斥）
    pub hunting: bool,            // 新增：主动狩猎动物
    pub item_collecting: bool,    // 新增：自动拾物
    pub torch_placing: bool,      // 新增：暗处自动插火把
    pub elbow_room: bool,         // 新增：保持周围空间
    pub idle_staring: bool,       // 新增：闲置环视
    pub cheat: false,             // 新增：作弊命令
}
```

**4 种模式预设**直接抄 Mindcraft 的 modes 组合（见第 3 节表格）。

#### 6.2.3 模板变量替换系统（P0）

**移植 Mindcraft 的 12 个 `$PLACEHOLDER`**，Craft-Agent 已有部分能力（SelfPrompter / WorldMemory / perceive），只需统一变量名：

```rust
fn replace_placeholders(template: &str, ctx: &PromptContext) -> String {
    template
        .replace("$NAME", &ctx.name)
        .replace("$SELF_PROMPT", &ctx.self_prompt)      // 已有 SelfPrompter
        .replace("$MEMORY", &ctx.memory)                // 已有 WorldMemory
        .replace("$STATS", &ctx.stats)                  // 已有 perceive
        .replace("$INVENTORY", &ctx.inventory)          // 已有 perceive
        .replace("$COMMAND_DOCS", &ctx.command_docs)    // 新增：从 ToolRegistry 生成
        .replace("$EXAMPLES", &ctx.examples)            // 已有 FEW_SHOT_EXAMPLES
        // Craft-Agent 不需要：$CODE_DOCS / $ACTION / $TO_SUMMARIZE / $CONVO / $LAST_GOALS / $BLUEPRINTS
}
```

### 6.3 需要适配的设计（P1）

#### 6.3.1 `saving_memory` 独立 prompt（P1）

Craft-Agent 已有 compaction 三级回退（`compaction.rs`），但**没有"500 字符硬限制"**。可借鉴 Mindcraft 的设计：

```
You are a minecraft bot named $NAME that has been talking and playing minecraft by using commands.
Update your memory by summarizing the following conversation and your old memory in your next response.
Prioritize preserving important facts, things you've learned, useful tips, and long term reminders.
Do Not record stats, inventory, or docs! Only save transient information from your chat history.
You're limited to 500 characters, so be extremely brief and minimize words. Compress useful information.
Old Memory: '$MEMORY'
Recent conversation:
$TO_SUMMARIZE
Summarize your old memory and recent conversation into a new memory, and respond only with the unwrapped memory text:
```

**适配点**：
- Craft-Agent 用中文，可改为中文版
- 500 字符限制可保留（强制压缩）
- "Do Not record stats, inventory" 规则可借鉴（避免易变状态污染记忆）
- 输出格式 `respond only with the unwrapped memory text` 可直接抄

#### 6.3.2 `bot_responder` 多 bot 通信决策（P1）

Craft-Agent 目前是单 bot，但未来若扩展多 bot 协作，这个 prompt 设计很值得参考：

```
Decide whether to 'respond' immediately or 'ignore' it and wait for your current action to finish.
Be conservative and only respond when necessary, like when you need to change/stop your action, or convey necessary information.
...
Decide by outputting ONLY 'respond' or 'ignore', nothing else. Your decision:
```

**核心可复用模式**：
- **二选一强制输出**：`ONLY 'respond' or 'ignore'`（单 token 决策，便于解析）
- **3 个 few-shot 嵌入 prompt**（不靠 embedding 检索，决策类 prompt 直接内联示例）
- **保守原则**：`Be conservative and only respond when necessary`

#### 6.3.3 任务 Profile + 协作 few-shot 扩库（P1）

Craft-Agent 已有 17 个 few-shot 示例（`prompt.rs` 行 21-182），但**缺少多 bot 协作示例**。可借鉴 `crafting_profile.json` 的 6 个协作示例模式：

```rust
// 新增示例：多 bot 协作合成
Example {
    keywords: &["collaborate", "协作", "give", "giveplayer", "trade", "合作"],
    turns: &[
        ("user", "philips: (FROM OTHER BOT) Hey! let's collaborate to build a stone pickaxe!"),
        ("assistant", "Cool, let me check what is in my inventory !inventory\nI have 10 oak logs. What do you have?"),
        ("user", "philips: (FROM OTHER BOT) Let me check !inventory \nI have 10 stones."),
        ("assistant", "Let me give you the stones and you can craft the pickaxe. !givePlayer(\"playername\", \"stone\", 10)"),
    ],
},
```

**注意**：Craft-Agent 当前没有 `givePlayer` 工具，需先实现该工具才能复用这些示例。

#### 6.3.4 任务场景知识注入（P1）

`cooking_profile.json` 的"场景知识注入"模式值得借鉴：

```
General Searching Tips:
- You will be spawned in a farm with many crops and animals nearby. The farm area is extensive - search thoroughly for needed resources (with searchForBlocks parameters like 64,128,256)
There is a crafting table, fully fueled furnace and fully fueled smoker with coal are also available nearby which you can use to your advantage.
```

**移植方案**：在 Craft-Agent 的 `WorldInfo` 库里新增"场景描述"类条目（常驻，无关键词触发）：

```rust
lib.add(WorldInfo::new(vec![], "场景：你出生在农场附近，有农作物和动物。熔炉已预热。"));
```

### 6.4 可选增强（P2）

#### 6.4.1 推理模型 Prompt 增强（P2）

Craft-Agent 当前用 `deepseek-chat`（无原生 thinking）。若改用推理模型，可借鉴 `andy-4-reasoning.json` 的"思考暗示"模式：

```
Think in high amounts before responding.
...
Reason before responding. Conversation Begin:
```

**适配建议**：
- 在 `agent.toml` 新增 `thinking_hint: bool` 配置
- 启用时自动在 system prompt 头尾插入思考暗示词
- 不启用时保持当前 prompt 不变（避免干扰 deepseek-chat）

#### 6.4.2 反幻觉正反例扩展（P2）

Mindcraft 的反幻觉设计很实用，可直接抄：

```
Don't pretend to act, use commands immediately when requested.
Do NOT say this: 'Sure, I've stopped. *stops*', instead say this: 'Sure, I'll stop. !stop'.
Do NOT say this: 'On my way! Give me a moment.', instead say this: 'On my way! !goToPlayer("playername", 3)'.
```

Craft-Agent 已有类似规则（`agent_loop.rs` 行 350：`禁止在 assistant 文字里写 tool(...) 伪调用`），但**缺少正反例对照**。可补充：

```
反例：'Sure, I'll goto. goto(x,y,z) → OK'（伪调用，不会执行）
正例：'Sure, I'll goto.' + function_call goto(x,y,z)（真实 tool_call）
```

#### 6.4.3 空响应 Tab 处理（P2）

Mindcraft 的 `If you have nothing to say or do, respond with an just a tab '\t'` 设计巧妙：
- 避免 LLM 强行编造回复
- 单字符响应便于检测和忽略

Craft-Agent 目前没有类似机制，可考虑加入（但需适配 function calling 场景，因为 Craft-Agent 要求"每轮必产出一个动作"）。

### 6.5 不需要移植的设计

| 设计 | 原因 |
|---|---|
| `coding` prompt（JS 代码生成） | Craft-Agent 用 rhai 脚本，不用 JS 代码执行 |
| `image_analysis` prompt | azalea 路线不用 VLM，perceive 返回结构化状态 |
| `goal_setting` prompt | 已废弃（Mindcraft 自己标记 deprecated） |
| `$CODE_DOCS` 变量 | Craft-Agent 用 OpenAI tool definitions，不需要文字代码文档 |
| embedding 检索 few-shot | Craft-Agent 用词重叠检索（离线可用），各有优劣，不切换 |
| `speak_model` TTS | Craft-Agent 不需要语音输出 |

### 6.6 优先级排序总表

| 优先级 | 移植项 | 工作量 | 收益 |
|---|---|---|---|
| **P0** | 三层 Profile 叠加机制（JSON 文件化） | 中 | 改 prompt 无需重编译 |
| **P0** | Modes 10 种布尔开关扩展 | 小 | 行为模式更精细 |
| **P0** | 4 种模式预设（survival/assistant/creative/god_mode） | 小 | 一键切换 bot 风格 |
| **P0** | 模板变量替换系统（12 个 $PLACEHOLDER） | 中 | prompt 与代码解耦 |
| **P1** | saving_memory 500 字符限制 prompt | 小 | 长期记忆更精炼 |
| **P1** | bot_responder 多 bot 通信 prompt | 中 | 为多 bot 协作铺路 |
| **P1** | 任务 Profile + 协作 few-shot 扩库 | 中 | 任务特化能力 |
| **P1** | 反幻觉正反例对照 | 小 | 减少 LLM 伪调用 |
| **P2** | 推理模型 thinking 暗示 | 小 | 推理模型适配 |
| **P2** | 空响应 Tab 处理 | 小 | 减少编造回复 |

### 6.7 移植路线图建议

**Phase 1（P0，1-2 天）**：
1. 在 `config/profiles/` 新建 `_default.json`，把当前 `agent_loop.rs` 行 346-383 的 system prompt 迁移过去
2. 实现 Rust 版 ProfileLoader（参考 prompter.js 行 17-43）
3. `agent.toml` 新增 `base_profile` 字段
4. 扩展 Modes 到 10 种

**Phase 2（P1，2-3 天）**：
5. 新增 4 种模式预设 profile
6. 实现 saving_memory prompt + 500 字符限制
7. 扩展 FEW_SHOT_EXAMPLES 加入协作示例（需先实现 givePlayer 工具）
8. 反幻觉正反例对照写入 system prompt

**Phase 3（P2，1 天）**：
9. 推理模型 thinking 暗示配置化
10. 空响应 Tab 检测（可选）

---

## 附录 A：Mindcraft 文件清单

| 文件路径 | 大小 | 用途 |
|---|---|---|
| `profiles/defaults/_default.json` | 19840 字节 | 主 system prompt 模板（6 种 prompt + 23 示例） |
| `profiles/defaults/assistant.json` | 13 行 | 助手模式（hunting=false） |
| `profiles/defaults/creative.json` | 13 行 | 创造模式（modes 大多关） |
| `profiles/defaults/god_mode.json` | 13 行 | 上帝模式（cheat=true） |
| `profiles/defaults/survival.json` | 13 行 | 生存模式（全开） |
| `profiles/tasks/construction_profile.json` | 41 行 | 建造任务（blueprint 协作） |
| `profiles/tasks/cooking_profile.json` | 10 行 | 烹饪任务（农场场景） |
| `profiles/tasks/crafting_profile.json` | 70 行 | 合成任务（6 协作示例） |
| `profiles/andy-4-reasoning.json` | 4052 字节 | 推理模型 prompt（思考暗示 + 中文输出） |
| `profiles/claude_thinker.json` | 14 行 | Claude thinking 模式（仅 model params） |
| `profiles/deepseek.json` | 6 行 | DeepSeek 极简配置 |
| `profiles/qwen.json` | 16 行 | Qwen 完整配置（含 URL） |
| `src/models/prompter.js` | 366 行 | Prompt 构建核心逻辑 |
| `settings.js` | 60 行 | 全局默认配置 |

## 附录 B：settings.js 关键配置

```javascript
const settings = {
    "minecraft_version": "auto",
    "host": "127.0.0.1",
    "port": 55916,
    "auth": "offline",
    "mindserver_port": 8080,
    "auto_open_ui": true,
    "base_profile": "assistant",           // 默认 base profile
    "profiles": ["./andy.json"],           // 个体 profile 列表
    "load_memory": false,
    "init_message": "Respond with hello world and your name",
    "only_chat_with": [],
    "speak": false,
    "chat_ingame": true,
    "language": "en",
    "render_bot_view": false,
    "allow_insecure_coding": false,        // 是否允许 newAction 写代码
    "allow_vision": false,
    "blocked_actions": ["!checkBlueprint", "!checkBlueprintLevel", "!getBlueprint", "!getBlueprintLevel"],
    "code_timeout_mins": -1,
    "relevant_docs_count": 5,              // $CODE_DOCS 检索数
    "max_messages": 15,                    // 上下文最大消息数
    "num_examples": 2,                     // $EXAMPLES 检索数
    "max_commands": -1,
    "show_command_syntax": "full",         // 命令文档详细度
    "narrate_behavior": true,
    "chat_bot_messages": true,
    "spawn_timeout": 30,
    "block_place_delay": 0,
    "log_all_prompts": false,              // 是否记录所有 prompt
};
```

**关键配置说明**：
- `num_examples: 2` — 每次给 LLM 的 few-shot 示例数（embedding 检索 top-2）
- `relevant_docs_count: 5` — 代码函数文档检索数
- `max_messages: 15` — 上下文窗口（Craft-Agent 是 10000 条，差距大）
- `blocked_actions` — 禁用的命令列表（从文档中移除）
- `show_command_syntax` — "full"/"shortened"/"none" 三档控制命令文档详细度

## 附录 C：prompter.js 关键方法签名

```javascript
class Prompter {
    constructor(agent, profile)              // 加载三层 profile
    getName()                                // 返回 bot 名字
    getInitModes()                           // 返回 modes 配置
    async initExamples()                     // 初始化 few-shot 示例库
    async replaceStrings(prompt, messages, examples, to_summarize, last_goals)
                                            // 12 个 $PLACEHOLDER 替换
    async checkCooldown()                    // LLM 调用冷却
    async promptConvo(messages)              // 主对话（3 次重试防幻觉）
    async promptCoding(messages)             // 代码生成
    async promptMemSaving(to_summarize)      // 记忆压缩
    async promptShouldRespondToBot(new_message)  // 多 bot 通信决策
    async promptVision(messages, imageBuffer)    // 视觉分析
    async promptGoalSetting(messages, last_goals) // 目标设置（已废弃）
    async _saveLog(prompt, messages, generation, tag)  // 日志记录
}
```

**`promptConvo` 的 3 次重试机制**（prompter.js 行 218-262）值得参考：
- 检测 `(FROM OTHER BOT)` 幻觉 → 重试
- 检测 `</think>` 标签 → 截取之后内容（兼容推理模型）
- 新消息到达时丢弃旧响应（`current_msg_time !== this.most_recent_msg_time`）

---

**报告完成。** 本报告已完整分析 14 份文件，包含所有 prompt 模板的原文摘录与可复用片段，可作为 Craft-Agent prompt 体系重构的直接参考。
