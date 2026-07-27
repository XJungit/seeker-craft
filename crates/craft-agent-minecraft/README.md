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

## 37 个 LLM 工具

工具注册于 `create_mc_azalea_tools`，按副作用分组并行执行（READ 同批、NETWORK+READ 同批、
WRITE/APPEND/PROCESS 各自单独一批，BARRIER 切批）。

| 类别 | 工具 | 副作用 |
|------|------|--------|
| 感知 | `perceive` | READ |
| 记忆 | `memory` (save/anchor/query/forget) | READ/WRITE |
| 移动 | `go` | WRITE |
| 挖掘 | `mine` / `mine_below` | WRITE |
| 交互方块 | `interact_block` | WRITE |
| 战斗 | `attack` / `defend` | WRITE |
| 合成 | `craft` (2×2) / `craft_3x3` | WRITE |
| 熔炼 | `smelt` | WRITE (等待) |
| 自动合成 | `auto_craft` | WRITE (递归) |
| 附魔 | `enchant` | WRITE |
| 采集 | `gather` | WRITE (寻路) |
| 放置 | `place` | WRITE |
| 容器 | `open` / `chest_view` / `chest_withdraw` / `chest_deposit` | READ/WRITE |
| 装备 | `equip` / `discard` | WRITE |
| 食用 | `consume` | WRITE (长按) |
| 实体交互 | `interact_entity` | WRITE |
| 交易 | `trade` | WRITE |
| 聊天 | `chat` | NETWORK |
| 设目标 | `set_goal` / `pause_goal` / `resume_goal` | WRITE |
| 建造 | `build` / `build_blueprint` / `list_blueprints` | WRITE |
| 复合计划 | `run_plan` | WRITE |
| 复合脚本 | `run_script` | WRITE (rhai) |
| 自定义动作 | `new_action` / `list_actions` | WRITE (持久化) |
| 知识搜索 | `search_wiki` | NETWORK |

## 关键模块

| 模块 | 作用 |
|------|------|
| `adapter_azalea.rs` | `GameAdapter` 实现：perceive / execute / state snapshot |
| `tools_azalea.rs` | 37 个 LLM 工具定义 |
| `azalea/mod.rs` | `AzaleaBot` + handler + 两层 modes 反应系统（Handler 层） |
| `azalea/craft.rs` | 合成/熔炼/附魔/切石机（含 mock 容器测试，对齐 mindcraft skills.js） |
| `azalea/gather.rs` | 自动采集：寻路+挖+掉落物统计（P55 部分成功返回 Ok） |
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

# 118 个测试覆盖：
#   - do_smelt / do_craft_3x3 状态机
#   - mindcraft skills.js 所有边界条件（背包满/原料不足/燃料不够/炉子被占用）
#   - P57 分批熔炼（15→8, 8→8, 9→8）
#   - 配方查询 / 燃料 fallback 链 / 产物收集验证
```

详见 [`docs/tutorials/adding-adapters.md`](../../docs/tutorials/adding-adapters.md)。
