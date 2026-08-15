# craft-agent-minecraft

Minecraft 游戏适配器与工具集（Azalea 客户端路线）。

唯一运行时路径：**`azalea-bot`** —— Rust 全栈客户端 bot 直连普通 MC 服务器（含局域网），
原生支持 MC 26.2，内置 Baritone 级 pathfinder。旧 `mod-bridge`（Fabric mod TCP 桥接）与
`real`（VLM 截图 + enigo 键鼠）路线已从源码删除。

## 启用特性

```toml
# Cargo.toml
craft-agent-minecraft = { features = ["azalea-bot"] }
```

## 49 个 LLM 工具

工具注册于 `create_mc_azalea_tools`，权威清单见 `tools_azalea.rs::ALL_TOOL_NAMES`。
DSH 桥接模式下由 DSH 大脑经 `/api/bot_tool` **逐工具驱动**（in-bot 时代的批处理
执行器已随主循环移除，2026-08-14）。

| 类别 | 工具 | 副作用 |
|------|------|--------|
| 感知 | `perceive` | READ |
| 记忆 | `memory` (save/anchor/query/forget) / `remember` (save/forget/list) | READ/WRITE |
| 方块搜索 | `search_for_block` | READ |
| 知识 | `search_wiki` | NETWORK |
| 移动 | `goto` / `goto_player` / `move_away` / `mine_below` / `mine_above` / `pickup` / `follow` / `stop_follow` | WRITE |
| 挖掘 | `mine` / `make_obsidian` | WRITE |
| 模式 | `set_mode` | WRITE |
| 交互 | `interact_block` / `interact_entity` | WRITE |
| 战斗 | `attack` / `defend` / `use_item` / `shoot` | WRITE |
| 睡觉 | `sleep` | WRITE |
| 合成 | `craft` (2×2) / `craft_3x3` | WRITE |
| 熔炼 | `smelt` | WRITE (等待) |
| 自动合成 | `auto_craft` | WRITE (递归) |
| 附魔 | `enchant` | WRITE |
| 采集 | `gather` / `till_and_sow` / `harvest` | WRITE (寻路) |
| 放置 | `place` / `build` / `build_blueprint` / `list_blueprints` | WRITE |
| 容器 | `open` / `chest_view` / `chest_withdraw` / `chest_deposit` | READ/WRITE |
| 装备 | `equip` / `discard` / `consume` | WRITE |
| 交易 | `trade` | WRITE |
| 社交 | `give` | WRITE |
| 聊天 | `chat` | NETWORK |
| 目标 | `set_goal` / `pause_goal` / `resume_goal` | WRITE |
| 复合 | `run_plan` / `run_script` | WRITE |
| 自定义动作 | `new_action` / `list_actions` | WRITE (持久化) |
| 任务链 | `task_complete` / `task_retry` | WRITE |

## 关键模块

| 模块 | 作用 |
|------|------|
| `adapter_azalea.rs` | `GameAdapter` 实现：perceive / execute / state snapshot |
| `tools_azalea.rs` | 49 个 LLM 工具定义 |
| `azalea/mod.rs` | `AzaleaBot` + connect + 动作 API + 背包三件套（1995 行，P2.2 已拆出 commands.rs / handler.rs） |
| `azalea/commands.rs` | `BotCommand` 33 变体 + `QueuedCommand` + `parse_chat_command`（probe 驱动） |
| `azalea/handler.rs` | `BotState` + tick 主体 handle + 两层 modes 反应系统（P2.2 拆出） |
| `azalea/craft.rs` | 合成/熔炼/附魔/切石机（含 mock 容器测试，对齐 mindcraft skills.js） |
| `azalea/gather.rs` | 自动采集：寻路+挖+掉落物统计（P55 部分成功返回 Ok） |
| `azalea/till.rs` | 种植：犁地+播种（P84/P100 自动靠近/P102 目标修正） |
| `azalea/harvest.rs` | 收割成熟作物（P86） |
| `azalea/sleep.rs` | 睡觉跳夜（P85） |
| `azalea/auto_craft.rs` | 递归配方满足 + 工具方块放置 |
| `azalea/place.rs` | 方块放置 + 容器开启 + 触及范围检查（P5/P11/P29 自动重定位） |
| `azalea/recipes.rs` | 配方知识库 |
| `azalea/perception.rs` | 位置读取 |
| `azalea/actions.rs` | 基础 bot 动作（goto/mine/chat） |
| `azalea/smart_actions.rs` | 多工具聚合动作 |
| `azalea/action_manager.rs` | 命令队列调度 |
| `azalea/table_flow.rs` | 工作台/熔炉自动放置 + open + 用完回收 |
| `azalea/recipe_book.rs` | vanilla 26.2 全量配方书（P48 替代手写表） |
| `blueprint.rs` | 蓝图系统（可复用建筑模板） |
| `action_lib.rs` | LLM 自定义动作（rhai 脚本持久化） |

## Mindcraft 哲学对齐

bot 工具只做能做的，做不了就 return Err 让 LLM 决策。详见 [`AGENTS.md`](../../AGENTS.md) 第 9-bis 节。

- 不自动合成工具方块（furnace/pickaxe/axe/sword 等）
- 不自动满足原料依赖（不在工具内部链式调用其他工具）
- 错误消息必须列出完整解决步骤
- 部分成功返回 Ok（如 gather 14/16 = Ok + 下一步建议）

## 测试

```bash
# 不需要 MC server 的 mock 容器集成测试
cargo test -p craft-agent-minecraft --features azalea-bot --lib

# 130 个测试覆盖：
#   - do_smelt / do_craft_3x3 状态机
#   - mindcraft skills.js 所有边界条件（背包满/原料不足/燃料不够/炉子被占用）
#   - P57 分批熔炼（15→8, 8→8, 9→8）
#   - 配方查询 / 燃料 fallback 链 / 产物收集验证
```

详见 [`docs/tutorials/adding-adapters.md`](../../docs/tutorials/adding-adapters.md)。
