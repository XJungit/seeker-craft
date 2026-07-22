# Mindcraft 对齐盘点

状态：**核心功能已对齐或超越**，剩余 6 个低优先级缺失项。

## 工具数量

- Mindcraft: 41 actions + 14 queries = 55
- Craft-Agent: 62 工具注册（smoke: 44 PASS / 10 FAIL / 9 SKIP）

## 缺失项

| 缺失功能 | 优先级 | 说明 |
|---|---|---|
| `!newAction` | 低 | 运行时代码生成，当前用 Rust 编译期注册 |
| `!stfu` | 低 | 聊天控制，当前无需聊天协议 |
| `!restart` | 低 | 重启 agent，当前用 viewer 重启动 |
| `!startConversation`/`!endConversation` | 低 | bot 间对话，单人世界不需要 |
| `!checkBlueprint` | 中 | 蓝图检查，当前用 VLM 看图替代 |

## SKIP 工具分析（9 个）

单人世界无法造或非功能性：
- 5 个 `*_player` 工具（无其他玩家）
- `trade_with_villager`, `villager_trades`（村民无交易环境）
- `transfer`（需要跨工具 GUI）
- `build_portal`（需开阔 4×5 地形）
- `goToBed`, `eat_item`, `collect_items`

## 差距

### 已优于 Mindcraft

- 工具数量（62 vs 55）
- A* 寻路（自研 + MC 原生双引擎，非 mineflayer-pathfinder）
- 合成/烧炼（批量自动，非单次）
- 精确放置/破坏（坐标指定，非准星依赖）
- 战斗 AI（Java tick 驱动 vs FSM）
- 自主目标分解（GoalEngine）
- 世界状态结构化感知

### 仍有差距

- 寻路无动态避障（移动实体/水域/门高宽判断）
- 建造无 JSON 蓝图系统（只能 `place_at` 单块放）
- 战斗无苦力怕专属规避距离
- 跨世界传送无寻路链
- 游泳/攀爬/船驾驶路径未专门优化
