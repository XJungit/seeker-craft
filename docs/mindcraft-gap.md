# Mindcraft 差距分析（主动对照清单）

> 用途：每次迭代前先扫本表找差距，避免"实机出问题才补"的被动模式。
> 来源：mindcraft-bots/mindcraft develop 分支（2026-08-01 拉取）。
> 状态：✅ 已实现 | 🟡 部分 | ❌ 缺失 | ➖ 不需要（设计取舍）

## 1. 命令层（mindcraft commands vs 我们的 LLM 工具）

| Mindcraft 命令 | 我们 | 状态 | 备注 |
|---|---|---|---|
| !newAction | new_action | ✅ | |
| !stop / !stfu / !restart / !clearChat | pause_goal | 🟡 | compaction 自动清 |
| !goToPlayer | — | ❌ | 按玩家名 goto（目前只能 follow） |
| !followPlayer | follow / stop_follow | ✅ | |
| !goToCoordinates | goto | ✅ | |
| !searchForBlock | gather + perceive | 🟡 | 无"全局搜块返回坐标"（有 scan 记忆） |
| !searchForEntity | perceive | 🟡 | 实体带坐标（P74） |
| !moveAway | — | ❌ | LLM 层躲怪无工具（cowardice 自动做） |
| !rememberHere / !savedPlaces / !goToRememberedPlace | memory | 🟡 | 记忆有，goto 锚点无 |
| !givePlayer | give | ✅ | |
| !consume / !equip / !discard | consume / equip / discard | ✅ | |
| !putInChest / !takeFromChest / !viewChest | chest_deposit/withdraw/view | ✅ | |
| !collectBlocks | gather | ✅ | |
| !craftRecipe / !smeltItem / !clearFurnace | craft/3x3/smelt/auto_craft | ✅ | P47 自动回收炉 |
| !placeHere / !useOn | place / interact_block | ✅ | |
| !attack / !attackPlayer | attack / interact_entity | ✅ | attackPlayer 不需要 |
| !goToBed | — | ❌ | 睡觉跳夜（低优先级） |
| !stay | pause_goal | 🟡 | |
| !setMode | — | ❌ | 模式开关（配置层，非 LLM） |
| !goal / !endGoal | set_goal / task_complete | ✅ | |
| !showVillagerTrades / !tradeWithVillager | trade | ✅ | |
| !startConversation / !endConversation | chat | 🟡 | 对话无需双端 |
| !lookAtPlayer / !lookAtPosition | — | ❌ | 转头动画（视觉无用） |
| !digDown | mine_below | ✅ | |
| !goToSurface | — | ❌ | 快速回地表（mine_above 替代，慢） |
| !checkBlueprint* / !getBlueprint* | list_blueprints / build_blueprint | ✅ | |
| !getCraftingPlan | auto_craft | ✅ | |
| !searchWiki | search_wiki | ✅ | |
| !help | — | ➖ | 命令文档在 prompt |

## 2. 技能层（skills.js）

| Mindcraft 技能 | 我们 | 状态 | 备注 |
|---|---|---|---|
| craftRecipe / smeltItem | craft / smelt | ✅ | P47 对齐取料循环 |
| attackNearest / attackEntity / defendSelf | attack + self_defense | ✅ | P77 8m 对齐 |
| equipHighestAttack | self_defense 自动换武器 | ✅ | P77 |
| collectBlock / pickupNearbyItems | gather / pickup | ✅ | |
| breakBlockAt / placeBlock | mine / place | ✅ | |
| putInChest / takeFromChest / viewChest | chest_* | ✅ | |
| goToGoal / goToPosition / goToNearestBlock / goToNearestEntity | goto / gather / mine | 🟡 | goToNearestEntity 无（perceive 带坐标） |
| goToPlayer | follow | 🟡 | |
| moveAway / moveAwayFromEntity / avoidEnemies | cowardice | ✅ | P77 阈值 10 |
| stay | pause_goal | 🟡 | |
| useDoor | interact_block | ✅ | 门自动开？ |
| goToBed | — | ❌ | |
| tillAndSow | — | ❌ | 种植（农场蓝图有，无自动化） |
| activateNearestBlock | interact_block | ✅ | |
| showVillagerTrades / tradeWithVillager | trade | ✅ | |
| autoLight（火把自动） | torch_placing | ✅ | |

## 3. 模式层（modes.js）

| Mindcraft 模式 | 我们 | 状态 | 备注 |
|---|---|---|---|
| self_preservation（水/火/低血/流沙） | ✅ 火/岩浆 | 🟡 | 落水跳、低血逃跑无（cowardice 补） |
| unstuck | ✅ 三看门狗 | ✅ | P65/66/67 更强 |
| cowardice（16m 无条件逃） | ✅ 20m/hp<10 | ✅ | 保留 hp 门槛（取舍） |
| self_defense（8m） | ✅ 8m | ✅ | P77 |
| hunting（8m 动物） | ✅ 8m+拾取 | ✅ | P77 |
| item_collecting（8m 物品） | 🟡 hunting 拾取窗 | ❌ | 无独立物品拾取模式（LLM pickup 工具在） |
| torch_placing | ✅ | ✅ | |
| elbow_room | ✅ | ✅ | |
| idle_staring | ✅ | ✅ | |
| cheat | ✅ | ✅ | |

## 4. 其他差异

| Mindcraft 能力 | 我们 | 状态 | 备注 |
|---|---|---|---|
| mineflayer-pvp（走位战斗） | self_defense 直打 | 🟡 | 无 strafe/进退走位 |
| auto-eat（startAt=14+bannedFood） | auto_eat（P58/P73） | ✅ | 白名单等价 banned |
| armor-manager 自动穿甲 | auto_armor（P79） | ✅ | 200 tick 检查，材料优先级 |
| 记忆（rememberPlace/记忆库） | WorldMemory 7 类 | ✅ | 锚点 goto 缺失 |
| full_state（世界全量查询） | perceive 分块 | 🟡 | |
| lockdown（限制物品/禁命令） | blocked_actions? | 🟡 | |

## 优先级队列（按主线收益排序）

1. ❌ goToPlayer / goToSurface——树冠/找队友场景
2. 🟡 item_collecting 独立化——hunting 拾取窗只覆盖打猎
3. ❌ tillAndSow 种植——食物农场（蓝图已有 farm_plot）
4. ❌ goToBed 睡觉——跳夜
5. 🟡 pvp 走位（strafe）——creeper 规避已有，正面对砍补进退
6. 🟡 自动穿甲已上线（P79）——待实机验证损坏甲/新甲替换

> 更新规则：每次实现/新增能力后更新本表状态；每次迭代开始先看"优先级队列"。
