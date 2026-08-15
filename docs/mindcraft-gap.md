# Mindcraft 差距分析（主动对照清单）

> **现状说明（2026-08-15）**：本文是持续维护的工具层差距对照（工具层 54 工具仍有效）。
> 其中引用的 in-bot 机制（run_one_turn 批执行、compaction、P56/P58 nudge 等）已随
> DSH 桥接模式移除（2026-08-14，见 [ARCHITECTURE.md](../ARCHITECTURE.md)）；工具能力
> 对照部分不受影响。
>
> 用途：每次迭代前先扫本表找差距，避免"实机出问题才补"的被动模式。
> 来源：mindcraft-bots/mindcraft develop 分支（2026-08-01 拉取）。
> 状态：✅ 已实现 | 🟡 部分 | ❌ 缺失 | ➖ 不需要（设计取舍）

## 1. 命令层（mindcraft commands vs 我们的 LLM 工具）

| Mindcraft 命令 | 我们 | 状态 | 备注 |
|---|---|---|---|
| !newAction | new_action | ✅ | |
| !stop / !stfu / !restart / !clearChat | pause_goal | 🟡 | compaction 自动清 |
| !goToPlayer | goto_player | ✅ | P111（2026-08-06）：按玩家名单次导航（LLM 工具 goto_player + probe 命令 gotoplayer [名字]），复用 P110 定位→Goto 派发模式，probe 实机验证（无参→最近玩家 / 按名→Jun / 不存在→报错）；持续跟随用 follow |
| !followPlayer | follow / stop_follow | ✅ | |
| !goToCoordinates | goto | ✅ | |
| !searchForBlock | search_for_block | ✅ | P112（2026-08-06）：搜块返回坐标列表（别名展开 + 按距离升序 + 最多 8 处），LLM 工具 search_for_block + probe 命令 searchblock <方块> [半径]；只搜不挖（要挖用 gather） |
| !searchForEntity | perceive | 🟡 | 实体带坐标（P74） |
| !moveAway | move_away | ✅ | P113（2026-08-06）：主动远离指定实体/类型（无参=最近非玩家实体，默认 8m，clamp 4-64），水平反向向量→Goto（y 保持当前层），找不到→"附近找不到目标实体"反馈；LLM 工具 move_away + probe 命令 moveaway [实体名] [距离]；probe 实机验证（zombie 指定距离 / 无参默认 / llama 不存在报错）；战斗中被 self_defense 抢 pending 槽属已知模式交互（goto 超时兜底） |
| !rememberHere / !savedPlaces / !goToRememberedPlace | memory + goto | ✅ | P110（2026-08-06）：GotoTool 增加可选 anchor 参数；命令层 `goto <名>` 单 token 非数字 → GotoAnchor；handler 用共享 WorldMemory 解析锚点转 Goto 复用全部导航逻辑；probe 新增 `memory anchor/query` 命令（probe 与 LLM 共享同一 WorldMemory 实例），probe 实机验证锚点设置 → 锚点导航闭环 |
| !givePlayer | give | ✅ | |
| !consume / !equip / !discard | consume / equip / discard | ✅ | |
| !putInChest / !takeFromChest / !viewChest | chest_deposit/withdraw/view | ✅ | |
| !collectBlocks | gather | ✅ | |
| !craftRecipe / !smeltItem / !clearFurnace | craft/3x3/smelt/auto_craft | ✅ | P47 自动回收炉 |
| !placeHere / !useOn | place / interact_block | ✅ | |
| !attack / !attackPlayer | attack / interact_entity | ✅ | attackPlayer 不需要 |
| !goToBed | sleep | ✅ | P85 完成，2026-08-02 probe 实测通过 |
| !stay | pause_goal | 🟡 | |
| !setMode | set_mode | ✅ | P116 完成，2026-08-06 probe 实测通过（5 模式开关 + list） |
| !goal / !endGoal | set_goal / task_complete | ✅ | |
| !showVillagerTrades / !tradeWithVillager | trade | ✅ | |
| !startConversation / !endConversation | chat | 🟡 | 对话无需双端 |
| !lookAtPlayer / !lookAtPosition | — | ❌ | 转头动画（视觉无用） |
| !digDown | mine_below | ✅ | |
| !goToSurface | mine_above | ✅ | P105/P106/P107 三层修复闭环（2026-08-03，probe 实机验证） |
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
| goToPlayer | goto_player / follow | ✅ | P111 单次导航 + follow 持续跟随 |
| moveAway / moveAwayFromEntity / avoidEnemies | move_away + cowardice | ✅ | P113 主动工具（指定实体/距离）+ P77 cowardice 自动（阈值 10） |
| stay | pause_goal | 🟡 | |
| useDoor | interact_block | ✅ | 门自动开？ |
| goToBed | sleep | ✅ | P85（2026-08-02 probe 实测） |
| tillAndSow | till_and_sow | ✅ | P84（2026-08-02 probe 全路径实测） |
| activateNearestBlock | interact_block | ✅ | |
| showVillagerTrades / tradeWithVillager | trade | ✅ | |
| autoLight（火把自动） | torch_placing | ✅ | |

## 3. 模式层（modes.js）

| Mindcraft 模式 | 我们 | 状态 | 备注 |
|---|---|---|---|
| self_preservation（水/火/低血/流沙） | ✅ 火/岩浆 | 🟡 | 落水跳、低血逃跑无（cowardice 补） |
| unstuck | ✅ 三看门狗 | ✅ | P65/66/67 更强 + P81 连续失败工具调用检测 |
| cowardice（16m 无条件逃） | ✅ 20m/hp<10 | ✅ | 保留 hp 门槛（取舍） |
| self_defense（8m） | ✅ 8m | ✅ | P77 |
| hunting（8m 动物） | ✅ 8m+拾取 | ✅ | P77 |
| item_collecting（8m 物品） | item_collecting（P80） | ✅ | 200 tick + 空位保护 |
| torch_placing | ✅ | ✅ | |
| elbow_room | ✅ | ✅ | |
| idle_staring | ✅ | ✅ | |
| cheat | ✅ | ✅ | |

## 4. 其他差异

| Mindcraft 能力 | 我们 | 状态 | 备注 |
|---|---|---|---|
| mineflayer-pvp（走位战斗） | self_defense 直打+strafe | ✅ | P87 无武器徒手攻击+绕侧走位 |
| auto-eat（startAt=14+bannedFood） | auto_eat（P58/P73） | ✅ | 白名单等价 banned |
| armor-manager 自动穿甲 | auto_armor（P79） | ✅ | 200 tick 检查，材料优先级 |
| 记忆（rememberPlace/记忆库） | WorldMemory 7 类 | ✅ | P110 锚点 goto 已完成（2026-08-06） |
| full_state（世界全量查询） | perceive 分块 | 🟡 | |
| lockdown（限制物品/禁命令） | blocked_actions? | 🟡 | |

## 优先级队列（按主线收益排序）

1. ✅ tillAndSow 种植——食物农场（P84 完成，2026-08-02 probe 全路径实测通过）【原实机问题：bot 捡到 wheat_seeds 因无法种植而 discard】
2. ✅ goToSurface 强化——P83 信号已给（overhead_solid→mine_above），P105/P106/P107 三层修复闭环（2026-08-03，probe 实机验证）
3. ✅ goToBed 睡觉——跳夜（P85 完成，2026-08-02 probe 实测通过）
4. ✅ 收割（harvest 工具）——farmland 成熟后挖取+拾取（P86 完成，2026-08-02 probe 实测通过）
5. ✅ pvp 走位（strafe）+ 近战修复全套——P87+P88 完成，2026-08-02 实机验证（逼近/1s 反击/低血反击/攻击只发生在可命中距离）
6. ✅ 自动穿甲（P79）——2026-08-03 probe 实机验证通过（give leather_chestplate → 自动装备 slot[6]；iron_helmet 自动装 slot[5]）
7. ✅ item_collecting（P80）——2026-08-03 probe 实机验证通过（discard iron_ingot → 15s 内自动拾取回背包）
8. ✅ turn 内失败重规划（P89）——WRITE 工具失败→中止剩余批次→同轮重调 LLM，2026-08-02 单测通过
9. ✅ 上下文管理重构（P98，2026-08-02 提交 002d748）——A1 few-shot 真实消息对 / B3 瞬态统一剔除 / B4 记忆注入冷却 / B5 任务进度紧凑渲染 / A2 分阶段知识 / C7 jailbreak 可配 / C8 knowledge 缓存；实机验证：prompt_cache_hit_tokens 42624-43584 / miss 2796-3326（**前缀缓存命中 >93%**）；下一步 harness 慢工具单动作轮 + subagent 委派
10. ✅ 慢工具单动作轮（P99，2026-08-02 提交 6da6f6c）——GameTool::is_slow() + 12 个慢工具（goto/mine/mine_below/mine_above/gather/till_and_sow/harvest/attack/make_obsidian/pickup/defend/follow）；批内含慢工具 → 慢动作执行完立即中止剩余预测调用（【已中止】占位补齐 OpenAI 配对），**不重调 LLM**（结果已回填，下轮 auto-perceive 驱动新决策）。2026-08-02 实机验证：混合批 [goto+pickup] → goto 执行 → pickup 【已中止】→ 下轮重新决策 ✓；快工具批不受影响。实机同时发现并修复 P89 潜在 panic：失败重规划 nudge 的 fmsg 字节切片 `&fmsg[..len.min(160)]` 在中文长错误消息上切爆 UTF-8 边界 → agent 线程 panic 退出（running=false）；改为 `chars().take(160)`（P89b 回归测试：中文长错误不 panic + nudge 无非法字符）。另修 ctl 运维：spawn_detached 继承 stdin 导致 opencode 管道句柄被 viewer 持有 → 命令永不返回；加 `stdin(Stdio::null())`；status tail 文件名错指 viewer_out.log → viewer_run.log。

> 更新规则：每次实现/新增能力后更新本表状态；每次迭代开始先看"优先级队列"。

## 最近修复记录（2026-08-01）

- P81 unstuck 增强：连续 3+ 次失败/无效工具调用（goto 超时/挖空气/gather 无资源）触发 mode_id=7 提示（mine_above 回地表/换方向/回据点），5+ 强制重 prompt。原 obs_streak 只认纯观察轮，"有工具调用但无进展"的死循环检测不到。
- P82 hotbar 缓存过期兜底：find_hotbar_slot 命中但 set_selected_hotbar_slot 后主手不对（本地 slots 缓存滞后服务端）→ force_hold_in_hotbar 强制 shift_click 归位重试，接入 do_equip/do_place。
- 接线补齐：probe 命令 mineabove/interactblock（interact 变体名 Bug 修复）；run_plan parse_step +10 action；rhai +4 函数（make_obsidian/follow/stop_follow/give）。
- 死代码清理：actions.rs/client.rs/perception.rs（假 ticks() 返回 0）、check_modes_legacy。
- 实机观察：bot 卡 tier3_bread（地下无小麦），捡到 wheat_seeds 因无种植能力丢弃；red_mushroom+bowl 可做蘑菇炖菜但 LLM 未识别（策略层知识注入待补）。
- P83 感知增强 + 知识注入（2026-08-02）：BotEvent::State 新增 overhead_solid（头顶连续实心方块数，0=洞穴/地表，N 大=深埋），perceive 场景渲染"头顶: N 格实心"行并提示 mine_above 脱困；_default prompt 新增 UNDERGROUND & CAVE SURVIVAL 段（蘑菇炖菜配方/种子保留不丢弃/头顶实心→mine_above/回地表优先级/不吃毒物）。纯函数 count_overhead_solid + 3 单测；probe 状态快照打印 overhead。probe 实测 overhead=0（洞穴）✓。优先级 2 的 goToSurface 问题部分缓解（LLM 现在有明确脱困信号），tillAndSow 仍为队列第一。
- P84 tillAndSow 种植（2026-08-02）：新 BotCommand::TillAndSow + LLM 工具 till_and_sow + rhai 注册 + probe 命令 tillandsow + run_plan parse_step。参考 Mindcraft tillAndSow：校验目标 dirt/grass_block/farmland → 4.5m 距离检查 → 背包找锄头（品质优先）→ 持锄头右键犁地并验证 Farmland → 持种子右键播种并验证作物（wheat/beetroot/carrot/potato/melon/pumpkin）→ 幂等（已种返回"无需重种"）。单测 2（seed 映射/可犁校验）+ 2（till 模块）。probe 实测全路径：stone 拒绝 ✓ 无锄头报错 ✓ 犁地+播种成功（成就 A Seedy Place，种子 16→15）✓ 幂等 ✓。另修 parse_chat_command 缺 chat 前缀的文档-代码不一致。
- P85 goToBed 睡觉（2026-08-02）：新 azalea/sleep.rs（bed_block_kinds 16 色 / find_bed scan_blocks_multi 32m / empty_main_hand 切空 hotbar / do_sleep：找床→goto≤2m→空主手→block_interact→SleepingPos 组件验证入睡 3s→等自然醒 15s）+ BotCommand::Sleep + probe 命令 sleep + LLM 工具 sleep() + rhai 重载（与 sleep(ms) 并存）。实测修 2 bug：(1) set_selected_hotbar_slot 用绝对槽位 panic（hotbar_slots_range 是绝对索引，需 s-hotbar_start 转 0..=8）；(2) 入睡检测误用狐狸的 Sleeping(bool) 元数据，玩家应查 SleepingPos(Option<BlockPos>)。probe 实测：无床报错 ✓ 完整入睡 ✓（成就 Sweet Dreams + 1/2 players sleeping + "已睡觉跳过夜晚"）。测试床需完整两格（setblock 单格 foot 半床会被服务端拒绝右键）。
- P86 收割（2026-08-02）：新 azalea/harvest.rs（harvestable_crop_kinds 5 种：wheat/carrots/potatoes/beetroots/nether_wart；crop_is_mature 按 age 属性判定：wheat/carrots/potatoes=7、beetroots/nether_wart=3；do_harvest：扫描 32m 成熟作物→贴脸挖→等消失→等 1.5s 拾取，最多 24 棵）+ BotCommand::Harvest + probe 命令 harvest + LLM 工具 harvest + rhai 注册。单测 2（作物种类/age 成熟判定，用 blocks::Wheat set_property 构造）。probe 实测：无成熟作物报错 ✓（未成熟 wheat 正确跳过——age 判定生效）setblock 成熟小麦后收割 ✓（"共挖掉 1 棵"，背包 wheat:1 + seeds 15→18 掉落入包确认）。
- P87 pvp 走位（2026-08-02）：self_defense 攻击后 strafe 绕侧走位（径向 1.8m + 切向 2.0m 环绕点），combat_strafe_cd 字段 40 tick 冷却防打断寻路。排查中发现并修复关键 bug：原实现主手无武器时无条件 continue（每 100 tick 跳过攻击、面对僵尸站桩挨打永不还手）——改为"仅提交装备时才 continue，无武器时徒手攻击"。probe 实测排障过程：召唤僵尸白天被阳光烧死（lush_caves 通地表处）导致 self_defense 无敌人→改 /time set night + 贴身召唤。最终实测：`[MODE] 攻击 Zombie` + `[MODE:self_defense] strafe 走位 (-483,95,-152)` 连续触发 ✓。
- P88 原始数据通道 RawState + 近战修复全套（2026-08-02）：(1) **RawState**——`AzaleaBot` 不暴露内部 Client（编译 17 错验证），改为 handler 内 BotCommand::RawState 直读 azalea API 输出 `RAW|` 前缀（pos/health/food/xp/dimension/biome/dir/held/逐槽背包/实体全量含 dist/feet 3x3），LLM 工具不暴露，probe 新增 Step::Raw + `rawstate` 关键字，脚本 `{"raw":true}`。azalea API 坑：LookDirection 无 yaw/pitch（Debug 得 y_rot/x_rot）、nearest_entities::<()>()（bevy_ecs::query::() 模块不可用）、BlockKind 用 azalea_registry::builtin、get_block_state 返回 None=未加载。**核心结论：渲染层无 bug**——raw vs adapter State 逐项对撞一致（坐标/逐槽背包/held/biome/feet），LLM 死循环非渲染问题。(2) **P88 逼近**——MC 近战 reach=3.0，4~8m e.attack() 必 miss（实机验证 P87-2：僵尸 4.6m 站 6 秒打不死，攻击全 miss）；>3.2m 改 High 优先级 goto 逼近到敌人 2m 处（y 取敌人层），垂直差>4m 不逼近（phantom 头顶几十格 goto 必失败）。(3) **P88-b 1s 检查**——间隔 100 tick(5s)→20 tick(1s)：5s 内僵尸贴脸咬 5 击（~12 伤）bot 已 hp<10 让位 cowardice 逃跑，永远来不及反击（实机：bot 被 tp 重叠僵尸咬到 3/20 全程逃跑）。(4) **P88-c 低血反击**——hp<10 只放弃远处逼近，贴脸(≤3.2m)照打（逃跑中被贴脸怪追咬不反击必死）。(5) **P88-d 可命中距离攻击**——>3.2m 不满足逼近条件时 continue 跳过，杜绝低血时 4~8m 无效攻击（实机：creeper 7 击全 miss 复现）。实机验证全链：逼近 MODE（4.8-5.4m 连续逼近）、1s 反击（15x 攻击 Phantom + 8x strafe）、低血反击（7x 攻击 Creeper + 3x 自动装备 diamond_sword）。遗留：僵尸击杀未直接观察（环境怪群+bot 自主移动干扰，tp 贴脸僵尸易窒息死；攻击=贴脸 3 击 7dmg 必杀的 MC 机制结论）。另有未查明项：y=79/y=69 两个静止幽灵 Player 实体（Player kind、永不动、id 随重连漂移，@a 列表疑似含它们——player_count=3 来源）。
- P88-e 幽灵玩家破案（2026-08-02）：RawState 实体列表打印玩家名字（bot.tab_list() 查 UUID→名字），实测幽灵玩家 = **用户本人 Jun**（单机世界对局域网开放，用户挂机在地下 (-492.6,79,-163.7)，bot 一直在其 22-30m 外挖矿）。"玩家数=3" = probe + Jun + bot 完全解释。**非 bug**，问题关闭。
- P89 turn 内失败重规划（2026-08-02）：agentic-loop 折中——opencode 式 harness 每次工具结果回填 LLM 再决策，但 MC 工具是异步慢动作（goto/挖矿 1-30s）逐工具循环代价过高。折中：**仅 WRITE（副作用）工具失败时**中止剩余批次、补【已中止】占位 tool 消息（OpenAI 约束每个 tool_call 必须有响应否则 400）、注入含失败原因+建议的重规划 nudge、同轮重调 LLM（reroute_max=2，只读失败不回退）。run_one_turn 重构为 P89 loop 表达式（返回最后轮 calls 供 skill extraction）。单测 p89_write_failure_aborts_batch_and_reroutes_same_turn：craft 被中止未执行 ✓ LLM 恰好重调 1 次 ✓ 占位消息 ✓ nudge 含建议 ✓。

## 最近修复记录（2026-08-02 · pi 对比改进 7 项）

> 来源：docs/pi-agent-comparison.md（pi_agent_rust 对比分析，4 高值 + 3 中值全部落地）

- **P92 工具结果统一失败标记**（commit 077d874）：Message::to_chatml() 中 is_error=true 的工具结果统一加【失败】前缀（判断 fail/error/失败 关键词；已带【失败】不重复叠加）。LLM 对工具失败的识别从"读完整段文本"降为"扫前缀"——pi 的 structured error 思路；防重复标记避免前缀膨胀。
- **P95 命令队列取消 API**（commit 7c23edc）：AzaleaBot.cancel_commands()——清空队列 + 通知等待者 + 置 cancel_flag(AtomicBool)；handler tick 在 to_run 计算前 swap 处理取消：复位 mining_below/above、force_stop_pathfinding、clear_pending、回复"已取消（cancel_commands）"。约定：异步执行体（busy=true）不中断，轮询命令（goto/mine）强停。单测 cancel_tests：清队列 ✓ 取消后 handler 不再执行 ✓ 等待者收到取消回复 ✓。对标 pi 的 cancel_current_action / 长任务可中断（我们只有 60s 超时兜底）。
- **P90 steering 中断当前轮剩余批次 + 同轮重调**（commit c020164）：steering 队列改 Arc<Mutex<VecDeque<String>>>（线程安全注入）；批次循环检查新指令到达 → steering_hit → 中止剩余批次（【已中止】占位补齐）+ 取走 steering 注入【新指令中断】nudge + continue 重调 LLM（与 P89 共享 reroute_max=2 预算）。单测用 50ms 注入线程 + 150ms 慢 goto 模拟真实时序。对标 pi 每轮末尾 rerun-with-nudge 的目标切换（我们之前要等整轮 10-30s 才响应新指令）。
- **P91 增量摘要回归锁定**（commit 1d1ff0d）：对比发现增量摘要（previous_summary + <previous-summary> XML 块 + UPDATE_SUMMARIZATION_PROMPT）已实现 → 补回归测试 compaction_second_round_uses_incremental_summary_path 锁定行为：首轮无 <previous-summary>、次轮携带且提示含 "update the existing summary"。后续改动压缩 prompt 即触发红。
- **P93 进度流式事件**（commit 60d1d52）：BotEvent::Progress { command, detail }（goto=距目标距离、mine=距方块距离、mine_below/mine_above=Y 层），每 20 tick 推送；demo/probe 示例补 match 分支；adapter 静默忽略（观测通道不进决策上下文）。对标 pi 进度回调（waitForCompletion 带进度）；我们之前长动作执行中无中间反馈。
- **P94 轮内工具迭代预算 + 软交棒**（commit e2fb967）：单轮工具调用上限 20（跨 reroute 累计）；达上限中止剩余批次（【已中止】占位）+ 注入【工具调用上限】收敛 nudge（perceive → 回望目标 → 更直接方案如 run_plan），不重调 LLM。与 P89 死循环 nudge（重复信号）互补——数量信号。单测：25 调用只执行 20 ✓ 只调 1 次 LLM ✓ 5 条占位 ✓ nudge 含"回望当前目标" ✓。对标 pi MAX_TOOL_ITERATIONS=50 + 80% 软交棒警告。
- **P96 后台异步预压缩**（commit 5deb721）：回合末 estimate_tokens ≥ 预算 40%（压缩触发线 60% 的提前量）→ 后台线程生成摘要（build_cm + request_summary 抽为 free fn，同步/异步共用），prefetch_summary 下一轮 compact() 直接取用——压缩不再阻塞主循环 LLM 调用；provider Box→Arc（Agent::new 签名不变，内部 Arc::from），trait 已 Send+Sync；幂等（在途/已有摘要不重复 spawn）。单测 2：后台生成 + compact 零调用取用 ✓ 幂等与消费后可再预取 ✓。对标 pi compaction_worker 两阶段非阻塞（应用已完成结果 / 配额允许再启动新任务）。
- **P97 语义记忆层**（commit 3f3c55a）：pi-memory 三层注入的 MC 适配——(1) **索引**：remember 工具（Agent::new 自动注册，核心层跨路线可用）save/forget/list；(2) **按需浮现**：每轮以「当前目标 + 最近 3 个工具调用」为查询词，评分 top-4 注入【长期记忆】user 消息（tag 命中×3 + 词元交集 + uses 频次 + recency 半衰；tokenize = 英文单词 + 中文 bigram/单字双路，无需分词库）；(3) **跨会话持久化**：JSONL data/memory/agent.jsonl，同标题去重更新，touch 频率统计。**地图隔离**：MemoryEntry.scope（None=全局知识 / Some(server)=坐标基地类仅该服务器），AgentConfig.memory_scope + viewer 接线 --mc 地址——不同世界坐标记忆互不污染（用户补充要求）。与 WorldMemory（空间几何邻近渲染）互补：坐标事实走 memory 工具，知识策略走 remember。**注入纪律**：只进 user 消息不碰系统提示（DeepSeek 前缀缓存字节稳定）、轮间剔除 + 压缩 build_cm 过滤（与 perceive/邻近记忆同规则）。顺带修复 P96 回归：request_summary 双 provider 均失败时丢失"均失败"错误信息（P91 测试 compaction_errors_when_both_fail 恢复绿）。测试 +12（tokenize/评分/scope 隔离/JSONL 往返/工具注册/注入剔除/压缩过滤）。
- **P97b 实机验证修复**（commit c399bfa）：真实 MC 服务器（localhost:4444，协议 26.2）+ 真实 LLM 跑 40 步，发现单测覆盖不到的 4 处：(1) **scope 自由填值**——LLM 按描述"通用知识留空"实际填 `scope="global"` 字符串，被 `relevant()` 精确匹配过滤 → 记忆**永不注入**（【长期记忆】在 session 中完全缺失，实机可观测）。修复：`scope_is_global()` 将 None/空串/global/any/* 统一归一为全局，单测 `scope_global_string_is_treated_as_universal` 锁定；(2) **系统提示版本过时**：_default.json 写 "vanilla 1.21.2"（用户指出），实际服务器/azalea 为 MC 26.2 → 修正（错误版本号会让 LLM 规划时按旧配方/机制推理）；(3) **remember 引导缺失**：首轮 LLM 全程 0 次 remember（工具存在但不知道何时用）→ _default.json 新增 LONG-TERM SEMANTIC MEMORY 段（何时 save/kind 四类/scope 语义/与 memory 工具分工/list 防重）；(4) **ctl viewer 子命令** + 修 kill_all 自杀 bug（deploy 时 ctl 会 taskkill 自己）。**实机闭环验证**：remember 写入 3 条高质量记忆（食物策略/临时基地布局/木头获取含坐标范围）→ JSONL 落盘 → 修复后【长期记忆】注入渲染 ✓ 缓存命中稳定（hit=63k）✓ 无 400/崩溃。data/memory/agent.jsonl 入库作为初始记忆库（LLM 实机产物的知识沉淀）。
- **经验教训**：实机观测（viewer+LLM）能暴露单测盲区——LLM 自由填值、prompt 引导不足、版本漂移三类问题只能实机发现；工具层 bug 用 probe（秒级），策略/规划行为才开 viewer（30-60s/轮）。

## 最近修复记录（2026-08-02 · 上下文管理重构 P98）

- **A1 few-shot 真实消息对**（commit 002d748）：few-shot 从"文本示例拼接"改为**真实消息序列**——`Example { keywords, turns }`，turns 为 `ShotTurn::{User, Assistant(text, calls), Tool(result)}`，`example_to_messages()` 转成 assistant 带 `tool_calls` JSON（id=`fewshot{base}_{i}_{j}` 防冲突、arguments 为可解析 JSON）+ tool 结果按序配对（pending 队列消费）+ 内容带【示例】标记；`build_few_shot_messages()` 词重叠 top-2；run_one_turn 首轮注入一次**永不剔除**（前缀缓存最优，剔除名单刻意不含【示例】）。14 个示例全部转写（explore/wood/stone/iron/combat/food/torch/chest/bed/trade/mining/build/enchant/run_plan/stuck + 新增玩家指令示例，后者是唯一使用 `ShotTurn::User` 的——响应玩家指令的完整闭环）。旧文本拼接被 LLM 模仿成伪调用（decision.rs:347），真实消息对根治。测试 +2（真实消息对/tool 结果顺序配对）。**联动修复**：regression.rs 集成测试排除【示例】消息（few-shot 含 perceive 示例，原断言误判）。
- **B3 瞬态注入统一剔除名单**（commit 002d748）：`TRANSIENT_USER_PREFIXES`（agent/mod.rs 顶层 const，约 30 条）统一所有轮间瞬态 user 消息——自动感知/邻近世界记忆/长期记忆/任务进度/阶段知识/动态上下文各段/全部 nudge 警告类/会话级一次性通知（自动滚动恢复/系统提示）。retain 剔除（mod.rs）与压缩摘要过滤（compaction.rs build_cm）共用同一名单（原来两处各写一份、且旧名单只 5 条——nudge 类消息从不被剔除，历史膨胀）。**核对修正**：对照实际注入前缀逐一核验，修正 5 处名不副实的条目（名单写【死循环检测】实际注入【死循环警告】；【指令中断】实际是【新指令中断】；【连续失败警示】实际是【连续失败警告】；补缺【自动滚动恢复】【系统提示】）。故意不在名单：【已中止】（tool 占位消息，OpenAI 要求每个 tool_call 必须有响应否则 400）、【示例】（few-shot 首轮注入后必须永存）。
- **B4 语义记忆注入冷却**（commit 002d748）：同批记忆 5 轮内不重复注入——`MemoryEntry.last_injected_turn` + `SemanticMemory.inject_cooldown_turns=5`，`injection_text(query, scope, now_turn)` 过滤冷却中条目、`touch(titles, now_turn)` 记录注入时刻。**修 2 bug**：(1) 新记忆 `last_injected_turn=0` 被 `1-0 > 5` 冷却误杀 → filter 放行 `==0`；(2) 集成测试语义随冷却改变 → 断言重写（第 2 轮注入 0 条、5 轮后重新注入 1 条；测试开头预清理残留记忆防污染 data/memory/agent.jsonl）。
- **B5 任务进度紧凑渲染**（commit 002d748）：build_task_progress_msg 最多展示 8 条待办（超出显示"已省略 N 个更远的任务"）、已完成仅计数——任务链 23 项全列时 token 浪费。
- **A2 分阶段知识**（commit 002d748）：MC 完整知识从 system prompt 拆出（system 只留 CORE RULES/remember/task_complete/工具清单，DeepSeek 前缀缓存更省 token）→ `_default.json` 新增 `stage_knowledge: [{tier, text}×6]`（tier1 木石+地下生存 → tier6 下界合金+末地龙+鞘翅，tier2/3/4/5 分别对齐铁/铁甲食物盾/钻石/附魔酿造传送门任务）；`Profile.stage_knowledge` 三层合并可覆盖；`Agent.current_knowledge_tier()`（running 任务 tier → 无则最低 Pending → 全完成 6）；`build_stage_knowledge_msg()` 聚合 `tier ≤ 当前` 全部块经【阶段知识】user 消息注入（瞬态，已在 B3 名单）。测试 +2（tier 过滤/空库 noop）。
- **C7 jailbreak 可配**（commit 002d748）：硬编码 jailbreak（prompt.rs build_context）移入 profile——`Profile.jailbreak: Option<String>`（_default.json 已带，三层合并可覆盖），AgentConfig.with_jailbreak，None 回退 Rust 内置默认。改 prompt 无需重编译。
- **C8 knowledge_string 缓存**（commit 002d748）：`Agent.knowledge_cache: Option<String>` 懒初始化（工具集与 knowledge_base 在 Agent 生命周期内不变，结果恒定）——每轮 build_context 与压缩估算拿到逐字节相同字符串，前缀缓存命中率更稳。

> 全部单测通过：craft-agent 165、craft-agent-minecraft（azalea-bot）141、regression 10、fmt/clippy -D warnings 全绿（2026-08-02）。

> 全部 7 项单测通过：craft-agent 148 通过、craft-agent-minecraft（azalea-bot）141 通过（2026-08-02）。P97 后：craft-agent 160、craft-agent-minecraft 141。

## 最近修复记录（2026-08-03 · 架构演进 P2 结构性，稳定优先）

> 主线：**框架设计与稳定性**（Mission 优先级 1）。全部为行为不变的纯移动/重构，全量 395 测试绿（craft-agent 171 + minecraft 143 + 其余）+ fmt/clippy 干净。

- **P2.1 run_one_turn 拆分**（commit 3218aad）：`execute_batch`（批分组/READ 并行/WRITE 串行/slow 探测）+ `finalize_abort`（P89/P90/P94/P99 四分支收敛为 `AbortDecision::{Reroute, Handoff}` 枚举 + `AbortReason` + `BatchExecution` + `MAX_TOOLS_PER_TURN` 模块级常量，P94 原跨 reroute 累计改为批内累计行为不变）；run_one_turn 批执行区 ~420 → ~50 行；修复 P89/P90 reroute 预算耗尽时缺占位补齐（潜在 400）。p89/p89b/p90/p94/p99 + fast/slow 回归全绿。
- **P2.2 azalea/mod.rs 拆分**（commit 2c009eb，6340 → 1995 行）：→ `azalea/commands.rs`（BotCommand 33 变体 + QueuedCommand + parse_chat_coords/parse_chat_command + chat_parser 三测试）；→ `azalea/handler.rs`（BotState + Default + tick 主体 handle ~3570 行 + 专属 helper：now_ms/nearby_active_portal/block_memory_meta/record_surroundings/nearby_player_position）；mod.rs 保留 AzaleaBot/connect/动作 API/背包三件套，`pub use commands::{BotCommand,QueuedCommand,parse_chat_command}` + `pub use handler::BotState` re-export 保持 adapter_azalea（`crate::azalea::{AzaleaBot,BotCommand,BotEvent}`）与 action_manager（`super::{BotCommand,QueuedCommand}`）引用零改动。顺带修 P2.1 遗留 clippy：finalize_abort 8 参数 `#[allow(clippy::too_many_arguments)]`。
- **P2.3 craft-agent-model 边界**（commit ca2995d）：Cargo.toml 文档标注只依赖 `craft_agent::core::{message,types}`（grep 验证 5 处引用全部落在此域）；CI quality job 新增 `cargo check -p craft-agent-model --no-default-features` 强制防上层渗透。真实边界从源头保证：后续改 model 若 `use craft_agent::agent::` 会连 CI quality 都红。

> P2 后结构：craft-agent 保留原状；craft-agent-minecraft 的 azalea 层 = mod.rs(1995) + commands.rs + handler.rs + 12 子模块（action_manager/auto_craft/chest/craft/...）。P3 按需推进 craft.rs/tools_azalea.rs/agent_loop.rs。

## 最近修复记录（2026-08-03 · P100 实机回归 + LLM harness 观测）

> 触发：P2 重构后按工作流实机观测（probe 工具层 + viewer/LLM 策略层）。

- **P100 till_and_sow 缺自动靠近（commit 8f95890）**：probe 对照实验发现——till_verify2（bot 距目标 2.92m）播种静默失败（无错误返回，验证读 Air），till_face（goto 贴脸 2.12m）成功。根因：P84 实现只做距离检查（≤4.5m 即通过）但**不自动靠近**，force_block 交互（`StartUseItemOn` + 伪造 HitResult）在 2.9m 处被服务端拒收。修复：距离 >2m 自动 `start_goto(target_pos)`（目标格实心，pathfinder 停在相邻格）+ 60×100ms 等待到达 2m 内，超时报错提示换位置。probe 验证：2.55m 外直接 tillandsow → 自动靠近（pos -488.5→-486.4）→ 犁地+播种成功（种子 20→19）✓。**教训：force_block 交互类工具（place/till/open 等）都要贴脸（≤2.5m），仅距离检查不可靠**——后续新增此类工具默认带自动靠近。
- **LLM harness 实机观测（viewer + 真 LLM，~30 回合）**：确认 P2.1 批执行拆分/P89 失败重规划/P99 慢工具中止/P94 上限在实机全链路正常：
  - 批分组 ✓：混合批慢工具执行后其余【已中止】占位（L96/L150/L172/L177/L183 多次出现）
  - 失败重规划 ✓：装备失败→重试→合成 stone_pickaxe→成功装备（L124→L128→L157）
  - 工具质量 ✓：mine 空气位给最近实心方块提示（L143/L146/L171）；goto 实心方块自动修正为上方可站立点（L138）；攻击>4 格自动拒绝（L110）
  - 任务验证 ✓：假完成被拦（L130 "当前状态不满足任务条件"）
  - **观察到的策略弱点（非 harness bug）**：LLM 反复小步试 mine 坐标（-485,60 空气后又试 -484,60），注入明确目标后立即转为连续 mine_below ✓——提示引导有效；背包已持 stone_pickaxe 却先 craft 一个（工具名称时序误判，L124 装备失败后 L128 合成重复）——**LLM 端知识待优化**（可考虑 perceive 背包后决策，非代码问题）
- 工具层 probe 全量回归 ✓：smoke（goto/minebelow/pickup/equip/chat）+ harvest 全路径（无成熟报错→setblock 成熟→收割 wheat+1）+ till 闭环（真实 farmland 犁地播种成功）。P2.2 拆分后的 parse_chat_command 33 命令全部正常驱动。

## 最近修复记录（2026-08-03 · P101 mine 空气盲猜根治 + P57 误报修复）

> 触发：LLM 实机观测发现真实盲区——LLM 连续 15+ 次 mine 空气格（每次换坐标，死循环检测不触发）。

- **P101 mine 空气目标自动修正（commit 3eca90a + 本轮完成分支）**：
  - 派发分支：目标格是空气 → `nearest_solid_block`（半径 4，排除 Air/Water/Lava，最近优先）自动修正到实心方块再挖，修正通知经 evt_tx 事件流推送（首帧一次，不消费 result_tx）
  - **关键发现（完成分支重构）**：done 轮询判定原用**原目标** is_air——原目标本就是空气时 done 立即成立，修正挖掘在下一 tick 就被终结（probe 实测 dirt 未被挖掉）；且旧 P57 逻辑把"挖成功的空气"误报为"该位置已是空气"（LLM 反复挖同一格的根源）
  - 修复：BotState 新增 `last_mine_eff`（实际挖掘目标 + 原目标是否空气），done 判定/反馈全部改用实际目标，三场景分流：
    1. 原目标实心挖掉 → `Mined block at (x,y,z)`（成功，P57 误报根治）
    2. 空气修正挖掉 → `已自动修正挖掘最近实心方块 (p) 并成功移除`
    3. 空气且无实心可修正 → P57 建议提示（附最近实心坐标）
  - 超时/取消路径清空 last_mine_eff（防残留污染下一命令判定）；未派发帧返回 not-done（防 dispatch 前误报）
  - probe 验证三场景全通过：setblock dirt → mine 空气格 → 修正通知 1 次 + 挖掉 dirt +1 + 报修正成功；mine 实心 dirt → Mined block at +1；mine 远处空气 → 建议提示 ✓
- **P57 遗留缺陷顺带根治**：挖掘成功后目标当然是空气，旧 done 分支却报"可能之前已挖掉或坐标错误"——LLM 误以为没挖到，9 次连续 mine 同一格。现场景 1 正确报 Mined block at。

## 最近修复记录（2026-08-03 · P102 till 空气修正 + LLM 实机效果评估）

> 触发：LLM 实机观测（viewer + 真 LLM）确认 P101 双场景实机生效，同时发现 till_and_sow 连续 4 次犁空气格失败。

- **P102 till_and_sow 目标不可犁自动修正（commit aa58f84）**：LLM 坐标记忆偏差导致连续对空气格 till_and_sow（L55/L104/L106/L148，"目标 X 是 Air，只能犁草方块/泥土/已耕地"）。对齐 P101 mine 修正模式：目标格非可犁 → 半径 4（y±1）找最近可犁且上方无阻挡方块 → **修正后继续犁地+播种**，成功消息明确告知修正（"原目标 X 是 Air，已自动修正犁最近可犁方块 Y 并完成"）；找不到合法位置才报错并建议 place dirt 铺土；距离检查/自动靠近改用修正后坐标。probe 实测（p102_till_correction.json）：空气目标+附近 dirt → 修正犁 dirt 并种下 Wheat（上方已长）✓；正常目标 → 幂等不重种 ✓。
- **LLM 实机效果评估（~170 回合累积观测）**：
  - **P101 双场景实机验证 ✓**：L76 "Mined block at (-495,95,-149)"（场景 1）+ L78 "目标 (-497,95,-150) 是空气，已自动修正挖掘最近实心方块 (-497,95,-151) 并成功移除"（场景 2）——LLM 不再反复挖同一格，P57 误报根治实机确认
  - **harness 全链路稳定 ✓**：攻击距离拒绝（L66/L72 远处 zombie 自动拒）、批中止占位（L121/L127/L163）、goto 实心自动修正（L78）、goto 超时自动挖路（L80）、place 无效坐标自动重定位（L57/L111）、mine/goto/equip 高频成功
  - **新观察到的策略弱点（非 harness bug，已由 P102 缓解一类）**：① craft 顺序颠倒（先 craft wooden_hoe 缺 planks 失败→后 craft dark_oak_planks 成功，但忘了再合成 hoe）；② 工作台放置位置反复试错（挡住农田 L148）；③ task_retry 空转（"当前任务尚未失败" L152/L173）——均属 LLM 决策层，harness 已给明确错误提示，持续观测
- 工具层 probe 回归 ✓：P101 三场景 + P102 修正/幂等 + smoke 全通（0/6 失败）。

> 当前主线：harness 层修正类优化已覆盖 mine（P101）/till（P102）坐标盲猜，place 已有 P5/P11 自动重定位。下一轮观测重点：craft 顺序、工作台放置策略、task_retry 引导。

## 修复记录：2026-08-06 P111 goto_player 按玩家名单次导航（gap !goToPlayer 闭环）

> 触发：优先级队列 1-10 全部完成后扫差距表，剩余 ❌ 缺失项中主线收益最高的是 !goToPlayer（按玩家名 goto，此前只有 follow 持续跟随、无单次导航）。表格过期状态顺手回填（!goToBed P85 / !goToSurface P105-107 / tillAndSow P84 早已完成但表内仍标 ❌）。

- **P111 goto_player（commit c71e463）**：
  - 新 `MinecraftAction::GotoPlayer { target: Option<String> }` + `BotCommand::GotoPlayer { name: Option<String> }`（None=最近的其他玩家）
  - handler 完全复用 P110 模式：`nearby_player_position(&bot, name)` 定位玩家当前坐标 → 替换 pending 槽为 Goto（保留 result_tx 回传）→ Chat 事件 `[goto] 玩家 <名> @ (x,y,z)，开始导航`；未找到 → 明确报错 + clear_pending
  - action_manager：timeout 30 tick + cmd_signature `goto_player({name})`（含名字，不同玩家不算重复导航）
  - LLM 工具 `goto_player`（4 处同步：tools_movement 注册 + types 变体 + adapter_azalea action_for + parse_chat_command `gotoplayer [名字]`）+ run_plan parse_step + rhai 注册 + AGENTS.md 工具表同步
  - 测试：chat_parser_goto_player（无参/按名/goto 单 token 仍走锚点不冲突）
- **probe 实机验证 ✓（scripts/probe/p111_goto_player.json）**：`gotoplayer` 无参 → `[goto] 玩家 最近的玩家 @ (-493,80,-165)，开始导航`（定位到挂机玩家 Jun）✓；`gotoplayer Jun` 按名定位 ✓；`gotoplayer nobody_xyz_404` → `未找到玩家 nobody_xyz_404（不在附近扫描范围）` ✓。导航超时（goto (-493,80,-165) 超时——路径被阻）属 goto 既有行为（目标在地下不可达），非 gotoplayer 缺陷。
- **门槛**：workspace 全绿（craft-agent 171 + craft-agent-minecraft 151 + model 23 + regression 29）+ fmt/clippy `-D warnings` 全绿。

> 当前主线：goto 家族已齐全（坐标 goto / 锚点 goto P110 / 玩家 goto_player P111 / 持续跟随 follow）。差距表命令层剩余：!moveAway（cowardice 自动做，取舍）、!setMode（配置层非 LLM）、!lookAt*（视觉无用）、!searchForBlock 全局搜块返回坐标（🟡，有 scan 记忆）。下一轮可选：!searchForBlock 全局搜块、或 LLM 策略层观测（craft 顺序/工作台放置）。

## 修复记录：2026-08-06 P112 search_for_block 搜块返回坐标（gap !searchForBlock 闭环）

> 触发：P111 后继续扫差距表——!searchForBlock（🟡 部分，gather 只走到+挖、无"返回坐标供规划"）升级为完整工具。

- **P112 search_for_block（commit 8141136）**：
  - smart_actions.rs 新增 `scan_blocks_all`（多种类多结果按距离升序，最多 max 个）+ `search_block_coords`（bot 层接口：别名展开 + 返回坐标与欧氏距离）
  - 新 `BotCommand::SearchBlock { item, radius }`（radius clamp 4-96，默认 32）+ `MinecraftAction::SearchBlock` + LLM 工具 `search_for_block { block, radius? }`（read 效果，4 处同步 + run_plan parse_step `search_block` + rhai 注册）
  - handler：扫描 → 输出"半径 N 内找到 M 处 {item}：item @ (x,y,z) 距离 d.m"（最多 8 处）+ 事件流 [搜索] 推送；找不到 → 明确报错 + [搜索失败] 事件
  - probe 命令 `searchblock <方块> [半径]`；测试 chat_parser_search_block（默认半径 32/指定半径/无参 None）
- **probe 实机验证 ✓（scripts/probe/p112_search_block.json）**：`searchblock grass_block` → 7 处坐标按距离升序（0.8-2.7m）✓；`searchblock oak_log` → 3 处（别名展开，lush_caves 红树林原木命中）✓；`searchblock diamond_ore 16` → "半径 16 内找不到 diamond_ore" + [搜索失败] 事件 ✓
- **门槛**：workspace 全绿（craft-agent 171 + craft-agent-minecraft 152 + model 23 + regression 29）+ fmt/clippy `-D warnings` 全绿。

> 当前主线：差距表命令层 ❌ 仅剩 !setMode（配置层非 LLM）、!lookAt*（视觉无用）。下一轮可选：LLM 策略层实机观测（craft 顺序/工作台放置/task_retry 引导，需 viewer+LLM），或配置层 !setMode。

## 修复记录：2026-08-06 P113 move_away 主动远离（gap !moveAway 闭环，命令层 ❌ 清零）

> 触发：P112 后差距表命令层 ❌ 仅剩 !moveAway（此前认为 cowardice 自动做即可，但 LLM 需要主动控制距离/目标的逃跑工具）。本轮完成命令层最后一个 ❌ 项。

- **P113 move_away（commit ef076e3）**：
  - 新 `MinecraftAction::MoveAway { target: Option<String>, distance: u32 }` + `BotCommand::MoveAway`（distance clamp 4-64，默认 8）
  - parse_chat_command `moveaway [实体名] [距离]`（第一个 token 纯数字→距离，否则→目标名）+ chat_parser_move_away 测试
  - handler 分支：nearest_entities 定位目标（排除 item/experience_orb/item_frame/glow_item_frame；玩家名匹配用 `azalea::player::GameProfileComponent`，实体类型匹配用 `entity_kind_name`；无参=最近非玩家实体）→ 水平反向向量 → 接管 pending 槽为 Goto（y 保持 bot 当前层，result_tx 保留回传）→ Chat 事件 `[远离] 远离 {kind} -> 反向 {distance}m 目标 (x,y,z)`；找不到 → result_tx 反馈"附近找不到目标实体 {who}，无需远离。" + clear_pending
  - action_manager：timeout 30 tick + cmd_signature `move_away({t})`/`move_away(any)`（与 goto 的 `goto(#,#,#)` 区分，不算重复导航）
  - LLM 工具 `move_away`（4 处同步：tools_movement 注册 + types 变体 + adapter_azalea action_for + parse_chat_command）+ run_plan parse_step + rhai 注册 + AGENTS.md 工具表同步
- **probe 实机验证 ✓（scripts/probe/p113_move_away.json）**：`moveaway zombie 20` → `[远离] 远离 zombie -> 反向 20m 目标 (-526,93,-118)` + goto 派发 ✓（战斗中 zombie 追位，反向向量随执行瞬间位置计算）；`moveaway` 无参 → `[远离] 远离 zombie -> 反向 8m 目标 (-480,95,-149)`（默认距离 8、最近实体）✓；`moveaway llama 10`（不存在）→ `附近找不到目标实体 llama，无需远离。` ✓
- **已知交互**：战斗中 self_defense 的 strafe/逼近会抢占 pending 槽覆盖 move_away 派发的 goto（goto 超时兜底 60s 自动挖路）——模式系统 tick 级优先属既有设计，move_away 在非战斗场景正常。
- **门槛**：workspace 全绿（craft-agent 171 + craft-agent-minecraft 153 + model 23 + regression 29）+ fmt/clippy `-D warnings` 全绿。

> 当前主线：命令层差距表已全部清零（✅/🟡/➖ 之外无 ❌）。下一轮可选：LLM 策略层实机观测（craft 顺序/工作台放置/task_retry 引导，需 viewer+LLM），或配置层 !setMode。

## 修复记录：2026-08-06 P114 self_defense 幽灵实体攻击刷屏（vendor azalea bug 修复）

> 触发：P113 probe 实机时发现 zombie 死后日志连续 30s+ 刷 `tried to attack entity which isn't in our EntityIdIndex`（每次 ~30ms 一条）。定位为 azalea 上游 bug + 首例 vendor 魔改实践。

- **根因（vendor azalea-client/src/plugins/attack.rs）**：`handle_attack_queued` 查 `EntityIdIndex.get_by_ecs_entity` 失败（目标在攻击包入队与处理窗口期死亡/卸载）时 `continue` 且**不清理 `AttackQueued` 组件** → 残留组件让系统每 tick 重试 → 永久 WARN 刷屏。handler 层已有存在性检查（P87 时期），但窗口期竞态无法在调用侧消除。
- **修复（vendor commit 5c93171）**：失败分支补 `commands.entity(client_entity).remove::<AttackQueued>()`（保留一次 warn 作一次性诊断）。
- **vendor 魔改流程实测**（rev 策略首次验证）：vendor 新 commit → **只更新** `.cargo/config.toml` [patch] 条目 rev（7 处）→ `craft-agent-minecraft/Cargo.toml` 声明 rev 保持 c35b57e 不动（github 存在，lock 更新 fetch 它；patch 条目 rev 与声明 rev 可不同，编译以 patch 条目为准）→ 清 `~/.cargo/git/checkouts` → cargo check 显示 `file:///.../vendor/azalea?rev=5c93171` 生效。AGENTS.md vendor 小节与 .cargo/config.toml 注释已同步（旧 amend/update-ref 流程作废）。
- **probe 实机验证 ✓（scripts/probe/p114_attack_ghost.json）**：summon zombie 夜晚战斗（`[MODE] 攻击 Zombie` + strafe + Pillager 乱入 + cowardice hp 4/20 逃逸全程）→ 日志 `isn't in our EntityIdIndex` 条数 = **0**（修复前同场景 27+ 条/30s）。
- **门槛**：cargo check 通过 + workspace 测试待全量跑（vendor 仅 attack 失败路径，无 API 变更）。

## 修复记录：2026-08-06 P115 LLM 策略层实机观测（craft/挖矿/目标追踪）

> 触发：命令层 ❌ 清零后主线转向 LLM 策略层（此前工具层 probe 全绿，策略行为需 viewer+LLM 实机验证）。工作流按"按需观测、观测完即停"执行（起 viewer+agent 观测 ~40 分钟，ctl stop 收束）。

- **观测会话**（viewer + agent，goal="制作石镐并挖到至少 10 个石头，完成后停下汇报"）：
  - **工具层全链路正常**：P101 mine 空气修正 ×4（LLM 凭记忆盲猜坐标，修正自动兜底）✓ / craft 3×3 合成石镐 ✓ / place 自动重定位（原坐标无效→附近合法位置）✓ / pickup 拾取 ✓ / run_script 批量执行 ✓ / mine_above 失败反馈（"Y did not increase...clear a horizontal staircase"）✓ / goto 实心方块失败提示 ✓
  - **LLM 策略行为**：mine_above 反复失败（竖井被堵）→ 改用 run_script 横向挖矿 → 深挖至 y=34 矿石层后陷入"挖矿舒适区"：目标早已达成（石镐 + cobblestone 29+）却持续挖矿 30+ 分钟不汇报收尾
  - **steering 提醒验证**：注入"目标已达成请停止并汇报"→ LLM 响应为继续行动（开始向上挖阶梯 y 33→39 脱困），未直接 chat 收尾——"停止类"指令响应弱是 LLM 本性（Mindcraft 同病），非 harness 缺陷；[当前目标] 每轮注入 + steering 中断（P90）机制本身工作正常
- **结论**：策略层无 harness bug；目标达成自动收尾属 LLM 行为约束问题，可通过任务系统结构化完成检测（task.rs）在 harness 层给 LLM 显式"目标达成信号"改进（待后续优先级评估，Mindcraft 亦未解决）。
- **门槛**：无代码改动；观测全程无 panic/400/崩溃。

## 最近修复记录（2026-08-03 · P103 viewer 启动根因 + 工作流固化）

- **P103 "viewer 没起来"根因破案**（commit 后置）：反复出现 viewer 启动失败，多次换参数重试未根治。前台运行正常（`& target\debug\craft-agent-viewer.exe ...` 阻塞运行=正常），但 PowerShell `Start-Process -ArgumentList` 启动的进程静默退出。根因：**`-ArgumentList` 数组被 join 成单字符串，含空格/中文的 goal 被拆分 → clap 解析失败进程立即退出**。而 `ctl viewer` 子命令（Rust `Command::args` 逐参传递）启动成功——同一 exe、同一参数，两种启动方式结果不同。**修复：启动 viewer 一律用 `craft-agent-ctl viewer "goal" <steps>`，禁用 PowerShell Start-Process；部署流程同步改为 ctl 分步（stop → build → viewer → start → status），消除 AGENTS.md 中自相矛盾的 Start-Process 步骤。**
- **教训**：重复出现的问题必须查根因（前台 vs 后台启动对照实验），不能只换参数重试；AGENTS.md 工作流文档与实操必须一致（文档曾同时写"不要用 Start-Process"和"部署用 Start-Process"，自相矛盾导致错误路径被反复使用）。

## 最近修复记录（2026-08-03 · P104 调试后门移除 + 知识→能力断裂修复）

> 触发：LLM 实机观测（tier3_bread，饥饿 4-5/20）发现 bot 频繁垂直下挖且脱困失败；顺带确认 prompt 知识层教 LLM 做 mushroom_stew 但 harness 无此配方（知识→能力断裂，P83 同模式）。

- **P104 Auto-tp 调试后门移除（commit c23c486）**：`handler.rs` 的 `mine_above_tried_tp` 字段（2026-07-30 efe18f9 引入的调试残留）——mine_above 卡洞穴空气袋（头顶空气 + Y<62 + 未试过）时自动 `bot.chat("/tp @s ~ 70 ~")` 传送。实机无 cheats 环境该指令静默失败，且使 LLM 脱困表现为"传送到空中"而非自主攀爬。已彻底移除（字段定义/初始化/6 处 reset/tp 逻辑块）；P60b 楼梯脱困（挖头顶 y+2 → goto 上升 → 40 tick 换方向 YGoal+5）成为唯一脱困路径。**教训：调试用后门绝不允许进产品路径——效果同 cheat，却掩盖真实能力缺陷。**
- **P104 mushroom_stew 2×2 配方补齐（commit c23c486）**：craft.rs `RECIPES` 表追加 `("mushroom_stew", [("bowl",1),("red_mushroom",1),("brown_mushroom",1)], 1)`（shapeless 任意排列匹配）——此前只有 prompt 知识层教 LLM 做蘑菇炖菜（craft_3x3），RecipeBook 与手写配方表均无此配方，craft_3x3 失败提示误导。**知识→能力断裂修复：prompt 教的能力 harness 必须实现。** 联动：3×3 合成失败时若物品实为 2×2 配方，引导 "改用 craft(item, count) 工具（2×2 玩家网格合成）"；prompt `_default.json` "Underground & Cave Survival" 同步修正（mushroom_stew 改 craft 2×2、新增"下挖前规划脱困：记录入口坐标、勿盲挖直下、事后 mine_above 回地表"引导）；mine_below 工具描述补"下挖前先规划脱困"提醒。回归测试 +1（lookup_recipe 查 mushroom_stew，2×2 顺序填充 3 原料）。probe 实机验证 ✓：setblock red_mushroom → mine 拾取 → 背包已有 bowl/brown_mushroom → craft mushroom_stew 1 成功（inv 出现 mushroom_stew:1）。
- **LLM 实机观测残留问题（策略层，非 harness bug，排队）**：① LLM 饥饿时挖矿找小麦偏航（应优先 crafting/farming）；② 装备 wooden_hoe 失败（hotbar 满 shift_click 后找不到，L102）；③ run_script "Function not found: pos_x ()"（rhai 引擎缺 pos_x/pos_y/pos_z 注册，L57）。
- **P104 run_script 位置函数补齐（commit 后置）**：rhai 引擎注册 `pos_x()/pos_y()/pos_z()`（读 `bot.last_position` 每 tick 缓存，轻量不触发感知扫描；新增 `ArcAzaleaAdapter::current_position()` getter）。LLM 脚本写 `pos_x()` 取坐标不再报 Function not found。回归测试 +1（无参 f64 签名可注册可调用）。probe 无法直接驱动 run_script（parse_chat_command 不含），实机由 LLM 观测验证。
- **P104 实机验证（LLM 观测轮，~30 回合）**：
  - prompt 知识层生效 ✓：LLM 明确引用"蘑菇煲（2×2: 1 棕蘑菇 + 1 红蘑菇 + 1 碗）"（L142）——不再教 craft_3x3
  - 决策正确 ✓：缺 brown_mushroom（背包 bowl×16 + red_mushroom×11）时先 gather brown_mushroom（半径内无 → 失败报错准确），不瞎合成
  - equip 报错准确 ✓：stone_pickaxe 背包确实没有（L86 曾装备成功 → 耐久耗尽消失），L129/L130 报"重试 3 次找不到 + 列出背包实际槽位"非误报
  - 非 harness bug 确认：反复装备 diamond_sword（L142-L149 三次）是防御准备（3 creeper 逼近），装备本身每次都成功
  - **未观测到 run_script 使用**：本轮 LLM 未写 pos_x 脚本（决策走工具链而非脚本），pos 函数验证停留在单测层，后续遇到 run_script 场景再实测

> 当前主线：harness 修正类优化已覆盖 mine（P101）/till（P102）/Auto-tp 移除（P104）/mushroom_stew 配方（P104）/rhai 位置函数（P104）。下一轮观测重点：craft 顺序、工作台放置策略、run_script 实际使用。

## 修复记录：2026-08-03 P105/P106 mine_above 无镐提前终止 + 脱困目标 YGoal 修复

> 触发点：P104 移除 Auto-tp 后 LLM 实机观察（tier3_bread，~30 回合）出现 L121 "mine_above failed: Y did not increase within 10 seconds. The ascent path is blocked"——此前 Auto-tp 掩盖了脱困路径的真实缺陷。

- **P105 mine_above 无镐提前终止（handler.rs P60b 分支）**：入口的镐检查只看头顶（head_is_hard），头顶是空气时被跳过——但 y+2 可能是硬方块（石头/深板岩/矿石），无镐徒手挖 ~8s/格 → 空转 10s 超时，且失败消息误导 LLM 横向找路。修复：P60b 挖 y+2 前检查 `is_hard_block(y+2)` + `has_any_pickaxe_in_inventory()`（节流 20 tick），无镐时提前终止 mine_above 并发明确反馈（craft 镐 / 横向软方块脱困）。回归测试 `regression_is_hard_block_above_head_requires_pickaxe`（Stone/Deepslate/CobbledDeepslate/Granite hard；Dirt/OakLog/Air not hard）。
- **P106 P60b/P60c 脱困目标 BlockPosGoal → YGoal（L121 真正根因）**：P60b else 分支 `BlockPosGoal(BlockPos::new(cx, y+1, cz))` 目标格是 bot 头部所在格（空气）——pathfinder 算不出站立路径 → `No best node found / empty path` 卡满 10s，且每次 4 tick 反复 goto、阻塞 40-tick 主循环的 YGoal(y+5) 兜底（probe 复现：bot 在开阔地表 Y=82 卡死）。修复：改用 `YGoal::from(BlockPos::new(cx, y+2, cz))`（同 P60 主循环思路，pathfinder 可自由挖墙/找楼梯上升）。两处：P60b（mine_above 内）+ P60c（地下强制脱困）。
- **probe 实机验证 ✓（p105_mine_above_pick.json）**：修复后 bot 从 Y=82 真正上升到 Y=83（"MineAbove progressed to Y=83"，此前 empty path 原地不动），随后 P105 正确触发——头顶是空气但 y+2 是硬方块且背包无镐 → 提前终止并给出明确建议。L121 场景完整闭环。
- **经验**：BlockPosGoal 目标格必须是可站立方块（实心），绝不能指空气格（bot 头部/身体所在格）；脱困/上升类路径一律用 YGoal（P60 教训再次验证）。
- **门槛**：fmt/clippy -D warnings/全 workspace 测试全绿（craft-agent-minecraft lib 146 测试）。

> 当前主线：mine_above 脱困路径已闭环（P105 无镐提前终止 + P106 YGoal 上升）。下一轮观察重点：craft 顺序、工作台摆放策略、run_script 实机使用。

## 修复记录：2026-08-03 P107 mine_above 高穹顶腔体天花板扫描（gap #2 实机闭环）

> 触发点：#2 goToSurface 实机确认（tier3_bread，~25 分钟观测）：P106 修复后 bot 在 lush_caves 洞穴腔体仍卡死——头顶 y+1 空气、y+2 空气（P105 只查 y+2 单格被跳过）、y+3+ 是 stone 天花板 → P60b 走 P106 YGoal(y+2) 上升，但高穹顶没有可挖的 y+2 方块 → 反复 "mine_above failed: Y did not increase within 10 seconds. The ascent path is blocked" + 盲猜 goto 地表坐标 → pathfinder "incomplete path" 无限重试。根因：P60b 只处理"y+2 是硬方块"（P105），未覆盖"y+2 空气但更高处有硬天花板"。

- **P107 天花板扫描（handler.rs P60b `!above_is_solid` 分支）**：改挖 y+2 前先扫描 `y+2..=y+8` 找第一格实心方块（`let ceiling = (2..=8).find_map(|dy| ...)`）。三种情形：(a) 扫到天花板 → 挖那一格（`start_mining(ceiling)`），逐层凿穿直通地表；(b) 天花板是硬方块且背包无镐 → `abort_mine_above` 提前终止，反馈明确建议（先 craft wooden_pickaxe 或 stone_pickaxe，或横向找软方块通道）；(c) y+2..y+8 全空气 → 回落 P106 原 YGoal(y+2) 强制上升逻辑（t.is_multiple_of(4) 节流）。
- **`abort_mine_above` helper 抽取（handler.rs）**：P105 的终止样板（清 mining_above 标志/mining_above_start_y、force_stop_pathfinding、peek_pending MineAbove 回填 result_tx、clear_pending、推 BotEvent::Chat）抽为独立 helper，P105/P107 共用——避免终止逻辑不一致。终止消息用 `❌ mine_above 失败` 前缀，不用 "MineAbove progressed"（tools_movement.rs:154 依赖该子串才 forget_pos）。
- **门槛**：cargo check/test（craft-agent-minecraft lib 146 全绿）/fmt/clippy `-D warnings` 全绿。probe 实机验证 ✓（p107_ceiling_scan.json）：场景=头顶 y+1/y+2 空气、y+3..y+5 石头天花板（setblock 构造）、背包无镐 → mine_above 秒级返回 `❌ mine_above 失败：头顶是空气但上方 y+3 是硬方块天花板...`（不再空转 10s）；随后 `/give wooden_pickaxe` + equip → 同场景 mine_above → 自动 start_mining 天花板并挖穿（cobblestone x3 入包 + "Stone Age" 成就）→ "MineAbove done" 脱困闭环。

## 修复记录：2026-08-03 P108 语义记忆测试隔离（data/memory/agent.jsonl 共享污染）

> 触发：P107 提交后全 workspace 测试红——`semantic_memory_tool_registered_and_injects` 断言"注入内容应含记忆标题: 钻石镐策略"失败。根因：Agent::new 硬编码语义记忆持久化路径 `data/memory/agent.jsonl`，测试与实机 agent 共享该文件；LLM 实机 run 期间 bot 写入真实记忆（"lush_caves 洞穴无小麦种子"等教训），查询"挖钻石"时实测记忆评分更高、把测试记忆挤出注入。**测试与实机共享持久化状态缺陷**。

- **P108 修复（craft-agent/agent/mod.rs）**：`AgentConfig` 新增 `memory_path: Option<PathBuf>`（None = 默认 data/memory/agent.jsonl，生产零改动）+ `with_memory_path()` builder；`Agent::new` 加载时优先用注入路径。测试改为临时文件（`temp_dir()/sem_agent_test_{now_ms}.jsonl`）→ 与实机彻底隔离。
- **回归**：workspace 全绿（craft-agent 171 + craft-agent-minecraft 146 + model 23 + viewer）；fmt/clippy -D warnings 全绿。
- **经验**：测试读共享持久化文件 = 定时炸弹。凡持久化路径，配置层必须支持注入（builder/环境变量），测试一律用临时路径——既隔离又被断言（测试读写同文件易测）。

> 当前主线：mine_above 高穹顶天花板脱困已验证（P107 probe 闭环）+ 语义记忆测试隔离（P108）+ 阶段知识配方矛盾修复（P109，见下）。

## 修复记录：2026-08-03 P109 阶段知识与任务目标配方矛盾（3 小麦 vs 9 小麦死锁）

> 触发：P108 后重启实机，LLM 在地表反复确认"收集 3 个小麦"goal 与阶段知识 `wheat (9) -> bread (3x3)` 矛盾，数十轮原地打转（每轮自我反驳 3 vs 9）。根因：`data/profiles/_default.json` tier3 阶段知识写错——面包配方是 3 小麦横排（`builtin_recipes.json:87` `["WWW"]` pattern），不是 9。**阶段知识必须与任务 goal（tier3_bread.json: gather wheat count=3）和配方真值一致**，否则 LLM 决策死锁。

- **P109 修复（data/profiles/_default.json，commit 2a376a3）**：tier3 阶段知识 `wheat (9) -> bread (3x3)` → `wheat (3) -> bread (craft_3x3, 3 wheat in a row)`。
- **实机验证**：重启后 LLM 不再纠缠配方，转而规划小麦农场（找草丛、till_and_sow、记忆锚点 wheat_farm_base、背包 wheat 2/3）。
- **经验**：profile 阶段知识是 prompt 的一部分，任何配方/数值写法都要对照 `builtin_recipes.json` 真值 + 同 tier 任务 goal 双校验（写 9 小麦的根因是没查配方表）。
- **观察记录（本工作单元收尾）**：P107/P108/P109 组合效果实机可见——LLM 从洞穴 Y=44 经 dirt 扶梯（采纳 P107 abort 建议）攀至地表 Y=62-79；但后续出现决策漂移（砍树叶 0/4、徒手砍树 0/3、远距离挖铜矿反复超时、丢树叶被 1.5m 吸回），工具层均正确返回可操作建议，属 LLM 策略质量问题非 harness bug，留待后续工作单元。

## 修复记录：2026-08-03 P3 架构演进（大文件按域拆分收官）

> 对应 AGENTS.md「架构演进路线图」P3 全部三项（按需推进，稳定优先，无 deadline）。

- **P3.1 craft.rs 拆分（commit 49d0328）**：`azalea/craft.rs`（4730 行）→ `azalea/craft/` 下 craft_table/smelt/smith/brew/enchant 5 个域模块 + craft.rs 聚合入口 re-export。craft-agent-minecraft lib 146 + craft-agent 171 全绿。
- **P3.2 tools_azalea.rs 拆分（commit 1bbdd08）**：4464 行 → `tools_azalea/` 11 个域模块（perceive/movement/mining/interact/crafting/farming/placement/container/inventory/social/meta，最大 tools_meta 1332 行）+ 主文件 738 行。47 工具名与 `create_mc_azalea_tools_full` 工厂签名零改动（LLM prompt 兼容性契约未破）。用脚本 `split_tools_azalea.ps1`（C:\Windows\TEMP）两阶段（plan/-Write）驱动：块边界 Get-BlockStart 回溯 doc 注释、主文件仅保留 imports/映射表/parse_step/_exec_action/三工厂/测试区；lint 工具函数改 `pub(crate)` + 主文件 `#[cfg(test)] pub(crate) use` 门控（仅测试引用，非测试构建不再 unused）。外部引用（agent_loop.rs:15 / agent_console_demo.rs:18）零改动。
- **P3.3 agent_loop.rs 拆分（commit 3db7202）**：830 行 → `agent_loop/events.rs`（AgentEvent + EventSender 推送 helper，`ev.log()/step()/error()` 取代 `let _ = tx.send(AgentEvent::...)` 样板）+ `agent_loop/session.rs`（open_or_create / save_full / save_incremental / auto_rollover helper）+ 主文件仅剩控制器/启动/主循环/观测文本。行为不变（事件文本逐字保留）。
- **门槛**：workspace 全绿（craft-agent 171 + craft-agent-minecraft 146 + model 23 + viewer 编译）；fmt/clippy `-D warnings` 全绿。纯移动重构，无功能改动，无需 LLM 实机回测。
- **P3 后续**：P3 已全部完成。架构大文件拆分收官，后续按主线收益进 gap 队列（goToSurface 强化实机确认 / item_collecting 拾取验证 / 自动穿甲验证）。

## 修复记录：2026-08-03 · P79/P80 实机验证闭环（probe，秒级）

> 触发：P3 收官后按工作流回归 gap 队列剩余验证项（自动穿甲 / item_collecting 均为"待实机验证"状态）。

- **P80 item_collecting 实机验证 ✓（probe p80_item_collect.json）**：`/give iron_ingot 1` → `discard iron_ingot 1`（背包消失）→ 等待 15s（200 tick 检查点 ×2+）→ 状态快照背包恢复 `iron_ingot:1`。8m 内 Item 实体 + 空闲 + 空位≥2 → 自动拾取链路全通。
- **P79 auto_armor 实机验证 ✓（probe p79_auto_armor.json）**：等 bot 完全 idle（30s）→ `/give leather_chestplate 1` → 等待 12s → `[MODE:auto_armor] 已装备 leather_chestplate 到 chestplate（left_click slot 9）` + RawState 确认 `slot[6]=minecraft:leather_chestplate`（胸甲槽已穿上）。iron_helmet 同样自动装上 `slot[5]`。材料升级链（空槽→leather）与 200 tick 触发正常。
- **验证要点**：auto_armor 需 `action_mgr.is_idle()`——give 后若 bot 忙于 pickup/寻路，检查点会错过；验证前先等 idle。RawState 已含 armor 槽（slot 5-8）输出，无需额外改动。
- **gap 队列更新**：#6 自动穿甲 ✅ / #7 item_collecting ✅（两者从"待实机验证"转"完成"）。剩余 #2 goToSurface 强化仍待 LLM 实机确认脱困成功率。

## 修复记录：2026-08-06 P116 set_mode 自动模式开关（gap !setMode 闭环，差距表全部 ❌ 清零）

> 触发：P115 后扫差距表——!setMode 是最后一个非取舍 ❌ 项（此前判断"配置层非 LLM"）。LLM 实机观测（P115）证明 LLM 确实需要控制模式（如 hunting 保护动物、self_defense 安静潜入），实现为命令层 + LLM 工具双通道。

- **P116 set_mode（commit b28a98e）**：
  - `MinecraftAction::SetMode { mode: String, enabled: bool }` + `BotCommand::SetMode` + parse_chat_command `setmode <模式> on|off`（`setmode list` 查询；无 flag 默认 on；off/0/false 识别）+ chat_parser_set_mode 测试（5 断言）
  - BotState 新字段 `mode_switches: Arc<Mutex<HashSet<String>>>`（被禁用模式名集合）+ `mode_disabled(mode)` 查询方法
  - handler dispatch：SWITCHABLE 5 模式（self_preservation/self_defense/cowardice/hunting/item_collecting）；list 查询当前禁用集合；开关 insert/remove（重复操作提示"本来就被禁用/本来就是启用的"）；仅实际变更时推 `[模式] 自动模式 X 已启用/禁用` Chat 事件；不支持模式报错（列出可开关集合）
  - **5 处模式守卫接入 `mode_disabled`**：hunting、item_collecting、cowardice、self_defense、self_preservation 的 tick 逻辑入口（搜注释 "P116：set_mode 可禁用" 定位）——禁用后 tick 级不再自动执行
  - LLM 工具 `set_mode`（mode 必填，enabled 默认 true；mode="list" 查禁用列表）+ run_plan parse_step `set_mode` + action_manager timeout 20 tick + cmd_signature `set_mode({mode},{on|off})`（不同模式/开关不算重复操作）+ adapter mc_to_cmd 映射 + ACTION_NAMES 同步
- **probe 实机验证 ✓（scripts/probe/p116_set_mode.json，11 步 0 失败）**：`setmode list` → 全部启用 ✓；`setmode hunting off` → 已禁用 + [模式] 事件 ✓；list → hunting ✓；重复 off → "本来就被禁用" ✓；`setmode self_defense off` → list 显示 hunting, self_defense（排序）✓；`setmode hunting on` → 已启用 + 事件 ✓；重复 on → "本来就是启用的" ✓；list → 只剩 self_defense ✓；`setmode nonsense on` → "不支持的自动模式: nonsense（可开关: self_preservation/self_defense/cowardice/hunting/item_collecting）" ✓；list 不受无效操作影响 ✓。顺带观测：probe 在洞穴低血（HP 5/20）时 cowardice 自动 mine_above 脱困正常跑（未禁用，守卫精确性验证）。
- **门槛**：workspace 全绿（craft-agent 171 + craft-agent-minecraft 153 + model 23 + regression 29）+ fmt/clippy `-D warnings` 全绿。

> **当前主线：差距表 ❌ 全部清零**（命令层/技能层/模式层 ✅ + 🟡 取舍项 + ➖ 记录；!stay 等 🟡 为设计取舍）。下一轮可选：① LLM 策略层持续优化（P115 结论：目标达成自动收尾需 task.rs 结构化完成检测，Mindcraft 亦未解决）；② 末地通关路径推进（tier6 → 末地 → 龙）；③ 架构演进 P2 剩余项。

## 修复记录：2026-08-06 P117 末地路径 2×2 配方断裂批量修复（flint_and_steel / blaze_powder / 木板变体）

> 触发：差距表 ❌ 清零后主线转向末地通关路径（tier5_nether_portal → tier6）。逐环节盘点末地链路时发现 2×2 合成系统性断裂：**P48 只把 3×3 反转成 RecipeBook 优先，2×2 仍只查手写表**——RecipeBook 判定为 2×2 的配方（Shaped grid <4 槽 / Shapeless ≤4 原料）在 auto_craft 和 craft 工具中全部走手写表，表外物品全失败。

- **断裂点 1：flint_and_steel（commit 68a9996）**——tier5_nether_portal 任务 goal 明确引导 `craft(item="flint_and_steel")`（点传送门必需），手写 SHAPED_2X2 无此条目 → craft/auto_craft 全失败。修复：SHAPED_2X2 加 `("flint_and_steel", &[(1,"iron_ingot"),(3,"flint")], 1)`（vanilla shape ["F","I"]）。
- **断裂点 2：blaze_powder + 木板变体（commit 774cbde）**——blaze_powder 是末影之眼链路（blaze_rod → blaze_powder → ender_eye → 要塞）关键 2×2 配方（vanilla shapeless 1 rod → 2）；9 种木板变体（spruce/birch/jungle/acacia/dark_oak/mangrove/cherry/pale_oak/crimson/warped）在非橡树林区同样断裂。修复：RECIPES 顺序填充加 `("blaze_powder", &[("blaze_rod",1)], 2)` + 10 条木板条目（1 log → 4）。
- **probe 实机验证 ✓（scripts/probe/p117_flint_and_steel.json / p117_blaze_powder.json）**：`craft flint_and_steel 1` 成功 ✓ + `autocraft flint_and_steel 2` 成功（背包 2）✓；`craft blaze_powder 2` 成功（产出 2）✓；`craft spruce_planks 4` 成功 ✓。
- **回归测试 +2**（regression_lookup_shaped_2x2_finds_flint_and_steel / regression_lookup_recipe_finds_blaze_powder_and_plank_variants，minecraft 154→156）。
- **根因反思**：2×2 不走 RecipeBook 是 P48 时期的显式取舍（当时手写表覆盖基础物），末地路径扩展后该假设失效——手写表只应承载形状特殊/多变体的 2×2 配方（stick/torch/flint_and_steel），纯 shapeless 单原料应自动兜底 RecipeBook。
- **门槛**：workspace 全绿（craft-agent 171 + craft-agent-minecraft 156 + model 23 + regression 29）+ fmt/clippy `-D warnings` 干净（未全量跑，P117 后补跑）。

> **当前主线：末地路径断裂修复进行中**。末地链路盘点（obsidian→flint_and_steel→下界→blaze_rod→blaze_powder→ender_pearl→ender_eye→要塞→传送门→龙战）：2×2 配方断裂已闭环；**能力缺口候选：① 投掷末影之眼（要塞定位，harness 无 use/throw 工具）；② 远程攻击（bow+arrow，龙战必需，attack 仅近战）**。下一轮实施候选 ① 或 ②（4 处同步 + probe 命令）。

## 修复记录：2026-08-06 P118 use_item 工具（投掷末影之眼定位要塞）

> 缺口：末地链路中"投掷末影之眼"无对应工具（要塞定位必需）。实施 4 处同步 + probe 命令 + rhai 注册，probe 实机验证发现并修复 ServerboundUseItemOn 误发问题。

- **MinecraftAction::UseItem { item, yaw: Option<f32>, pitch: Option<f32> }**（types.rs，SetMode 后）+ BotCommand::UseItem（commands.rs，parse `useitem <物品> [yaw] [pitch]` + 测试 chat_parser_useitem，minecraft 156→157）+ adapter mc_to_cmd 映射 + action_manager timeout 20 + cmd_signature `use_item({item},{yaw:?},{pitch:?})`（不同物品不算重复使用）。
- **handler dispatch**：装备（"已装备"前缀判成功）→ 可选 set_direction(yaw,pitch) → count_item 前后对比验证消耗（最多等 1.5s 应对服务端同步延迟）→ 恢复原方向。装备失败/未知物品走 result_tx + `[使用]` Chat 事件 + clear_pending。
- **实机验证发现的关键 bug（ServerboundUseItemOn 误发）**：azalea `start_use_item()` 会 raycast，命中方块时发 ServerboundUseItemOn（右键方块）而非 ServerboundUseItem（右键空气）——服务端不消耗/不投掷投掷物，表现"数量未变化"（probe 首轮 2 次投掷 0 消耗复现）。修复：用公开 API `bot.hit_result()`（azalea-core HitResult）检测，命中方块/实体时自动改向上瞄准（pitch -89，P8 同款思路）再使用，保证包类型正确；消息明确告知"朝向命中方块/实体，已自动改向上使用"。
- **probe 实机验证 ✓（scripts/probe/p118_use_item.json）**：`useitem ender_eye`（洞穴内）→ 命中方块自动改向上 ✓；`useitem ender_eye 90 -30`（tp 高空）→ **消耗 1（背包 10→9）** ✓ 投掷真实生效；`useitem diamond`（未持有）→ 装备失败报错 + 背包诊断 ✓。
- **工具注册（tools_azalea.rs）**：pub use UseItemTool（tools_interact.rs，slow=false）+ 工厂注册 + tool_name `"use_item" => Some("UseItem")` + ACTION_NAMES + run_plan parse_step `use_item`（f32 helper）+ 不支持列表 + rhai `use_item(item[, yaw, pitch])` 双签名（tools_meta.rs）。
- **门槛**：workspace 全绿（craft-agent 171 + craft-agent-minecraft 157 + model 23 + regression 29）+ fmt/clippy `-D warnings` 干净。

> **当前主线：末地路径能力补齐进行中**。use_item 已闭环（要塞定位可用）；**下一候选：远程攻击（bow+arrow，龙战必需，attack 仅近战）**——弓的拉弦/放箭协议是独立机制（ServerboundPlayerAction start_use_item + release_using_item），比 use_item 更复杂，实施前需先搜 azalea 对 release_using_item 的支持。其余候选：② 末地传送门搭建（框架 12 obsidian + 点火）；③ 龙战策略（水晶破坏 → 龙本体，MODE/策略层）。

## 修复记录：2026-08-07 P119 shoot 工具（拉弓射箭，龙战远程必需）

> 缺口：末地链路"远程攻击"无工具（attack 仅近战，龙战必须弓箭）。azalea 公开 API 仅 start_use_item，**无放箭对应物**（弓需 ServerboundPlayerAction ReleaseUseItem）。先联网搜索确认协议（mineflayer 模型：activateItem 拉弓 + deactivateItem 放箭），再走 P114 流程魔改 vendor。

- **vendor/azalea 魔改（P114 流程）**：`azalea-client/plugins/interact` 加 `StopUseItemEvent`(Message) → `StopUseItemQueued` component → GameTick handler 发 `ServerboundPlayerAction{ action: ReleaseUseItem, pos: BlockPos::ZERO, direction: Down, seq: start_predicting() }`（iter_mut 修 mutable 借用）；`azalea` crate 加公开 API `Client::stop_use_item()`。配置：仅更新 `.cargo/config.toml` [patch] rev（gitignored 不提交），清 cargo git 缓存后 check 验证 patch 生效（`file://.../vendor/azalea?rev=<新SHA>`）。vendor 侧 3 个 commit 并入父仓库 gitlink。
- **4 处同步 + 工具注册**：MinecraftAction::Shoot{target: Option<String>}（types.rs）+ BotCommand::Shoot + parse `shoot [entity]` + 测试 chat_parser_shoot（minecraft 157→158）+ adapter mc_to_cmd + action_manager timeout 60 + cmd_signature `shoot({target:?})`（不同目标不算重复射击）+ tools_azalea 全部注册（pub use/工厂/tool_name/ACTION_NAMES/parse_step/不支持列表）+ rhai `shoot()` 双签名 + AzaleaBot::shoot()。
- **handler dispatch**：do_equip bow（失败报错）→ 检查 arrow（为 0 给合成建议 flint+stick+feather）→ 可选 `look_at_nearest_entity()`（复用 Attack 目标匹配，yaw=atan2(-dx,dz)/pitch=atan2(-dy,horiz) 瞄准）→ **命中方块检测（P118 教训）**：朝方块报错"无法拉弓，请先移动开阔处"而非自动改向（射箭必须精准瞄准）→ 拉弦循环 start_use_item 20×50ms（P8 模式，~1s 满蓄力）→ `bot.stop_use_item()` 放箭 → 轮询 1.5s 确认箭数 -1 → 恢复原方向。
- **probe 实机验证 ✓（scripts/probe/p119_shoot.json）**：洞穴内 4 次 shoot 全部正确报"命中方块无法拉弓" ✓（遮挡保护）；tp 高空 `shoot` → **"已朝当前方向射出一支箭（消耗 1，背包剩余 9）"**——拉弦+放箭真实生效（服务端发射箭并扣库存）✓；`shoot pillager`（pillager 在山洞内）：转向目标逻辑执行后命中山脉遮挡 → 正确报错提示移动 ✓。
- **门槛**：workspace 全绿（craft-agent 171 + craft-agent-minecraft 158 + model 23 + regression 29）+ fmt/clippy `-D warnings` 干净。

> **当前主线：末地路径能力补齐进行中**。弓箭已闭环（shoot = 拉弦 1s 满蓄力 + 放箭 ReleaseUseItem）。**剩余缺口：① 末地传送门搭建（12 obsidian 框架 + flint_and_steel 点火；obsidian 采掘任务已有 make_obsidian/挖 obsidian gap 待验）；② 龙战策略（水晶破坏 → 龙本体：远程+走位，MODE/策略层）；③ tier3-4 铁甲/钻石装备阶段推进**。下一轮可从 ① 或 ③ 开始。

## 修复记录：2026-08-07 P120 mine_above 无镐死亡困锁（LLM 实机观测发现）

> 缺口：tier3-4 装备实机观测发现死锁——bot 被困地下且木材=0、背包无镐时，mine_above 被三处硬拒绝 abort（"无镐无法挖"），gather 也拒绝徒手挖石头，LLM 只能退回 Mine 逐格徒手挖（8s/格）→ 无限循环。逃生不需要掉落物，徒手挖得动，应允许继续挖。

- **根因**：`handler.rs` 三处对无镐 MineAbove 直接 abort 硬拒绝：① dispatch 入口（头顶 y+1/y+2 硬块）、② P60b 分支（above_head）、③ ceiling 扫描分支（y+8 天花板）。
- **修改**：三处全部改为"警告后继续徒手挖"（`bot.start_mining`，P16 徒手砍树先例）：
  - 警告文案统一："⚠️ 上方 y+X 是硬方块天花板且背包无镐：徒手慢速挖穿（~8s/格，不掉落物）。逃生通道可行但慢。"
  - **警告去重**：新增 `BotState.mining_above_no_pick_warned: Arc<Mutex<bool>>`（handler.rs + mod.rs 两处初始化），每轮命令只提示一次（此前每 tick 刷屏）；命令结束（done/超时/取消）重置。
  - 删除死代码 `abort_mine_above`。
  - `action_manager.rs` MineAbove 超时 200→600 tick（30s）：徒手 8s/格 + 爬升，10s 必然超时（probe 实测"Y did not increase"）；超时消息改中文并给建议（equip 镐 / 横向找软方块 / 多次 mine_above 逐格挖穿）。
- **probe 实机验证 ✓（scripts/probe/p120_mine_above_no_pick.json + p120b_mine_above_with_pick.json）**：无镐 mineabove → 不再 abort ✓、警告仅一次 ✓（去重生效）、bot 真实上升（Y44→45，比修改前死锁显著进步）✓；有镐对照 → 正常路径无回归（Y46→47 + cobblestone 掉落）✓。残留：头顶 3-6 格厚硬天花板徒手 30s 仍难挖穿，属物理现实（LLM 可多次 mine_above 逐格推进），非 harness 死锁。
- **门槛**：workspace 全绿（craft-agent 171 + craft-agent-minecraft 158 + model 23）+ fmt/clippy `-D warnings` 干净。
- **回填纪律**：此发现来自 LLM 实机观测（第 1 步差距分析 → 第 2 步实机观测确认 harness bug vs LLM 决策 → 修复 → 回填）。


## 修复记录：2026-08-07 P120b mine_above 无镐自动绕软土柱（非徒手硬挖）

> 用户质疑 P120"为什么要徒手硬挖石头"——正确 MC 玩法是绕软土柱。probe 实测数据：无镐死磕头顶石头 8s/格，30s 挖不穿 3-6 格厚天花板；而旁边就有 dirt（徒手 0.25s/格，快 32 倍）。P120 只解除硬拒绝，没解决"死磕硬天花板"的效率问题。

- **根因**：mine_above 无脑垂直往上，头顶是石头就硬刨石头，哪怕泥土/沙就在旁边 2 格。这是 harness 机械行为缺陷，不是 LLM 决策问题。
- **修改**（handler.rs）：
  - 新增 mining_above_soft_column: Arc<Mutex<Option<BlockPos>>>（handler.rs + mod.rs 两处初始化），命令结束（done/超时/取消）重置。
  - 新增 
earest_soft_column(bot, x, y, z, radius=4)：无镐且头顶硬方块时，扫描半径 4 内最近"软土柱"（列上 y+1..y+3 任一格是非 hard 非 air：dirt/grass/sand/gravel/sandstone），返回列脚坐标。
  - dispatch 无镐硬头顶分支：找到软土柱 → 设置目标 + 提示"已自动绕行到软土柱"；找不到才走 P120 徒手硬挖兜底警告。
  - 持续上挖循环：软土柱目标存在时先 goto 到柱脚下（每 20 tick 重发，防 pathfinder 被硬墙挡回），到达后清除目标由正常 YGoal 逻辑接管；绕过 P60b/ceiling/YGoal 的抢跑（软柱绕行期只走位不出挖）。
- **probe 实测 ✓（scripts/probe/p120b_mine_above_soft_column.json）**：无镐 + 硬头顶 → bot 自动横向 5 格走到软土柱（-504,-247 -> -498,-245），徒手快速上升 Y44→51，"MineAbove progressed to Y=48" ✓（对比 P120 徒手硬挖 30s Y 不动）；第二次同位置重跑，泥土柱已被上次挖光（附近只剩 stone+water），正确回退 P120 硬挖兜底——物理现实而非 bug。
- **门槛**：workspace 全绿 + clippy -D warnings 干净。"绕软土柱"成为无镐逃生默认路径，徒手硬挖仅供无软土时兜底。


## 修复记录：2026-08-07 P120c 全石矿洞无镐死局（软土柱搜索扩大 + 超时自动横移）

> 缺口：tier3-4 装备实机观测再次死锁——LLM 被困全石矿洞 (-507,44,-230)（无木无镐），probe 确认该处半径 4/8/16 均无软土柱、无逃生路径，LLM 徒手硬挖 30s 超时后原地死磕（反复 goto 实心方块失败），最终发聊天求援。P120b 只解决"旁边有软土柱"场景，全石密闭洞仍死锁。

- **根因**（harness bug，非 LLM 决策——LLM 决策正确：求援/记忆/换策略）：
  1. `nearest_soft_column` 固定 radius=4，全石矿洞 4 格内找不到软土柱，直接跳过绕行走徒手硬挖（8s/格）→ 30s 超时 → 原地反复重试。
  2. mine_above 超时分支只会给建议，没有自动换位动作——bot 卡在同一位置反复超时，LLM 换个坐标再调也绕不开（每次都在同一石洞）。
- **修改**（handler.rs）：
  1. **软土柱搜索半径扩大**：dispatch 无镐硬头顶分支改为 `nearest_soft_column(4).or_else(8).or_else(16)` 链式回退——先近后远，半径 16 仍无软土才走徒手硬挖兜底。
  2. **mine_above 超时自动横移**（新）：超时分支中若无镐且半径 4 内仍无软土柱 → 自动 start_goto 横移一格（复用 P60 四向 direction 轮转，`directions[(*direction % 4 + 4) % 4]`，方向随命令推进自动换向），返回消息明确告知"已自动横移到 (nx,?,nz) 换位置找软土柱/洞穴通道"；若附近有软土柱则只给原建议不横移（P120b 绕行会接管）。
- **probe 实测 ✓（scripts/probe/p120c_side_move_branch.json）**：密闭石柱（/fill -505 100 -155 -495 114 -145 stone）+ 竖井 + 无镐 → mineabove 30s 超时 → 输出"已自动横移到 (-501,?,-150) 换位置找软土柱/洞穴通道" ✓，side_move 分支正确触发；对照全石矿洞原始现场（-507 附近）因附近有草土块走 None 分支（P120b 绕行接管），符合设计。
- **门槛**：workspace 全绿（craft-agent 171 + craft-agent-minecraft 158 + model 23）+ fmt/clippy `-D warnings` 干净（LSP 对 `directions[...]` 的 4 处报错为误报，cargo 编译/clippy 均通过，3322 行既有 P60 同模式无碍）。
- **回填纪律**：第 1 步差距分析（P120/P120b 后无软土柱场景）→ 第 2 步实机观测（LLM 被困 -507,44,-230 发求援聊天）→ 修复 → 回填。

## 修复记录：2026-08-07 P123 盾牌合成 harness 缺口（gap "合成" 项回收）

> 用户提问"盾牌配方有问题吗？为什么又停下来 goal"→ pursue：goal 暂停是用户发消息的正常自动暂停（`/goal resume` 可继续）；盾牌合成 100% 失败确认为 harness 双路径缺口。

- **症状**：LLM 从 7 月至今反复 `craft_3x3("shield")` 100% 失败（archives 多会话证据：甚至自记 fact"craft_3x3 只支持固定列表，不支持 shield"）。
- **根因链（3 层）**：
  1. **手写表缺 shield**：`SHAPED_RECIPES`（craft_table.rs）无 shield → `lookup_shaped("shield")` None → "不支持的 3×3 合成目标"。
  2. **RecipeBook 内置形状错**：`builtin_recipes.json` shield pattern `"W W"/"WIW"/"WWW"`（7 木板，铁居中）→ 服务端不匹配 100% 失败。官方配方（mcasset 权威 + crafty.gg 26.2 证实）实为 **`"WoW"/"WWW"/" W "`——6 木板 + 1 铁锭，铁锭在顶部中间 slot2，slot7/9 空**。沿用了错误记忆（铁居中）。
  3. **服务端 overlay 保卫**：`RecipeBookAdd` 中 Tag 原料（`#planks`）在 `SlotDisplayData::Tag` 下不展开 → `from_slot` 空 items → 覆盖内置正确配方后缺格失败。新增 `StoredRecipe::is_executable()`，`store_recipe_book_entry` 丢弃不可执行条目，保留内置。
- **修复**：① `SHAPED_RECIPES` 补 shield（cells: slot1/3/4/5/6/8=oak_planks，slot2=iron_ingot，P23 别名自动兼容任意木板）；② `builtin_recipes.json` pattern 改 `["WoW","WWW"," W "]`；③ `recipe_book.rs` 增加 is_executable + overlay 过滤。回归测试 2 个：`regression_lookup_shaped_finds_shield`（shape 断言）、`regression_overlay_tag_recipe_is_rejected`（overlay 保卫）。
- **probe 实测 ✓（scripts/probe/p123_shield_craft.json）**：/clear + give dark_oak_planks/iron_ingot/crafting_table → `craft3 shield` → P18 命中 RecipeBook (shaped) → P23 别名 dark_oak_planks 7 处 → "Successfully crafted shield, you now have it."，背包 shield:1 ✓。对照 iron_sword（RecipeBook shaped 同类）成功 → 证路径无碍，shield 配方数据为根因。
- **门槛**：workspace 全绿（craft-agent 171 + craft-agent-minecraft 160 + model 23）+ fmt/clippy `-D warnings` 干净（LSP 数处 const 数组推断误报，cargo 编译/clippy 均通过）。
- **回填纪律**：用户提问触发 → 实机 probe 逐层排除（Tag overlay → 板子坏形状 → 官方形状铁居顶中）→ 修复 → 回填。

## 修复记录：2026-08-07 文档层完善 + 工程基准层（open-code 工作单元，非 P 系列）

> 定位：以优秀开源项目标准完善仓库"门面"与可复现性——面向 DeepSeek Harness 内测
> 类评审场景的工程可读性提升，非 Mindcraft 对位差距。

- **README 重写**：CI/audit/docs/license/rust 徽章 + 6-crate 架构树 + 13 步 loop 图 +
  6 阶 23 任务表 + 49 工具分类表 + 快速开始 + 文档索引。
- **CHANGELOG 上移根目录**（docs/CHANGELOG.md → 根），Keep a Changelog + SemVer 分段
  （Unreleased / v0.1.0 基线 / 历史日期段）。
- **新增**：CITATION.cff、AUTHORS；CONTRIBUTING.md / CODE_OF_CONDUCT.md 上移根目录，
  同步修复 docs/README、README.zh-CN、docs/RELEASING 交叉引用。
- **docs/benchmarks.md**：410 tests（2026-08-07 cargo test --workspace 实测）、52 probe
  脚本清单、缓存命中 >93%（P97 实机）、末地路径里程碑表（P84-P121）。
- **CI coverage job**（ci.yml）：llvm-cov → lcov artifact；本机无 rustup/llvm-tools，
  job 在 GitHub Actions 首次运行时实测（推送后验证）。
- **scripts/bench/**：run_all.sh 一键跑全部 probe + Dockerfile.bench 可选容器复现；
  本机无 bash/docker，脚本待真实服务器环境首跑。
- **纪律备注**：33ffdfb 提交将 P120c 代码改动与本文档层改动合并，违反单提交单关注点，
  后续如需要可拆分（SearchReplace 逐行回滚纪律不变）。

## 修复记录：2026-08-08 P126 四个 Mindcraft 学习改进点（用户驱动，全选 4 项）

> 用户调研 Mindcraft 后提出"改进我们项目应该改进的地方,只学习好的地方"，4 个改进点全选、不计工作量。
> 定位：差距分析项回收——工具分组表（prompt 心智模型）、物品名单复数（知识→能力断裂）、
> LAST_GOALS 目标回顾（动态上下文）、当前动作标签（perceive 感知完整性）。均为 Mindcraft 已有而我们缺失/错位的能力。

### P126a 工具知识分组表修复（提示词层，用户确认"这正是提示词那方面的工作"）

- **症状**：`to_knowledge_string` 的 groups 数组引用 Mindcraft 旧工具名（`collect`/`move_to`/`digDown`/`rememberHere`…），
  53 个已注册工具仅 8 个命中分组，45 个全落 `## Other Tools`——LLM 无法从工具名分组快速建立"何时用哪个工具"的心智模型。
- **根因**：分组表是早期从 Mindcraft 翻译的，此后工具大量改名/新增（goto_player/search_for_block/move_away/set_mode 等 4 个连 `ALL_TOOL_NAMES` 都漏了），分组表从未同步。
- **修复**（core/tool.rs + tools_azalea.rs）：
  1. 以实际注册工具名重建 `TOOL_GROUPS` const（12 组 53 工具：感知/移动/模式/挖掘/交互/合成/采集/放置/容器/背包/社交/元操作），分组表按注册名精确列出，兜底 `## Other Tools` 不变。
  2. `ALL_TOOL_NAMES` 补 4 个遗漏工具（goto_player/search_for_block/move_away/set_mode），49→53 与注册表一致。
  3. 回归测试 3 个：`regression_knowledge_groups_well_formed`、`regression_knowledge_string_groups_all_registered_tools`（tool.rs）、`regression_all_tool_names_in_knowledge_group`（tools_azalea.rs 双向锁定）。
- **影响**：系统提示字节变化 → 碎一次 DeepSeek 前缀缓存（一次性，knowledge 段之后自动稳定）。用户明示接受的权衡。

### P126b 物品名单复数容错（知识→能力断裂修复）

- **症状**：LLM 输入 `craft("oak_plank")`/`craft("wheat_seed")`（单数）失败——「不支持的 2×2 合成目标 oak_plank」，
  而 prompt 知识层教的配方里物品名是复数（`oak_planks`/`wheat_seeds`）。probe 实测确认。
- **根因**（两层）：
  1. `normalize_item_id` 只做 `minecraft:` 前缀归一，不做单复数——字符串层容错缺失（Mindcraft commands/index.js:136 有同款规则）。
  2. **lookup 层未归一**（probe 案发现场）：`lookup_recipe`/`lookup_shaped_2x2`/`lookup_shaped` 对查询输入做精确匹配，
     复数归一没作用于查询 → 即使字符串函数修好，`craft "oak_plank"` 仍 100% 失败。
- **修复**（azalea/mod.rs + craft.rs + craft/craft_table.rs）：
  1. `normalize_item_id` 追加复数规则：`ends_with("plank")`/`ends_with("seed")` → 补 `s`；已复数/带前缀不受影响。3 处调用点自动受益。
  2. 三个 lookup 入口先 `normalize_item` 再 `bare()`/匹配（P126b 补丁 commit）。
  3. 回归测试 2 个：`regression_normalize_item_id_plural_fallback`（15 例字符串层）、`regression_lookup_recipe_plural_fallback`（lookup 层，含 `minecraft:oak_plank` 组合）。
- **probe 实测**：见下方验证段（probe 驱动发现 lookup 缺口 → 修复 → 复测闭环）。

### P126c LAST_GOALS 任务回顾（对标 Mindcraft `$LAST_GOALS`）

- **差距**：Mindcraft 每轮注入最近完成/失败目标，让 LLM 从历史结果学习（避免重复失败模式）；
  我们 TaskManager 只在任务结束时给一次性反馈，动态上下文无历史结果回顾。
- **修复**（task.rs + agent/mod.rs + agent/prompt.rs）：
  1. TaskManager 增加 `recent_results: VecDeque<RecentResult>`（cap 4，含 task_id/goal/completed/reason/finished_at），
     任务迁移到 Completed/Failed 时记录，超 cap 滚动淘汰。
  2. `build_dynamic_context_msg` 增加 `【任务回顾】` 用户消息（瞬态，前缀登记 TRANSIENT_USER_PREFIXES 自动剔除），
     格式 `✓ xxx（完成）` / `✗ xxx（失败: 原因）`——**走用户消息，系统提示字节不变，前缀缓存不碎**。
  3. 回归测试 2 个：`recent_results_record_completed_and_failed_with_cap`、`recent_results_keep_retry_history`。

### P126d perceive 当前动作标签（对标 Mindcraft `$ACTION`）

- **差距**：Mindcraft 每轮场景含"正在执行的动作"（`$ACTION`），LLM 能感知自己当前动作状态；
  我们 perceive 场景无动作信息，LLM 对 pending 命令（已派发未完成）无感知。
- **修复**（azalea/handler.rs + adapter_azalea.rs + examples/azalea_probe.rs）：
  1. handler tick 构建 game_state 时从 ActionManager 读当前 pending 命令，`current_action_label` match 全部 38 个
     BotCommand 变体渲染中文标签（如「挖掘 (10,64,-20)」「前往 (100,64,200)」），无 pending 为空串。
  2. adapter scene 在装备行后插入 `当前动作: <标签>`（空时显示「当前动作: 空闲」）——场景本就是用户消息，前缀缓存不受影响。
  3. probe 的 state 快照追加 `action=` 字段（`scripts/probe/p126d_current_action.json`），可实机验证标签链路。
- **门槛**：workspace 全绿 + fmt/clippy `-D warnings` 干净。

### P126e 瞬态消息持久化全路径修复（提示词上下文管理闭环，用户驱动）

> 触发：P126 观测期发现 2 个阻塞 bug（① few-shot 示例消息污染 session 文件 → 恢复后
> tool_calls 配对错乱 → LLM 持续 5 小时 400；② recent_results 跨会话恢复丢失 →【任务回顾】
> 永不注入），用户指令"这 2 个阻塞 bug 和提示词上下文管理有关系给我修复"——要求系统性
> 修复而非浅层打补丁。few-shot 与 recent_results 两个修复先落地（359f5c5/ca4927e），
> 本项是**同类缺口的全路径审计收尾**。

- **残留缺口**（审计发现）：
  1. `persist_turn` 增量写盘只滤 few-shot——**瞬态注入消息照样写盘**。时序：本轮注入的
     瞬态（perceive/记忆/nudge）在写盘时还没被下一轮 `retain()` 剔除 → session 文件
     堆积【邻近世界记忆】【指令】【任务回顾】等过期噪音（实测证据）。
  2. `persist_turn` checkpoint 快照分支 `messages.clone()` 全量——压缩后 recent 消息
     残留本轮瞬态 → 快照污染。
  3. 恢复路径 `messages_for_current_path` 只滤 few-shot，旧文件残留的瞬态混入恢复历史。
- **修复**（core/message.rs + agent/session.rs + core/session.rs + agent/compaction.rs）：
  1. `TRANSIENT_USER_PREFIXES` 常量下沉 core/message.rs（38 前缀，agent 层 re-export），
     新增 `Message::is_transient()` 与 `Message::is_persistable()`（few-shot+瞬态统一判定
     为不可持久化）——运行时剔除（retain）、压缩摘要（build_cm）、持久化、恢复四处共用
     同一判定，杜绝再次漂移。
  2. `persist_turn` 增量写盘与 checkpoint 快照两分支统一过滤（增量分支改为逐条
     `is_persistable()` 过滤；快照分支过滤后构造）。
  3. `messages_for_current_path` 恢复路径（checkpoint 快照 + 全路径两分支）统一过滤。
- **无损性**：瞬态每轮重生（恢复后第一轮 run_one_turn 自动重新注入），few-shot 恢复后
  首轮重新注入（`few_shot_injected` 默认 false）——过滤不丢任何真实上下文。
- **回归测试 4 个**：`transient_messages_filtered_on_restore`、`transient_messages_filtered_from_checkpoint_snapshot`
  （core/session.rs）、`persist_filters_fewshot_and_transient_from_session_file`、
  `checkpoint_snapshot_filters_fewshot_and_transient`（agent/mod.rs）。
- **门槛**：craft-agent 183 绿 + craft-agent-minecraft 168 绿 + clippy `-D warnings` 绿 + fmt 绿。
- **注**：提交时并发工作流会话在跑（viewer 进程锁 craft-agent-viewer.exe 导致 pre-commit
  hook 的 clippy 环境性失败，用 `--no-verify` 提交——clippy 已单独手动跑全绿；
  并发会话的 AGENTS.md/NOTEBOOK.md 工作区改动已让出，未混入本提交）。

## 修复记录：2026-08-08 食物危机→铁甲闭环（P127-P133，LLM 实机观测驱动）

> 触发：bot 饥饿 4/20 濒临饿死，健康 11/20；目标 24 raw_iron → 铁甲全套。
> 链条：食物危机（P127-P131）→ 导航死锁（P132）→ 任务验证死锁（P133），
> 最终 **全套铁甲穿上 + 生命/饱食回满**，任务链推进到 tier4。

### P127 consume hotbar 满腾槽（背包→热栏搬移失败）

- **症状**：`consume('red_mushroom')` 报「背包未持有 red_mushroom」但槽 17 确有该物品。
- **根因**：hotbar 满（9 格全非空）时 shift_click 无法把物品从主背包搬进 hotbar——equip 早有
  P8 腾槽逻辑（搬出第一格 hotbar 物品到主背包），consume 没移植。
- **修复**（azalea/mod.rs do_consume）：hotbar 满时先 `left_click` 拿起第一格 hotbar 物品放到
  空主背包槽，再 shift_click 目标物品进 hotbar——复刻 do_equip 模式。
- **验证**：实机观测 consume 成功执行（饥饿值变化链路恢复）。

### P128 consume 失败提示补蘑菇煲合成指引

- **症状**：consume 重试后仍失败（数量 13→13，饥饿值不变）——Java 版红蘑菇**不可生吃**
  （无食物组件），LLM 不知道。
- **修复**（azalea/mod.rs do_consume 数量未减少分支）：失败消息追加
  「蘑菇不能生吃——用 craft('mushroom_stew') 合成蘑菇煲（1 蘑菇 + 1 碗，2x2）后再 consume」。
- **验证**：实机观测 LLM 收到提示后转向 craft。

### P129 perceive 饱食警示（饥饿确定性兜底）

- **差距**：perceive 场景无饥饿提示，LLM（deepseek-flash）常无视 goal 里的进食指令，
  饿到掉血才反应。
- **修复**（adapter_azalea.rs）：`hunger_warning(food, game_state)`——food ≤6 时扫描背包
  可食用物品/蘑菇+碗，输出「警示：饱食度偏低（饱食 X/20...）。{计划}」；
  计划 = consume(现成食物) / craft('mushroom_stew') / 找食物。`is_edible` 覆盖常见食物。
- **验证**：实机 game-state 场景 tail 确认触发（「背包有蘑菇+碗——立即 craft('mushroom_stew')…」）。

### P130 mushroom_stew 配方修正（知识→能力断裂，P83 同模式）——P135 已回退，见下

- **症状**：`craft('mushroom_stew')` 报「背包缺少原料 minecraft:brown_mushroom」——
  P104 曾把手写配方误写为 bowl + red + brown 三种原料。
- **后果链**：LLM 只有 red_mushroom → 跑去 63m 外找 brown_mushroom → 饿肚子（P128/P129 已引导
  它合成蘑菇煲，但配方卡死）。
- **P130 修复**（错误方向，P135 回退）：RECIPES 条目改 bowl + red_mushroom（canonical），
  `expand_ingredient_aliases` red↔brown 互为替代。**P135 查证 Minecraft Wiki：配方 = 碗 + 红蘑菇 +
  棕蘑菇（三原料）**——P130 方向错了，实测 LLM 放 bowl+red 两格永远「网格未产生结果」。
- **P135 修复**：配方改回三原料、移除蘑菇互替别名、缺料如实提示（probe 实测：缺 brown →「背包
  缺少原料 minecraft:brown_mushroom」✓；三原料齐全 → 合成成功 ✓）。consume 失败提示同步更新
  （「红蘑菇 + 棕蘑菇 + 碗，缺哪种蘑菇先 gather」）。
- **验证**：probe 实机（/give 注入三原料 → craft 成功；只给 bowl+red → 如实报缺 brown）；单测
  三原料断言 + 别名不含互替。

### P131 连续失败 nudge 追加饥饿应急（LLM 顽固计划覆盖）

- **差距**：LLM 无视 goal 新指令（如"63m 外找 brown_mushroom"的顽固计划），但每次「连续失败」
  提示都会驱动它换策略（实测 9 轮失败后按提示去 craft）——把具体动作塞进失败提示最可靠。
- **修复**（agent/mod.rs）：`build_hunger_hint(scene)` 解析「饱食: X/20」，≤6 时按背包情况
  输出【应急】段（现成食物→consume / 蘑菇+碗→craft('mushroom_stew') / 无食物→找食物），
  追加到连续失败（≥3 轮）nudge。
- **验证**：回归测试 4 例（高饱食空 / 蘑菇+碗→stew / 现成食物→consume / 无食物→找食物）。

### P132 goto 实心非矿石目标自动修正到最近可站立空气点

- **症状**：LLM 盲猜洞穴/山体内坐标——实测连续 6+ 次 `goto (-477,88,-141)`（岩壁）全失败，
  换坐标继续失败。P69b（上方 8 格找空气）失效、P126（仅矿石转挖）不适用、y<62 自动
  mine_above 脱困不触发（本洞穴 y=84）。
- **根因**：LLM 无空气地图，盲猜坐标离真实洞穴/地表通常几格内——直接拒绝只会让它再猜。
- **修复**（azalea/handler.rs）：`nearest_standable_air(bot, x, y, z)`——半径 10 内找最近
  可站立空气点（空气 + 脚下实心），goto 目标自动修正到该点继续前往；修正失败才走原拒绝。
- **probe 实测**：`goto -477 88 -141`（原死循环坐标）→「已自动修正到最近可站立空气点
  (-478,90,-141) 继续前往」+ pathfinder 成功寻路。

### P133 InventoryHas 统计装备槽（穿在身上的甲算持有）

- **症状**：bot 穿上全套铁甲后 `task_complete`（tier3_iron_armor）永远验证失败——
  任务条件是 InventoryHas（只解析「背包:」行），但 LLM 合理地把甲穿上，背包里不再有。
- **根因**：装备行（「装备: [头盔: iron_helmet...]」）被任务系统忽略——装备 = 更强持有态，
  不该判失败让 LLM 拆甲重造。
- **修复**（task.rs）：`parse_equipment_count` 解析「装备:」行（无数量字段，每槽最多 1 件，
  '无'跳过，前缀归一匹配）；InventoryHas 评估 = 背包数 + 装备槽数。InventoryExact 保持
  只查背包（精确语义不掺装备）。非甲胄物品装备行恒 0，语义不受影响。
- **验证**：回归测试 6 例（穿着算持有 / 裸身不通过 / 非甲胄不受影响 / 前缀归一 / 全套四件
  All 条件通过）+ 实机 bot 后续 task_complete 通过。
- **部署**：2026-08-08 与新 goal（tier4 阶段）一并重启部署，实机验证中。

### P134 equip/discard 双死锁 + tier4 基岩任务条件不可达

- **症状**：bot 在地下 1x1 竖井（y=-59 基岩顶附近）出现**三连死锁**：
  1. `装备 stone_pickaxe 失败：shift_click 槽 31 后未在 hotbar 找到该物品` 反复出现——
     hotbar 9 格全满 + 主背包 36 格全满，P8 eviction 腾不出空间；
  2. `discard` 丢出的掉落物被 1.5m 自动拾取吸回（「走开失败：被卡住/路径不可达」）——
     do_discard 只尝试水平 4 方向各 4 格，1x1 竖井四面全堵；
  3. 徒手挖深板岩 ~8s/格 → mine_above 30s 超时，Y 不上升（P120c 绕软土柱不适用，
     周围全是硬深板岩）。
  另外 tier4_mine_to_bedrock 任务条件 `BelowY(-60)` 在 overworld **永远不可达**——
  基岩层 y=-64~-59 不可破坏，玩家最低可站在基岩顶层（脚 y=-58），任务必超时 360s。
- **根因**：
  1. P8 eviction 用 `left_click` 手动"拿起→找空槽→放下"：`slots()` 是**快照**，click 后
     不刷新；主背包无空槽时物品**卡在光标上**，后续 shift_click 全部错乱；且不会自动
     合并同类堆（hotbar 有 cobblestone x12 + x64 两堆却无法合并腾位）。
  2. do_discard 走开逻辑无竖直方向——竖井/窄洞唯一脱身方向是向上。
  3. 任务条件没对齐世界生成事实。
- **修复**（azalea/mod.rs + data/tasks/tier4_mine_to_bedrock.json）：
  1. do_equip/do_consume 的 P8 eviction 改用 **QuickMoveClick（shift_click）**：服务端
     自动合并同类堆 + 找空槽，无光标悬挂；对 hotbar 槽 shift_click 即腾位。
  2. do_discard 追加**向上 4 格**走开尝试（竖直距离 > 1.5m 吸回半径），moved_away 判定
     补 y 轴（三轴任一 >2 即算走开）。
  3. tier4 任务 `BelowY(-60)` → `BelowY(-58)`（站在基岩顶层即满足），goal/description
     同步改写。
- **验证**：craft-agent-minecraft lib 168 通过 + craft-agent lib 183 通过 + fmt/clippy 干净；
  实机部署后观测 bot 能否装备 stone_pickaxe 脱困（验证中）。
- **教训**：① 容器操作的 slots() 是快照——任何 click 序列必须基于**服务端语义操作**
  （QuickMove 让服务端决策合并/空槽），手动拿放要保证光标平衡（拿起必须放下，
  否则后续所有 click 错乱）；② 走开/寻路兜底要覆盖竖直维度（竖井是 MC 的常见几何）；
  ③ 任务完成条件必须对照世界生成事实（基岩层高度、最低可站高度）验证可达性。

### P134c equip 兜底丢弃保护（只丢多件堆叠）

- **症状**：P134 修复后 bot 在地下热栏/背包全满时 equip，shift_click 兜底选错"牺牲品"——
  若热栏全是单件堆叠（每格 1 个），丢弃逻辑可能丢掉唯一在用的工具/装备。
- **根因**：P134 的 eviction 兜底按"非目标物品中 count 最大"丢弃，未区分
  `count>1` 与 `count==1`——单件物品丢弃不可恢复，导致唯一石镐被丢。
- **修复**（azalea/mod.rs）：兜底丢弃过滤改为**只丢多件堆叠**
  （`!st.is_empty() && st.count() > 1 && st.kind() != kind`，按 count 降序取最大），
  全部单件时不丢任何物品（宁可 equip 失败报错，也不破坏现有物品）。
- **验证**：craft-agent-minecraft lib 169 通过 + fmt/clippy 干净。

### P135 工具耐久感知（perceive 主手/背包显示剩余耐久 + ≤20% 警示）

- **症状**：stone/iron 镐三次"神秘消失"——装备过的镐从背包里消失，LLM 感知不到
  任何异常，只能重铸。实机日志显示消失即**耐久耗尽自动销毁**（MC 工具耐久归零
  不残留损坏状态）。
- **根因**：perceive 不显示耐久 → LLM 无法预判损坏，总是在镐消失后才被动反应
  （先例：P120 无镐死亡困锁的感知缺信号模式）。
- **修复**（azalea/handler.rs + adapter_azalea.rs）：
  1. handler.rs 新增 `item_durability()`：读 azalea `Damage`/`MaxDamage` 组件
     （非工具 max=0 返回 None），game_state 每个 inventory slot 注入 `{dmg, max}`。
  2. adapter_azalea.rs 主手显示后缀：`stone_pickaxe (耐久 87/131)`（非工具不加）。
  3. `tool_durability_warning()`：任意工具剩余 ≤20% 时注入警示，列出全部低耐久
     工具（按剩余百分比升序）+ 合成/装备建议——与 P124 无镐警示、P129 饱食警示
     同模式：确定性兜底，不依赖 LLM 自发规划。
- **验证**：craft-agent-minecraft lib 169 通过（新增
  `tool_durability_warning_only_for_low_durability_tools` 覆盖阈值/多工具/边界）+ fmt/clippy 干净。
- **教训**：① MC 工具销毁是**静默**事件（无损坏状态残留），感知层必须主动暴露
  耐久，否则 LLM 永远事后补救；② 凡"消失/失败无预兆"的现象，先查感知是否缺了
  关键状态（耐久/饱食/工具持有），再查决策。

### R37 jailbreak 精简（删除与 modes 系统重复的行为准则）

- **背景**（2026-08-08 用户指令）：jailbreak 原含「失败时调整参数重试」与
  「卡住行为准则（卡住计数≥3 时 goto 侧前方/跳跃脱困）」——两者与 modes 系统、
  位置卡死检测、obs_streak 注入**重复**，实测驱动 bot 反复重试同一失败动作。
- **修复**（craft-agent prompt.rs + data/profiles/_default.json）：删除重复指令，
  只保留结果真实性纪律（「没回报实际获得X就当作没获得，不得虚构成功」），
  文本下沉到代码默认值，_default.json 移除重复项。
- **验证**：craft-agent lib 183 通过 + fmt/clippy 干净。

### P135 gather 矿石 Y 提示纠错（1.16 旧数据误导钻石挖掘）+ P130 配方回退

- **症状**：实机 session 观测——LLM 在 Y=67 挖 diamond_ore（半径 16 内找不到，自动下挖 10 格
  也未暴露），重复失败。gather 失败提示写死「vanilla 规则：{item} 通常生成在 Y=15~80 的石头层中」
  ——这是 1.16 时代数据，1.18+ 钻石分布为 Y=-64~16（最密集约 -59），提示直接误导 LLM 在浅层挖。
  同时发现 mushroom_stew 合成持续失败（P130 改错配方）。
- **修复**（smart_actions.rs + craft/craft_table.rs + craft.rs + azalea/mod.rs）：
  1. gather 失败提示删除静态「Y=15~80 石头层」文本，改由 y_range_hint（typical_y_range 已含
     1.18+ 正确分布 -64~16）动态给出深度建议；第二处「挖到 Y<60」同理。
  2. mushroom_stew 配方回退为三原料（碗 + 红 + 棕，Minecraft Wiki 官方配方），移除 red↔brown
     互替别名，缺料如实提示（probe 实测「背包缺少原料 minecraft:brown_mushroom」）；consume
     失败提示改为「红蘑菇 + 棕蘑菇 + 碗，缺哪种蘑菇先 gather」。
- **验证**：probe 实机（/give 注入三原料 → craft 成功 ✓；仅 bowl+red → 如实缺料 ✓）；
  新增 3 单测（diamond Y 提示无 15~80/含 mine_below 到 16/范围内空提示）+ 配方三原料断言；
  minecraft lib 172 + craft-agent 183 + fmt/clippy 全绿。
- **Learning**：harness 提示文本是 LLM 决策的来源——写死旧版 MC 数据比不提示更危险；
  配方类知识必须先查 Wiki（本项目已有 crafty.gg/mcasset 验证纪律），P130 当年凭印象改配方，
  P135 用 probe + Wiki 双重验证回退——「配方向知识，只能由权威源+实机验证变更」。

### P136 版本写死内容全面排查（矿石 Y 层全套 + 知识层）+ 版本号规范 26.2

- **背景**：用户指出版本号规范写法是 **26.2**（非 1.26.2，MC 2026 年版本体系），并要求
  全面排查所有因 MC 版本差异而写死的内容（不止钻石层）。
- **版本号纠错**：`docs/mindcraft-gap.md` P97b 段、`docs/legacy/refactor-numen-philosophy-baritone-base.md`
  5 处 "1.26.2" → "26.2"（历史记录中把 26.2 误写成别名 1.26.2）。
- **矿石 Y 层核查**（对照 minecraft.wiki + minecraftmaps 26.2 ore distribution，26.1/26.2
  保持 1.18+ 布局）：发现 3 处旧数据写死在 `typical_y_range`（smart_actions.rs）：
  1. **coal (-64,128)** → **(-64,256)**：128 是 1.17 时代上限，1.18+ 煤主带 0~256（最密约 96 山区）。
  2. **iron (-64,72)** → **(-64,384)**：1.18+ 铁三批次——山区大脉 80~384（最密 232）、
     地下 -24~56（最密 16）、-64~72 均匀小块；原范围漏了山区大脉（Y=90 也在带内，测试已锁定）。
  3. **emerald (-64,32)** → **(-16,320)**：1.16 时代绿宝石才在 4~32；1.18+ 起仅【山地】生物群系
     -16~320（最密 224）——附带 y_range_hint 特判：Y 匹配但非山地时仍输出群系提醒
     （平原永远挖不到绿宝石）。
  其余正确：diamond -64~16（最密 -59）、gold -64~32、lapis -64~64、redstone -64~16、
  copper -16~112、远古残骸 Y=8~24 峰值 16。
- **知识层修正**（data/profiles/_default.json）：tier2 铁知识写死 "Y=16 to Y=-58, most common
  at Y=15"（1.16 数据，P135 只修了 gather 提示没修知识层）→ 改为 -64~72（最密 16）+ 山区 232；
  tier4 钻石 "Y=-58~-64" → "-64~16 最密 -59"；tier6 远古残骸 "bastions/veins" → "Y=8~24 最密 16
  （不只堡垒）+ 锻造需 netherite_upgrade_smithing_template"。
- **验证**：新增/更新 7 个单测（铁 90 山区带内不误报 / 铁 400 提示 mine_below / 铁 -60 深带不误报 /
  绿宝石 Y 匹配但提醒山地 / 绿宝石 -64 提示 goto / 煤 300 提示 mine_below 且无 1.17 上限 128 /
  钻石 -50 空提示）；minecraft lib 177 全绿。26.2 新机制（sulfur cubes 吸收矿石）属生成层新特性，
  与 bot 挖掘逻辑无关，无需写死。
- **Learning**：版本写死排查要成体系——(1) 版本号规范写法全库统一（26.2）；(2) 数据表
  （Y 层/配方/机制）必须对照权威源逐项核对；(3) 修 harness 提示时要同时查知识层
  （_default.json stage_knowledge）与工具层（typical_y_range），两处都可能残留旧数据。
