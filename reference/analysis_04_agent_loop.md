# Mindcraft Agent 主循环 / 自我提示 / 模式系统 / 记忆系统分析报告

> 源码路径：`d:\Craft-Agent\reference\mindcraft\src\agent\`
> 对照 Craft-Agent 当前实现：`d:\Craft-Agent\crates\craft-agent\src\agent\`
> 报告生成：2026-07-25

---

## 0. 总览：Mindcraft 的「事件驱动 + 单循环 + 自提示」三件套

Mindcraft 的 agent 与 Craft-Agent 在架构上**根本范式不同**：

| 维度 | Mindcraft | Craft-Agent |
|---|---|---|
| 主循环位置 | `handleMessage()` 单次响应循环 + `startEvents()` 后台 300ms tick | `run_one_turn()` 同步 13 步管线，外层 `continue_run()` for 循环 |
| 触发模型 | **事件驱动**（chat/whisper 触发 → max_responses 循环；idle 触发 → self_prompter loop） | **目标驱动**（一次性 `run(goal)`，迭代到 `max_iterations` 或 done） |
| 动作执行 | ActionManager 异步队列 + 超时 + 中断 + resume | ToolRegistry 同步执行 + ToolEffects 副作用分组并行 |
| 模式系统 | 10 个模式，每 tick 调用 `update()`，可直接 `execute()` 动作 + re-prompt LLM | 3 个模式，每轮注入 `[MODE: ...]` 提示，不直接执行动作 |
| 自提示 | SelfPrompter 独立 while 循环，STOPPED/ACTIVE/PAUSED 三态，可 pause/resume | 单一字段 `self_prompt: Option<String>`，每轮覆盖式注入 `[当前目标]` |
| 历史压缩 | max_messages 触发 → splice 5 条 → LLM 生成 500 字摘要覆盖 memory | 10_000 条 或 token 超预算 → LLM 摘要 + 三级回退 + 硬截断兜底 |
| 长期记忆 | MemoryBank（name→[x,y,z] 命名点位，极简） | WorldMemory（空间-状态结构化记忆，TTL/锚点/半径渲染） |
| 多 bot 对话 | ConversationManager 完整实现（队列/调度/超时/断线检测） | 无 |
| 断线恢复 | 7 类错误分类，isFatal 直接 process.exit(1)，依赖外部 supervisor | 无 |

---

## 1. Mindcraft Agent 主循环完整流程图

Mindcraft 的"主循环"实际由**两个并行轨道**组成：

### 轨道 A：消息响应循环（`handleMessage`，agent.js:254-382）

由 `bot.on('whisper'/'chat')` 触发。每收到一条消息执行一次。

```
[输入] source, message, max_responses
   │
   ▼
[1] checkTaskDone() —— 任务完成则 killAll()
   │
   ▼
[2] 强制命令检测（!commandName）—— 仅来自 user 的消息
   │  ├── 命令存在 → executeCommand + routeResponse → return
   │  └── 命令不存在 → routeResponse 报错 → return
   │
   ▼
[3] handleEnglishTranslation(message) —— 翻译为英文
   │
   ▼
[4] flushBehaviorLog() —— modes 累积的行为日志（≤500 字符）
   │  └── 非空 → history.add('system', 'Recent behaviors log: \n...')
   │
   ▼
[5] history.add(source, message) + history.save()
   │  └── 若 turns.length >= max_messages → splice 5 条做摘要
   │
   ▼
[6] 决定 max_responses：
   │  ├── 来自 user 且 self_prompter.isActive() → max_responses=1
   │  └── 否则保持原值（默认 settings.max_commands 或 Infinity）
   │
   ▼
[7] for (i=0; i<max_responses; i++) {
   │     │
   │     ▼
   │  [7a] checkInterrupt() —— self_prompter.shouldInterrupt || shut_up || responseScheduledFor
   │     │  └── true → break
   │     │
   │     ▼
   │  [7b] history = history.getHistory()  // 深拷贝 turns
   │     │
   │     ▼
   │  [7c] res = await prompter.promptConvo(history)  // 调 LLM
   │     │
   │     ▼
   │  [7d] res.trim()==='' → break（空响应结束循环）
   │     │
   │     ▼
   │  [7e] command_name = containsCommand(res)
   │     │
   │     ├── [有命令分支]：
   │     │     │  res = truncCommandMessage(res)  // 命令后的内容忽略
   │     │     │  history.add(this.name, res)
   │     │     │  if !commandExists → history.add('system', 'Command X does not exist') + continue
   │     │     │  if checkInterrupt() → break
   │     │     │  self_prompter.handleUserPromptedCmd(self_prompt, isAction)
   │     │     │  routeResponse(source, res)  // 按 show_command_syntax 配置
   │     │     │  execute_res = await executeCommand(this, res)
   │     │     │  used_command = true
   │     │     │  if execute_res → history.add('system', execute_res)
   │     │     │  else → break
   │     │     │  history.save()
   │     │     ▼
   │     │
   │     └── [纯对话分支]：
   │           history.add(this.name, res)
   │           routeResponse(source, res)
   │           break  // 对话响应结束循环
   │  }
   │
   ▼
[输出] used_command: bool
```

**核心原文摘录（agent.js:317-379）**：

```javascript
for (let i=0; i<max_responses; i++) {
    if (checkInterrupt()) break;
    let history = this.history.getHistory();
    let res = await this.prompter.promptConvo(history);

    console.log(`${this.name} full response to ${source}: ""${res}""`);

    if (res.trim().length === 0) {
        console.warn('no response')
        break; // empty response ends loop
    }

    let command_name = containsCommand(res);

    if (command_name) { // contains query or command
        res = truncCommandMessage(res); // everything after the command is ignored
        this.history.add(this.name, res);
        ...
        if (checkInterrupt()) break;
        this.self_prompter.handleUserPromptedCmd(self_prompt, isAction(command_name));
        ...
        let execute_res = await executeCommand(this, res);
        ...
        used_command = true;

        if (execute_res)
            this.history.add('system', execute_res);
        else
            break;
    }
    else { // conversation response
        this.history.add(this.name, res);
        this.routeResponse(source, res);
        break;
    }
    
    this.history.save();
}
```

### 轨道 B：后台 tick 循环（`startEvents`，agent.js:503-518）

```
[启动] setTimeout(INTERVAL=300ms) → while(true) {
   │
   ▼
[1] start = Date.now()
   │
   ▼
[2] await this.update(delta = start - last)
   │     │
   │     ├── bot.modes.update()       —— 所有模式每 tick 检查
   │     ├── self_prompter.update(delta) —— 自动重启 self-prompt loop
   │     └── checkTaskDone()
   │
   ▼
[3] remaining = INTERVAL - (Date.now() - start)
   │  └── remaining > 0 → sleep(remaining)
   │
   ▼
[4] last = start
   │
   ▼
   (循环)
```

**核心原文摘录（agent.js:502-518）**：

```javascript
// This update loop ensures that each update() is called one at a time, even if it takes longer than the interval
const INTERVAL = 300;
let last = Date.now();
setTimeout(async () => {
    while (true) {
        let start = Date.now();
        await this.update(start - last);
        let remaining = INTERVAL - (Date.now() - start);
        if (remaining > 0) {
            await new Promise((resolve) => setTimeout(resolve, remaining));
        }
        last = start;
    }
}, INTERVAL);

async update(delta) {
    await this.bot.modes.update();
    this.self_prompter.update(delta);
    await this.checkTaskDone();
}
```

### 轨道 C：SelfPrompter 自提示循环（self_prompter.js:56-87）

由 `task.setAgentGoal()` 启动，独立 while 循环：

```
[启动] startLoop() → while(!interrupt) {
   │
   ▼
[1] msg = `You are self-prompting with the goal: '${this.prompt}'. Your next response MUST contain a command...`
   │
   ▼
[2] used_command = await agent.handleMessage('system', msg, -1)
   │  └── 复用轨道 A 的循环（max_responses=-1 → Infinity）
   │
   ▼
[3] if !used_command:
   │     no_command_count++
   │     if no_command_count >= 3 → openChat("Agent did not use command...") + state=STOPPED + break
   │
   ▼
[4] else: no_command_count = 0; sleep(cooldown=2000ms)
   │
   ▼
   (循环)
```

### 关键设计要点

1. **三个循环解耦**：消息响应（A）是同步 for 循环；后台 tick（B）是 300ms 定时；自提示（C）是独立 while。三者通过 `bot.modes.update()` / `self_prompter.update()` / `agent.isIdle()` 协同。
2. **`max_responses` 控制单次响应链长度**：用户消息默认 `settings.max_commands`；self_prompter 调用时传 `-1`（Infinity）；用户在 self_prompt 活跃时发消息则强制 `max_responses=1`（让 self_prompt 接管）。
3. **空响应即退出**：`res.trim()===''` 直接 break，避免无限循环。
4. **命令执行后必有 system 反馈**：`execute_res` 为空也 break（命令未产生输出，视为完成）。
5. **错误恢复**：`bot.on('death')` → `actions.cancelResume() + actions.stop()`；死亡位置写入 `memory_bank.rememberPlace('last_death_position', ...)` + 自动发 system 消息告知 LLM。

---

## 2. ActionManager 执行模型详解

源文件：`action_manager.js`（177 行）

### 2.1 状态字段

```javascript
this.executing = false;           // 是否有动作在执行
this.currentActionLabel = '';     // 当前动作标签（如 "mode:self_defense" / "!goto"）
this.currentActionFn = null;      // 当前动作函数
this.timedout = false;            // 是否因超时结束
this.resume_func = null;          // 可恢复动作的函数
this.resume_name = '';            // 可恢复动作的名字
this.last_action_time = 0;        // 上次动作时间戳（用于快速循环检测）
this.recent_action_counter = 0;   // 快速动作计数器
```

### 2.2 队列模型：**单槽串行 + 抢占式中断**

ActionManager **没有队列**——任意时刻最多一个动作在执行（`executing` 布尔）。新动作来了先 `await this.stop()` 强杀当前动作，再执行新的。

```javascript
async _executeAction(actionLabel, actionFn, timeout = 10) {
    ...
    if (this.executing) {
        console.log(`action "${actionLabel}" trying to interrupt current action "${this.currentActionLabel}"`);
    }
    await this.stop();  // 强杀当前动作（10s 超时 cleanKill）
    this.agent.clearBotLogs();
    this.executing = true;
    this.currentActionLabel = actionLabel;
    ...
}
```

### 2.3 并发控制：`stop()` 的暴力终止

```javascript
async stop() {
    if (!this.executing) return;
    const timeout = setTimeout(() => {
        this.agent.cleanKill('Code execution refused stop after 10 seconds. Killing process.');
    }, 10000);
    while (this.executing) {
        this.agent.requestInterrupt();  // bot.interrupt_code = true + stopDigging + cancelBlock + pathfinder.stop + pvp.stop
        console.log('waiting for code to finish executing...');
        await new Promise(resolve => setTimeout(resolve, 300));
    }
    clearTimeout(timeout);
}
```

- 每 300ms 调一次 `requestInterrupt()`（设置 `bot.interrupt_code=true` + 停所有 mineflayer 子系统）
- 10 秒未停下 → `cleanKill('Code execution refused stop after 10 seconds')` 进程退出

### 2.4 超时机制

```javascript
_startTimeout(TIMEOUT_MINS = 10) {
    return setTimeout(async () => {
        console.warn(`Code execution timed out after ${TIMEOUT_MINS} minutes. Attempting force stop.`);
        this.timedout = true;
        this.agent.history.add('system', `Code execution timed out after ${TIMEOUT_MINS} minutes. Attempting force stop.`);
        await this.stop(); // last attempt to stop
    }, TIMEOUT_MINS * 60 * 1000);
}
```

- **超时单位是分钟**（默认 10 分钟）
- 超时后设置 `timedout=true` + 写 history 警告 + 调 `stop()`
- **注意**：超时回调本身没有再上一层 cleanKill，若 `stop()` 也卡住会无限挂起

### 2.5 中断（interrupt）

`requestInterrupt()` 在 agent.js:233-239：

```javascript
requestInterrupt() {
    this.bot.interrupt_code = true;     // 让 actionFn 内部检查此标志自行退出
    this.bot.stopDigging();             // mineflayer-pathfinder
    this.bot.collectBlock.cancelTask(); // mineflayer-collectblock
    this.bot.pathfinder.stop();
    this.bot.pvp.stop();
}
```

**协作式中断**：`interrupt_code` 是软标志，actionFn 需自行检查；mineflayer 子系统是硬停止。

### 2.6 Resume 机制（可恢复动作）

```javascript
async _executeResume(actionLabel = null, actionFn = null, timeout = 10) {
    const new_resume = actionFn != null;
    if (new_resume) {
        this.resume_func = actionFn;
        this.resume_name = actionLabel;
    }
    if (this.resume_func != null
        && (this.agent.isIdle() || new_resume)
        && (!this.agent.self_prompter.isActive() || new_resume)) {
        // 执行 resume_func
        let res = await this._executeAction(this.resume_name, this.resume_func, timeout);
        return res;
    } else {
        return { success: false, message: null, interrupted: false, timedout: false };
    }
}
```

- `resume_func` 在 `bot.on('idle')` 事件中通过 `actions.resumeAction()` 自动恢复
- 死亡时 `actions.cancelResume()` 清除（避免死亡后继续原动作）
- **触发条件**：bot 空闲 + self_prompter 未激活（避免与自提示冲突）

### 2.7 快速循环检测（防死循环）

```javascript
if (this.last_action_time > 0) {
    let time_diff = Date.now() - this.last_action_time;
    if (time_diff < 20) {                // 20ms 内连续动作
        this.recent_action_counter++;
    } else {
        this.recent_action_counter = 0;
    }
    if (this.recent_action_counter > 3) {
        console.warn('Fast action loop detected, cancelling resume.');
        this.cancelResume();
    }
    if (this.recent_action_counter > 5) {
        console.error('Infinite action loop detected, shutting down.');
        this.agent.cleanKill('Infinite action loop detected, shutting down.');
    }
}
```

- **20ms 内连续 4 次** → 取消 resume
- **20ms 内连续 6 次** → cleanKill 进程退出

### 2.8 输出汇总（getBotOutputSummary）

```javascript
getBotOutputSummary() {
    const { bot } = this.agent;
    if (bot.interrupt_code && !this.timedout) return '';
    let output = bot.output;
    const MAX_OUT = 500;
    if (output.length > MAX_OUT) {
        output = `Action output is very long (${output.length} chars) and has been shortened.\n
      First outputs:\n${output.substring(0, MAX_OUT / 2)}\n...skipping many lines.\nFinal outputs:\n ${output.substring(output.length - MAX_OUT / 2)}`;
    }
    ...
}
```

- 超过 500 字符：保留前 250 + 后 250，中间省略
- 这与 Craft-Agent 的 `format!("{:.120}", msg)`（截断到 120 字符）思路一致但更宽松

---

## 3. SelfPrompter 目标管理

源文件：`self_prompter.js`（146 行）

### 3.1 三态状态机

```javascript
const STOPPED = 0
const ACTIVE  = 1
const PAUSED  = 2
```

| 状态 | 含义 | 进入条件 |
|---|---|---|
| STOPPED | 未自提示 | 初始 / `stop()` / 连续 3 次无命令 |
| ACTIVE | 自提示循环运行中 | `start(prompt)` |
| PAUSED | 暂停（可恢复） | `pause()`（被对话打断） |

### 3.2 注入时机：**每轮发新 system 消息**

```javascript
async startLoop() {
    ...
    while (!this.interrupt) {
        const msg = `You are self-prompting with the goal: '${this.prompt}'. Your next response MUST contain a command with this syntax: !commandName. Respond:`;
        let used_command = await this.agent.handleMessage('system', msg, -1);
        ...
    }
}
```

- **每次循环都重新发一条 system 消息**，不是改 system prompt（避免破坏 prefix cache 概念，虽然 Mindcraft 没考虑这个）
- 消息模板强调"MUST contain a command"，逼 LLM 产出可执行动作
- `max_responses=-1` → Infinity，让 LLM 在单次 system 触发内可执行多步命令链

### 3.3 目标漂移检测：**无显式检测，靠每轮重注入**

Mindcraft **不做**目标漂移检测。它的策略是简单粗暴的"每轮重注入同样的 prompt"——因为 prompt 字符串完全不变，LLM 不会丢失目标。

对比 Craft-Agent 的 `mod.rs:626-631`：

```rust
// SelfPrompter
if self.config.enable_self_prompt
    && let Some(prompt) = &self.self_prompt
{
    self.messages
        .push(Message::user(format!("[当前目标] {prompt}")));
}
```

两者思路一致（每轮重注入），但 Craft-Agent 用 user 消息且加了 `[当前目标]` 标签便于覆盖式清理。

### 3.4 结束判定：**3 次无命令即停**

```javascript
let no_command_count = 0;
const MAX_NO_COMMAND = 3;
while (!this.interrupt) {
    let used_command = await this.agent.handleMessage('system', msg, -1);
    if (!used_command) {
        no_command_count++;
        if (no_command_count >= MAX_NO_COMMAND) {
            let out = `Agent did not use command in the last ${MAX_NO_COMMAND} auto-prompts. Stopping auto-prompting.`;
            this.agent.openChat(out);
            this.state = STOPPED;
            break;
        }
    }
    else {
        no_command_count = 0;
        await new Promise(r => setTimeout(r, this.cooldown));  // 2s 冷却
    }
}
```

- LLM 连续 3 次回复纯对话（无 `!command`）→ 视为目标完成或卡死，停止自提示
- 每次成功执行命令后 2s 冷却（避免狂暴连击）

### 3.5 自动重启（update）

```javascript
update(delta) {
    if (this.state === ACTIVE && !this.loop_active && !this.interrupt) {
        if (this.agent.isIdle())
            this.idle_time += delta;
        else
            this.idle_time = 0;

        if (this.idle_time >= this.cooldown) {
            console.log('Restarting self-prompting...');
            this.startLoop();
            this.idle_time = 0;
        }
    }
}
```

- loop 异常退出（如被 modes 中断）后，bot 空闲超过 2s 自动重启 loop
- 由轨道 B 的 300ms tick 调用

### 3.6 与对话/模式的协同

| 事件 | 对 SelfPrompter 的影响 |
|---|---|
| 用户发消息 | `max_responses=1`，自提示不停止；若 LLM 回复含 action → `handleUserPromptedCmd` 调 `stopLoop()` |
| 其他 bot 发消息（startConversation） | `self_prompter.pause()` |
| 对话结束 | `_resumeSelfPrompter()` 5s 后恢复 |
| Mode 触发 execute() | `self_prompter.stopLoop()` |
| bot 死亡 | `actions.cancelResume() + actions.stop()`（间接清空 resume_func） |

---

## 4. Modes.js 所有模式清单 + 触发条件 + 优先级

源文件：`modes.js`（446 行）

### 4.1 模式清单（顺序即优先级，先列先处理）

| # | 模式名 | 描述 | interrupts | 默认 on | 触发条件 | 动作 |
|---|---|---|---|---|---|---|
| 1 | `self_preservation` | 溺水/着火/低血量应急 | `['all']` | ON | 头顶是水 / 头顶是下落方块 / 脚下或头顶是 lava/fire / 3s 内受伤且 health<5 或 lastDamageTaken>=health | setControlState('jump') / moveAway(2) / 放水桶或找水或 moveAway(5) / moveAway(20) |
| 2 | `unstuck` | 卡住脱困 | `['all']` | ON | **bot 非 idle** + 位置 2 格内停留 >20s（挖黑曜石时 40s）+ 挖的方块未变 | moveAway(5)，10s 未脱困 cleanKill |
| 3 | `cowardice` | 见敌逃跑 | `['all']` | ON | 16 格内有敌对实体 + 路径清晰 | avoidEnemies(24) |
| 4 | `self_defense` | 近敌攻击 | `['all']` | ON | 8 格内有敌对实体 + 路径清晰 | defendSelf(8) |
| 5 | `hunting` | 狩猎动物 | `['action:followPlayer']` | ON | 8 格内有可猎动物（牛/猪/羊/鸡...）+ 路径清晰 | attackEntity |
| 6 | `item_collecting` | 拾取掉落物 | `['action:followPlayer']` | ON | 8 格内有 item 实体 + 背包空位>1 + 注意到 2s 后 | pickupNearbyItems |
| 7 | `torch_placing` | 放火把照明 | `['action:followPlayer']` | ON | shouldPlaceTorch(bot) + 距上次放置 >5s | placeBlock('torch') |
| 8 | `elbow_room` | 推开近身玩家 | `['action:followPlayer']` | ON | 0.5 格内有 player 实体 | 随机等 0-1s + moveAwayFromEntity(0.5) |
| 9 | `idle_staring` | 空闲时看附近实体（动画） | `[]` | ON | 空闲 + 10 格内有非 enderman 实体 | bot.lookAt + 随机环顾 |
| 10 | `cheat` | 作弊（瞬放/瞬移） | `[]` | OFF | （空实现） | — |

### 4.2 模式字段语义

```javascript
{
    name: 'self_defense',
    description: '...',
    interrupts: ['all'],        // 可中断哪些动作标签：'all' 或 ['action:followPlayer', ...]
    on: true,                   // 是否启用（LLM 可通过 ModeController.setOn 切换）
    active: false,              // 当前是否正在执行动作（执行期间不再触发）
    paused: false,              // 是否被其他动作暂停（如 followPlayer 自带 self_defense）
    update: async function(agent) { ... },  // 每 tick 调用
    unpause: function() { ... } // 可选：unpause 时重置内部状态
}
```

### 4.3 优先级与互斥（ModeController.update）

```javascript
async update() {
    if (_agent.isIdle()) {
        this.unPauseAll();     // 空闲时全部解暂停
    }
    for (let mode of modes_list) {
        let interruptible = mode.interrupts.some(i => i === 'all')
                         || mode.interrupts.some(i => i === _agent.actions.currentActionLabel);
        if (mode.on && !mode.paused && !mode.active
            && (_agent.isIdle() || interruptible)) {
            await mode.update(_agent);
        }
        if (mode.active) break;  // ★ 一旦某模式触发动作，后续模式本 tick 不再检查
    }
}
```

**关键设计**：
- **顺序即优先级**：`modes_list` 数组顺序决定检查顺序，self_preservation 永远先于 self_defense
- **`active` 短路**：任一模式触发 `execute()` 后立即 break，本 tick 不再触发其他模式
- **`interrupts` 控制**：仅 `interrupts.includes('all')` 或 `interrupts.includes(currentActionLabel)` 的模式能在 bot 非 idle 时触发
- **`paused` 覆盖**：被 pause 的模式完全不检查（如 followPlayer 自带 self_defense 时 pause 掉 modes.self_defense）

### 4.4 `execute()` 函数：动作执行 + 自动 re-prompt

```javascript
async function execute(mode, agent, func, timeout=-1) {
    if (agent.self_prompter.isActive())
        agent.self_prompter.stopLoop();              // 中断自提示循环
    let interrupted_action = agent.actions.currentActionLabel;
    mode.active = true;
    let code_return = await agent.actions.runAction(`mode:${mode.name}`, async () => {
        await func();
    }, { timeout });
    mode.active = false;

    let should_reprompt = 
        interrupted_action &&          // 它打断了一个正在执行的动作
        !agent.actions.resume_func &&  // 没有待恢复动作
        !agent.self_prompter.isActive() &&  // 自提示未激活
        !code_return.interrupted;      // 此模式动作本身没被打断

    if (should_reprompt) {
        // 自动发消息让 LLM 应对中断
        let role = convoManager.inConversation() ? agent.last_sender : 'system';
        let logs = agent.bot.modes.flushBehaviorLog();
        agent.handleMessage(role, `(AUTO MESSAGE)Your previous action '${interrupted_action}' was interrupted by ${mode.name}.
        Your behavior log: ${logs}\nRespond accordingly.`);
    }
}
```

**核心设计**：
- 模式动作执行 → **自动调 LLM 重新决策**（无需用户介入）
- `interrupted_action` 让 LLM 知道原本在做什么
- `behavior_log` 提供 mode 的发言记录（"I'm on fire!" / "Fighting zombie!"）
- 这是 Mindcraft 的"反应式"核心：mode 触发 → 动作 → LLM 重规划

### 4.5 ModeController 暴露给 LLM

```javascript
class ModeController {
    exists(mode_name) { ... }
    setOn(mode_name, on) { ... }      // LLM 可关掉某模式
    isOn(mode_name) { ... }
    pause(mode_name) { ... }          // LLM 可暂停某模式
    unpause(mode_name) { ... }
    unPauseAll() { ... }
    getMiniDocs() { ... }             // 给 LLM 看的精简文档
    getDocs() { ... }                 // 给 LLM 看的完整文档
    getJson() / loadJson() { ... }    // 持久化
}
```

**LLM 可在执行中动态调整模式开关**——这是 Mindcraft 的"可调反射"设计：反射动作（mode）默认开启，但 LLM 可基于场景关闭（如建造时关掉 hunting 避免乱跑）。

### 4.6 行为日志（behavior_log）

```javascript
async function say(agent, message) {
    agent.bot.modes.behavior_log += message + '\n';
    if (agent.shut_up || !settings.narrate_behavior) return;
    agent.openChat(message);
}
```

- 所有 mode 的发言累积到 `behavior_log`
- `flushBehaviorLog()` 在 `handleMessage` 入口被调用，作为 system 消息注入 history
- 即便 `shut_up=true` 不发公共聊天，日志仍写入 history 让 LLM 知道发生了什么

---

## 5. History 压缩策略

源文件：`history.js`（121 行）

### 5.1 数据结构

```javascript
this.turns = [];                  // 当前上下文消息数组 {role, content}
this.memory = '';                 // 自然语言摘要（≤500 字符）
this.max_messages = settings.max_messages;     // 触发压缩的消息数阈值
this.summary_chunk_size = 5;      // 每次拿 5 条做摘要
this.memory_fp = `./bots/${name}/memory.json`;
this.full_history_fp = undefined; // 完整历史归档文件（首次压缩时创建）
```

### 5.2 压缩触发条件

**单一阈值**：`turns.length >= max_messages`（来自 settings，无 token 估算）。

对比 Craft-Agent 双触发：`messages.len() >= 10_000` **或** `estimate_tokens() > context_window - reserve`。

### 5.3 压缩算法（history.add 内联）

```javascript
async add(name, content) {
    ...
    this.turns.push({role, content});

    if (this.turns.length >= this.max_messages) {
        let chunk = this.turns.splice(0, this.summary_chunk_size);  // 拿最旧 5 条
        while (this.turns.length > 0 && this.turns[0].role === 'assistant')
            chunk.push(this.turns.shift());  // 移除开头连续的 assistant 直到 user/system

        await this.summarizeMemories(chunk);      // 生成新 memory
        await this.appendFullHistory(chunk);      // 归档到完整历史文件
    }
}
```

**关键点**：
- **每次只拿 5 条**做摘要（不是 Craft-Agent 那样按 token 切分）
- **chunk 边界修正**：移除开头连续的 assistant 消息，确保 chunk 末尾是 user/system（避免 assistant 消息被孤立）
- **摘要覆盖式更新**：`summarizeMemories(chunk)` 直接 `this.memory = await promptMemSaving(turns)`，**不是追加**

### 5.4 摘要生成（summarizeMemories）

```javascript
async summarizeMemories(turns) {
    console.log("Storing memories...");
    this.memory = await this.agent.prompter.promptMemSaving(turns);

    if (this.memory.length > 500) {
        this.memory = this.memory.slice(0, 500);
        this.memory += '...(Memory truncated to 500 chars. Compress it more next time)';
    }
}
```

- 调 LLM 生成摘要（`promptMemSaving` 在 prompter.js，本报告未读）
- **500 字符硬截断**（不像 Craft-Agent 的 `keep_recent: 200_000` tokens 保留近期完整消息）
- 截断时附加提示让下次压缩更狠

### 5.5 完整历史归档

```javascript
async appendFullHistory(to_store) {
    if (this.full_history_fp === undefined) {
        const string_timestamp = new Date().toLocaleString()...;
        this.full_history_fp = `./bots/${this.name}/histories/${string_timestamp}.json`;
        writeFileSync(this.full_history_fp, '[]', 'utf8');
    }
    const data = readFileSync(this.full_history_fp, 'utf8');
    let full_history = JSON.parse(data);
    full_history.push(...to_store);
    writeFileSync(this.full_history_fp, JSON.stringify(full_history, null, 4), 'utf8');
}
```

- 每次压缩的 chunk 同时归档到 `histories/<timestamp>.json`
- **仅追加，不读回**——归档纯粹是为了 debug/审计，不参与上下文

### 5.6 持久化（save/load）

```javascript
async save() {
    const data = {
        memory: this.memory,                              // 摘要
        turns: this.turns,                                // 当前上下文
        self_prompting_state: this.agent.self_prompter.state,
        self_prompt: this.agent.self_prompter.isStopped() ? null : this.agent.self_prompter.prompt,
        taskStart: this.agent.task.taskStartTime,
        last_sender: this.agent.last_sender
    };
    writeFileSync(this.memory_fp, JSON.stringify(data, null, 2));
}
```

- **整 agent 状态快照**：history + self_prompter 状态 + task 时间 + last_sender
- 加载时 `self_prompter.handleLoad(save_data.self_prompt, save_data.self_prompting_state)` 恢复

### 5.7 与 Craft-Agent 压缩策略对比

| 维度 | Mindcraft | Craft-Agent |
|---|---|---|
| 触发条件 | 单一消息数阈值 | 消息数 **或** token 估算双触发 |
| Token 估算 | **无** | 实测优先（累加 usage.total_tokens）+ 尾部启发式（CHARS_PER_TOKEN=2 + IMAGE_TOKENS=1200） |
| 切分粒度 | 固定 5 条/次 | 按 `keep_recent` tokens 反向累加 |
| 摘要模型 | 主模型（promptMemSaving） | 专用 Agnes-2.0-flash → 主模型 → 硬截断 三级回退 |
| 摘要长度 | 500 字符硬截断 | 无硬限制，保留 `keep_recent: 200_000` tokens 近期消息 |
| 摘要累积 | **覆盖**（每次新生成替换旧 memory） | **增量更新**（`<previous-summary>` + `UPDATE_SUMMARIZATION_PROMPT`） |
| 易变消息处理 | 无特殊处理 | 序列化旧历史时**剔除** perceive 快照 / 邻近记忆（避免过期坐标污染摘要） |
| 边界修正 | 移除开头连续 assistant | 反向扫描确保 assistant(tool_calls) 与 ToolResult 不分离 |
| 失败兜底 | 无（LLM 失败就抛错） | 硬截断兜底 + 注入系统提示告知 LLM 上下文已截断 |
| 完整历史归档 | 是（histories/<ts>.json） | 是（Session JSONL + Checkpoint） |

**关键差距**：Mindcraft 的 memory 是**覆盖式**，每次压缩把旧摘要直接扔掉换成新的；Craft-Agent 是**增量更新**，旧摘要作为 `<previous-summary>` 上下文给 LLM 参考。这意味着 Mindcraft 长时间运行后早期信息会完全丢失，Craft-Agent 保留累积摘要。

---

## 6. MemoryBank 记忆机制

源文件：`memory_bank.js`（仅 25 行）

### 6.1 完整实现

```javascript
export class MemoryBank {
    constructor() {
        this.memory = {};
    }

    rememberPlace(name, x, y, z) {
        this.memory[name] = [x, y, z];
    }

    recallPlace(name) {
        return this.memory[name];
    }

    getJson() { return this.memory }
    loadJson(json) { this.memory = json; }
    getKeys() { return Object.keys(this.memory).join(', '); }
}
```

### 6.2 设计特点

- **极简**：仅 `name → [x,y,z]` 命名点位存储
- **无检索**：只有 `recallPlace(name)` 精确查找，无关键词/嵌入向量检索
- **无注入**：MemoryBank 不主动注入 prompt，需 LLM 通过 `!recallPlace` 命令查询
- **无 TTL**：记下来就永久存在（除非整体 loadJson 覆盖）

### 6.3 实际使用

在 agent.js 中仅一处使用：

```javascript
this.bot.on('messagestr', async (message, _, jsonMsg) => {
    if (jsonMsg.translate && jsonMsg.translate.startsWith('death') && message.startsWith(this.name)) {
        let death_pos = this.bot.entity.position;
        this.memory_bank.rememberPlace('last_death_position', death_pos.x, death_pos.y, death_pos.z);
        ...
        this.handleMessage('system', `You died at position ${death_pos_text}... Your place of death is saved as 'last_death_position' if you want to return...`);
    }
});
```

仅记录死亡位置。LLM 可通过 `!rememberPlace("home", x, y, z)` / `!recallPlace("home")` 命令手动管理其他点位。

### 6.4 与 Craft-Agent WorldMemory 对比

| 维度 | Mindcraft MemoryBank | Craft-Agent WorldMemory |
|---|---|---|
| 数据结构 | `{name: [x,y,z]}` 扁平字典 | 结构化 `MemoryEntry {pos, kind, label, ttl, depleted, ...}` |
| 记忆类型 | 仅命名点位 | Resource / Structure / Container / Entity / Hazard / Portal / Note |
| 自动记录 | 无（仅死亡位置） | handler 每 20 tick 自动 `record_surroundings` 扫描 8 格半径 |
| 检索方式 | 精确 key 查找 | 锚点 + 半径空间检索（`render_nearby(pos, 64)`） |
| 注入 prompt | 不主动注入（LLM 显式查询） | 每轮自动渲染 `__self__` 锚点周边 64 格注入 user 消息 |
| TTL / 过期 | 无 | 30s TTL，资源 depleted 标记，结构消失自动遗忘 |
| 工具暴露 | `!rememberPlace` / `!recallPlace` / `!getMemoryBank` | `memory(action=save/anchor/query/forget, ...)` 工具 |

**结论**：Mindcraft 的 MemoryBank 是**手动查询式**的极简点位记忆；Craft-Agent 的 WorldMemory 是**自动注入式**的空间-状态结构化记忆。后者在主动性、丰富度、上下文相关性上都远超前者。

---

## 7. Craft-Agent 移植对比

### 7.1 主循环对比：Craft-Agent 13 步 vs Mindcraft 三轨道

#### Craft-Agent 当前主循环（mod.rs::run_one_turn，13 步）

```
[1] drain_queues() — steering / follow_up 队列
[2] 压缩检查 — messages.len() >= 10_000 或 estimate_tokens() > budget
[3] 易变注入清理 — retain 移除上一轮的 perceive / 邻近记忆 / [当前目标]
[4] auto_perceive — 调 perceive 工具注入结构化状态快照
[5] modes 反应 — check_modes() 注入 [MODE: ...] user 消息
[6] SelfPrompter — 重新注入 [当前目标]（覆盖式）
[7] 动态上下文 — WorldInfo scan + Skill 示例 + Few-shot（词重叠 top-2）+ obs 警告
[8] WorldMemory 邻近记忆 — __self__ 锚点周边 64 格
[9] 动态指令 — knowledge_bootstrapped / obs_streak 警告
[10] LLM complete — RetryConfig 退避重试 + retry_abort 用户中止
[11] 纯文字回复检测 — 伪调用识别 + 续跑 nudge
[12] 死循环检测 — recent_calls 容量 10，4+ 重复注入 nudge（在 tool result 之后）
[13] 并行执行工具 — ToolEffects 副作用分组（BARRIER 切批），批内 std::thread::scope 并行
[14] 技能抽取 — 非 obs 工具调用提取 SkillLibrary 经验
```

#### 差距分析

| 维度 | Craft-Agent | Mindcraft | 差距 |
|---|---|---|---|
| 循环结构 | 单一同步 for 循环 | 三轨道并行（消息响应 + 300ms tick + self_prompt loop） | Craft-Agent **无后台 tick**，模式反应只能挤在每轮 LLM 调用前 |
| 触发模型 | 目标驱动（一次性 run(goal)） | 事件驱动（chat 触发 + self_prompt 持续推进） | Craft-Agent 无外部消息触发能力 |
| 单轮 LLM 调用次数 | 1 次（多工具并行） | 1~max_responses 次（每命令一次 LLM） | Mindcraft 命令链可单轮多 LLM 调用，Craft-Agent 一轮一调 |
| 动作执行 | 同步 ToolRegistry + ToolEffects 分批并行 | 异步 ActionManager + 单槽串行 + 抢占式中断 | Craft-Agent 无中断/超时/resume 机制 |
| 中断机制 | `retry_abort: AtomicBool`（仅中止 LLM 重试） | `requestInterrupt()` + 10s 强杀 + `cleanKill` | Craft-Agent 无法中断正在执行的工具 |
| 死亡/断线恢复 | 无 | `bot.on('death')` cancelResume + stop + 自动 system 消息 | Craft-Agent 无运行时事件恢复 |
| 任务完成检测 | `done: bool` 返回 | `task.isDone()` 每 tick 检查 | Craft-Agent 仅在轮末判断 |

### 7.2 Modes 系统对比

#### Craft-Agent modes.rs 当前实现

```rust
pub fn check_modes(&mut self) -> Option<String> {
    let perception = self.messages.iter().rev().find_map(|m| match m {
        Message::User(u) if u.content.starts_with("【当前游戏状态") => Some(u.content.as_str()),
        _ => None,
    })?;

    let health_low = (0..=6).any(|n| perception.contains(&format!("生命: {n}/")));
    let hunger_low = (0..=6).any(|n| perception.contains(&format!("饱食: {n}/")));
    if health_low || hunger_low {
        if self.last_mode_trigger != 1 {
            self.last_mode_trigger = 1;
            return Some(format!("[MODE: self_preservation] {action}"));
        }
        return None;
    }
    // ... self_defense / unstuck ...
}
```

#### 对比表

| 维度 | Craft-Agent modes.rs | Mindcraft modes.js |
|---|---|---|
| 模式数量 | 3（self_preservation / self_defense / unstuck） | 10（含 cowardice / hunting / item_collecting / torch_placing / elbow_room / idle_staring / cheat） |
| 触发时机 | 每轮 LLM 调用前（同步） | 每 tick 300ms（异步，独立于 LLM） |
| 触发依据 | perceive 文本字符串匹配 | bot 实时状态（blockAt / entity / health / position） |
| 执行方式 | **仅注入 `[MODE: ...]` 提示给 LLM**，不直接动作 | 调 `execute()` 直接执行 mineflayer 动作 |
| 优先级 | 无（一次只触发一个，去重） | 数组顺序，`active` 短路 |
| 互斥/暂停 | 无 | `paused` / `interrupts` 精细控制 |
| LLM 可调 | 无 | `ModeController.setOn/pause/unpause` 暴露给 LLM |
| 中断恢复 | 无 | mode 打断动作后自动 re-prompt LLM |
| 行为日志 | 无 | `behavior_log` 累积 + flush 到 history |
| 双层执行 | agent 层（提示） + handler 层（Tick 直接动作） | 单层（mode.update 直接 execute） |

**关键差距**：
1. Craft-Agent 的 modes 是**建议性**的（只发提示），Mindcraft 的 modes 是**强制性**的（直接执行动作）
2. Craft-Agent 的 modes 触发**仅靠 perceive 文本**，Mindcraft 触发**直接读 bot 状态**（更实时、更准确）
3. Craft-Agent 没有**中断 re-prompt**机制——mode 触发后 LLM 不知道发生了什么
4. Craft-Agent 的 modes 不可被 LLM 动态开关

**Craft-Agent 已有的"双层 modes"设计**（AGENTS.md 提到）：
- Agent 层（modes.rs）：每轮检查 perceive，注入提示
- handler 层（azalea/mod.rs Tick）：直接执行 self_preservation / self_defense 动作

这其实**已经接近 Mindcraft 的设计**——handler 层 Tick 等价于 Mindcraft 的 `bot.modes.update()`。差距在于：
- handler 层只有 2 个模式（self_preservation / self_defense），Mindcraft 有 10 个
- handler 层触发后**不 re-prompt LLM**（Mindcraft 会调 `handleMessage` 让 LLM 重新决策）
- 缺 `cowardice` / `hunting` / `item_collecting` / `torch_placing` / `elbow_room` / `unstuck` 等

### 7.3 SelfPrompter 对比

| 维度 | Craft-Agent | Mindcraft |
|---|---|---|
| 数据结构 | `self_prompt: Option<String>` 单字段 | `SelfPrompter` 类，三态状态机 + loop_active + interrupt + idle_time |
| 注入方式 | 每轮 `Message::user("[当前目标] {prompt}")` 覆盖式重注入 | 每轮 `handleMessage('system', "You are self-prompting with the goal: ...")` |
| 循环结构 | 无独立循环（嵌入 run_one_turn） | 独立 while 循环（`startLoop`） |
| 状态管理 | 无（只有有/无 goal） | STOPPED / ACTIVE / PAUSED 三态 |
| pause/resume | 无 | `pause()` / `_resumeSelfPrompter()`（对话结束后 5s 恢复） |
| 结束判定 | 无（一直跑到 max_iterations） | 连续 3 次无命令自动停止 |
| 自动重启 | 无 | `update(delta)`：idle_time >= cooldown 自动 startLoop |
| 与对话协同 | 无 | 用户消息时 `max_responses=1`；其他 bot 对话时 pause |
| 与 modes 协同 | 无 | mode execute 时 `stopLoop()` |
| 冷却 | 无 | 2s cooldown 避免狂暴连击 |

**关键差距**：
1. Craft-Agent 的 SelfPrompter **不是独立循环**，只是每轮注入目标文本——没有"连续无命令停止"机制，会一直跑到 max_iterations
2. Craft-Agent **无 pause/resume**——对话打断会丢失自提示上下文
3. Craft-Agent **无冷却**——LLM 可能狂暴连击工具
4. Craft-Agent **无结束判定**——无法自动判断目标完成

### 7.4 MemoryBank vs WorldMemory

见 §6.4。**Craft-Agent 的 WorldMemory 在所有维度上都优于 Mindcraft 的 MemoryBank**——这是 Craft-Agent 唯一明显领先的部分。无需移植 Mindcraft 的 MemoryBank。

### 7.5 Craft-Agent 应该抄过来的架构理念

按优先级排序：

#### P0（核心架构缺失）：

1. **ActionManager 风格的动作执行层**
   - 当前 Craft-Agent 的 ToolRegistry 是同步执行，无超时、无中断、无 resume
   - 应移植：超时机制（分钟级）、`requestInterrupt` 协作式中断、快速循环检测（20ms 内 4+ 次取消 resume，6+ 次 abort）、输出汇总（500 字符前后保留）
   - **适配**：AzaleaBot 已有"命令队列 + 200 tick 超时"机制（AGENTS.md 提到），但只在 handler 层；agent 层的 ToolRegistry 调用没有超时
   - 实现位置：`crates/craft-agent/src/core/tool.rs` 的 `ToolRegistry::execute` 包装一层 `ActionManager`

2. **Mode 触发后的 re-prompt 机制**
   - 当前 Craft-Agent 的 handler 层 mode 触发动作后，LLM 完全不知道
   - 应移植：mode 触发 → 注入 system 消息 `(AUTO MESSAGE)Your previous action '...' was interrupted by ${mode.name}. Respond accordingly.`
   - 实现位置：`crates/craft-agent-minecraft/src/azalea/mod.rs` 的 handler，触发后通过 `Agent::queue_steering` 注入消息

3. **行为日志（behavior_log）**
   - 当前 handler 层 mode 动作（self_defense 攻击、self_preservation 脱困）LLM 完全无感知
   - 应移植：handler 层 mode 动作时累积日志，每轮 `flushBehaviorLog` 注入 user 消息
   - 实现位置：`BotState` 加 `behavior_log: String` 字段，`Agent::drain_queues` 时拉取

#### P1（增强能力）：

4. **SelfPrompter 三态状态机 + 结束判定**
   - 当前 Craft-Agent 一直跑到 max_iterations，无法自动停止
   - 应移植：连续 N 次纯文字回复（无工具调用）自动判定目标完成
   - **适配**：Craft-Agent 已有 `obs_streak` 计数器，可扩展为"无工具调用 streak"，>=3 触发 done
   - 实现位置：`crates/craft-agent/src/agent/mod.rs`，在纯文字回复分支检查 streak

5. **更多 modes**
   - 当前 Craft-Agent 只有 3 个模式，缺 `cowardice` / `hunting` / `item_collecting` / `torch_placing` / `unstuck`（实时版本）
   - 应移植：至少补 `unstuck`（基于位置未变化检测，不是 obs_streak）和 `item_collecting`（拾取掉落物）
   - **适配**：handler 层 Tick 实现，触发后走 P0-2 的 re-prompt 机制
   - 实现位置：`crates/craft-agent-minecraft/src/azalea/mod.rs` 的 Tick 系统

6. **LLM 可调模式开关**
   - 当前 Craft-Agent 的 modes 不可被 LLM 动态开关
   - 应移植：暴露 `set_mode(name, on/off)` 工具给 LLM
   - 实现位置：`crates/craft-agent-minecraft/src/tools_azalea.rs` 新增工具

#### P2（架构优化）：

7. **历史压缩：保留近期 + 增量摘要**（Craft-Agent 已有，Mindcraft 反而落后）
   - Craft-Agent 的 `keep_recent: 200_000` tokens + 增量 `<previous-summary>` 比 Mindcraft 的 500 字符覆盖式更好
   - **无需移植**，但可借鉴 Mindcraft 的 `summary_chunk_size=5` 边界修正（移除开头连续 assistant）

8. **断线错误分类**
   - 当前 Craft-Agent 无连接错误处理（azalea 路线）
   - 应移植：`ERROR_DEFINITIONS` 7 类错误分类 + isFatal 标记
   - **适配**：azalea 的 `Client` 断开事件可分类处理
   - 实现位置：`crates/craft-agent-minecraft/src/azalea/client.rs`

9. **多 bot 对话（ConversationManager）**
   - 当前 Craft-Agent 是单 bot
   - 若未来支持多 bot，可参考 Mindcraft 的 `Conversation` + `ConversationManager` 设计
   - **暂不移植**，但保留参考

#### P3（不推荐移植）：

10. **MemoryBank**——Craft-Agent 的 WorldMemory 已全面超越，无需移植
11. **覆盖式摘要**——Craft-Agent 的增量摘要更优
12. **300ms 后台 tick**——Craft-Agent 的 handler 层 Bevy ECS 已有更细粒度的 tick（每 tick = 50ms），无需额外加

### 7.6 关键架构差异的本质

Mindcraft 的设计哲学是**"反应式 agent"**：
- 后台 tick 持续监测世界 → mode 触发 → 直接动作 → re-prompt LLM
- LLM 是"高层决策者"，mode 是"低层反射弧"
- 三轨道解耦：LLM 思考时 mode 仍可动作

Craft-Agent 的设计哲学是**"思考式 agent"**：
- 单一同步管线：感知 → 模式提示 → LLM → 工具并行 → 技能抽取
- LLM 是"全权决策者"，mode 只发建议
- 强一致性强，但反应慢（一轮 LLM 调用 + 工具执行期间无法响应世界变化）

**根本差距**：Craft-Agent 缺一个**独立于 LLM 调用的反应层**。当前 handler 层 Tick 部分弥补了这点（self_preservation / self_defense 直接动作），但：
- 触发后不通知 LLM（无 re-prompt）
- 无行为日志（LLM 不知道发生了什么）
- 模式数量少（仅 2 个 vs Mindcraft 10 个）

**移植建议**：在不破坏 Craft-Agent 同步管线的前提下，增强 handler 层 Tick 的反应能力 + 加 re-prompt 通道（通过 `queue_steering`），而非引入 Mindcraft 的三轨道并行模型（与 Rust 的所有权模型冲突较大）。

---

## 8. 附录：关键源码索引

### Mindcraft

| 文件 | 行数 | 关键函数 |
|---|---|---|
| agent.js | 553 | `Agent.start` / `handleMessage` / `startEvents` / `update` / `requestInterrupt` |
| action_manager.js | 177 | `ActionManager.runAction` / `_executeAction` / `_executeResume` / `stop` / `_startTimeout` |
| self_prompter.js | 146 | `SelfPrompter.start` / `startLoop` / `update` / `stop` / `pause` / `shouldInterrupt` |
| modes.js | 446 | `modes_list` / `execute` / `ModeController.update` / `initModes` |
| history.js | 121 | `History.add` / `summarizeMemories` / `appendFullHistory` / `save` / `load` |
| memory_bank.js | 25 | `MemoryBank.rememberPlace` / `recallPlace` |
| conversation.js | 353 | `ConversationManager.startConversation` / `receiveFromBot` / `_scheduleProcessInMessage` |
| connection_handler.js | 96 | `parseKickReason` / `handleDisconnection` / `validateNameFormat` |
| speak.js | 150 | `speak` / `processQueue` / `fetchRemoteAudio` |

### Craft-Agent 当前

| 文件 | 行数 | 关键函数 |
|---|---|---|
| mod.rs | 1468 | `Agent::new` / `run` / `continue_run` / `run_one_turn` / `drain_queues` |
| modes.rs | 77 | `Agent::check_modes` |
| prompt.rs | 360 | `build_dynamic_context_msg` / `build_dynamic_instructions_msg` / `build_memory_context_msg` / `build_context` |
| compaction.rs | 342 | `estimate_tokens` / `compact` / `hard_truncate` / `serialize_msg` |
| session.rs | 175 | `with_session` / `persist_turn` / `manage_knowledge` |

---

## 9. 总结

Mindcraft 的 agent 主循环是**事件驱动 + 三轨道并行**的经典反应式架构，强项在于：
- **实时反应**：mode 每 300ms 检查，可直接动作无需等 LLM
- **闭环反馈**：mode 动作后自动 re-prompt LLM，形成"反射 → 思考"闭环
- **精细中断**：ActionManager 的抢占式中断 + 超时 + resume 机制
- **自提示状态机**：三态 + 自动重启 + 结束判定

Craft-Agent 的 agent 主循环是**目标驱动 + 同步管线**的思考式架构，强项在于：
- **强一致性**：13 步管线顺序明确，状态转换清晰
- **工具并行**：ToolEffects 副作用分组 + `std::thread::scope` 并行
- **上下文压缩**：三级回退 + 增量摘要 + 易变消息剔除
- **结构化记忆**：WorldMemory 空间-状态自动注入

**移植核心建议**：在不破坏 Craft-Agent 同步管线的前提下，补齐以下三件套：
1. **ActionManager 层**（超时 + 中断 + 快速循环检测）
2. **Mode re-prompt 通道**（handler 层 mode 触发后通过 `queue_steering` 通知 LLM）
3. **行为日志**（handler 层 mode 动作累积日志，每轮注入 user 消息）

这三项能补上 Craft-Agent 的"反应层"短板，且与现有架构兼容（不需引入异步事件循环）。
