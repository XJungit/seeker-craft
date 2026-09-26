/**
 * dsh-preset-craft-bot — craft-bot 预设包的宿主面载体插件（no-op）。
 *
 * 为什么需要这个文件：DSH 0.1.7 的 client-modules 只扫描 **host 面** 的 loader
 * 条目来发现 `dsh.client` 声明（预设组合内部的行不可见）。本包的 package.json
 * 声明了 `dsh.client { platform: "web", inject: [三个上游包名] }` 与
 * `exports["./client"]`（client.js，由生成器从 tools/dsh-bridge/client.js 镜像），
 * 因此只要有一条 host 面的行以包名加载本包，浏览器就会拿到 viewer 仪表盘面板。
 * 这条行就是 cordis.patch.yml 里的「载体行」：
 *
 *     - insert:
 *         - id: dsh-preset-craft-bot
 *           name: dsh-preset-craft-bot
 *
 * ⚠️ inject 有**两个不同平面**，写反会让整个 web boot 挂掉（2026-09-26 桌面端事故）：
 *
 *   1) package.json `dsh.client.inject` = **包名**（本处，如
 *      `@deepseek-ai/dsh-client-ui-renderer`）→ 浏览器 boot 图的**到达顺序**依赖。
 *      client-modules 的 arriveGraphRow 据此预载这些包的行；不存在的名字静默跳过，
 *      所以多写不会卡启动。本机 dsh-client-auto-continue / dsh-watcher /
 *      @omdp/dsh-connector 三个第三方插件同样在此写包名，可作先例。
 *   2) client.js `exports.inject` = **ctx 服务名**（如 `sessions`、`slots`）→
 *      cordis **fiber 激活门**。client-runner 按 Object.keys(fiber.inject) 逐个
 *      `ctx.get(name)`，查不到就永远 pending，而 boot 激活审计对 pending 与
 *      failed 一视同仁地抛 `web boot: N entries did not activate`。
 *
 * 本次事故就是把**包名**误写进了 2)（client.js exports.inject）：服务门永远等不到
 * 一个叫 `@deepseek-ai/dsh-client-ui-renderer` 的服务 → 桌面端起不来。修复后
 * 1) 写包名、2) 写服务名，两平面各自归位；tools/dsh-bridge/scripts 下的
 * verify-cordis-service-gate.mjs（真实 cordis 运行时复现两分支）、verify-boot-inject.mjs、
 * verify-client.mjs 会双向断言，生成器自检也会拦截回退。
 *
 * 本载体自身不注册任何工具 / 服务 / 代理（apply 为 no-op，无 ctx 依赖）：
 *   - 三工具 + prompt 变量 + bot_state：由预设内部的 dsh-bridge 行（file:// URL）
 *     提供（hostTools:true）；
 *   - /craft/api/* 代理：由同一条 dsh-bridge 行提供（0.1.7 生成器翻转为
 *     proxy:true；预设 scope 每进程只 mount 一次，不会重复注册）；
 *   - 浏览器面板：由本包的 dsh.client 声明提供，client.js 按主会话（mainView
 *     保留行）的 agentPreset ∈ {craft-bot, code} 显隐（顶层与 projectionValues
 *     两层都查，快照无 current），其他预设不受影响。面板双模式：overlay 浮层
 *     + sidebar-right tab（kind='craft-bot'），placement 经 localStorage
 *     dsh-bridge.placement 配置（'both' 默认 | 'sidebar' | 'overlay'）。
 * 0.1.7 起 profile 层不再注册 dsh-bridge bundle，本包是面板的唯一来源。
 *
 * @module dsh-preset-craft-bot
 */

/** Cordis 插件契约：插件名（DSH 插件列表里显示的就是它）。 */
export const name = 'dsh-preset-craft-bot'

/**
 * Cordis 插件契约：本载体自身不注入任何 ctx 服务。
 * 注意这是**宿主面**的这一行（host 面 no-op 载体），不是浏览器侧的服务门；
 * 浏览器侧的服务名声明在 client.js 的 `exports.inject`（见上方长注释）。
 */
export const inject = []

/** Cordis 插件契约：载体无需任何启动逻辑。 */
export function apply() {}
