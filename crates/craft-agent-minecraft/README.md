# craft-agent-minecraft

Minecraft 游戏适配器与工具集（Azalea 客户端路线）。

唯一运行时路径：**`azalea-bot`** —— Rust 全栈客户端 bot 直连普通 MC 服务器（含局域网），
原生支持 MC 26.2，内置 Baritone 级 pathfinder。旧 `mod-bridge`（Fabric mod TCP 桥接）与
`real`（VLM 截图 + enigo 键鼠）路线已从源码删除。

## 选择特性

```toml
# Cargo.toml
craft-agent-minecraft = { features = ["azalea-bot"] }
```

## 工具集

azalea 路线工具（注册于 `create_mc_azalea_tools`）：

- **感知**：perceive
- **导航**：goto（A* pathfinder）
- **采集**：mine_below（下挖）/ mine（精确挖掘）
- **交互**：interact_block（放置/右键激活）
- **通信**：chat（向玩家汇报）

详见 [`docs/tutorials/adding-adapters.md`](../../docs/tutorials/adding-adapters.md)。
