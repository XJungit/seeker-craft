# dsh-bridge

craft-bot 预设的 viewer 桥插件（DSH 侧）。让 [DSH](https://github.com/deepseek-ai/deepseek-harness)
作为 Minecraft bot（Craft-Agent）的**唯一大脑**：经 craft-agent-viewer 的 HTTP API 驱动 live bot。

## 工具

| 工具 | 端点 | 说明 |
|---|---|---|
| `game_state()` | `GET /api/game-state` | 读取实时世界状态（scene_desc 中文摘要 + 结构化字段） |
| `bot_tool(name, args)` | `POST /api/bot_tool` | 执行 53 个 Minecraft 工具之一（含自动修正） |
| `set_goal(text)` | `POST /api/goal` | 设置 bot 运营目标 |

- viewer 地址默认 `http://127.0.0.1:8080`，可用环境变量 `DSH_CRAFT_VIEWER_URL` 覆盖。
- `bot_tool` 复用与 agent_loop 完全相同的工具注册表（`create_mc_azalea_tools_full`），
  P100/P101/P102/P132 的派发时自动修正在 `GameTool::execute` 闭包内，桥接天然保留。

## 安装

本插件作为本地包通过 profile 的 `cordis.patch.yml` 注册：

```yaml
# ~/.dsh/profiles/web/cordis.patch.yml 追加
- insert:
    - id: dsh-bridge
      name: dsh-bridge
```

并在 `~/.dsh/profiles/web/package.json` 的 dependencies 加 link 依赖指向本目录。

## 前置

- craft-agent-viewer 已启动且 bot 已连接（`craft-agent-ctl status` 显示 viewer 存活、game-state 可读）。
- bot 连接：viewer 启动后经 `POST /api/connect`（DSH 模式不启动 in-bot LLM 循环）。

## 验证

```bash
node scripts/verify-bridge.mjs
```

逐一验证 `game_state` / `bot_tool` / `set_goal` 三个端点的连通性。
