# MC 桥接 mod（mod-bridge）—— MindFlayer 式直接读游戏数据

`craft-agent-bridge` 是一个 Fabric 客户端 mod，在 Minecraft 进程内用 Java API **直接读取结构化游戏状态**
（物品栏精确数量、方块/实体的世界坐标与距离、玩家坐标/朝向/血量），并接受精确动作指令
（`look`/`press`/`mine`/`move`/`look_at`）。外部 Rust Agent 通过本机 TCP（127.0.0.1:25567）的
JSON 行协议驱动它——**不抢鼠标键盘、可后台运行**，根治之前"看而不动 / 只走不挖 / 挖空气"的感知缺陷。

这是 B 全量 mod 控制方案：感知和动作都走 mod，enigo 的 OS 级键鼠模拟被整个移除（enigo 路径仍保留作 `real` 特性，互不影响）。

> ⚠️ 构建环境已升级。当前使用 Minecraft **26.2** + **JDK 25**。
> 本章节保留原始 1.21.11 + JDK 21 文档以供参考。
> 最新版本要求见 [`mods/craft-agent-bridge/README.md`](../../mods/craft-agent-bridge/README.md)。

## 一、前置条件（你的机器，Java 21）

- Minecraft **1.21.11**（Java 版）
- Fabric Loader ≥ 0.16.0 + Fabric API（安装到 1.21.11 客户端）
- **JDK 21**（构建 mod 用；你跑 MC 1.21.11 本身就需要 21）

## 二、构建 mod

mod 是独立 Gradle 工程，不在 Rust workspace 内：

```bash
cd mods/craft-agent-bridge
./gradlew build        # Windows: gradlew.bat build
```

产物：`mods/craft-agent-bridge/build/libs/craft-agent-bridge-0.1.0.jar`

> 若 `./gradlew` 首次运行需联网下载 Gradle 分发与 Fabric 依赖（maven.fabricmc.net）。
> 若构建报 mapping/API 名错误（如 `getYRot`/`getXRot`、`getEntities().getAll()`），
> 多半是 1.21.x 小版本 mapping 差异——按报错改 `src/main/java/com/craftagent/bridge/CraftAgentBridge.java`
> 里对应调用即可（文件顶部有协议与 API 说明注释）。

## 三、安装并启动

1. 把 `craft-agent-bridge-0.1.0.jar` 放进 `.minecraft/mods/`。
2. 启动 Minecraft 1.21.11，进入一个世界（单人/集成服务端均可）。
3. mod 在客户端启动时自动开 TCP 服务 `127.0.0.1:25567`（控制台打印
   `[craft-agent-bridge] TCP 服务线程已启动`）。
4. Agent 连不上会报 "连接 MC 桥接 mod 失败" —— 确认 MC 已进世界且 mod 已加载。

## 四、运行 Agent（全量 mod 控制）

```bash
cargo run -p craft-agent-minecraft --example agent_multi_step_mod --features mod-bridge \
  -- --steps=40 --goal="收集木头做工作台" --session=sessions/mc_run_mod.jsonl
```

- 感知 `perceive` 返回的是**结构化状态文本**（精确数据），不再靠 VLM 看图猜；同时仍截一张图供 viewer 核对。
- `mine` 回执带"原木前后数量差"，agent 能确认是否真挖到木头。
- 可选另开 viewer：`cargo run -p craft-agent-viewer -- --session sessions/mc_run_mod.jsonl`

## 五、TCP 协议（JSON 行，一行一对象，`\n` 结尾）

请求 → 响应（同一连接持久复用）：

| 请求 | 响应要点 |
|---|---|
| `{"type":"state"}` | `position/yaw/pitch/health/hunger`、 `inventory[]`(slot,id,count)、`targeted_block`(准星所指)、`nearby_blocks[]`(白名单扫描)、`entities[]`(附近生物+各自 velocity/effects)、`time/dimension/biome/gamemode`、`velocity`(玩家速度)、`effects[]`(玩家状态效果)、`experience_level`/`experience_progress`、`raining`/`thundering`、`sky_light`/`block_light`(光照 0~15) |

**state 字段明细（扩展字段旧版 mod 缺失时按默认值解析，不报错）：**

| 字段 | 类型 | 说明 |
|---|---|---|
| `velocity` | `[f64;3]` | 玩家运动速度 (vx,vy,vz)，米/秒。坠崖/被击退/在移动时非零 |
| `effects[]` | `[{id,amplifier,duration}]` | 玩家状态效果；`amplifier` 0=Ⅰ级，`duration` 单位 tick(20/s) |
| `experience_level` | `u32` | 经验等级 |
| `experience_progress` | `f32` | 当前级经验进度 0~1 |
| `raining` / `thundering` | `bool` | 是否下雨 / 雷暴 |
| `sky_light` / `block_light` | `i32` (0~15) | 玩家所在处天空/方块光照等级 |
| `entities[].velocity` | `[f64;3]` | 实体速度（生物/掉落物均有） |
| `entities[].effects[]` | `[{id,amplifier,duration}]` | 生物身上的状态效果（掉落物为空数组） |
| `{"type":"look","dx":300,"dy":0}` | 相对转视角（dx>0 右转, dy>0 低头；约 300≈90°） |
| `{"type":"look_at","x":..,"y":..,"z":..}` | 绝对朝向某坐标（精确对准，供 aim_and_mine） |
| `{"type":"press","keys":"w","ticks":40}` | 按住按键 ticks×50ms（w/a/s/d/space/shift/ctrl/e/1-9） |
| `{"type":"mine","ticks":60}` | 按住左键挖 ticks×50ms；回执 `logs_before`/`logs_after` 用于成败判断 |
| `{"type":"move","dir":"forward","ticks":40}` | 朝某方向移动 |
| `{"type":"move_to","x":..,"y":..,"z":..}` | 简易寻路走到坐标（水平 <1.5m 或超时停） |

所有动作响应含 `status`("ok"/"fail") 与 `detail`。

## 六、与既有架构的关系

- 新增 `craft-agent-minecraft` 模块：`bridge.rs`(TCP 客户端) / `adapter_mod.rs`(MinecraftModAdapter) / `tools_mod.rs`(mod 工具集)，
  全部 `#[cfg(feature="mod-bridge")]`，实现同一 `GameAdapter` trait —— 核心/决策/记忆层零改动。
- `Cargo.toml`：`mod-bridge = ["real", "dep:serde"]`（复用 xcap 截图供 viewer + craft-agent-model 的 LLM 客户端）。
- enigo 路径（`real` 特性）保持不变，可随时回退对比。

## 七、已知风险 / 待打磨

- `look_at` 的 yaw 公式按 MC 前向向量反解；若实测瞄准镜像/偏移，翻转 `CraftAgentBridge.java` 中 `Math.atan2(-ddx, ddz)` 的 `-ddx` 符号即可。
- `move_to` 是粗粒度直线导航（遇障跳过），复杂地形仍需 LLM 用 `look`+`press w` 分步走。
- 方块白名单在 `BLOCK_WHITELIST` 里调。
