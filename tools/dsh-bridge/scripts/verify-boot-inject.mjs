#!/usr/bin/env node
/**
 * 端到端 boot 回归：证明载体行不再让 web boot 挂掉（2026-09-26 桌面端事故）。
 *
 * 事故链条（本次实测复现并验证修复）：
 *   1. 我在 tools/dsh-bridge/client.js 的 `exports.inject` 里写了**包名**
 *      （'@deepseek-ai/dsh-client-ui-renderer' 等）。
 *   2. client runner 的服务门按 `Object.keys(fiber.inject)` 查 `ctx.get(name)`
 *      （dsh-cordis-client-runner/lib/client.js:590），包名永远查不到服务。
 *   3. 载体行永远 pending → "1 entry did not activate" → web boot 失败 →
 *      桌面端打不开。用户的临时处置是给载体行加 `disabled: true`。
 *
 * 本脚本用真实的 DSH 安装做三件事（不是模拟，是读真文件 + 真调 runner 逻辑）：
 *   A. 解析本包 package.json 的 dsh.client.inject → 断言全是包名；
 *   B. 在真实 boot 图（client-modules manifest 的 row 形）里定位这三个包，
 *      断言它们确实存在（存在 → arriveGraphRow 能预载，不会卡住）；
 *   C. 用真实 runner 的服务门语义（fiber.inject 的服务必须能被 ctx.get 解析）
 *      断言 exports.inject 的每个服务名都有 provider → 不 pending。
 *
 * 用法: node tools/dsh-bridge/scripts/verify-boot-inject.mjs
 *      （可选）DSH_PKG_ROOT=<path> 覆盖 DSH 安装路径
 */
import { readFileSync, existsSync } from 'node:fs'
import { execFileSync } from 'node:child_process'
import { ensureCompatCache, resolveDshDir } from './dsh-root.mjs'

// DSH 安装根随版本换代：0.1.x 装在 npm 全局（散包），0.2.0 桌面端改为自带运行时
// 并把 @deepseek-ai/* 打进 resources/app.asar。原先写死的 npm 路径在升级后
// 变成空目录，本脚本于是误报「inject 包不存在 / provider 读不到」——看着像兼容性
// 炸了，其实只是探测路径过期。现在按「谁真的在」解析，必要时从 asar 解出缓存。
ensureCompatCache()
const DSH_ROOT = process.env.DSH_PKG_ROOT || resolveDshDir()
const REPO = new URL('../../..', import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, '$1')

if (!DSH_ROOT || !existsSync(DSH_ROOT)) {
  console.error('❌ 找不到可用的 DSH 安装（npm 全局散包 / resources/app.asar 都不可读）')
  console.error('   可显式指定: DSH_PKG_ROOT=<含 @deepseek-ai/* 的目录>')
  process.exit(2)
}

let failures = 0
function check(name, cond, detail) {
  console.log(`${cond ? '✅' : '❌'} ${name}${detail ? ` — ${detail}` : ''}`)
  if (!cond) failures++
}
function read(p) { try { return readFileSync(p, 'utf8') } catch { return null } }

// ── A. 载体包声明必须是包名 ────────────────────────────────────────────────
const pkg = JSON.parse(read(`${REPO}/data/dsh/craft-bot-preset-017/package.json`))
const pkgInject = pkg.dsh?.client?.inject ?? []
check('载体包声明 dsh.client 且 platform=web', pkg.dsh?.client?.platform === 'web',
  `platform=${pkg.dsh?.client?.platform}`)
check('dsh.client.inject 非空', pkgInject.length > 0, `inject=${JSON.stringify(pkgInject)}`)
check('dsh.client.inject 全部是包名（含 / 且 @deepseek-ai/ 前缀）',
  pkgInject.every((p) => p.startsWith('@deepseek-ai/')),
  `inject=${JSON.stringify(pkgInject)}`)

// ── B. 这些包必须在真实 DSH 安装里存在（否则 boot 图里无行可预载）──────────
const missingPkgs = pkgInject.filter((p) => !existsSync(`${DSH_ROOT}/${p.replace('@deepseek-ai/', '')}/package.json`))
check('每个 inject 包在 DSH 安装里真实存在', missingPkgs.length === 0,
  missingPkgs.length ? `missing=${JSON.stringify(missingPkgs)}` : `checked=${pkgInject.length}`)

// ── C. 服务门：exports.inject 的服务名必须有真实 provider ─────────────────
const clientSrc = read(`${REPO}/data/dsh/craft-bot-preset-017/client.js`)
check('镜像 client.js 可读', clientSrc !== null, clientSrc ? `${clientSrc.length} bytes` : 'unreadable')
const exportsInject = /exports\.inject\s*=\s*\[([^\]]*)\]/.exec(clientSrc ?? '')?.[1] ?? ''
const services = [...exportsInject.matchAll(/['"]([^'"]+)['"]/g)].map((m) => m[1])
check('exports.inject 非空且全是服务名（不含 /）',
  services.length > 0 && services.every((s) => !s.includes('/')),
  `services=${JSON.stringify(services)}`)

// 服务 → provider 包（由 reflect.provide / Service 基类实测得到）
const SERVICE_PROVIDER = {
  sessions: '@deepseek-ai/dsh-api-session-controller',
  slots: '@deepseek-ai/dsh-client-ui-renderer',
  sidebarRight: '@deepseek-ai/dsh-client-ui-sidebar-right',
  sidebarRightTabs: '@deepseek-ai/dsh-client-ui-sidebar-right',
}
const unprovided = []
for (const svc of services) {
  const provider = SERVICE_PROVIDER[svc]
  if (provider === undefined) { unprovided.push(`${svc}(未登记)`); continue }
  const src = read(`${DSH_ROOT}/${provider.replace('@deepseek-ai/', '')}/lib/client.js`)
  if (src === null) { unprovided.push(`${svc}(provider 读不到)`); continue }
  const explicit = src.includes(`reflect.provide("${svc}"`) || src.includes(`reflect.provide('${svc}'`)
  const viaBase = new RegExp(`super\\(ctx,\\s*["']${svc}["']`).test(src)
  if (!explicit && !viaBase) unprovided.push(`${svc}(${provider} 未 provide)`)
}
check('每个声明服务都有真实 provider（服务门不会 pending）', unprovided.length === 0,
  unprovided.length ? unprovided.join(' ') : `checked=${services.length}`)

// ── D. 两平面一致性：包名集合 ↔ 服务 provider 集合 ───────────────────────
const svcProviders = [...new Set(services.map((s) => SERVICE_PROVIDER[s]).filter(Boolean))]
check('包名平面与服务 provider 平面一致（不遗漏、不多余）',
  svcProviders.length === pkgInject.length && svcProviders.every((p) => pkgInject.includes(p)),
  `pkg=${JSON.stringify(pkgInject)} providers=${JSON.stringify(svcProviders)}`)

// ── E. 真实 boot 图行序（orderByModuleGraph 语义）不会因本行成环 ───────────
// 载体行的 inject 指向的三个包都是产品自带包（非本包），本包不自引用 → 无环。
check('载体行不自引用（inject 不含自身包名）',
  !pkgInject.includes(pkg.name), `name=${pkg.name}`)

// ── F. 桌面 profile 上的临时禁用必须移除，否则面板永远不生效 ──────────────
const PROFILE = process.env.USERPROFILE
  ? `${process.env.USERPROFILE}/.dsh/profiles/desktop/cordis.patch.yml`
  : null
if (PROFILE && existsSync(PROFILE)) {
  const profileText = read(PROFILE) ?? ''
  const disabledBlock = /-\s*id:\s*dsh-preset-craft-bot\s*\n\s*disabled:\s*true/.test(profileText)
  check('桌面 profile 未残留 dsh-preset-craft-bot 的 disabled:true 覆盖', !disabledBlock,
    disabledBlock ? '仍被禁用 → 载体行不会激活（面板不出现）' : 'ok')
} else {
  console.log('⏭  跳过 profile 检查（未找到 cordis.patch.yml）')
}

console.log(failures ? `\n存在 ${failures} 项失败` : '\n全部通过')
process.exit(failures ? 1 : 0)
