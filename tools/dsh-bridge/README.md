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

- **挂载方式（body-portal 全局面板）**：参照 DSH 官方插件开发规范，面板是“跨会话、固定在
  shell 角落”的全局面板，故用 **body portal + fixed 定位**（而非塞进某个语义 slot）。`apply` 在
  `document.body` 下挂载一个 DOM 单例 host（`[data-dsh-craft-host]`），并**返回 cordis disposer**
  （卸载/HMR 时清理订阅、监听、让位与 DOM）。
- **DOM 单例守卫（根治多仪表盘）**：`apply` 开头若发现 `[data-dsh-craft-host]` 已存在则直接返回
  no-op disposer（参考 whale-girl 的 `[data-whale-girl]` 守卫）。DSH 的 client bundle 是一个 cordis
  plugin entry（一个包 = 一个 loader entry = 一次 apply，见 web/src/boot.tsx），即使全局行与
  craft-bot 预设行同时挂载同一插件，页面中也**始终只存在一个**仪表盘。
- **面板**：craft-bot 会话加载即**自动打开**，固定右侧停靠（"页面旁"），iframe 嵌入
  viewer（`http://127.0.0.1:8080`，无 X-Frame-Options 可直接嵌）实时显示状态流；iframe 只加载一次、
  保留 viewer 的 SSE 连接，切换会话只显隐不重载。用户可点 ✕ 关闭，关闭后右上角出现 “🎮 Craft Bot
  仪表盘” 启动器用于重开（尊重手动关闭，本会话内不强制重开）。
- **对话列让位（真正“页面旁”而非遮挡）**：DSH 布局是三列 CSS grid（sidebar/center/details），列类名是
  哈希过的、无稳定选择器。实现用 JS 动态让位：面板打开时给 grid frame 加 `padding-right`（宽度与面板
  一致），把对话区让到面板左侧。稳定锚点是 layout 的 `[data-shell-overlay]` 的父元素（即 grid frame）；
  找不到时退化为纯 fixed 停靠（仍可用）。
- **仅 craft-bot 显示**：通过 `ctx.sessions.list.subscribe()` 订阅会话列表，当前会话切到/离开
  craft-bot 时自动显隐（`agentPreset === 'craft-bot'` 才显示；其他预设/普通会话面板保持隐藏、侧边栏
  无任何入口）。离开 craft-bot 会重置“手动关闭”标志，下次进入自动重开。
- **多 craft-bot 会话共享同一个仪表盘**：因为 host 是 DOM 单例，无论同时打开几个 craft-bot 会话，
  页面旁始终只有一块仪表盘（各自会话的入口按钮/启动器都操控同一面板）。
- **同源代理**：host 端挂 `/craft/api/*` → viewer `/api/*` 转发（GET/POST 透传），
  浏览器端零跨域读取 viewer API。

**双行配置**（避免 webServer 路径重复注册）：

| 位置 | hostTools | proxy | 作用 |
|---|---|---|---|
| profile 全局行（`cordis.patch.yml`，包名 `dsh-bridge`） | `false` | `true` | client 半边（面板 + 代理），不污染其他项目工具 |
| craft-bot 预设行（`agent.cordis.yml`，绝对路径） | `true` | `false` | host 工具（三工具 + prompt 变量）驱动 bot |

> client 半边（client.js 浏览器面板）不依赖 hostTools：只要包被 loader 以包名加载，
> DSH 的 client-modules 就会独立发现 `dsh.client` 声明并注入浏览器。

## Prompt 贡献（{{...}} 变量 与 动态上下文）

插件向 DSH 的 `systemPrompt` 注册**两类** prompt 贡献——**静态变量**（进 system 提示段，
字节稳定，保 DeepSeek 前缀缓存命中）与**动态上下文**（每次 pre-step 装配，作为
**user 角色快照**追加到对话末尾，新快照取代旧快照，不碎前缀缓存）：

| 名称 | 注册方式 | 来源 | 说明 |
|---|---|---|---|
| `{{tool_list}}` | `systemPrompt.variable`（→ system 段） | 静态镜像 `ALL_TOOL_NAMES` | 53 工具清单（`·` 分隔），字节稳定 |
| `{{viewer_url}}` | `systemPrompt.variable`（→ system 段） | `DSH_CRAFT_VIEWER_URL` 或默认 | viewer 地址，字节稳定 |
| `bot_state` | `systemPrompt.context`（→ user 快照） | `GET /api/game-state`（30s 缓存后台刷新） | 当前 bot 状态快照（中文摘要）；内容变化时才追加，模型历史中始终只有最新一份 |

> **注意**：`bot_state` **不是** system 变量（不存在 `{{bot_state}}` 占位符），而是
> `systemPrompt.context` 动态上下文。由 agent-loop 每次 pre-step 装配，渲染成 user
> 角色快照（固定前缀 `Current runtime context. This snapshot supersedes earlier
> runtime-context snapshots.`）追加到对话末尾；内容与上一份相同时**不重复追加**
> （`RuntimeContextProjection.project()` 变更检测，`dsh-agent-loop`），因此不累积、
> 不进 system 提示、不破坏前缀缓存。模型需要**实时/更详细**状态时主动调
> `game_state()`（会显示为 `Tool call · game_state` 卡片，与自动注入的 user 快照不同）。

内置变量（agent-loop 注册）：`{{model}}`、`{{cwd}}`、`{{provider}}`。

> **契约约束**：`systemPrompt.variable` 与 `systemPrompt.context` 的 provider 都是
> **同步**调用（assemble 不 await），因此 `bot_state` 只读缓存、由 `setInterval`
> 后台刷新（首装配前最多落后 TTL 30s）；变量名必须匹配 `[a-z][a-z0-9_]*`；
> persona 引用未知变量会在装配期抛错（严格插值）。

## 安装

> **1.0 推荐方式**：直接运行仓库根 `scripts/setup.ps1`，它会自动完成本节全部
> 步骤（注册插件、链接依赖、pnpm install、生成 craft-bot 预设、运行验证）。
> 以下为手动安装参考（等价于 setup.ps1 的 3/4 步）。

本插件作为本地包通过 profile 的 `cordis.patch.yml` 注册：

```yaml
# ~/.dsh/profiles/web/cordis.patch.yml 追加
- insert:
    - id: dsh-bridge
      name: dsh-bridge
```

并在 `~/.dsh/profiles/web/package.json` 的 dependencies 加 link 依赖指向本目录
（`<repo-root>` 为你的仓库克隆路径，如 `D:/Craft-Agent`）：

```json
"dsh-bridge": "link:<repo-root>/tools/dsh-bridge"
```

然后 `cd ~/.dsh/profiles/web && pnpm install`。

### 依赖解析（node_modules 链接）

`index.js` import `@deepseek-ai/dsh-tools` / `@deepseek-ai/schemastery`。link 包的真实路径在
仓库内，Node 默认从该路径向上解析依赖会失败。需要把 DSH 实际使用的包链接到插件本地：

```powershell
# 定位 DSH 的 npx 安装根（dsh CLI 所在 node_modules/@deepseek-ai）
$npx = "<DSH 安装根>\node_modules\@deepseek-ai"
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
仪表盘面板：重启 DSH 后，进入 craft-bot 会话即于页面右侧自动内嵌显示 viewer 仪表盘（"页面旁"）；
切到非 craft-bot 会话自动收起，多 craft-bot 会话共享同一块面板。
