#!/usr/bin/env node
/**
 * dsh-bridge client.js 核心逻辑验证（主会话判定 + 双模式 + apply 注册）。
 *
 * client.js 是浏览器 bundle（依赖 window.__ModuleLoader__），这里用 vm 模拟
 * 浏览器环境，验证：
 *   1. overlay 模式 + craft-bot 主会话（retainedBy.mainView>0）→ apply 构建
 *      body-portal 面板 host（DOM 单例）
 *   2. 快照无 current 字段时仍能判定（回归：旧代码读 snap.current 永远 undefined）
 *   3. 主会话切到 code 预设 → 面板隐藏，不新建仪表盘
 *   4. 切回 craft-bot 主会话 → 面板按 userOpened 显隐（订阅驱动）
 *   5. sidebar/body slot 注册：wantSidebar 时 tabs.register + keyed body/title
 *      注册（ctx.effect 直跑），kind='craft-bot'，key=TYPE_ID
 *   6. 进 craft 主会话 → sidebarRight.openTab('craft-bot') 被调用（幂等打开）
 *   7. 非 craft 主会话 → openTab 不被调用（其他预设不受影响）
 *   8. 全新模块实例 + 多次 apply → DOM 单例守卫，仍只 1 个 host
 *   9. disposer 清理 DOM（host/样式移除）
 *
 * 快照形状贴近真实 SessionListState（service.d.ts:43-52）：
 *   { ids, byId: { id: { id, retainedBy: { mainView }, agentPreset/projectionValues } },
 *     phase, projectionsBySession } —— 注意：没有 current 字段。
 *
 * 用法: node tools/dsh-bridge/scripts/verify-client.mjs
 */
import { readFileSync } from 'node:fs'
import vm from 'node:vm'
import { createRequire } from 'node:module'

const clientSrc = readFileSync(new URL('../client.js', import.meta.url), 'utf8')

// 跨 apply 共享的 body 子节点表（按 _attrs 追踪仪表盘 host，用于断言“只存在一个仪表盘”）
const bodyChildren = []

// 通用元素桩（供 element.querySelector 返回，避免 .addEventListener 报错）
function genericEl() {
  return {
    addEventListener() {},
    removeEventListener() {},
    setAttribute() {},
    removeAttribute() {},
    querySelector: () => genericEl(),
    querySelectorAll: () => [],
    textContent: '',
    src: '',
  }
}

// 模拟 __ModuleLoader__：加载 client.js，捕获 load 定义
function loadClient() {
  let loaded = null
  bodyChildren.length = 0
  const sandbox = {
    window: {},
    document: {
      querySelector: (sel) => {
        // 仅支持 [attr] 形式的存在性选择器（仪表盘单例判断 / 样式注入判断）
        const m = /\[([\w-]+)(?:=["'][^"']*["'])?\]/.exec(sel || '')
        if (m) {
          const attr = m[1]
          return bodyChildren.find((c) => c._attrs && Object.prototype.hasOwnProperty.call(c._attrs, attr)) || null
        }
        return null
      },
      querySelectorAll: () => [],
      getElementById: (id) => bodyChildren.find((c) => c._attrs && c._attrs.id === id) || null,
      createElement: (tag) => {
        const el = {
          tagName: String(tag).toUpperCase(),
          id: '',
          _attrs: {},
          style: {},
          dataset: {},
          children: [],
          setAttribute(k, v) { this._attrs[k] = v },
          getAttribute(k) { return this._attrs[k] },
          hasAttribute(k) { return Object.prototype.hasOwnProperty.call(this._attrs, k) },
          removeAttribute(k) { delete this._attrs[k] },
          appendChild(c) { el.children.push(c); c.parentElement = el; return c },
          removeChild(c) {
            const i = el.children.indexOf(c)
            if (i >= 0) el.children.splice(i, 1)
            if (c.parentElement === el) c.parentElement = null
            return c
          },
          addEventListener() {},
          removeEventListener() {},
          querySelector: (sel) => {
            // 在自身 children 里按 .class / [attr] / 标签名查找真实子节点；找不到给 genericEl
            const cls = /\.([\w-]+)/.exec(sel || '')
            if (cls) {
              const found = el.children.find((c) => c.className && String(c.className).split(' ').includes(cls[1]))
              if (found) return found
            }
            const attr = /\[([\w-]+)\]/.exec(sel || '')
            if (attr) {
              const found = el.children.find((c) => c._attrs && Object.prototype.hasOwnProperty.call(c._attrs, attr[1]))
              if (found) return found
            }
            if (sel === 'iframe') return el.children.find((c) => c.tagName === 'IFRAME') || genericEl()
            return genericEl()
          },
          querySelectorAll: () => [],
          closest: () => null,
          textContent: '',
          innerHTML: '',
          className: '',
          parentElement: null,
        }
        return el
      },
      head: {
        appendChild() {},
        removeChild() {},
      },
      addEventListener() {},
      removeEventListener() {},
      dispatchEvent() {},
      body: {
        children: bodyChildren,
        contains: (el) => bodyChildren.includes(el),
        appendChild: (el) => { bodyChildren.push(el); el.parentElement = { removeChild(c) { bodyChildren.splice(bodyChildren.indexOf(c), 1) } }; return el },
        removeChild: (c) => {
          const i = bodyChildren.indexOf(c)
          if (i >= 0) bodyChildren.splice(i, 1)
          return c
        },
      },
      documentElement: { removeAttribute() {}, setAttribute() {} },
    },
    CustomEvent: class { constructor(type, opts) { this.type = type; this.detail = opts?.detail } },
    MutationObserver: class { constructor() {} observe() {} disconnect() {} },
    localStorage: { getItem: () => null, setItem() {}, removeItem() {} },
    location: { search: '' },
    console,
    setTimeout,
    clearTimeout,
    setInterval,
    clearInterval,
  }
  sandbox.window.__ModuleLoader__ = {
    load: (def) => { loaded = def },
  }
  vm.createContext(sandbox)
  vm.runInContext(clientSrc, sandbox)
  return { def: loaded, sandbox }
}

// 执行 factory 得到模块 exports（apply）。localStorage 内容可定制（placement 开关）。
function factoryExports(def, sandbox, storage = {}) {
  const req = createRequire(new URL('../__probe__.js', import.meta.url))
  const fakeRequire = (name) => {
    if (name === 'react') {
      return {
        createElement: (type, props, ...children) => {
          // 函数组件：延迟求值——调用方显式 render 时才执行，避免 hooks 污染断言
          if (typeof type === 'function') return { type, props: props ?? {}, children, __lazy: true }
          return { type, props: props ?? {}, children }
        },
      }
    }
    return req(name)
  }
  const localStore = { ...storage }
  sandbox.localStorage = {
    getItem: (k) => (k in localStore ? localStore[k] : null),
    setItem: (k, v) => { localStore[k] = String(v) },
    removeItem: (k) => { delete localStore[k] },
  }
  // window 与全局同对象：client.js 用 `var W = window` 缓存指纹/状态
  sandbox.window.__dshCraftBuild = undefined
  sandbox.window.__dshCraftUserOpened = undefined
  sandbox.window.__dshCraftIsCraft = undefined
  sandbox.window.__dshCraftDbg = undefined
  sandbox.window.localStorage = sandbox.localStorage
  const factoryWindow = sandbox.window
  const globals = { window: factoryWindow, localStorage: sandbox.localStorage, document: sandbox.document }
  const names = Object.keys(globals)
  const vals = names.map((k) => globals[k])
  const runFactory = new Function(...names, `return (${def.factory.toString()})(require)`)
  void runFactory
  // 更简单：直接用 vm 在 sandbox 上下文里重跑 factory 调用
  sandbox.__fakeRequire = fakeRequire
  sandbox.__factoryDef = def
  vm.runInContext(`__factoryResult = __factoryDef.factory(__fakeRequire)`, sandbox)
  return sandbox.__factoryResult
}

// 求值一个 lazy 函数组件（CraftSidebarBody），返回其渲染的 element 描述
function renderLazy(el, sandbox) {
  sandbox.__lazyEl = el
  vm.runInContext(`__rendered = __lazyEl.type(__lazyEl.props)`, sandbox)
  return sandbox.__rendered
}

let failures = 0
function check(name, cond, detail) {
  console.log(`${cond ? '✅' : '❌'} ${name}${detail ? ` — ${detail}` : ''}`)
  if (!cond) failures++
}

const { def, sandbox } = loadClient()
check('client.js 通过 __ModuleLoader__.load 注册', def !== null && typeof def.factory === 'function',
  def ? `id=${def.id}` : '')
// exports.inject 条目必须是 **ctx 服务名**（fiber 激活门：runner 按
// Object.keys(fiber.inject) 查 ctx.get(name)，查不到永远 pending）。
// 包名属于 package.json 的 dsh.client.inject（boot 图到达顺序）——两个平面写反
// 会让 “1 entry did not activate”，整个 web boot 失败（2026-09-26 桌面端事故）。
const EXPECT_INJECT = ['sessions', 'slots', 'sidebarRight', 'sidebarRightTabs']

// ── ctx 构造：贴近真实 SessionListState（无 current，主会话经 mainView）────
// rows: { [id]: { agentPreset|projectionValues, mainView:bool } }
// fire() 触发 sessions 订阅；effect 回调直跑（同步注册，返回 disposer 收集）。
function makeCtx(rows, storage = {}) {
  const subs = []
  const effDisposers = []
  const tabsRegs = []
  const slotRegs = []
  const opened = []
  const byId = {}
  const ids = Object.keys(rows)
  for (const id of ids) {
    const r = rows[id]
    byId[id] = {
      id,
      retainedBy: r.mainView ? { mainView: 1 } : {},
      ...(r.preset !== undefined ? { agentPreset: r.preset } : {}),
      ...(r.pvPreset !== undefined ? { projectionValues: { agentPreset: r.pvPreset } } : {}),
    }
  }
  const sessions = {
    list: {
      getSnapshot: () => ({ ids, byId, phase: 'ready', projectionsBySession: {} }),
      subscribe: (fn) => { subs.push(fn); return () => {} },
    },
  }
  const slots = {
    inject: (ownerKey, fn) => { const d = fn(); effDisposers.push(d); return () => {} },
    register: (opts, comp) => { slotRegs.push({ ...opts, comp }); return () => {} },
  }
  const sidebarRightTabs = {
    register: (def) => { tabsRegs.push(def); return () => {} },
  }
  const sidebarRight = {
    openTab: (kind, opts) => { opened.push({ kind, opts }) },
  }
  const ctx = {
    sessions,
    slots,
    sidebarRight,
    sidebarRightTabs,
    get: (name) => ({ sessions, slots, sidebarRight, sidebarRightTabs }[name]),
    effect: (fn) => { const d = fn(); effDisposers.push(d); return () => {} },
  }
  return {
    ctx, subs, tabsRegs, slotRegs, opened, effDisposers,
    fire() { for (const fn of subs) fn() },
    sandbox,
  }
}

// placement=both：overlay + sidebar 双开
const BOTH = { 'dsh-bridge.placement': 'both' }

// ── 两个 inject 平面的“真机对照”检查（2026-09-26 boot 崩溃的回归测试）──────
// 崩溃根因：exports.inject 写了包名，服务门按 ctx.get(name) 永远查不到 → pending
// →“1 entry did not activate”→ web boot 失败。这里真的去 DSH 安装目录里读
// provider，逐一确认声明的服务确实有人 provide、声明的包确实是 graph 行。
const DSH_ROOT = process.env.DSH_PKG_ROOT || 'C:/Users/xj/AppData/Roaming/npm/node_modules/@deepseek-ai/dsh/node_modules/@deepseek-ai'
// 服务名 → 提供它的包（由 ctx.reflect.provide(...) 实测得出）
const SERVICE_PROVIDER = {
  sessions: '@deepseek-ai/dsh-api-session-controller',
  slots: '@deepseek-ai/dsh-client-ui-renderer',
  sidebarRight: '@deepseek-ai/dsh-client-ui-sidebar-right',
  sidebarRightTabs: '@deepseek-ai/dsh-client-ui-sidebar-right',
}

function countHosts() {
  return bodyChildren.filter((c) => c._attrs && Object.prototype.hasOwnProperty.call(c._attrs, 'data-dsh-craft-host')).length
}

function findHost() {
  return bodyChildren.find((c) => c._attrs && Object.prototype.hasOwnProperty.call(c._attrs, 'data-dsh-craft-host')) || null
}
function findPanel() {
  const host = findHost()
  return host ? host.querySelector('.dsh-craft-panel') : null
}
// 面板是否处于“隐藏”态（data-hidden 属性存在）
function panelHidden() {
  const p = findPanel()
  return p ? p.hasAttribute('data-hidden') : true
}

// 1) overlay + craft-bot 主会话（无 current 字段，mainView>0）→ host 唯一 + 订阅注册
const clientMod = factoryExports(def, sandbox, BOTH)
check('factory 导出 apply', typeof clientMod.apply === 'function', `apply=${typeof clientMod.apply}`)
check('factory 声明 inject 为 ctx 服务名（sessions/slots/sidebarRight/sidebarRightTabs）',
  Array.isArray(clientMod.inject) && EXPECT_INJECT.every((s) => clientMod.inject.includes(s)),
  `inject=${JSON.stringify(clientMod.inject)}`)
// 反向断言：绝不能把包名写进 exports.inject（2026-09-26 桌面端 boot 崩溃的根因）
check('exports.inject 不含任何包名（服务门不认包名，会导致整页 boot 失败）',
  Array.isArray(clientMod.inject) && clientMod.inject.every((s) => !s.includes('/')),
  `inject=${JSON.stringify(clientMod.inject)}`)

// ── 真机对照 1：每个声明的服务，确实有包在 provide 它 ─────────────────────
// 这是本次事故的直接回归测试：声明了没人提供的服务 = 载体行永远 pending。
// 两种 provide 机制都要认：
//   · 显式 ctx.reflect.provide("sidebarRight", …)（sidebar-right 就是这么做的）；
//   · cordis Service 基类 super(ctx, "slots")（lib/index.js:1782 内部调
//     reflect.provide，ui-renderer 的 SlotService 用这种）。
{
  const missing = []
  for (const svc of (clientMod.inject || [])) {
    const provider = SERVICE_PROVIDER[svc]
    if (provider === undefined) { missing.push(`${svc}(未登记 provider)`); continue }
    let src = ''
    try { src = readFileSync(`${DSH_ROOT}/${provider.replace('@deepseek-ai/', '')}/lib/client.js`, 'utf8') } catch { missing.push(`${svc}(${provider} 读不到)`); continue }
    const explicit = src.includes(`reflect.provide("${svc}"`) || src.includes(`reflect.provide('${svc}'`)
    const viaBaseClass = new RegExp(`super\\(ctx,\\s*["']${svc}["']`).test(src)
    if (!explicit && !viaBaseClass) missing.push(`${svc}(${provider} 未 provide)`)
  }
  check('声明的每个服务都有真实 provider（reflect.provide / Service 基类实测）',
    missing.length === 0, missing.join(' ') || `checked=${(clientMod.inject || []).length}`)
}

// ── 真机对照 2：package.json 的 dsh.client.inject 是包名，且都在 boot 图里 ──
{
  const pkgJsonPath = new URL('../../../data/dsh/craft-bot-preset-017/package.json', import.meta.url)
  const pkg = JSON.parse(readFileSync(pkgJsonPath, 'utf8'))
  const pkgInject = pkg.dsh?.client?.inject ?? []
  const namesOk = pkgInject.every((p) => p.startsWith('@deepseek-ai/'))
  const exist = pkgInject.filter((p) => {
    try { readFileSync(`${DSH_ROOT}/${p.replace('@deepseek-ai/', '')}/package.json`, 'utf8'); return true } catch { return false }
  })
  check('package.json dsh.client.inject 为包名且真实存在',
    namesOk && exist.length === pkgInject.length,
    `inject=${JSON.stringify(pkgInject)}`)
  // 每个包名必须对应一个本插件真正用到的服务（否则是无关依赖）
  const covered = new Set(pkgInject.map((p) => SERVICE_PROVIDER[Object.keys(SERVICE_PROVIDER).find((s) => SERVICE_PROVIDER[s] === p)]).filter(Boolean))
  void covered
  check('package.json 包名与 exports.inject 服务名一一对应（两平面一致）',
    new Set(pkgInject).size === new Set(Object.values(SERVICE_PROVIDER)).size &&
    new Set(pkgInject).size === pkgInject.length,
    `pkg=${JSON.stringify(pkgInject)} svcProviders=${JSON.stringify([...new Set(Object.values(SERVICE_PROVIDER))])}`)
}

const shared = makeCtx({ s1: { preset: 'craft-bot', mainView: true } }, BOTH)
const disposer = clientMod.apply(shared.ctx)
check('apply 返回 disposer（cordis 契约）', typeof disposer === 'function', `type=${typeof disposer}`)
check('craft-bot 主会话构建 overlay host（快照无 current 也能判定）', countHosts() === 1, `hosts=${countHosts()}`)
check('订阅已注册', shared.subs.length === 1, `subs=${shared.subs.length}`)
check('sidebar tab 类型已注册（kind=craft-bot）',
  shared.tabsRegs.length === 1 && shared.tabsRegs[0].kind === 'craft-bot',
  `regs=${JSON.stringify(shared.tabsRegs.map((r) => [r.id, r.kind]))}`)
check('sidebar body/title 走 keyed slot（key=TYPE_ID）',
  shared.slotRegs.some((r) => r.name === 'sidebar.right.pane.tab' && r.key === 'dsh-preset-craft-bot/craft-bot') &&
  shared.slotRegs.some((r) => r.name === 'sidebar.right.pane.tab.title' && r.key === 'dsh-preset-craft-bot/craft-bot'),
  `slots=${JSON.stringify(shared.slotRegs.map((r) => [r.name, r.key]))}`)
check('进 craft 主会话触发 openTab（幂等打开）',
  shared.opened.length === 1 && shared.opened[0].kind === 'craft-bot', `opened=${JSON.stringify(shared.opened)}`)

// 2) 主会话切到 ptc 预设（真·非 craft；注意 code 在 PRESET_IDS 名单里，仍算自己人）
//    → overlay 隐藏 + openTab 不再调用，不新建 host
shared.opened.length = 0
shared.ctx.sessions.list.getSnapshot = () => ({
  ids: ['s1'],
  byId: { s1: { id: 's1', retainedBy: { mainView: 1 }, agentPreset: 'ptc' } },
  phase: 'ready', projectionsBySession: {},
})
shared.fire()
check('切到 ptc 预设后面板隐藏（不显示）', panelHidden(), `hidden=${panelHidden()}`)
check('切到 ptc 预设后不新建仪表盘（仍只 1 个）', countHosts() === 1, `hosts=${countHosts()}`)
check('非 craft 主会话不调用 openTab（其他预设不受影响）', shared.opened.length === 0, `opened=${shared.opened.length}`)

// 3) 主会话切回 craft-bot（agentPreset 藏 projectionValues，顶层无）→ 兜底判定命中 + openTab
shared.opened.length = 0
shared.ctx.sessions.list.getSnapshot = () => ({
  ids: ['s1'],
  byId: { s1: { id: 's1', retainedBy: { mainView: 1 }, projectionValues: { agentPreset: 'craft-bot' } } },
  phase: 'ready', projectionsBySession: {},
})
shared.fire()
check('projectionValues 兜底：切回 craft-bot 后 openTab 恢复', shared.opened.length === 1, `opened=${shared.opened.length}`)

// 4) sidebar body 组件：craft 主会话渲染 iframe；非 craft 渲染提示行
{
  const bodyReg = shared.slotRegs.find((r) => r.name === 'sidebar.right.pane.tab')
  const Body = bodyReg.comp
  const craftEl = renderLazy(
    { type: Body, props: { useSessions: (sel) => sel({ ids: ['s1'], byId: { s1: { retainedBy: { mainView: 1 }, agentPreset: 'craft-bot' } } }) } },
    sandbox,
  )
  check('sidebar body 在 craft 会话渲染 viewer iframe',
    craftEl && craftEl.type === 'iframe' && /compact=1/.test(craftEl.props.src || ''),
    `type=${craftEl && craftEl.type} src=${craftEl && craftEl.props.src}`)
  const otherEl = renderLazy(
    { type: Body, props: { useSessions: (sel) => sel({ ids: ['s9'], byId: { s9: { retainedBy: { mainView: 1 }, agentPreset: 'ptc' } } }) } },
    sandbox,
  )
  check('sidebar body 在非 craft 会话渲染提示行（非 iframe）',
    otherEl && otherEl.type === 'div', `type=${otherEl && otherEl.type}`)
}

// 5) 全新模块实例 + 多次 apply（模拟重复挂载/多 entry）→ DOM 单例守卫，仍只 1 个 host
{
  const freshMod = factoryExports(def, sandbox, BOTH) // 独立闭包
  const again = makeCtx({ s1: { preset: 'craft-bot', mainView: true } }, BOTH)
  const d2 = freshMod.apply(again.ctx)
  freshMod.apply(again.ctx)
  freshMod.apply(again.ctx)
  check('重复 apply 只产生一个仪表盘元素（DOM 单例守卫）', countHosts() === 1, `hosts=${countHosts()}`)
  check('重复 apply 返回 no-op disposer', typeof d2 === 'function', `type=${typeof d2}`)
}

// 6) 调用首次 apply 的 disposer → 清理 DOM（host/样式移除）
disposer()
check('disposer 清理后 host 移除', countHosts() === 0, `hosts=${countHosts()}`)

console.log(failures ? `\n存在 ${failures} 项失败` : '\n全部通过')
process.exit(failures ? 1 : 0)
