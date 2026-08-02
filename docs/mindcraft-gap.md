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
| 记忆（rememberPlace/记忆库） | WorldMemory 7 类 | ✅ | 锚点 goto 缺失 |
| full_state（世界全量查询） | perceive 分块 | 🟡 | |
| lockdown（限制物品/禁命令） | blocked_actions? | 🟡 | |

## 优先级队列（按主线收益排序）

1. ✅ tillAndSow 种植——食物农场（P84 完成，2026-08-02 probe 全路径实测通过）【原实机问题：bot 捡到 wheat_seeds 因无法种植而 discard】
2. ❌ goToSurface 强化——P83 信号已给（overhead_solid→mine_above），待实机确认 LLM 脱困成功率
3. ✅ goToBed 睡觉——跳夜（P85 完成，2026-08-02 probe 实测通过）
4. ✅ 收割（harvest 工具）——farmland 成熟后挖取+拾取（P86 完成，2026-08-02 probe 实测通过）
5. ✅ pvp 走位（strafe）+ 近战修复全套——P87+P88 完成，2026-08-02 实机验证（逼近/1s 反击/低血反击/攻击只发生在可命中距离）
6. 🟡 自动穿甲（P79）待实机验证损坏甲/新甲替换
7. 🟡 item_collecting（P80）待实机验证挖矿掉落物自动拾取
8. ✅ turn 内失败重规划（P89）——WRITE 工具失败→中止剩余批次→同轮重调 LLM，2026-08-02 单测通过

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

> 全部 7 项单测通过：craft-agent 148 通过、craft-agent-minecraft（azalea-bot）141 通过（2026-08-02）。
