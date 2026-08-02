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
- **P97b 实机验证修复**（commit c399bfa）：真实 MC 服务器（localhost:4444，协议 1.26.2）+ 真实 LLM 跑 40 步，发现单测覆盖不到的 4 处：(1) **scope 自由填值**——LLM 按描述"通用知识留空"实际填 `scope="global"` 字符串，被 `relevant()` 精确匹配过滤 → 记忆**永不注入**（【长期记忆】在 session 中完全缺失，实机可观测）。修复：`scope_is_global()` 将 None/空串/global/any/* 统一归一为全局，单测 `scope_global_string_is_treated_as_universal` 锁定；(2) **系统提示版本过时**：_default.json 写 "vanilla 1.21.2"（用户指出），实际服务器/azalea 为 MC 1.26.2 → 修正（错误版本号会让 LLM 规划时按旧配方/机制推理）；(3) **remember 引导缺失**：首轮 LLM 全程 0 次 remember（工具存在但不知道何时用）→ _default.json 新增 LONG-TERM SEMANTIC MEMORY 段（何时 save/kind 四类/scope 语义/与 memory 工具分工/list 防重）；(4) **ctl viewer 子命令** + 修 kill_all 自杀 bug（deploy 时 ctl 会 taskkill 自己）。**实机闭环验证**：remember 写入 3 条高质量记忆（食物策略/临时基地布局/木头获取含坐标范围）→ JSONL 落盘 → 修复后【长期记忆】注入渲染 ✓ 缓存命中稳定（hit=63k）✓ 无 400/崩溃。data/memory/agent.jsonl 入库作为初始记忆库（LLM 实机产物的知识沉淀）。
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
