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

## Prompt 占位符变量（{{...}} 动态注入）

插件同时注册 prompt 占位符（`systemPrompt.variable`），让预设 persona 用 `{{...}}`
引用运行时数据——**改外部数据不碰预设文件、不重启 DSH**：

| 占位符 | 来源 | 说明 |
|---|---|---|
| `{{bot_state}}` | `GET /api/game-state`（30s 缓存后台刷新） | 当前 bot 状态快照（中文摘要） |
| `{{tool_list}}` | 静态镜像 `ALL_TOOL_NAMES` | 53 工具清单（`·` 分隔） |
| `{{viewer_url}}` | `DSH_CRAFT_VIEWER_URL` 或默认 | viewer 地址 |

内置占位符（agent-loop 注册）：`{{model}}`、`{{cwd}}`、`{{provider}}`。

> **契约约束**：`systemPrompt.variable` 的 provider 是**同步**调用（assemble 不 await），
> 因此 `{{bot_state}}` 只读缓存、由 `setInterval` 后台刷新；变量名必须匹配
> `[a-z][a-z0-9_]*`；persona 引用未知变量会在装配期抛错（严格插值）。

## 安装

本插件作为本地包通过 profile 的 `cordis.patch.yml` 注册：

```yaml
# ~/.dsh/profiles/web/cordis.patch.yml 追加
- insert:
    - id: dsh-bridge
      name: dsh-bridge
```

并在 `~/.dsh/profiles/web/package.json` 的 dependencies 加 link 依赖指向本目录：

```json
"dsh-bridge": "link:D:/Craft-Agent/tools/dsh-bridge"
```

然后 `cd ~/.dsh/profiles/web && pnpm install`。

### 依赖解析（node_modules 链接）

`index.js` import `@deepseek-ai/dsh-tools` / `@deepseek-ai/schemastery`。link 包的真实路径在
仓库内，Node 默认从该路径向上解析依赖会失败。需要把 DSH 实际使用的包链接到插件本地：

```powershell
# 定位 DSH 的 npx 安装根（dsh CLI 所在 node_modules/@deepseek-ai）
$npx = "C:\Users\xj\AppData\Local\npm-cache\_npx\<hash>\node_modules\@deepseek-ai"
New-Item -ItemType Junction tools\dsh-bridge\node_modules\@deepseek-ai\dsh-tools -Target "$npx\dsh-tools"
New-Item -ItemType Junction tools\dsh-bridge\node_modules\@deepseek-ai\schemastery -Target "$npx\schemastery"
```

`node_modules/` 已加入 `.gitignore`（机器相关，勿提交）。

## 前置

- craft-agent-viewer 已启动且 bot 已连接（`craft-agent-ctl status` 显示 viewer 存活、game-state 可读）。
- bot 连接：viewer 启动后经 `POST /api/connect`（DSH 模式不启动 in-bot LLM 循环）。

## 验证

```bash
# 1) viewer API 连通性（独立于 DSH，需 viewer 运行且 bot 已连接）
node scripts/verify-bridge.mjs

# 2) DSH 模块图内加载（模拟 DSH loader 的 harness-base 解析，无需 DSH 重启）
node scripts/verify-in-harness.mjs
```

端到端（DSH 会话内）：`game_state` 感知 → `bot_tool(name, args)` 执行 → `set_goal(text)` 设目标。
三工具出现在工具目录即挂载成功。`bot_tool` 的参数按各工具 schema 传（如 `equip` 需
`{item, slot}`，`slot` 枚举 hand/helmet/chestplate/leggings/boots）。
