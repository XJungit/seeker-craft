# craft-agent-bridge-1.21（旧版工具链备份）

本目录是 **MC 1.21.11** 的构建配置备份。主目录 `craft-agent-bridge/` 已升级到 26.2。

## 使用方法

1. 把本目录的文件覆盖到 `craft-agent-bridge/`（保留 `src/` 源码不动）
2. 用 Java 21（`java-runtime-delta`）+ Gradle 8.14.2 编译
3. 对应 MC 1.21.11 + fabric-loom 1.13.2

## 工具链版本

| 组件 | 版本 |
|------|------|
| Gradle | 8.14.2 |
| fabric-loom | 1.13.2 |
| Minecraft | 1.21.11 |
| fabric-api | 0.141.4+1.21.11 |
| Java | 21 |

## 网络问题

若直连 maven.fabricmc.net 失败，启动本地代理：
```bat
cd D:\Craft-Agent\mods\craft-agent-bridge
python tools\maven_proxy.py
```
