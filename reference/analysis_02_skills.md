# Mindcraft 技能库系统化分析（skills.js + 周边模块）

> 源码路径：`d:\Craft-Agent\reference\mindcraft\src\agent\library\`
> 对标对象：Craft-Agent (`crates/craft-agent-minecraft/`)
> 分析日期：2026-07-25

---

## 1. Mindcraft 技能库总览（架构 + 设计哲学）

### 1.1 目录结构与职责划分

```
src/agent/library/
├── skills.js          ← 核心：37 个 export async function，全部 bot 动作技能
├── world.js           ← 纯查询：18 个 export function，只读世界状态（不触发动作）
├── index.js           ← 文档抽取器：从 JSDoc /** ... **/ 生成 LLM 可见的技能说明
├── skill_library.js   ← 检索器：用 embedding / word-overlap 选 top-K 技能文档注入 prompt
├── full_state.js      ← 仪表盘状态：聚合 world.* + agent 内部状态成单个 JSON
└── lockdown.js        ← SES 沙箱：在 Compartment 里跑 LLM 生成的代码，限制全局污染
```

**职责切分非常清晰**：
- `skills.js` = **副作用技能**（移动、挖、放、合成、战斗 …），全部 `async`，返回 `Promise<boolean>`
- `world.js` = **纯查询**（`getNearestBlock` / `getInventoryCounts` / `getBiomeName` …），不修改世界
- `index.js` 的 `docHelper()` 用正则从函数源码里抽 `/** ... **/` 注释块 → 拼成 LLM 系统提示里的「可用函数清单」。**技能的 JSDoc 既是人读文档也是 LLM 读 prompt**，二者同源
- `skill_library.js` 在 `initSkillLibrary()` 里对每个 skill doc 的「前两行（函数名+一句话描述）」算 embedding 缓存；运行时用 cosine similarity（或退化为 word overlap）选 top-K + 3 个 always-show（`placeBlock` / `wait` / `breakBlockAt`）

### 1.2 设计哲学

1. **代码即技能，技能即文档**：LLM 不是调用结构化 JSON tool，而是写 JavaScript 代码 `await skills.collectBlock(bot, "oak_log", 4)`。JSDoc 直接进 prompt 当函数说明。这让 LLM 能写循环、条件、变量——表达能力远高于 JSON schema 工具
2. **SES 沙箱执行 LLM 生成代码**（`lockdown.js` + `coder.js` 的 `makeCompartment({skills, world, Vec3, log})`），LLM 只能碰这 4 个对象 + Math/Date
3. **统一返回契约**：所有 skill 返回 `boolean`（成功/失败），失败的细节通过 `log(bot, msg)` 写入 `bot.output`，由 `ActionManager.getBotOutputSummary()` 汇总回传 LLM。**没有抛异常给 LLM 看的约定**——异常被 `ActionManager._executeAction` 的 try/catch 兜底
4. **modes 系统作为开关阀门**：几乎每个 skill 开头都 `bot.modes.pause('self_defense')` / `bot.modes.pause('cowardice')`，临时关掉自主反应模式以免干扰当前动作；动作结束后由 modes 自身恢复
5. **cheat mode 双路径**：很多 skill（`placeBlock` / `goToPosition` / `tillAndSow` / `breakBlockAt`）有 `if (bot.modes.isOn('cheat'))` 分支，直接 `/setblock` / `/tp` 走创造模式命令，绕过物理模拟
6. **路径优先非破坏**：`goToGoal()` 先尝试 `nonDestructiveMovements`（玻璃不能破、digCost=10、placeCost=2），失败才退到 `destructiveMovements`
7. **placeBlock / collectBlock 是复合枢纽**：很多技能内联调用它们（`craftRecipe` 临时放/收工作台、`smeltItem` 临时放/收熔炉、`tillAndSow` 先 `breakBlockAt` 再 `placeBlock`）

### 1.3 LLM 调用链（从高到低）

```
LLM 输出代码 → coder.js _stageCode() 包成 main(bot) 函数
            → SES Compartment.evaluate(src) 得到 main
            → coder.js 调 executionModule.main(bot)
            → 代码内调 await skills.xxx(bot, ...)
            → ActionManager._executeAction() 包裹：超时 / 中断 / 日志汇总
            → 返回 { success, message, interrupted, timedout }
```

`commands/actions.js` 里的 `runAsAction()` 是另一条入口：把单个 skill 包成 `!goToPlayer` 之类的命令，也走 `agent.actions.runAction()` 统一管线。

---

## 2. 所有 skill 完整清单

> 表格列：函数名 / 参数 / 返回 / 前置条件 / 失败处理 / 复合度
> 复合度：原子 = 单一 mineflayer 调用；复合 = 多步组合（含 goto+动作+回收）

### 2.1 合成 / 熔炼类

| 函数 | 参数 | 返回 | 前置 | 失败处理 | 复合度 |
|---|---|---|---|---|---|
| `craftRecipe` | `itemName, num=1` | bool | 物品有配方 | 无配方→false；无原料→false；自动放/收工作台 | **高度复合**：找桌→放桌→走过去→craft→收桌→equipAll armor |
| `smeltItem` | `itemName, num=1` | bool | `mc.isSmeltable` | 不可熔→false；无炉→放炉；无燃料→false；11s 无产出→break | **高度复合**：找炉→放炉→走过去→开炉→投料→等输出→取走剩余→收炉 |
| `clearNearestFurnace` | 无 | bool | 32 内有炉 | 无炉→false | 复合：goto+open+takeOutput/Input/Fuel |

### 2.2 战斗类

| 函数 | 参数 | 返回 | 前置 | 失败处理 | 复合度 |
|---|---|---|---|---|---|
| `attackNearest` | `mobType, kill=true` | bool | 24 内有该 mob | 暂停 cowardice；水下 mob 暂停 self_preservation；找不到→false | 复合：找+attackEntity |
| `attackEntity` | `entity, kill=true` | bool | 实体存在 | kill=false 时先 goto 再单次 attack；kill=true 用 pvp 循环到死，interrupt 则 stop | 复合：equipHighestAttack+goto+pvp.loop+pickup |
| `defendSelf` | `range=9` | bool | range 内有敌对 | 暂停 self_defense/cowardice；远处 GoalFollow 近处 GoalInvert 拉开距离；interrupt→stop | 复合：循环 equipHighestAttack+goto+pvp |

### 2.3 挖掘 / 采集类

| 函数 | 参数 | 返回 | 前置 | 失败处理 | 复合度 |
|---|---|---|---|---|---|
| `collectBlock` | `blockType, num=1, exclude=null` | bool | num≥1 | 自动展开别名（coal→coal_ore+deepslate_coal_ore；dirt→grass_block）；`NoChests` 异常→break（背包满）；其它异常→continue 下一块；interrupt→break | **最复杂采集**：findBlocks→equipForBlock→canHarvest 检查→goto+dig 或 collectBlock.collect→pickupNearbyItems→autoLight |
| `pickupNearbyItems` | 无 | bool | 8 内有 item 实体 | 用 GoalFollow 追；若前后是同一个实体（追不上）→break | 复合：循环 pathfinder.goto |
| `breakBlockAt` | `x, y, z` | bool | 坐标非空 | cheat→`/setblock air`；远→goto GoalNear 4；canHarvest 失败→false | 复合：goto+equipForBlock+dig |
| `digDown` | `distance=10` | bool | 无 | 遇岩浆/水→false；下方≥3 格空气（跌落）→false；air 块跳过；dig 失败→false | **状态恢复**：螺旋安全下挖 |
| `goToSurface` | 无 | bool | 无 | 从 y=360 扫到 -64 找首个非 air 块，goto 其上方 | 状态恢复 |

### 2.4 放置 / 交互类

| 函数 | 参数 | 返回 | 前置 | 失败处理 | 复合度 |
|---|---|---|---|---|---|
| `placeBlock` | `blockType, x, y, z, placeOn='bottom', dontCheat=false` | bool | 背包有该物 | cheat→`/setblock`（含 door/bed/torch/button 朝向修正）；目标位有阻挡→先 breakBlockAt；无 buildOff→false；太近→GoalInvert 后退；太远→goto GoalNear 4；placeBlock 异常→false | **最复杂放置**：朝向计算+6 方向找 buildOff+距离调节+placeBlock/useToolOnBlock |
| `equip` | `itemName` | bool | 背包有 / 创造 | 'hand'→unequip；按名后缀选 body part（legs/feet/head/torso/off-hand/hand） | 原子 |
| `useDoor` | `door_pos=null` | bool | 16 内有门 | 自动找 11 种木门；GoalNear 1→等 1s→lookAt→activate→forward 600ms→activate 关门 | 复合 |
| `activateNearestBlock` | `type` | bool | 16 内有该方块 | 远→goto GoalNear 4；activateBlock | 复合 |
| `tillAndSow` | `x, y, z, seedType=null` | bool | 方块为 grass_block/dirt/farmland | cheat→`/setblock farmland`+`/setblock <seed>`；上方非 air→breakBlockAt；无锄→false；无种子→false | 复合：break+equip hoe+activate+equip seed+activate |
| `goToBed` | 无 | bool | 32 内有床 | 找不到→false；sleep 后暂停 unstuck，循环等 isSleeping false | 复合 |
| `useToolOn` | `toolName, targetName` | bool | 背包有工具 | target='nothing'→activateItem；entity→useOn；block→useToolOnBlock | 复合分发 |
| `useToolOnBlock` | `toolName, block` | bool | 工具可装备 | 距离调节（water_bucket 1.5 / 其它 2）；viewBlocked→随机偏移再 goto；bucket→activateItem 否则 activateBlock | 复合 |

### 2.5 物品 / 容器类

| 函数 | 参数 | 返回 | 前置 | 失败处理 | 复合度 |
|---|---|---|---|---|---|
| `discard` | `itemName, num=-1` | bool | 背包有 | 循环 toss 直到 num 或清空；找不到→false | 原子循环 |
| `putInChest` | `itemName, num=-1` | bool | 32 内有箱 | 无箱→false；无物→false；goto+openContainer+deposit+close | 复合 |
| `takeFromChest` | `itemName, num=-1` | bool | 32 内有箱 | 多 slot 累计取；无物→false | 复合 |
| `viewChest` | 无 | bool | 32 内有箱 | 列出所有 containerItems | 复合 |
| `consume` | `itemName=""` | bool | 背包有 | 找不到→false；equip+consume | 原子 |
| `giveToPlayer` | `itemType, username, num=1` | bool | 玩家在线 | 不能给自己；玩家太近→moveAway 2-5 拉开（3s 超时）；discard 后等 playerCollect 3s | **高度复合**：goto+距离调节+discard+事件等待 |

### 2.6 移动 / 寻路类

| 函数 | 参数 | 返回 | 前置 | 失败处理 | 复合度 |
|---|---|---|---|---|---|
| `goToGoal` | `goal` | bool | pathfinder 已初始化 | 先试 nonDestructive（玻璃不破）→destructive→找不到也硬走；startDoorInterval 每 200ms 检查卡住 1.2s 自动开门 | **核心寻路**：双策略 fallback + 卡门检测 |
| `goToPosition` | `x, y, z, min_distance=2` | bool | 坐标非空 | cheat→`/tp`；每 1s 检查 targetDigBlock 是否 canHarvest，不能破则 stopDigging+pathfinder.stop；到达后 distance≤min+1→true | 复合：goToGoal+挖矿进度守卫 |
| `goToNearestBlock` | `blockType, min_distance=2, range=64` | bool | range≤512 | 水熔岩找 metadata=0 源块；找不到→false | 复合：getNearestBlock+goToPosition |
| `goToNearestEntity` | `entityType, min_distance=2, range=64` | bool | 实体存在 | 找不到→false | 复合 |
| `goToPlayer` | `username, distance=3` | bool | 玩家在线 | 自己→true；cheat→`/tp`；暂停 self_defense/cowardice；GoalFollow | 复合 |
| `followPlayer` | `username, distance=4` | bool | 玩家存在 | **永不返回直到 interrupt_code**；>100 距离 cheat→tp；>30 距离暂停 item_collecting/hunting/torch_placing；近时暂停 unstuck/elbow_room | **持续型**：setGoal(dynamic=true)+循环 |
| `moveAway` | `distance` | bool | 无 | cheat→算路径终点 tp；GoalInvert | 复合 |
| `moveAwayFromEntity` | `entity, distance=16` | bool | 实体存在 | GoalInvert(GoalFollow) | 原子 |
| `avoidEnemies` | `distance=16` | bool | 敌对存在 | 暂停 self_preservation；循环 GoalInvert+setGoal；<3 距离→attackEntity(kill=false)；interrupt→break | **状态恢复**：持续避险 |
| `stay` | `seconds=30` | bool | 无 | 暂停 6 个 modes；-1 表示无限；循环等 interrupt 或超时 | 持续型 |

### 2.7 交易类

| 函数 | 参数 | 返回 | 前置 | 失败处理 | 复合度 |
|---|---|---|---|---|---|
| `showVillagerTrades` | `id` | bool | 实体是村民且非幼体 | 找不到→列附近村民；距离>4→GoalFollow 2；打开失败→false；无 trade→false | 复合 |
| `tradeWithVillager` | `id, index, count` | bool | 同上 | trade 不存在/disabled/达上限/资源不足→false；执行 `bot.trade` 异常→false | 复合 |

### 2.8 工具类

| 函数 | 参数 | 返回 | 说明 | 复合度 |
|---|---|---|---|---|
| `log` | `bot, message` | undefined | `bot.output += message+'\n'`，所有 skill 的日志都走这里 | 原子 |
| `wait` | `milliseconds` | bool | sleep，但每 2s 检查 interrupt_code；setTimeout 被禁用所以手写循环 | 原子 |

### 2.9 内部 helper（非 export 给 LLM）

- `autoLight(bot)` — 若 `shouldPlaceTorch` 则自动放火把
- `equipHighestAttack(bot)` — 按 attackDamage 排序选最强武器（sword→axe→pickaxe→shovel）
- `startDoorInterval(bot)` — 每 200ms 检测卡住 1.2s 后随机扫描 8 邻居找门/栏门/活板门自动 activate（**唯一的卡住自恢复机制**）
- `findAndGoToVillager(bot, id)` — 交易前置：找村民+goto+校验成年有职业
- `hasResources` / `stringifyTrades` / `stringifyItem` — 交易辅助

### 2.10 world.js 查询 API 清单（18 个，纯只读）

| 函数 | 作用 | 备注 |
|---|---|---|
| `getNearestFreeSpace(bot, size, distance)` | 找大小 size×size 的空地（下方实心） | craftRecipe/smeltItem 放工作台用 |
| `getBlockAtPosition(bot, x, y, z)` | 相对位置取方块 | full_state 用 |
| `getSurroundingBlocks(bot)` | 脚下/腿/头三块 | 返回 string[] |
| `getFirstBlockAboveHead(bot, ignore_types, distance)` | 头顶第一块实心 | full_state 用 |
| `getNearestBlocks(bot, block_types, distance, count)` | 批量找方块 | 别名展开在 collectBlock 里做 |
| `getNearestBlocksWhere(bot, predicate, distance, count)` | 谓词版 | collectBlock 核心查询 |
| `getNearestBlock(bot, block_type, distance)` | 单个最近 | 多处用 |
| `getNearbyEntities(bot, maxDistance)` | 全部实体排序 | attack/defend 用 |
| `getNearestEntityWhere(bot, predicate, maxDistance)` | 谓词版 | defendSelf/avoidEnemies 用 |
| `getNearbyPlayers(bot, maxDistance)` | 玩家列表 | 默认 16 |
| `getVillagerProfession(entity)` | metadata[18] 解析职业 | 交易前置 |
| `getInventoryCounts(bot)` | `{name: count}` | craftRecipe/smeltItem 前置检查 |
| `getCraftableItems(bot)` | 当前可合成列表 | 用 recipesFor |
| `getPosition(bot)` | bot.entity.position | 多处用 |
| `getNearbyEntityTypes(bot)` | 去重实体类型 | full_state 用 |
| `isEntityType(name)` | 校验 | useToolOn 用 |
| `getNearbyPlayerNames(bot)` | 玩家名去重 | full_state 用，默认 64 |
| `getNearbyBlockTypes(bot, distance)` | 方块类型去重 | full_state 用 |
| `isClearPath(bot, target)` | 不破不放的可达性 | 内部用 |
| `shouldPlaceTorch(bot)` | 是否该放火把 | autoLight 用 |
| `getBiomeName(bot)` | 群系名 | full_state 用 |

---

## 3. 关键 skill 实现原文摘录

> 以下摘自 `d:\Craft-Agent\reference\mindcraft\src\agent\library\skills.js`，行号对应该文件。

### 3.1 `craftRecipe`（行 36-115）— 合成的"放桌-合成-收桌"三段式

```javascript
export async function craftRecipe(bot, itemName, num=1) {
    /**
     * Attempt to craft the given item name from a recipe. May craft many items.
     * @param {MinecraftBot} bot, reference to the minecraft bot.
     * @param {string} itemName, the item name to craft.
     * @returns {Promise<boolean>} true if the recipe was crafted, false otherwise.
     * @example
     * await skills.craftRecipe(bot, "stick");
     **/
    let placedTable = false;

    if (mc.getItemCraftingRecipes(itemName).length == 0) {
        log(bot, `${itemName} is either not an item, or it does not have a crafting recipe!`);
        return false;
    }

    // get recipes that don't require a crafting table
    let recipes = bot.recipesFor(mc.getItemId(itemName), null, 1, null); 
    let craftingTable = null;
    const craftingTableRange = 16;
    placeTable: if (!recipes || recipes.length === 0) {
        recipes = bot.recipesFor(mc.getItemId(itemName), null, 1, true);
        if(!recipes || recipes.length === 0) break placeTable; //Don't bother going to the table if we don't have the required resources.

        // Look for crafting table
        craftingTable = world.getNearestBlock(bot, 'crafting_table', craftingTableRange);
        if (craftingTable === null){

            // Try to place crafting table
            let hasTable = world.getInventoryCounts(bot)['crafting_table'] > 0;
            if (hasTable) {
                let pos = world.getNearestFreeSpace(bot, 1, 6);
                await placeBlock(bot, 'crafting_table', pos.x, pos.y, pos.z);
                craftingTable = world.getNearestBlock(bot, 'crafting_table', craftingTableRange);
                if (craftingTable) {
                    recipes = bot.recipesFor(mc.getItemId(itemName), null, 1, craftingTable);
                    placedTable = true;
                }
            }
            else {
                log(bot, `Crafting ${itemName} requires a crafting table.`)
                return false;
            }
        }
        else {
            recipes = bot.recipesFor(mc.getItemId(itemName), null, 1, craftingTable);
        }
    }
    if (!recipes || recipes.length === 0) {
        log(bot, `You do not have the resources to craft a ${itemName}. It requires: ${Object.entries(mc.getItemCraftingRecipes(itemName)[0][0]).map(([key, value]) => `${key}: ${value}`).join(', ')}.`);
        if (placedTable) {
            await collectBlock(bot, 'crafting_table', 1);
        }
        return false;
    }
    
    if (craftingTable && bot.entity.position.distanceTo(craftingTable.position) > 4) {
        await goToNearestBlock(bot, 'crafting_table', 4, craftingTableRange);
    }

    const recipe = recipes[0];
    console.log('crafting...');
    //Check that the agent has sufficient items to use the recipe `num` times.
    const inventory = world.getInventoryCounts(bot); //Items in the agents inventory
    const requiredIngredients = mc.ingredientsFromPrismarineRecipe(recipe); //Items required to use the recipe once.
    const craftLimit = mc.calculateLimitingResource(inventory, requiredIngredients);
    
    await bot.craft(recipe, Math.min(craftLimit.num, num), craftingTable);
    if(craftLimit.num<num) log(bot, `Not enough ${craftLimit.limitingResource} to craft ${num}, crafted ${craftLimit.num}. You now have ${world.getInventoryCounts(bot)[itemName]} ${itemName}.`);
    else log(bot, `Successfully crafted ${itemName}, you now have ${world.getInventoryCounts(bot)[itemName]} ${itemName}.`);
    if (placedTable) {
        await collectBlock(bot, 'crafting_table', 1);
    }

    //Equip any armor the bot may have crafted.
    //There is probablly a more efficient method than checking the entire inventory but this is all mineflayer-armor-manager provides. :P
    bot.armorManager.equipAll(); 

    return true;
}
```

**关键模式**：
- 三段式：放桌（`placeBlock`）→ 合成（`bot.craft`）→ 收桌（`collectBlock`）
- 资源不足时仍按 `craftLimit` 部分合成，不直接失败
- 用 `placedTable` 标志位决定是否回收（避免误收别人的桌）
- 末尾 `armorManager.equipAll()` 自动穿装备

### 3.2 `collectBlock`（行 417-529）— 别名展开 + 安全挖掘 + 异常分类

```javascript
export async function collectBlock(bot, blockType, num=1, exclude=null) {
    /**
     * Collect one of the given block type.
     * @param {MinecraftBot} bot, reference to the minecraft bot.
     * @param {string} blockType, the type of block to collect.
     * @param {number} num, the number of blocks to collect. Defaults to 1.
     * @param {list} exclude, a list of positions to exclude from the search. Defaults to null.
     * @returns {Promise<boolean>} true if the block was collected, false if the block type was not found.
     * @example
     * await skills.collectBlock(bot, "oak_log");
     **/
    if (num < 1) {
        log(bot, `Invalid number of blocks to collect: ${num}.`);
        return false;
    }
    let blocktypes = [blockType];
    if (blockType === 'coal' || blockType === 'diamond' || blockType === 'emerald' || blockType === 'iron' || blockType === 'gold' || blockType === 'lapis_lazuli' || blockType === 'redstone')
        blocktypes.push(blockType+'_ore');
    if (blockType.endsWith('ore'))
        blocktypes.push('deepslate_'+blockType);
    if (blockType === 'dirt')
        blocktypes.push('grass_block');
    if (blockType === 'cobblestone')
        blocktypes.push('stone');
    const isLiquid = blockType === 'lava' || blockType === 'water';

    let collected = 0;

    const movements = new pf.Movements(bot);
    movements.dontMineUnderFallingBlock = false;
    movements.dontCreateFlow = true;

    // Blocks to ignore safety for, usually next to lava/water
    const unsafeBlocks = ['obsidian'];

    for (let i=0; i<num; i++) {
        let blocks = world.getNearestBlocksWhere(bot, block => {
            if (!blocktypes.includes(block.name)) {
                return false;
            }
            if (exclude) {
                for (let position of exclude) {
                    if (block.position.x === position.x && block.position.y === position.y && block.position.z === position.z) {
                        return false;
                    }
                }
            }
            if (isLiquid) {
                // collect only source blocks
                return block.metadata === 0;
            }
            
            return movements.safeToBreak(block) || unsafeBlocks.includes(block.name);
        }, 64, 1);

        if (blocks.length === 0) {
            if (collected === 0)
                log(bot, `No ${blockType} nearby to collect.`);
            else
                log(bot, `No more ${blockType} nearby to collect.`);
            break;
        }
        const block = blocks[0];
        await bot.tool.equipForBlock(block);
        if (isLiquid) {
            const bucket = bot.inventory.findInventoryItem('bucket');
            if (!bucket) {
                log(bot, `Don't have bucket to harvest ${blockType}.`);
                return false;
            }
            await bot.equip(bucket, 'hand');
        }
        const itemId = bot.heldItem ? bot.heldItem.type : null
        if (!block.canHarvest(itemId)) {
            log(bot, `Don't have right tools to harvest ${blockType}.`);
            return false;
        }
        try {
            let success = false;
            if (isLiquid) {
                success = await useToolOnBlock(bot, 'bucket', block);
            }
            else if (mc.mustCollectManually(blockType)) {
                await goToPosition(bot, block.position.x, block.position.y, block.position.z, 2);
                await bot.dig(block);
                await pickupNearbyItems(bot);
                success = true;
            }
            else {
                await bot.collectBlock.collect(block);
                success = true;
            }
            if (success)
                collected++;
            await autoLight(bot);
        }
        catch (err) {
            if (err.name === 'NoChests') {
                log(bot, `Failed to collect ${blockType}: Inventory full, no place to deposit.`);
                break;
            }
            else {
                log(bot, `Failed to collect ${blockType}: ${err}.`);
                continue;
            }
        }
        
        if (bot.interrupt_code)
            break;  
    }
    log(bot, `Collected ${collected} ${blockType}.`);
    return collected > 0;
}
```

**关键模式**：
- **别名表硬编码**：coal→coal_ore、xxx_ore→deepslate_xxx_ore、dirt→grass_block、cobblestone→stone
- **安全过滤**：`movements.safeToBreak(block) || unsafeBlocks.includes(block.name)`，`dontCreateFlow=true` 防止挖开流体
- **三种采集路径**：液体用桶、`mustCollectManually` 走 goto+dig+pickup、其余用 `bot.collectBlock.collect`（mineflayer-collectblock 插件自动 pathfind+dig+pickup）
- **异常分类**：`NoChests`（背包满无处存）→ break 整个循环；其它异常 → continue 下一块（容错）
- 每次成功后 `autoLight(bot)` 自动放火把

### 3.3 `goToPosition`（行 1181-1235）+ `goToGoal`（行 1070-1113）— 双策略寻路 + 卡门守护

```javascript
export async function goToPosition(bot, x, y, z, min_distance=2) {
    /**
     * Navigate to the given position.
     * ...doc...
     **/
    if (x == null || y == null || z == null) {
        log(bot, `Missing coordinates, given x:${x} y:${y} z:${z}`);
        return false;
    }
    if (bot.modes.isOn('cheat')) {
        bot.chat('/tp @s ' + x + ' ' + y + ' ' + z);
        log(bot, `Teleported to ${x}, ${y}, ${z}.`);
        return true;
    }
    
    const checkDigProgress = () => {
        if (bot.targetDigBlock) {
            const targetBlock = bot.targetDigBlock;
            const itemId = bot.heldItem ? bot.heldItem.type : null;
            if (!targetBlock.canHarvest(itemId)) {
                log(bot, `Pathfinding stopped: Cannot break ${targetBlock.name} with current tools.`);
                bot.pathfinder.stop();
                bot.stopDigging();
            }
        }
    };
    
    const progressInterval = setInterval(checkDigProgress, 1000);
    
    try {
        await goToGoal(bot, new pf.goals.GoalNear(x, y, z, min_distance));
        clearInterval(progressInterval);
        const distance = bot.entity.position.distanceTo(new Vec3(x, y, z));
        if (distance <= min_distance+1) {
            log(bot, `You have reached at ${x}, ${y}, ${z}.`);
            return true;
        }
        else {
            log(bot, `Unable to reach ${x}, ${y}, ${z}, you are ${Math.round(distance)} blocks away.`);
            return false;
        }
    } catch (err) {
        log(bot, `Pathfinding stopped: ${err.message}.`);
        clearInterval(progressInterval);
        return false;
    }
}

export async function goToGoal(bot, goal) {
    /**
     * Navigate to the given goal. Use doors and attempt minimally destructive movements.
     **/
    const nonDestructiveMovements = new pf.Movements(bot);
    const dontBreakBlocks = ['glass', 'glass_pane'];
    for (let block of dontBreakBlocks) {
        nonDestructiveMovements.blocksCantBreak.add(mc.getBlockId(block));
    }
    nonDestructiveMovements.placeCost = 2;
    nonDestructiveMovements.digCost = 10;

    const destructiveMovements = new pf.Movements(bot);

    let final_movements = destructiveMovements;

    const pathfind_timeout = 1000;
    if (await bot.pathfinder.getPathTo(nonDestructiveMovements, goal, pathfind_timeout).status === 'success') {
        final_movements = nonDestructiveMovements;
        log(bot, `Found non-destructive path.`);
    }
    else if (await bot.pathfinder.getPathTo(destructiveMovements, goal, pathfind_timeout).status === 'success') {
        log(bot, `Found destructive path.`);
    }
    else {
        log(bot, `Path not found, but attempting to navigate anyway using destructive movements.`);
    }

    const doorCheckInterval = startDoorInterval(bot);

    bot.pathfinder.setMovements(final_movements);
    try {
        await bot.pathfinder.goto(goal);
        clearInterval(doorCheckInterval);
        return true;
    } catch (err) {
        clearInterval(doorCheckInterval);
        // we need to catch so we can clean up the door check interval, then rethrow the error
        throw err;
    }
}
```

**关键模式**：
- **双策略 fallback**：先试 nonDestructive（玻璃禁破、digCost=10 placeCost=2 高成本避免破坏），失败才退到 destructive
- **找不到也硬走**：第三分支 "Path not found, but attempting to navigate anyway" 仍执行 goto，依赖 pathfinder 运行时可能成功
- **挖矿进度守卫**：`goToPosition` 每 1s 检查 `targetDigBlock.canHarvest`，发现工具不够立刻 `stopDigging + pathfinder.stop`
- **startDoorInterval 卡门恢复**：200ms tick，位置变化 <0.1 累加 stuck_time，>1.2s 触发随机扫描 8 邻居（含上下）找门/栏门/活板门自动 activate

### 3.4 `attackEntity` + `defendSelf`（行 334-413）— PVP 循环 + 距离调节

```javascript
export async function attackEntity(bot, entity, kill=true) {
    /**
     * Attack mob of the given type.
     **/
    let pos = entity.position;
    await equipHighestAttack(bot)

    if (!kill) {
        if (bot.entity.position.distanceTo(pos) > 5) {
            console.log('moving to mob...')
            await goToPosition(bot, pos.x, pos.y, pos.z);
        }
        console.log('attacking mob...')
        await bot.attack(entity);
    }
    else {
        bot.pvp.attack(entity);
        while (world.getNearbyEntities(bot, 24).includes(entity)) {
            await new Promise(resolve => setTimeout(resolve, 1000));
            if (bot.interrupt_code) {
                bot.pvp.stop();
                return false;
            }
        }
        log(bot, `Successfully killed ${entity.name}.`);
        await pickupNearbyItems(bot);
        return true;
    }
}

export async function defendSelf(bot, range=9) {
    /**
     * Defend yourself from all nearby hostile mobs until there are no more.
     **/
    bot.modes.pause('self_defense');
    bot.modes.pause('cowardice');
    let attacked = false;
    let enemy = world.getNearestEntityWhere(bot, entity => mc.isHostile(entity), range);
    while (enemy) {
        await equipHighestAttack(bot);
        if (bot.entity.position.distanceTo(enemy.position) >= 4 && enemy.name !== 'creeper' && enemy.name !== 'phantom') {
            try {
                bot.pathfinder.setMovements(new pf.Movements(bot));
                await bot.pathfinder.goto(new pf.goals.GoalFollow(enemy, 3.5), true);
            } catch (err) {/* might error if entity dies, ignore */}
        }
        if (bot.entity.position.distanceTo(enemy.position) <= 2) {
            try {
                bot.pathfinder.setMovements(new pf.Movements(bot));
                let inverted_goal = new pf.goals.GoalInvert(new pf.goals.GoalFollow(enemy, 2));
                await bot.pathfinder.goto(inverted_goal, true);
            } catch (err) {/* might error if entity dies, ignore */}
        }
        bot.pvp.attack(enemy);
        attacked = true;
        await new Promise(resolve => setTimeout(resolve, 500));
        enemy = world.getNearestEntityWhere(bot, entity => mc.isHostile(entity), range);
        if (bot.interrupt_code) {
            bot.pvp.stop();
            return false;
        }
    }
    bot.pvp.stop();
    if (attacked)
        log(bot, `Successfully defended self.`);
    else
        log(bot, `No enemies nearby to defend self from.`);
    return attacked;
}
```

**关键模式**：
- **kill=true 用 pvp 插件托管**：`bot.pvp.attack(entity)` 后只循环检查实体是否还在 24 范围内
- **kill=false 单次攻击**：先 goto 5 内再 `bot.attack`
- **defendSelf 距离双调节**：≥4 用 GoalFollow 接近（creeper/phantom 例外，不靠近）；≤2 用 GoalInvert(GoalFollow) 后退拉开
- **异常静默**：pathfinder 报错（实体死亡等）直接吞掉
- **循环重选目标**：每 500ms 重新 `getNearestEntityWhere`，符合"防御所有"语义

### 3.5 `placeBlock`（行 611-789）— 朝向计算 + 6 方向 buildOff + cheat 朝向修正

```javascript
export async function placeBlock(bot, blockType, x, y, z, placeOn='bottom', dontCheat=false) {
    /**
     * Place the given block type at the given position. ...
     **/
    const target_dest = new Vec3(Math.floor(x), Math.floor(y), Math.floor(z));

    if (blockType === 'air') {
        log(bot, `Placing air (removing block) at ${target_dest}.`);
        return await breakBlockAt(bot, x, y, z);
    }

    if (bot.modes.isOn('cheat') && !dontCheat) {
        if (bot.restrict_to_inventory) {
            let block = bot.inventory.findInventoryItem(blockType);
            if (!block) {
                log(bot, `Cannot place ${blockType}, you are restricted to your current inventory.`);
                return false;
            }
        }

        // invert the facing direction
        let face = placeOn === 'north' ? 'south' : placeOn === 'south' ? 'north' : placeOn === 'east' ? 'west' : 'east';
        if (blockType.includes('torch') && placeOn !== 'bottom') {
            blockType = blockType.replace('torch', 'wall_torch');
            if (placeOn !== 'side' && placeOn !== 'top') {
                blockType += `[facing=${face}]`;
            }
        }
        if (blockType.includes('button') || blockType === 'lever') {
            if (placeOn === 'top') {
                blockType += `[face=ceiling]`;
            }
            else if (placeOn === 'bottom') {
                blockType += `[face=floor]`;
            }
            else {
                blockType += `[facing=${face}]`;
            }
        }
        if (blockType === 'ladder' || blockType === 'repeater' || blockType === 'comparator') {
            blockType += `[facing=${face}]`;
        }
        if (blockType.includes('stairs')) {
            blockType += `[facing=${face}]`;
        }
        if (useDelay) { await new Promise(resolve => setTimeout(resolve, blockPlaceDelay)); }
        let msg = '/setblock ' + Math.floor(x) + ' ' + Math.floor(y) + ' ' + Math.floor(z) + ' ' + blockType;
        bot.chat(msg);
        if (blockType.includes('door'))
            if (useDelay) { await new Promise(resolve => setTimeout(resolve, blockPlaceDelay)); }
            bot.chat('/setblock ' + Math.floor(x) + ' ' + Math.floor(y+1) + ' ' + Math.floor(z) + ' ' + blockType + '[half=upper]');
        if (blockType.includes('bed'))
            if (useDelay) { await new Promise(resolve => setTimeout(resolve, blockPlaceDelay)); }
            bot.chat('/setblock ' + Math.floor(x) + ' ' + Math.floor(y) + ' ' + Math.floor(z-1) + ' ' + blockType + '[part=head]');
        log(bot, `Used /setblock to place ${blockType} at ${target_dest}.`);
        return true;
    }

    let item_name = blockType;
    if (item_name == "redstone_wire")
        item_name = "redstone";
    else if (item_name === 'water') {
        item_name = 'water_bucket';
    }
    else if (item_name === 'lava') {
        item_name = 'lava_bucket';
    }
    let block_item = bot.inventory.findInventoryItem(item_name);
    if (!block_item && bot.game.gameMode === 'creative' && !bot.restrict_to_inventory) {
        await bot.creative.setInventorySlot(36, mc.makeItem(item_name, 1));
        block_item = bot.inventory.findInventoryItem(item_name);
    }
    if (!block_item) {
        log(bot, `Don't have any ${item_name} to place.`);
        return false;
    }

    const targetBlock = bot.blockAt(target_dest);
    if (targetBlock.name === blockType || (targetBlock.name === 'grass_block' && blockType === 'dirt')) {
        log(bot, `${blockType} already at ${targetBlock.position}.`);
        return false;
    }
    const empty_blocks = ['air', 'water', 'lava', 'grass', 'short_grass', 'tall_grass', 'snow', 'dead_bush', 'fern'];
    if (!empty_blocks.includes(targetBlock.name)) {
        log(bot, `${targetBlock.name} in the way at ${targetBlock.position}.`);
        const removed = await breakBlockAt(bot, x, y, z);
        if (!removed) {
            log(bot, `Cannot place ${blockType} at ${targetBlock.position}: block in the way.`);
            return false;
        }
        await new Promise(resolve => setTimeout(resolve, 200));
    }
    // get the buildoffblock and facevec based on whichever adjacent block is not empty
    let buildOffBlock = null;
    let faceVec = null;
    const dir_map = {
        'top': Vec3(0, 1, 0),
        'bottom': Vec3(0, -1, 0),
        'north': Vec3(0, 0, -1),
        'south': Vec3(0, 0, 1),
        'east': Vec3(1, 0, 0),
        'west': Vec3(-1, 0, 0),
    }
    let dirs = [];
    if (placeOn === 'side') {
        dirs.push(dir_map['north'], dir_map['south'], dir_map['east'], dir_map['west']);
    }
    else if (dir_map[placeOn] !== undefined) {
        dirs.push(dir_map[placeOn]);
    }
    else {
        dirs.push(dir_map['bottom']);
        log(bot, `Unknown placeOn value "${placeOn}". Defaulting to bottom.`);
    }
    dirs.push(...Object.values(dir_map).filter(d => !dirs.includes(d)));

    for (let d of dirs) {
        const block = bot.blockAt(target_dest.plus(d));
        if (!empty_blocks.includes(block.name)) {
            buildOffBlock = block;
            faceVec = new Vec3(-d.x, -d.y, -d.z); // invert
            break;
        }
    }
    if (!buildOffBlock) {
        log(bot, `Cannot place ${blockType} at ${targetBlock.position}: nothing to place on.`);
        return false;
    }

    const pos = bot.entity.position;
    const pos_above = pos.plus(Vec3(0,1,0));
    const dont_move_for = ['torch', 'redstone_torch', 'redstone', 'lever', 'button', 'rail', 'detector_rail', 
        'powered_rail', 'activator_rail', 'tripwire_hook', 'tripwire', 'water_bucket', 'string'];
    if (!dont_move_for.includes(item_name) && (pos.distanceTo(targetBlock.position) < 1.1 || pos_above.distanceTo(targetBlock.position) < 1.1)) {
        let goal = new pf.goals.GoalNear(targetBlock.position.x, targetBlock.position.y, targetBlock.position.z, 2);
        let inverted_goal = new pf.goals.GoalInvert(goal);
        bot.pathfinder.setMovements(new pf.Movements(bot));
        await bot.pathfinder.goto(inverted_goal);
    }
    if (bot.entity.position.distanceTo(targetBlock.position) > 4.5) {
        let pos = targetBlock.position;
        let movements = new pf.Movements(bot);
        bot.pathfinder.setMovements(movements);
        await goToGoal(bot, new pf.goals.GoalNear(pos.x, pos.y, pos.z, 4));
    }

    try {
        if (item_name.includes('bucket')) {
            await useToolOnBlock(bot, item_name, buildOffBlock);
        }
        else {
            await bot.equip(block_item, 'hand');
            await bot.lookAt(buildOffBlock.position.offset(0.5, 0.5, 0.5));
            await bot.placeBlock(buildOffBlock, faceVec);
            log(bot, `Placed ${blockType} at ${target_dest}.`);
            await new Promise(resolve => setTimeout(resolve, 200));
            return true;
        }
    } catch (err) {
        log(bot, `Failed to place ${blockType} at ${target_dest}.`);
        return false;
    }
}
```

**关键模式**：
- **cheat 分支带朝向修正**：torch/wall_torch、button/lever 的 face=floor/ceiling/facing、ladder/repeater/comparator/stairs 的 facing、door 的 half=upper、bed 的 part=head，全部用 setblock 方括号语法
- **物品别名**：redstone_wire→redstone、water→water_bucket、lava→lava_bucket
- **目标位非空先破**：empty_blocks 表 9 项，不在表里就先 `breakBlockAt` 再 200ms 等待
- **6 方向找 buildOff**：按 placeOn 优先（side=四水平方向、bottom/top/south/north/east/west 单方向），失败 fallback 其余方向；faceVec 取反
- **距离双调节**：太近（<1.1）GoalInvert 后退；太远（>4.5）GoalNear 4 接近；dont_move_for 列表内物品（torch/redstone/rail 等）不调节
- **bucket 走 useToolOnBlock**（含 viewBlocked 检测），其它走 `bot.placeBlock(buildOffBlock, faceVec)`

### 3.6 `startDoorInterval`（行 1115-1179）— 卡门自恢复（唯一的"unstuck"机制）

```javascript
let _doorInterval = null;
function startDoorInterval(bot) {
    /**
     * Start helper interval that opens nearby doors if the bot is stuck.
     **/
    if (_doorInterval) {
        clearInterval(_doorInterval);
    }
    let prev_pos = bot.entity.position.clone();
    let prev_check = Date.now();
    let stuck_time = 0;


    const doorCheckInterval = setInterval(() => {
        const now = Date.now();
        if (bot.entity.position.distanceTo(prev_pos) >= 0.1) {
            stuck_time = 0;
        } else {
            stuck_time += now - prev_check;
        }
        
        if (stuck_time > 1200) {
            // shuffle positions so we're not always opening the same door
            const positions = [
                bot.entity.position.clone(),
                bot.entity.position.offset(0, 0, 1),
                bot.entity.position.offset(0, 0, -1), 
                bot.entity.position.offset(1, 0, 0),
                bot.entity.position.offset(-1, 0, 0),
            ]
            let elevated_positions = positions.map(position => position.offset(0, 1, 0));
            positions.push(...elevated_positions);
            positions.push(bot.entity.position.offset(0, 2, 0)); // above head
            positions.push(bot.entity.position.offset(0, -1, 0)); // below feet
            
            let currentIndex = positions.length;
            while (currentIndex != 0) {
                let randomIndex = Math.floor(Math.random() * currentIndex);
                currentIndex--;
                [positions[currentIndex], positions[randomIndex]] = [
                positions[randomIndex], positions[currentIndex]];
            }
            
            for (let position of positions) {
                let block = bot.blockAt(position);
                if (block && block.name &&
                    !block.name.includes('iron') &&
                    (block.name.includes('door') ||
                     block.name.includes('fence_gate') ||
                     block.name.includes('trapdoor'))) 
                {
                    bot.activateBlock(block);
                    break;
                }
            }
            stuck_time = 0;
        }
        prev_pos = bot.entity.position.clone();
        prev_check = now;
    }, 200);
    _doorInterval = doorCheckInterval;
    return doorCheckInterval;
}
```

**关键模式**：
- **位置变化 <0.1 累加 stuck_time**，>1.2s 触发
- **10 个候选位置**（5 水平 + 5 抬高一格 + 头顶 + 脚下）洗牌后扫描
- **铁门跳过**（`!block.name.includes('iron')`，需要红石信号）
- **找到第一个门/栏门/活板门就 activate 并 break**
- **全局单例 `_doorInterval`**（防重复）

### 3.7 `smeltItem`（行 142-273）— 熔炉全流程（节选关键片段）

```javascript
export async function smeltItem(bot, itemName, num=1) {
    // ...doc...
    if (!mc.isSmeltable(itemName)) {
        log(bot, `Cannot smelt ${itemName}. Hint: make sure you are smelting the 'raw' item.`);
        return false;
    }

    let placedFurnace = false;
    let furnaceBlock = undefined;
    const furnaceRange = 16;
    furnaceBlock = world.getNearestBlock(bot, 'furnace', furnaceRange);
    if (!furnaceBlock){
        let hasFurnace = world.getInventoryCounts(bot)['furnace'] > 0;
        if (hasFurnace) {
            let pos = world.getNearestFreeSpace(bot, 1, furnaceRange);
            await placeBlock(bot, 'furnace', pos.x, pos.y, pos.z);
            furnaceBlock = world.getNearestBlock(bot, 'furnace', furnaceRange);
            placedFurnace = true;
        }
    }
    if (!furnaceBlock){
        log(bot, `There is no furnace nearby and you have no furnace.`)
        return false;
    }
    if (bot.entity.position.distanceTo(furnaceBlock.position) > 4) {
        await goToNearestBlock(bot, 'furnace', 4, furnaceRange);
    }
    bot.modes.pause('unstuck');
    await bot.lookAt(furnaceBlock.position);

    const furnace = await bot.openFurnace(furnaceBlock);
    let input_item = furnace.inputItem();
    if (input_item && input_item.type !== mc.getItemId(itemName) && input_item.count > 0) {
        log(bot, `The furnace is currently smelting ${mc.getItemName(input_item.type)}.`);
        if (placedFurnace)
            await collectBlock(bot, 'furnace', 1);
        return false;
    }
    let inv_counts = world.getInventoryCounts(bot);
    if (!inv_counts[itemName] || inv_counts[itemName] < num) {
        log(bot, `You do not have enough ${itemName} to smelt.`);
        if (placedFurnace)
            await collectBlock(bot, 'furnace', 1);
        return false;
    }

    if (!furnace.fuelItem()) {
        let fuel = mc.getSmeltingFuel(bot);
        if (!fuel) {
            log(bot, `You have no fuel to smelt ${itemName}, you need coal, charcoal, or wood.`);
            if (placedFurnace)
                await collectBlock(bot, 'furnace', 1);
            return false;
        }
        log(bot, `Using ${fuel.name} as fuel.`);

        const put_fuel = Math.ceil(num / mc.getFuelSmeltOutput(fuel.name));

        if (fuel.count < put_fuel) {
            log(bot, `You don't have enough ${fuel.name} to smelt ${num} ${itemName}; you need ${put_fuel}.`);
            if (placedFurnace)
                await collectBlock(bot, 'furnace', 1);
            return false;
        }
        await furnace.putFuel(fuel.type, null, put_fuel);
        log(bot, `Added ${put_fuel} ${mc.getItemName(fuel.type)} to furnace fuel.`);
    }
    await furnace.putInput(mc.getItemId(itemName), null, num);
    let total = 0;
    let smelted_item = null;
    await new Promise(resolve => setTimeout(resolve, 200));
    let last_collected = Date.now();
    while (total < num) {
        await new Promise(resolve => setTimeout(resolve, 1000));
        if (furnace.outputItem()) {
            smelted_item = await furnace.takeOutput();
            if (smelted_item) {
                total += smelted_item.count;
                last_collected = Date.now();
            }
        }
        if (Date.now() - last_collected > 11000) {
            break; // if nothing has been collected in 11 seconds, stop
        }
        if (bot.interrupt_code) {
            break;
        }
    }
    if (furnace.inputItem()) {
        await furnace.takeInput();
    }
    if (furnace.fuelItem()) {
        await furnace.takeFuel();
    }

    await bot.closeWindow(furnace);

    if (placedFurnace) {
        await collectBlock(bot, 'furnace', 1);
    }
    if (total === 0) {
        log(bot, `Failed to smelt ${itemName}.`);
        return false;
    }
    if (total < num) {
        log(bot, `Only smelted ${total} ${mc.getItemName(smelted_item.type)}.`);
        return false;
    }
    log(bot, `Successfully smelted ${itemName}, got ${total} ${mc.getItemName(smelted_item.type)}.`);
    return true;
}
```

**关键模式**：
- **placedFurnace 标志**：每个失败分支都要判断是否要回收自放的炉
- **三重前置**：可熔 / 有炉（找/放）/ 有燃料（`mc.getSmeltingFuel` 按效率选）
- **燃料量计算**：`put_fuel = ceil(num / getFuelSmeltOutput(fuel.name))`
- **11s 无产出超时**：last_collected 节奏检测，防止空转
- **结束清理**：取走 input/fuel 残余，closeWindow

---

## 4. 错误处理 + 重试 + 降级模式总结

### 4.1 三层错误模型

| 层级 | 机制 | 例子 |
|---|---|---|
| **L1: skill 内部 try/catch** | 单步失败 log 后 `continue` 或 `return false` | collectBlock 挖单块失败 continue 下一块；placeBlock 失败 return false |
| **L2: ActionManager try/catch** | 整个 actionFn 异常 → 失败信息含 stack trace 回传 LLM | 任何 skill 抛出未捕获异常 |
| **L3: modes 反应层** | 完全独立于 LLM，每 tick 检查危险自动 push 动作 | self_preservation / self_defense（在 azalea/mod.rs handler 里） |

### 4.2 失败处理标准模式（按频次排序）

1. **`log(bot, reason) + return false`**（最常见，~70 处）：返回布尔，细节进 bot.output 给 LLM 看
2. **`continue` 跳过当前迭代**（collectBlock）：单块失败不放弃整体目标
3. **`break` 退出循环**（collectBlock 遇 NoChests、smeltItem 11s 超时）：不可恢复时停止
4. **`try/catch 静默吞`**（defendSelf 的 pathfinder.goto、attackEntity 的 GoalFollow）：实体死亡等预期错误
5. **`placedTable/placedFurnace` 标志回收**（craftRecipe/smeltItem）：每个失败分支都判断是否回收自放的工作台/熔炉
6. **`bot.modes.pause(name)`**（几乎所有 skill）：临时关干扰模式（self_defense/cowardice/unstuck 等）
7. **`bot.interrupt_code` 检查**（所有长循环）：用户中断信号
8. **双策略 fallback**（goToGoal）：nonDestructive → destructive → 硬走

### 4.3 重试逻辑（其实很少硬重试）

- **`generateCode` 的 5 次重试**（coder.js）：LLM 写代码失败/抛异常时，把错误塞回 messages 让 LLM 修。这是**唯一的 LLM 层重试**
- **`collectBlock` 的 continue**：换一块挖，不算重试同一块
- **`giveToPlayer` 的 3s 拉开距离循环**：太近时反复 moveAway 5，3s 超时放弃
- **没有"挖失败重挖同块"的逻辑**——遇到失败直接换目标或返回

### 4.4 死循环检测（ActionManager 行 64-81）

```javascript
if (this.last_action_time > 0) {
    let time_diff = Date.now() - this.last_action_time;
    if (time_diff < 20) {
        this.recent_action_counter++;
    }
    else {
        this.recent_action_counter = 0;
    }
    if (this.recent_action_counter > 3) {
        console.warn('Fast action loop detected, cancelling resume.');
        this.cancelResume(); // likely cause of repetition
    }
    if (this.recent_action_counter > 5) {
        console.error('Infinite action loop detected, shutting down.');
        this.agent.cleanKill('Infinite action loop detected, shutting down.');
    }
}
```

**两级保护**：20ms 内连发 >3 次 → 取消 resume；>5 次 → kill 整个进程。

### 4.5 状态恢复技能清单

| 技能 | 触发 | 动作 |
|---|---|---|
| `startDoorInterval` | 寻路中卡住 1.2s | 随机扫描 8 邻居开门 |
| `avoidEnemies` | 附近有敌对 | GoalInvert 持续逃离，<3 距离反击 |
| `digDown` | 主动下挖 | 遇岩浆/水/≥3 跌落立即停止 |
| `goToSurface` | 在地下 | 从 y=360 扫到 -64 找首个实心块 goto |
| `defendSelf` | 附近有敌对 | 循环 pvp 直到清场 |
| `clearNearestFurnace` | 熔炉被占用 | 取走所有 slot |
| `pickupNearbyItems` | 战斗/挖掘后 | 自动捡物 |

**注意**：Mindcraft **没有"掉水自动上岸"、"被卡自动挖通"**这类主动 unstuck 技能——`unstuck` 是个 mode（每 X 秒跳一下），不是 skill。

---

## 5. action_manager 的执行模型

### 5.1 状态字段

```javascript
this.executing = false;            // 是否正在执行
this.currentActionLabel = '';      // 当前动作标签（如 "action:goToPlayer"）
this.currentActionFn = null;       // 当前动作闭包
this.timedout = false;             // 是否超时
this.resume_func = null;           // resume 模式的待执行闭包
this.resume_name = '';
this.last_action_time = 0;         // 死循环检测
this.recent_action_counter = 0;
```

### 5.2 主入口 `runAction(actionLabel, actionFn, { timeout, resume })`

- `resume=true` → `_executeResume`（followPlayer 这类持续型用）
- `resume=false` → `_executeAction`（默认）

### 5.3 `_executeAction` 流程（行 61-150）

```
1. 死循环检测（见 4.4）
2. last_action_time = now
3. if (executing) → 调 stop() 等待当前动作结束（最多 10s）
4. clearBotLogs() 清空 bot.output
5. executing = true; currentActionLabel/Fn 设置
6. if (timeout > 0) → _startTimeout(timeout) 启动分钟级超时
7. await actionFn()  ← 真正执行 skill
8. executing = false; 清理 currentActionFn; clearTimeout
9. output = getBotOutputSummary()  ← 截断到 500 字符（首 250 + 末 250）
10. interrupted = bot.interrupt_code; timedout = this.timedout
11. clearBotLogs()
12. if (!interrupted) → bot.emit('idle')
13. return { success: true, message: output, interrupted, timedout }

catch (err):
  executing = false; 清理; clearTimeout; cancelResume()
  message = output + '!!Code threw exception!!\n' + err + stack
  if (!interrupted) → bot.emit('idle')
  return { success: false, message, interrupted, timedout: false }
```

### 5.4 关键设计点

1. **串行单任务**：`executing` 互斥锁；新动作来要先 `stop()` 当前
2. **stop() 软中断 + 10s 硬杀**（行 26-37）：循环 `requestInterrupt()` + 300ms sleep，10s 还不退就 `cleanKill`
3. **timeout 是分钟级**（默认 10）：`_startTimeout` 触发后设 timedout=true，加 history system 消息，再调 stop()
4. **输出截断**（行 152-166）：>500 字符时首 250 + "..." + 末 250，防止 LLM context 爆炸
5. **idle 事件**：每次 action 结束（成功/失败）只要不是中断就 emit 'idle'，触发 self_prompter 选下一动作
6. **resume 模式**（行 44-59）：`_executeResume` 仅在 agent 空闲且 self_prompter 未激活时才执行 resume_func。用于"followPlayer 这类被中断后，agent 闲下来自动续跑"

### 5.5 调用方对比

**coder.js 路径**（LLM 写代码）：
```javascript
await executionModule.main(this.agent.bot);  // 不直接走 runAction
const code_output = this.agent.actions.getBotOutputSummary();
```
注意 `coder.js` **绕过了 runAction 的超时/中断包装**——`main(bot)` 是直接调，但 `!newAction` 命令包了 `runAction('action:newAction', actionFn, {timeout: settings.code_timeout_mins})`。

**commands/actions.js 路径**（结构化命令）：
```javascript
const runAsAction = (actionFn, resume=false, timeout=-1) => {
    const wrappedAction = async (agent, ...args) => {
        const actionFnWithAgent = async () => { await actionFn(agent, ...args); };
        const code_return = await agent.actions.runAction(`action:${actionLabel}`, actionFnWithAgent, { timeout, resume });
        if (code_return.interrupted && !code_return.timedout) return;
        return code_return.message;
    };
    return wrappedAction;
};
```
每个 `!command` 都包一层 runAction，统一走超时/中断管线。

---

## 6. Craft-Agent 移植建议（按 ROI 排序）

### 6.1 当前 Craft-Agent 工具盘点（23 个，`tools_azalea.rs`）

```
perceive / goto / mine_below / mine / interact_block / attack /
craft / craft_3x3 / smelt / gather / place / open /
auto_craft / enchant / trade / interact_entity / chat /
memory / set_goal / run_plan / search_wiki / run_script / build
```

**对比 Mindcraft 37 skill，主要缺口**：
- ❌ `collectBlock`（带别名展开+安全过滤+三种采集路径）→ 当前只有 `mine`（挖单块）和 `gather`（采集）
- ❌ `placeBlock`（带朝向计算+6方向buildOff+cheat朝向修正）→ 当前 `place` 仅放不能拆
- ❌ `pickupNearbyItems`（自动捡物）→ 完全缺失
- ❌ `defendSelf` / `avoidEnemies`（多目标战斗+距离调节）→ 当前 `attack` 只打单实体
- ❌ `giveToPlayer` / `putInChest` / `takeFromChest` / `viewChest`（玩家/容器交互）→ 完全缺失
- ❌ `tillAndSow` / `goToBed` / `useDoor`（农业/睡眠/门）→ 完全缺失
- ❌ `digDown` / `goToSurface`（垂直方向恢复）→ 完全缺失
- ❌ `goToNearestBlock` / `goToNearestEntity` / `goToPlayer`（按类型寻路）→ 当前 `goto` 只接坐标
- ❌ `followPlayer` / `stay` / `moveAway`（持续型行为）→ 完全缺失
- ❌ `equip` / `discard` / `consume`（装备/丢弃/吃喝）→ 完全缺失
- ❌ `showVillagerTrades` / `tradeWithVillager`（村民交易查询）→ 当前 `trade` 只接 offer 索引
- ❌ `clearNearestFurnace`（熔炉清理）→ 缺失
- ❌ `startDoorInterval`（卡门自恢复）→ 缺失（但有 `unstuck` mode）

### 6.2 按 ROI 排序的移植清单

> ROI = 价值 ÷ 实现成本。价值按"解锁多少 LLM 任务"评估，成本按"azalea API 现成度 + Rust 实现复杂度"评估。

#### 🔴 P0 必须抄（核心闭环，缺失就废）

| # | Mindcraft skill | Craft-Agent 对应 | azalea API | 实现要点 |
|---|---|---|---|---|
| 1 | `collectBlock` | 升级 `gather` | `bot.dig()` + pathfinder + `bot.pickup()` | 抄别名表（coal→coal_ore 等 5 条）、`safeToBreak` 检查（azalea 有 `is_safe_to_break`）、`NoChests` 异常分类。**当前 `gather` 已有 auto_craft 的递归木链，但缺别名+安全过滤** |
| 2 | `placeBlock` | 升级 `place` | `bot.place_block()` + pathfinder | 抄 6 方向 buildOff 计算（dir_map + faceVec 取反）、empty_blocks 9 项表、太近 GoalInvert/太远 GoalNear 双调节。cheat 朝向修正可跳过（azalea 路线无 cheat） |
| 3 | `pickupNearbyItems` | **新增 `pickup` 工具** | azalea `bot.pickup(item_entity)` 或 pathfinder GoalFollow | 简单循环，但**必须独立**——collectBlock/attackEntity 后都要调。当前 Craft-Agent 战斗后不捡物是个明显短板 |
| 4 | `goToNearestBlock` / `goToNearestEntity` / `goToPlayer` | 升级 `goto` 或新增 `goto_nearest` | pathfinder + world 查询 | 当前 `goto` 只接坐标，LLM 要先 perceive 拿到坐标再 goto，多一轮调用。改成接 `block_type`/`entity_type`/`player_name` 任一 |
| 5 | `defendSelf` | 升级 `attack` | azalea `bot.attack()` + pathfinder GoalFollow/GoalInvert | 当前 `attack` 只打单实体。加 `range` 参数 + 循环清场 + 距离调节（creeper 不靠近）|

#### 🟡 P1 强烈建议（显著提升 LLM 表达力）

| # | Mindcraft skill | Craft-Agent 对应 | azalea API | 实现要点 |
|---|---|---|---|---|
| 6 | `craftRecipe` 的"放桌-合成-收桌"模式 | 已有 `craft`/`craft_3x3`/`auto_craft` | `bot.craft()` + place + break | **当前 `craft_3x3` 假设工作台已存在**。应抄 placedTable 标志：无桌时自动 place→craft→collect。`auto_craft` 已有递归但只补原料不补桌 |
| 7 | `smeltItem` 的"放炉-熔-收炉"模式 | 已有 `smelt` | azalea `bot.open_furnace()` + place + break | 抄 placedFurnace 标志 + 三重前置（isSmeltable/有炉/有燃料）+ 11s 无产出超时 + 末尾取残余 |
| 8 | `giveToPlayer` / `putInChest` / `takeFromChest` / `viewChest` | 已有 `open`，需扩 | azalea container API | `open` 当前只读。加 `deposit(item,num)` / `withdraw(item,num)` / `give_to_player(name,item,num)`。容器交互是中期任务必备 |
| 9 | `equip` / `discard` / `consume` | **新增 3 工具** | azalea `bot.equip()` / `bot.toss()` / `bot.consume()` | 简单原子，但战斗前 equipHighestAttack、饥饿时 consume、清背包 discard 都依赖这些 |
| 10 | `followPlayer` / `stay` / `moveAway` | **新增 3 工具** | pathfinder setGoal(dynamic) | followPlayer 是"持续型"行为，需要在 azalea handler 里 setGoal 后**直接返回**，靠 interrupt 停止。当前 Craft-Agent 工具都是"完成型"，缺持续型语义 |

#### 🟢 P2 锦上添花（特定场景才用）

| # | Mindcraft skill | 实现要点 |
|---|---|---|
| 11 | `digDown` / `goToSurface` | 矿物采集恢复。azalea pathfinder 支持垂直 GoalXZ。digDown 的岩浆/水/跌落检查值得抄 |
| 12 | `tillAndSow` / `goToBed` | 农业任务。azalea 有 `bot.use_on(block)` + `bot.sleep_in_bed()` |
| 13 | `useDoor` / `activateNearestBlock` | 门/拉杆/按钮交互。azalea `bot.activate_block()` |
| 14 | `showVillagerTrades` | 当前 `trade` 工具应拆为 `view_trades` + `trade`两步，让 LLM 先看再选 |
| 15 | `clearNearestFurnace` | 熔炉清理，简单 |
| 16 | `avoidEnemies` | 与 defendSelf 互逆，配合 self_preservation mode |

#### ⚪ P3 不建议抄

| Mindcraft 概念 | 原因 |
|---|---|
| `cheat` mode 分支 | Craft-Agent azalea 路线不走创造模式命令 |
| `coder.js` 的 LLM 写 JS 代码 | Craft-Agent 已用结构化 JSON tool + run_plan/run_script（rhai），更安全 |
| `lockdown.js` SES 沙箱 | 同上，不需要 |
| `skill_library.js` embedding 检索 | Craft-Agent 已有 `SkillLibrary`（`crates/craft-agent/src/core/skill.rs`）+ few-shot 词重叠，思路一致 |

### 6.3 关键 azalea API 速查表

> 用于估算 Rust 移植成本。azalea 的 `Client` 方法在 handler 闭包内调用，外部通过 `BotCommand` 队列代理（见 `crates/craft-agent-minecraft/src/azalea/mod.rs::AzaleaBot`）。

| mineflayer (Mindcraft) | azalea (Rust) | 现状 |
|---|---|---|
| `bot.dig(block)` | `client.dig(block)` / `client.dig_no_break_progress(block)` | ✅ 已用（mine 工具） |
| `bot.placeBlock(refBlock, faceVec)` | `client.place_block(block_pos, direction)` | ✅ 已用（place 工具），但缺 buildOff 计算 |
| `bot.collectBlock.collect(block)` | 无原生等价，需 pathfinder goto + dig + pickup 组合 | ⚠️ 需自己拼 |
| `bot.attack(entity)` | `client.attack(entity)` | ✅ 已用（attack 工具） |
| `bot.pvp.attack(entity)` | 无原生 pvp 插件，需循环 client.attack | ⚠️ 需自己写循环 |
| `bot.equip(item, slot)` | `client.hold_item(slot)` / `inventory.equip` | ⚠️ API 存在但未封装为工具 |
| `bot.toss(itemType, metadata, count)` | `inventory.toss_item(item_kind, count)` | ⚠️ 未封装 |
| `bot.consume()` | `client.use_item()`（手持食物时） | ⚠️ 未封装 |
| `bot.openContainer(chest)` | `client.open_container(block_pos)` | ✅ 已用（open 工具），但缺 deposit/withdraw |
| `container.deposit(type, metadata, count)` | `container.click_slot` + 协议层操作 | ⚠️ 需手写 |
| `bot.openFurnace(block)` | 同 container，furnace 是特殊 container | ⚠️ smelt 工具已封装 |
| `bot.craft(recipe, count, table)` | azalea 无原生 craft API → Craft-Agent 自己实现了 `craft.rs`（2×2/3×3） | ✅ 已有 |
| `bot.recipesFor(item, ...)` | azalea 无原生 → Craft-Agent 用 `recipes.rs` 静态图 + `recipe_book.rs` 数据驱动 | ✅ 已有 |
| `bot.pathfinder.goto(goal)` | `client.goto(BlockPosGoal)` / `client.goto(GoalNear)` | ✅ 已用 |
| `pf.goals.GoalFollow(entity, dist)` | azalea `GoalKind::FollowEntity`（若存在）或自己 tick 调 goto | ⚠️ 需确认 |
| `pf.goals.GoalInvert(goal)` | 需自己实现 invert 逻辑 | ⚠️ 需自己写 |
| `pf.Movements` + `safeToBreak` | azalea pathfinder 有 `Moves` 配置但 API 不同 | ⚠️ 需对照 |
| `bot.blockAtCursor(5)` | `client.lookup_block_at(eye_pos + look_dir * 5)` | ⚠️ 需自己算 raycast |
| `bot.findBlocks({matching, maxDistance, count})` | 遍历 `bot.world()` 半径内方块 | ✅ 已在 `record_surroundings` 实现 |
| `bot.activateBlock(block)` | `client.use_block_on(block_pos)` | ⚠️ interact_block 工具已有 |
| `bot.useOn(entity)` | `client.interact_entity(entity)` | ✅ 已用（interact_entity） |
| `bot.activateItem()` | `client.use_item()` | ⚠️ 未单独封装 |
| `bot.sleep(bed)` | `client.sleep_in_bed(bed_pos)` | ❌ 未封装 |
| `bot.lookAt(pos)` | `client.look_at(direction)` | ⚠️ 需自己算方向向量 |
| `bot.creative.setInventorySlot` | 不需要（azalea 路线无 cheat） | — |

### 6.4 Rust 移植的具体代码模式建议

#### 模式 A：升级现有工具（最小改动）

以 `collectBlock` 为例，升级现有 `GatherTool`（`tools_azalea.rs:534`）：

```rust
// 现有：GatherTool { item, count } → adapter.gather(item, count)
// 升级：增加别名展开 + 安全过滤 + 失败分类
pub struct GatherTool { ctx: Arc<AzaleaToolCtx> }

impl GameTool for GatherTool {
    fn name(&self) -> &str { "gather" }
    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "item": { "type": "string", "description": "物品名（支持别名：coal→coal_ore+deepslate_coal_ore）" },
                "count": { "type": "integer", "default": 1 }
            },
            "required": ["item"]
        })
    }
    fn execute(&self, _call_id: &str, args: Value, _: Option<ToolUpdateFn>) -> anyhow::Result<ToolResult> {
        let item = args.get("item").and_then(|v| v.as_str()).ok_or_else(|| anyhow!("缺 item"))?.to_string();
        let count = args.get("count").and_then(|v| v.as_u64()).unwrap_or(1) as u32;
        // 别名展开（抄 collectBlock 行 432-440）
        let block_types = expand_block_aliases(&item);
        // 调 adapter.gather，但传入别名列表
        match self.ctx.adapter.gather_with_aliases(&block_types, count) {
            Ok(msg) => Ok(ToolResult { message: msg, is_error: false, images: vec![] }),
            Err(e) => {
                // 异常分类（抄 collectBlock 行 513-522）
                let msg = if e.to_string().contains("inventory full") {
                    format!("背包满无处存放：{}", e)
                } else {
                    format!("采集失败：{}", e)
                };
                Ok(ToolResult { message: msg, is_error: true, images: vec![] })
            }
        }
    }
}

fn expand_block_aliases(name: &str) -> Vec<String> {
    let mut v = vec![name.to_string()];
    match name {
        "coal" | "diamond" | "emerald" | "iron" | "gold" | "lapis_lazuli" | "redstone" => {
            v.push(format!("{}_ore", name));
        }
        _ if name.ends_with("ore") => v.push(format!("deepslate_{}", name)),
        "dirt" => v.push("grass_block".into()),
        "cobblestone" => v.push("stone".into()),
        _ => {}
    }
    v
}
```

#### 模式 B：新增工具（中等改动）

以 `pickup_nearby_items` 为例：

```rust
pub struct PickupTool { ctx: Arc<AzaleaToolCtx> }
impl GameTool for PickupTool {
    fn name(&self) -> &str { "pickup" }
    fn description(&self) -> &str {
        "拾取 8 格内的所有掉落物。战斗/挖掘后必调。无参数。"
    }
    fn parameters(&self) -> Value { serde_json::json!({}) }
    fn effects(&self) -> ToolEffects { ToolEffects::write() }  // 修改背包
    fn execute(&self, _: &str, _: Value, _: Option<ToolUpdateFn>) -> anyhow::Result<ToolResult> {
        let msg = self.ctx.adapter.pickup_nearby()?;
        Ok(ToolResult { message: msg, is_error: false, images: vec![] })
    }
}
// 在 AzaleaBot 加：
// pub fn pickup_nearby(&self) -> anyhow::Result<String> { ... push_cmd_and_wait(PickupNearby, 30_000) }
// handler: 找 8 格内 item 实体，pathfinder GoalFollow，循环直到无 item
```

#### 模式 C：持续型行为（需要新语义）

`followPlayer` / `stay` 这类"永不返回直到中断"的 skill 与当前 Craft-Agent 的"工具调用→完成→返回"模型不兼容。两种方案：

1. **方案 A（推荐）**：复用 `set_goal` 工具。`set_goal` 已经写入 SelfPrompter 每轮重注，可以扩展为"持续型 goal"——LLM 调 `set_goal("跟随玩家 Steve，距离 4")`，agent 主循环每轮检查并执行 followPlayer 等价逻辑
2. **方案 B**：新增 `run_continuously` 工具，启动后立即返回 "已启动"，靠 `stop` 工具或新 goal 中断。需要在 AzaleaBot 加 "持续型 BotCommand"（不释放 pending 槽直到 cancel）

**推荐方案 A**：因为 Craft-Agent 的 SelfPrompter 机制本就是为持续型目标设计的，followPlayer 本质是"目标=跟随"，每轮主循环里 goto player 即可。

### 6.5 移植优先级最终建议

**第一波（1-2 天，解锁 80% 价值）**：
1. 升级 `gather` 加 collectBlock 的别名表 + 异常分类
2. 升级 `place` 加 6 方向 buildOff 计算 + empty_blocks 表
3. 新增 `pickup` 工具
4. 升级 `goto` 接受 `block_type`/`entity_type`/`player_name`（或新增 `goto_nearest`）
5. 升级 `attack` 加 `range` 参数 + 循环清场

**第二波（3-5 天，补全闭环）**：
6. `craft_3x3` 加 placedTable 自动放/收桌
7. `smelt` 加 placedFurnace 自动放/收炉
8. 新增 `equip` / `discard` / `consume` 三个原子工具
9. `open` 拆为 `view_container` / `deposit` / `withdraw`
10. 新增 `give_to_player`

**第三波（按需）**：
11. `dig_down` / `go_to_surface`
12. `till_and_sow` / `go_to_bed`
13. `use_door` / `activate_block`
14. `view_trades`（拆自 `trade`）
15. `clear_furnace`

**不要抄**：
- cheat mode 任何分支
- LLM 写 JS 代码的 coder.js 路径（已有 run_plan/run_script 替代）
- SES 沙箱
- embedding 技能检索（已有 SkillLibrary）

---

## 7. 附：world.js 查询函数的 Craft-Agent 对应

Craft-Agent 的 `perceive` 工具 + `WorldMemory` 已经覆盖了 world.js 的大部分查询能力：

| world.js 函数 | Craft-Agent 对应 | 状态 |
|---|---|---|
| `getPosition` | `perceive` 输出含 `self_hint` 坐标 | ✅ |
| `getBiomeName` | `perceive` 输出 | ✅ |
| `getInventoryCounts` | `perceive` 输出背包前 5 格 | ⚠️ 应扩为全量计数 |
| `getNearbyPlayerNames` | `perceive` 输出附近玩家 | ✅ |
| `getNearbyEntityTypes` | `perceive` 输出 | ✅ |
| `getSurroundingBlocks` | `perceive` 输出 below/legs/head | ✅ |
| `getNearestBlock(type, dist)` | `WorldMemory` query | ⚠️ 应在 perceive 里渲染周边 |
| `getNearestBlocks(...)` | `record_surroundings` 已扫描 8 半径 | ✅ |
| `getCraftableItems` | 无直接对应 | ❌ 可基于 `recipes.rs` 静态图实现 |
| `isClearPath(target)` | 无 | ❌ 低优先级 |
| `shouldPlaceTorch` | 无（无 torch_placing mode） | ❌ |
| `getVillagerProfession` | `trade` 工具内 | ⚠️ 应在 perceive 里展示附近村民职业 |

**建议**：将 `perceive` 工具的输出扩展为类似 `full_state.js` 的结构化 JSON（位置/生物群系/天气/时间/装备/附近实体类型/附近方块类型/可合成物品清单），让 LLM 一次调用拿到全量上下文，减少多轮 perceive + memory 查询。

---

## 8. 总结

Mindcraft 技能库的核心价值不在单个 skill 的实现，而在三个设计决策：

1. **代码即技能**：LLM 写 JS 代码而非填 JSON 参数，表达力提升一个数量级（循环/条件/变量）。Craft-Agent 已用 `run_plan`（JSON 步骤）+ `run_script`（rhai 嵌入式脚本）替代，方向正确但 rhai 表达力弱于 JS，可考虑未来支持 WASM 沙箱跑 LLM 生成代码
2. **统一返回契约**：所有 skill 返回 bool + log 到 bot.output，由 ActionManager 汇总截断回传。Craft-Agent 的 `ToolResult { message, is_error, images }` 已是更结构化的版本，**无需改**
3. **modes 暂停机制**：skill 执行时暂停干扰性自主反应（self_defense/cowardice/unstuck 等）。Craft-Agent 的 modes 系统（`modes.rs`）已有类似设计，但**当前 skill 工具内未调用 modes.pause**——应在 P0 工具升级时补上（如 attack 暂停 self_defense，goto 暂停 unstuck）

**移植核心原则**：抄"业务逻辑"（别名表/朝向计算/异常分类/双策略寻路/卡门恢复），不抄"基础设施"（沙箱/LLM 写代码/embedding 检索）——后者 Craft-Agent 已有等价或更好的方案。

最关键的 P0 五件套（collectBlock 别名 / placeBlock buildOff / pickup / goto_nearest / defendSelf 循环）能在 1-2 天内补齐，将使 Craft-Agent 的 LLM 工具表达力接近 Mindcraft 水平。
