# craft-agent-minecraft

Minecraft 游戏适配器与工具集。支持两条运行时路径：

| 路径 | 特性 | 感知 | 执行 |
|---|---|---|---|
| `mod-bridge` | Fabric mod TCP 桥接，可后台 | 结构化 JSON（精确坐标/数量） | Mod 主线程精确执行 |
| `real` | VLM 截图 + enigo 键鼠 | 截图 + VLM 分析 | OS 级键鼠模拟 |

## 选择特性

```toml
# Cargo.toml
craft-agent-minecraft = { features = ["real"] }       # 或
craft-agent-minecraft = { features = ["mod-bridge"] }  # 默认
```

## 工具集

约 20 个 Minecraft 工具，覆盖：
- **采集**：collect / mine_block
- **合成**：craft
- **建造**：place / move_to / look_at
- **战斗**：combat / attack
- **信息**：perceive / inventory / world_state

## McAgentBuilder

统一构造入口，同时支持 mod-bridge 和 real 路径：

```rust
let agent = McAgentBuilder::new(goal)
    .with_mod_bridge("127.0.0.1", 25567)?
    .build(provider, compaction)?;
```

详见 [`docs/tutorials/adding-adapters.md`](../../docs/tutorials/adding-adapters.md)。
