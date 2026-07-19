# craft-agent-bridge

Minecraft Fabric 客户端 Mod，为 Craft-Agent 提供**结构化感知 + 精确执行**的 TCP 桥接。

## 职责

- 通过 Minecraft Java API 直接读取游戏世界状态（物品栏、方块、实体、玩家属性）
- 接收并执行精确动作指令（采集、合成、放置、移动、战斗等）
- 通过本地 TCP（127.0.0.1:25567）JSON 行协议与 Rust Agent 通信
- 不抢鼠标键盘，可后台运行

## 构建

```bash
cd mods/craft-agent-bridge
$env:JAVA_HOME = 'C:\Users\xj\AppData\Roaming\.minecraft\runtime\java-runtime-epsilon'
.\gradlew.bat build
```

产物：`build/libs/craft-agent-bridge-0.1.0.jar`

## 前提条件

- Minecraft **26.2** "Chaos Cubed"
- Fabric Loader ≥ 0.16.0 + Fabric API 0.154.2+26.2
- JDK 25（构建用；运行 MC 26.2 本身需要）

## 协议

详见 [`docs/design/mod-bridge.md`](../../docs/design/mod-bridge.md)。

## 构建辅助

编译工具链说明见 [`tools/README.md`](tools/README.md)。
