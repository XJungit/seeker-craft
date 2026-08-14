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

## 内嵌仪表盘（client 半边）

把 craft-agent-viewer 的 Web 仪表盘内嵌进 DSH 页面，在对话区**旁**实时显示 bot 状态
（位置/生命/饱食/背包/附近/会话流）。**只在 craft-bot 预设（DSH 控制 Minecraft bot 的
会话）显示**——判断依据 `ctx.sessions.list.getSnapshot()` 当前会话 `agentPreset === 'craft-bot'`；
其他预设/普通会话完全不注册 UI（不控制 bot 时显示无意义）。

- **入口**：侧边栏底部按钮（`sidebar.footer.action` 插槽；插槽未声明时 DOM 注入兜底），点击开合面板。
- **面板**：craft-bot 会话加载即**自动打开**，固定右侧停靠（"页面旁"，不遮挡对话列），iframe 嵌入
  viewer（`http://127.0.0.1:8080`，无 X-Frame-Options 可直接嵌）实时显示状态流；用户可点 ✕ 或入口
  按钮关闭（尊重手动关闭，本会话内不强制重开）。
- **仅 craft-bot 显示**：`apply` 对 `agentPreset !== 'craft-bot'` 直接返回、不注册任何 UI；并通过每秒
  `reconcile` 在**会话切换**时动态隐藏（切到非 craft-bot 立即收起、切回自动重开），跨插件激活互斥
  （`dsh-panel-activate` 事件，与 task-board/ssh 面板互不打架）。
- **同源代理**：host 端挂 `/craft/api/*` → viewer `/api/*` 转发（GET/POST 透传），
  浏览器端零跨域读取 viewer API。

**双行配置**（避免 webServer 路径重复注册）：

| 位置 | hostTools | proxy | 作用 |
|---|---|---|---|
| profile 全局行（`cordis.patch.yml`，包名 `dsh-bridge`） | `false` | `true` | client 半边（面板 + 代理），不污染其他项目工具 |
| craft-bot 预设行（`agent.cordis.yml`，绝对路径） | `true` | `false` | host 工具（三工具 + prompt 变量）驱动 bot |

> client 半边（client.js 浏览器面板）不依赖 hostTools：只要包被 loader 以包名加载，
> DSH 的 client-modules 就会独立发现 `dsh.client` 声明并注入浏览器。

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

# 3) 仪表盘代理 /craft/api/* 单元验证（mock viewer，测 GET/POST/404/502）
node scripts/verify-proxy.mjs

# 4) client 半边 agentPreset 判断（craft-bot 注册 / code 不注册）
node scripts/verify-client.mjs
```

端到端（DSH 会话内）：`game_state` 感知 → `bot_tool(name, args)` 执行 → `set_goal(text)` 设目标。
三工具出现在工具目录即挂载成功。`bot_tool` 的参数按各工具 schema 传（如 `equip` 需
`{item, slot}`，`slot` 枚举 hand/helmet/chestplate/leggings/boots）。
仪表盘面板：重启 DSH 后，craft-bot 会话侧边栏出现 "Craft Bot 仪表盘" 按钮，点击内嵌显示。
