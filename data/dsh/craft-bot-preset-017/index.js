/**
 * dsh-preset-craft-bot — craft-bot 预设包的宿主面载体插件（no-op）。
 *
 * 为什么需要这个文件：DSH 0.1.7 的 client-modules 只扫描 **host 面** 的 loader
 * 条目来发现 `dsh.client` 声明（预设组合内部的行不可见）。本包的 package.json
 * 声明了 `dsh.client { platform: "web", inject: ["sessions"] }` 与
 * `exports["./client"]`（client.js，由生成器从 tools/dsh-bridge/client.js 镜像），
 * 因此只要有一条 host 面的行以包名加载本包，浏览器就会拿到 viewer 仪表盘面板。
 * 这条行就是 cordis.patch.yml 里的「载体行」：
 *
 *     - insert:
 *         - id: dsh-preset-craft-bot
 *           name: dsh-preset-craft-bot
 *
 * 本载体不注册任何工具 / 服务 / 代理（inject 为空，apply 为 no-op）：
 *   - 三工具 + prompt 变量 + bot_state：由预设内部的 dsh-bridge 行（file:// URL）
 *     提供（hostTools:true）；
 *   - /craft/api/* 代理：由同一条 dsh-bridge 行提供（0.1.7 生成器翻转为
 *     proxy:true；预设 scope 每进程只 mount 一次，不会重复注册）；
 *   - 浏览器面板：由本包的 dsh.client 声明提供，client.js 按 agentPreset ∈
 *     {craft-bot, code} 显隐，其他预设不受影响。
 * 0.1.7 起 profile 层不再注册 dsh-bridge bundle，本包是面板的唯一来源。
 *
 * @module dsh-preset-craft-bot
 */

/** Cordis 插件契约：插件名（DSH 插件列表里显示的就是它）。 */
export const name = 'dsh-preset-craft-bot'

/** Cordis 插件契约：不注入任何服务——一切由预设内部的 dsh-bridge 行提供。 */
export const inject = []

/** Cordis 插件契约：载体无需任何启动逻辑。 */
export function apply() {}
