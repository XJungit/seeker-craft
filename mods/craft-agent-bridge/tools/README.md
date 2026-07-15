# 构建辅助工具（craft-agent-bridge mod）

本目录存放 Fabric mod 编译所需的本地辅助脚本。**直接编译会因网络问题失败**，原因与解法见下。

## 为什么需要本地代理

本机 Java/Gradle 运行栈**无法直连 `maven.fabricmc.net`**（TLS 握手失败 / 连接挂起），
但 **Python（urllib）能正常访问**。因此用 `maven_proxy.py` 起一个本地 HTTP 代理：

- Gradle 只访问 `http://127.0.0.1:8099`（明文、本地），
- 代理用 urllib 去真实上游（`maven.fabricmc.net`、`repo1.maven.org` 等）拉取并**分块流式回传 + 磁盘缓存**。

> Minecraft 客户端本体（Mojang CDN）由 Gradle 直连 IPv4 拉取（本机 IPv4 到 CDN 通畅），
> 不经过代理，无需额外处理。

## 编译步骤（每次都按此顺序）

1. **启动代理**（构建期间必须保持运行）：
   ```bat
   cd D:\Craft-Agent\mods\craft-agent-bridge
   python tools\maven_proxy.py
   ```
2. **用 Gradle 8.14.2 + Java 21 构建**（Java 21 复用 MC 启动器自带）：
   ```bat
   set JAVA_HOME=C:\Users\xj\AppData\Roaming\.minecraft\runtime\java-runtime-delta
   C:\Users\xj\gradle\gradle-8.14.2\bin\gradle.bat --no-daemon build
   ```
3. 产物：`build\libs\craft-agent-bridge-<version>.jar`

## 关键版本（已验证可用，勿随意升降）

| 组件 | 版本 | 说明 |
|------|------|------|
| Gradle | 8.14.2 | loom 1.13.x 要求 Gradle ≥ 8.12；1.14+ 要 Gradle 9，不可用 |
| fabric-loom | 1.13.2 | 1.10.4 不支持 1.21.11 的 unpick v4 映射；1.14+ 要 Gradle 9 |
| Minecraft | 1.21.11 | |
| fabric-api | 0.141.4+1.21.11 | 0.115.0 等旧号在 maven 上不存在 |
| Java | 21（MC 启动器 java-runtime-delta） | loom 1.13 要求 Java 21 |

## 文件说明

- `maven_proxy.py` — 本地 Maven 代理（端口 8099），带磁盘缓存 `proxy_cache/`。
- `download_gradle.py` — Gradle 发行包断点续传下载器（仅当本地 Gradle 被清掉时重下）。
- `proxy_cache/` — 已下载的 maven 工件缓存（约 50MB）。**保留**，可让后续构建免重复联网；
  若网络环境恢复直连，可整目录删除（构建会重新拉取）。

## 排错

- 报 `Repository ... is disabled` / 大量 `Could not resolve`：代理没起或挂了 → 确认 8099 在监听，重启代理。
- 报 `requires at least Gradle 8.12`：Gradle 版本太低，用 8.14.2。
- 报 `Unsupported unpick version`：loom 太老，升到 1.13.2+。
- 报 `No matching variant ... plugin-api-version`：loom 版本要求 Gradle 9，降到 1.13.2。
