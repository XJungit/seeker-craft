# dsh-bridge

craft-bot 预设的 viewer 桥插件（DSH 侧）。让 [DSH](https://github.com/deepseek-ai/deepseek-harness)
作为 Minecraft bot（Craft-Agent）的**唯一大脑**：经 craft-agent-viewer 的 HTTP API 驱动 live bot。

## 支持的 DSH 版本

| 依赖项 | 支持范围 | 说明 |
|---|---|---|
| `@deepseek-ai/dsh`（CLI / harness） | `0.1.5-rc.3` / `0.1.7-rc.1` | 两个版本均已实测通过（跑 `scripts/verify-in-harness.mjs`，17/17） |
| `@deepseek-ai/dsh-tools` | `0.1.5-rc.3 \|\| 0.1.7-rc.1`（peerDependency） | 提供 `defineTool`；由 DSH CLI 自带副本经 `link:` 解析 |
| `@deepseek-ai/schemastery` | `3.18.2 \|\| 3.18.4` | 配置 schema（分别对应 rc.3 与 0.1.7） |

> peerDependencies 只枚举**本机实测过**的精确版本，不使用开放区间（`^`/`>=`）——开放区间会在
> DSH 新版本发布时被动“自动通过”却从未验证。新增支持版本时需先实测、再逐条追加。

本插件依赖的 DSH 契约（升级 DSH 时优先回归验证以下几处，任一变更都会让插件**静默失效**）：

- **client 半边**：`window.__ModuleLoader__.load({ id, factory })`；`package.json` 的
  `dsh.client.{platform,inject}` 声明；`ctx.sessions.list`（ObservableSnapshot，
  `getSnapshot() -> { current, byId }`）与会话字段 `agentPreset`。
- **host 半边**：`inject = ['tools', 'systemPrompt', 'webServer']` 三个服务名；`ctx.tools.register(defineTool(...))`；
  `ctx.systemPrompt.variable(name, fn)` / `ctx.systemPrompt.context({...})`；
  `ctx.webServer.register({ kind: 'prefix', path, handler })`（经此外挂 `/craft/api/*` 同源代理）。
  以上 4 处 API 的签名已在 0.1.5-rc.3 与 0.1.7-rc.1 上逐一比对，**完全一致**。
- **已知破坏性变更**：harness 已将 Code Mode 更名为 **PTC**（Programmatic Tool Calling），
  `@deepseek-ai/dsh-agent-tool-presentation` 的 schema 只接受 `native|ptc|both`；旧值 `"code"`
  会导致预设挂载失败（craft-bot 预设已改为 `mode: ptc`）。

> **维护约定**：每次改动本插件时，同步更新上表的实测版本与契约清单。

## 工具

| 工具 | 端点 | 说明 |
|---|---|---|
| `game_state()` | `GET /api/game-state` | 读取实时世界状态（scene_desc 中文摘要 + 结构化字段） |
| `bot_tool(name, args)` | `POST /api/bot_tool` | 执行 49 个 Minecraft 工具之一（含自动修正） |
| `set_goal(text)` | `POST /api/goal` | 设置 bot 运营目标 |

- viewer 地址默认 `http://127.0.0.1:8080`，可用环境变量 `DSH_CRAFT_VIEWER_URL` 覆盖。
- `bot_tool` 复用与 agent_loop 完全相同的工具注册表（`create_mc_azalea_tools_full_with_semantic`，
  含 49 工具 + `remember` 语义记忆），
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
- **面板（不自动打开，仅手动）**：进入 craft-bot 会话时面板**保持隐藏**，只在**右边缘垂直居中**
  显示 “🎮 Craft” 启动器小标签（writing-mode 竖排）；点击它才打开面板。面板固定右侧停靠（"页面旁"），
  iframe 嵌入 viewer（`http://127.0.0.1:8080`，无 X-Frame-Options 可直接嵌）实时显示状态流；iframe 只加载
  一次、保留 viewer 的 SSE 连接，切换会话只显隐不重载。**关闭按钮 “关闭 ✕” 位于面板底部栏右下角**
  （不放右上角，避免与 DSH 原生右上 UI 如 Session log 重叠）；关闭后启动器重现，可再次手动打开。
  显隐由 `window.__dshCraftUserOpened` 单一标志决定（默认 false = 不自动打开）。
- **对话列让位（真正“页面旁”而非遮挡）**：DSH 布局是三列 CSS grid（sidebar/center/details），列类名是
  哈希过的、无稳定选择器。实现用 JS 动态让位：面板打开时给 grid frame 加 `padding-right`（宽度与面板
  一致），把对话区让到面板左侧。稳定锚点是 layout 的 `[data-shell-overlay]` 的父元素（即 grid frame）；
  找不到时退化为纯 fixed 停靠（仍可用）。
- **仅 craft-bot 显示**：通过 `ctx.sessions.list.subscribe()` 订阅会话列表，当前会话切到/离开
  craft-bot 时自动显隐（`agentPreset === 'craft-bot'` 才注册 UI；其他预设/普通会话面板保持隐藏、
  侧边栏无任何入口）。离开 craft-bot 会把 `__dshCraftUserOpened` 重置为 `false`，**下次进入仍需手动打开**。
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

> **client inject 最小声明（维护注意）**：`package.json` 的 `dsh.client.inject` 必须与
> `client.js` 的 `exports.inject` **一致且最小**——只声明实际注入的 Service 短名。
> 当前两者都是 `["sessions"]`（client 只订阅 `ctx.sessions.list` 判断 craft-bot 预设）。
> 不要加未使用的注入名：DSH client-modules 的 boot 会为 manifest 每个 inject 建 fiber
> 注入等待，未解析的 Service 名会导致 `pending (waiting for service...)` 乃至 boot
> fail-loud（`assertEntriesActive`）。`slots`/`locale` 等 UI 包不注册短名 Service，不要列入。

## Prompt 贡献（{{...}} 变量 与 动态上下文）

插件向 DSH 的 `systemPrompt` 注册**两类** prompt 贡献——**静态变量**（进 system 提示段，
字节稳定，保 DeepSeek 前缀缓存命中）与**动态上下文**（每次 pre-step 装配，作为
**user 角色快照**追加到对话末尾，新快照取代旧快照，不碎前缀缓存）：

| 名称 | 注册方式 | 来源 | 说明 |
|---|---|---|---|
| `{{tool_list}}` | `systemPrompt.variable`（→ system 段） | 静态镜像 `ALL_TOOL_NAMES` | 49 工具清单（`·` 分隔），字节稳定 |
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
（`<repo-root>` 为你的仓库克隆路径，如 `D:/SeekerCraft`）：

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
#    自动探测 DSH node_modules；找不到时用 DSH_NPX_ROOT 显式指定
node scripts/verify-in-harness.mjs

# 3) 仪表盘代理 /craft/api/* 单元验证（mock viewer，测 GET/POST/404/502）
node scripts/verify-proxy.mjs

# 4) client 半边 agentPreset 判断（craft-bot 注册 / code 不注册）
node scripts/verify-client.mjs

# 5) 纯工具清单比对（零依赖、任何环境可跑；index.js::TOOL_NAMES == rust ALL_TOOL_NAMES）
node scripts/verify-tool-names.mjs
```

> `verify-in-harness.mjs`（第 2 步）依赖 DSH node_modules，自动探测 npm/pnpm 全局根下的
> `@deepseek-ai/dsh/node_modules`（接受 scoped 路径 `@deepseek-ai/schemastery`/`dsh-tools`）；
> 探测失败时用 `DSH_NPX_ROOT` 显式指定。若只想快速确认工具清单同步，用第 5 步的
> `verify-tool-names.mjs`（无需 DSH）。

端到端（DSH 会话内）：`game_state` 感知 → `bot_tool(name, args)` 执行 → `set_goal(text)` 设目标。
三工具出现在工具目录即挂载成功。`bot_tool` 的参数按各工具 schema 传（如 `equip` 需
`{item, slot}`，`slot` 枚举 hand/helmet/chestplate/leggings/boots）。
仪表盘面板：重启 DSH 后，进入 craft-bot 会话会在**右边缘**出现 “🎮 Craft” 启动器小标签，
**点击它才打开**面板（不自动弹出）；切到非 craft-bot 会话自动收起，多 craft-bot 会话共享同一块面板。
面板内容依赖 viewer 存活（`127.0.0.1:8080`）——viewer 未启动时标签仍会出现，但面板内 iframe 会空白/报错。
