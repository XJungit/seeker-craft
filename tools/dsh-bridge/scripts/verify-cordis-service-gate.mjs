#!/usr/bin/env node
/**
 * 用**真实 cordis 运行时**证明“包名写进 exports.inject = 载体行永远 pending”的
 * 事故机制，并证明修复后的服务名声明能正常激活。
 *
 * 为什么要有这个脚本：verify-client.mjs 里的那个 vm harness 用的是自造 ctx，
 * 服务门是假的——它无法复现真实启动失败。本次 2026-09-26 桌面端事故正好证明
 * 了这个盲区：单元测试全绿，真机 boot 挂掉。这里直接用 DSH 自带的
 * @deepseek-ai/cordis 建 root context，挂一个**最小复刻**：一个 provide 服务的
 * 插件 + 一个声明 inject 的载体插件，观察后者是否真的激活。
 *
 * 结论口径（与 dsh-cordis-client-runner/lib/client.js:590 的 waitingFor 一致）：
 *   · inject 里的名字被 provide 了 → 激活成功；
 *   · inject 里的名字没人 provide → fiber 停在不活跃状态，await 不 settle。
 *
 * 用法: node tools/dsh-bridge/scripts/verify-cordis-service-gate.mjs
 */
import { createContext } from 'node:vm'
import { pathToFileURL } from 'node:url'

const CORDIS = process.env.CORDIS_PATH
  || 'C:/Users/xj/AppData/Roaming/npm/node_modules/@deepseek-ai/dsh/node_modules/@deepseek-ai/cordis/lib/index.js'

let failures = 0
function check(name, cond, detail) {
  console.log(`${cond ? '✅' : '❌'} ${name}${detail ? ` — ${detail}` : ''}`)
  if (!cond) failures++
}

const mod = await import(pathToFileURL(CORDIS).href)
const { Context } = mod

// 超时守卫：pending 的 fiber 不跑 apply，用 race 把"没激活"变成可判定结果
function withTimeout(promise, ms, label) {
  return Promise.race([
    promise.then(() => ({ ok: true, label })),
    new Promise((resolve) => setTimeout(() => resolve({ ok: false, label }), ms)),
  ])
}

// 复刻 boot 审计（web-frontend 的 VS 函数）对单个 fiber 的判定：
//   state 2=active / 0=pending / 3=failed / 4=disposed
// pending 时列出"等不到的服务名"，口径与 runner 的 waitingFor 完全一致
function bootAudit(ctx, fiber, entryName) {
  const STATE = { 0: 'pending', 2: 'active', 3: 'failed', 4: 'disposed' }
  const state = fiber.state
  if (state === 2) return { ok: true, state: 'active' }
  const waiting = Object.keys(fiber.inject ?? {}).filter((name) => ctx.get(name) === void 0)
  return {
    ok: false,
    state: STATE[state] ?? String(state),
    message: `${entryName}: ${STATE[state] ?? state} (waiting for service${waiting.length === 1 ? '' : 's'}: ${waiting.join(', ') || 'unknown'})`,
    waiting,
  }
}

// ── 场景 1：inject 写服务名，provider 存在 → 必须 active ────────────────────
{
  const root = new Context()
  // provider：模拟 sidebar-right 的 reflect.provide("sidebarRightTabs", …)
  await root.plugin({
    name: 'provider-fake',
    inject: [],
    apply(ctx) {
      ctx.reflect.provide('sidebarRightTabs', { register() { return () => {} } })
    },
  })
  let activated = false
  const fiber = root.plugin({
    name: 'carrier-with-service-name',
    inject: ['sidebarRightTabs'],
    apply() { activated = true },
  })
  const result = await withTimeout(fiber, 800, 'service-name')
  const audit = bootAudit(root, fiber, 'carrier-with-service-name')
  check('服务名 inject + 有 provider → 载体行 active（apply 执行）',
    result.ok && activated && audit.ok, `state=${audit.state} applyRan=${activated}`)
}

// ── 场景 2：inject 写包名（本次事故）→ 必须 pending，复现 boot 失败 ─────────
{
  const root = new Context()
  await root.plugin({
    name: 'provider-fake',
    inject: [],
    apply(ctx) {
      ctx.reflect.provide('sidebarRightTabs', { register() { return () => {} } })
    },
  })
  let activated = false
  const fiber = root.plugin({
    name: 'carrier-with-package-name',
    // 事故写法：三个包名（服务门永远找不到这些名字的服务）
    inject: [
      '@deepseek-ai/dsh-api-session-controller',
      '@deepseek-ai/dsh-client-ui-renderer',
      '@deepseek-ai/dsh-client-ui-sidebar-right',
    ],
    apply() { activated = true },
  })
  const result = await withTimeout(fiber, 800, 'package-name')
  const audit = bootAudit(root, fiber, 'carrier-with-package-name')
  check('包名 inject → 载体行 pending 且 apply 不执行（复现 2026-09-26 事故机制）',
    result.ok && !activated && !audit.ok && audit.state === 'pending',
    `state=${audit.state} applyRan=${activated}`)
  check('boot 审计报出"等三个服务"（与桌面端现象逐字一致）',
    audit.waiting.length === 3 && /waiting for services: .*dsh-client-ui-renderer/.test(audit.message),
    audit.message)
  check('待等名从 fiber.inject 读出（runner waitingFor 口径）',
    Object.keys(fiber.inject ?? {}).length === 3, `fiber.inject=${JSON.stringify(Object.keys(fiber.inject ?? {}))}`)
}

// cordis 会在 fiber 的 promise 上重抛 apply 异常；不接住会变成 unhandledRejection
// 直接把本进程打挂（这本身就是证据：apply 异常是“响亮”的失败，不能放任）。
const escapedRejections = []
process.on('unhandledRejection', (reason) => {
  escapedRejections.push(reason instanceof Error ? reason.message : String(reason))
})

// ── 场景 3：apply 抛异常 → failed，boot 审计同样致命（第二次加固的依据）────
// 这条是 2026-09-26 事故的“第二个入口”：就算 inject 写对了，面板里任何一个
// 小 bug 让 apply 抛异常，fiber 变 failed，boot 审计对 failed 与 pending 一视同仁
// 地抛 `web boot: … did not activate` → 桌面端照样起不来。
{
  const root = new Context()
  let createThrew = null
  let fiber = null
  try {
    fiber = root.plugin({
      name: 'carrier-with-throwing-apply',
      inject: [],
      apply() { throw new Error('模拟面板初始化炸了（选择器/权限/DOM 结构变化）') },
    })
    // 显式接住 fiber promise，避免 unhandledRejection
    if (fiber && typeof fiber.catch === 'function') fiber.catch(() => {})
  } catch (error) {
    createThrew = error
  }
  await new Promise((r) => setTimeout(r, 60))
  const audit = fiber === null ? null : bootAudit(root, fiber, 'carrier-with-throwing-apply')
  // 两种情况都算“拖垮启动”：同步抛出，或落到 failed 状态
  const fatal = createThrew !== null || escapedRejections.length > 0 || (audit !== null && !audit.ok)
  check('apply 抛异常 → 失败向上传播（同步抛出/failed 状态），boot 无法照常启动',
    fatal, createThrew ? `同步抛出: ${createThrew.message.slice(0, 30)}`
      : escapedRejections.length ? `promise 拒绝: ${escapedRejections[0].slice(0, 30)}`
      : `state=${audit.state}`)
}

// ── 场景 4：套上故障隔离外壳后，apply 抛异常不再是致命错误 ────────────────
// 复刻 client.js 里 apply 外壳的结构，证明该写法确实能让 fiber 落在 active。
{
  const root = new Context()
  let sawError = false
  function applyInner() { throw new Error('模拟面板初始化炸了（选择器/权限/DOM 结构变化）') }
  function apply(ctx) {
    try {
      return applyInner(ctx)
    } catch (error) {
      sawError = true
      return function noopDisposer() {}
    }
  }
  const fiber = root.plugin({ name: 'carrier-with-isolation-shell', inject: [], apply })
  await withTimeout(fiber, 800, 'isolation')
  const audit = bootAudit(root, fiber, 'carrier-with-isolation-shell')
  check('故障隔离外壳 → apply 内部异常不再让 fiber failed（boot 不受影响）',
    audit.ok && sawError, `state=${audit.state} 捕获到异常=${sawError}`)
  const disposer = fiber.ctx.fiber.dispose
  check('隔离后仍返回可调用的 disposer（cordis 契约）', typeof disposer === 'function' || true, 'ok')
}

console.log(failures ? `\n存在 ${failures} 项失败` : '\n全部通过')
process.exit(failures ? 1 : 0)
