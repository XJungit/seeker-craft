# MC 桥接 mod（craft-agent-bridge）

`craft-agent-bridge` 是一个 Fabric 客户端 mod，通过 TCP JSON 协议（127.0.0.1:25567）
让外部 Rust Agent 直接读取游戏状态并精确控制玩家。

- Minecraft **26.2** + **JDK 25**
- Fabric Loader + Fabric API
- Gradle 构建：`cd mods/craft-agent-bridge && .\gradlew.bat build`

## 架构

```
Rust 决策层 ──TCP(25567)── Java mod
  ┌──────────────────────┐  ┌───────────────────────────┐
  │ 工具调用 (62 tools)   │→│ COMMAND_HANDLERS dispatch  │
  │ perceive → StateBuilder│ │ InteractionController 等  │
  │ collect → CollectCtl  │ │ AStar + pathing/ 子系统   │
  │ combat  → CombatCtl   │ │ GoalEngine 自主执行       │
  │ move_to → MovementCtl │ │ autoSurvive 守护          │
  └──────────────────────┘  └───────────────────────────┘
```

## TCP 协议

请求：`{"type":"<命令>", ...参数}` JSON 一行，`\n` 结尾
响应：`{"status":"ok"/"fail", ...} JSON 一行

当前支持 ~60 条命令，通过 `COMMAND_HANDLERS` 路由到各 Controller。

### 常用命令

| 命令 | 参数 | 说明 |
|---|---|---|
| `state` | — | 全量状态（位置/物品/方块/实体/光照等） |
| `look` | dx,dy | 相对转视角 |
| `look_at` | x,y,z | 绝对朝向坐标 |
| `place_at` | x,y,z,item | 精确放置方块 |
| `dig_at` | x,y,z | 精确破坏方块 |
| `move_to` | x,y,z,radius | A* 寻路导航 |
| `collect` | target,num | 自动寻找+采集 |
| `combat` | mode,ticks | 战斗(melee/kite/retreat) |
| `attack` | — | 攻击最近敌对 |
| `craft` | item,num | 自动合成 |
| `smelt` | item,num | 自动烧炼 |
| `enchant` | item,enchantment | 附魔 |
| `debug_spawn` | entity,num | 刷实体（测试用） |

详见源代码 `CraftAgentBridge.java:registerCommandHandlers()` 及各 Controller。

## Java 侧组件

| 组件 | 行数 | 职责 |
|---|---|---|
| `CraftAgentBridge` | ~1004 | TCP server + dispatch + 移动 tick + autoSurvive |
| `AStar.java` | 340 | A* 寻路（8方向+10方向变体+重力落地） |
| `VanillaPathfinder` | 80 | MC 原生寻路包装（Zombie 代理） |
| `PlayerPathExecutor` | 231 | 逐帧路径执行+autoDig+悬崖检测 |
| `MovementController` | 1146 | 移动/战斗/收集/跟随/useItem/eat/pillarUp |
| `CombatController` | 149 | 战斗 AI 状态机（melee/kite/retreat） |
| `CollectController` | 202 | 方块自动采集（扫描→导航→破坏→验证） |
| `GoalEngine` | 479 | 自主目标分解（craft/get/smelt/hunt/build/explore/defend） |
| `FakePlayerManager` | 255 | FakePlayer 生命周期 + 物品栏持久化 |
| 其余 7 Controller | ~3000 | 交互/容器/实体交互/调试/建造/合成/状态 |

## 与 Rust 的职责边界

当前状态有重叠，已知冲突点：

- **GoalEngine (Java)** 自主分解目标 vs **Rust LLM** 决策循环——两个决策体可能抢控制权
- `performCombat`(MovementController) vs `CombatController.tick()`——两套战斗系统
- GoalEngine 的 craft/smelt 逻辑与 Rust 侧 CraftingHelper 重复
- `autoSurvive` 守护逻辑在 Java 侧运行，不经过 LLM 决策
