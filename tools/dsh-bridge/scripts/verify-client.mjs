#!/usr/bin/env node
/**
 * dsh-bridge client.js 核心逻辑验证（agentPreset 判断 + apply 注册）。
 *
 * client.js 是浏览器 bundle（依赖 window.__ModuleLoader__），这里用 vm 模拟
 * 浏览器环境，验证：
 *   1. craft-bot 会话 → apply 构建 body-portal 面板 host（DOM 单例）
 *   2. code 预设会话 → apply 复用同一 host，不再新建（不出现第二个仪表盘）
 *   3. 会话列表含 craft-bot + 其他 → 仅当前为 craft-bot 时才显示（其余隐藏）
 *   4. 全新模块实例 + 同一 craft-bot ctx 多次 apply（模拟多会话/模块重求值）
 *      → document 中依旧只存在一个 [data-dsh-craft-host]（DOM 单例，杜绝多仪表盘）
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
  return loaded
}

// 执行 factory 得到模块 exports（apply）
function factoryExports(def) {
  const req = createRequire(new URL('../__probe__.js', import.meta.url))
  const fakeRequire = (name) => {
    if (name === 'react') {
      return { createElement: (type, props, ...children) => ({ type, props: props ?? {}, children }) }
    }
    return req(name)
  }
  return def.factory(fakeRequire)
}

let failures = 0
function check(name, cond, detail) {
  console.log(`${cond ? '✅' : '❌'} ${name}${detail ? ` — ${detail}` : ''}`)
  if (!cond) failures++
}

const def = loadClient()
check('client.js 通过 __ModuleLoader__.load 注册', def !== null && typeof def.factory === 'function',
  def ? `id=${def.id}` : '')
const clientMod = factoryExports(def)
check('factory 导出 apply', typeof clientMod.apply === 'function', `apply=${typeof clientMod.apply}`)
check('factory 声明 inject=[sessions]', Array.isArray(clientMod.inject) && clientMod.inject.includes('sessions'),
  `inject=${JSON.stringify(clientMod.inject)}`)

// 构造带可变 sessions 的 ctx（getSnapshot + subscribe，贴近真实 ObservableSnapshot）。
// setPreset() 模拟“当前会话切换到另一 preset”，随后手动触发订阅监听器来驱动显隐
// ——真实 DSH 里 apply 只跑一次，显隐由 sessions 订阅驱动，而非重新 apply。
function makeCtx() {
  const subs = []
  let preset = 'craft-bot'
  const sessions = {
    list: {
      getSnapshot: () => ({
        current: 's1',
        byId: { s1: { agentPreset: preset } },
      }),
      subscribe: (fn) => { subs.push(fn); return () => {} },
    },
  }
  const ctx = {
    sessions,
    get: (name) => (name === 'sessions' ? sessions : undefined),
    effect: () => {},
  }
  return {
    ctx,
    subs,
    setPreset(p) { preset = p },
    fire() { for (const fn of subs) fn() },
  }
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

// 1) 首次 apply（craft-bot 会话）→ 构建唯一 host、面板可见、返回 disposer、注册订阅
//    （真实 DSH：一个 plugin 包 = 一个 entry = 一次 apply，见 web/src/boot.tsx）
const shared = makeCtx()
const disposer = clientMod.apply(shared.ctx)
check('apply 返回 disposer（cordis 契约）', typeof disposer === 'function', `type=${typeof disposer}`)
check('craft-bot 会话构建仪表盘 host', countHosts() === 1, `hosts=${countHosts()}`)
check('订阅已注册', shared.subs.length === 1, `subs=${shared.subs.length}`)
check('craft-bot 会话面板可见（未隐藏）', !panelHidden(), `hidden=${panelHidden()}`)

// 2) 会话切到 code 预设 → 触发订阅 → 面板隐藏（其他会话不显示），不新建仪表盘
shared.setPreset('code')
shared.fire()
check('切到 code 预设后面板隐藏（不显示）', panelHidden(), `hidden=${panelHidden()}`)
check('切到 code 预设后不新建仪表盘（仍只 1 个）', countHosts() === 1, `hosts=${countHosts()}`)

// 3) 会话切回 craft-bot → 触发订阅 → 面板自动重开
shared.setPreset('craft-bot')
shared.fire()
check('切回 craft-bot 后面板自动可见', !panelHidden(), `hidden=${panelHidden()}`)

// 4) 全新模块实例 + 多次 apply（模拟重复挂载/多 entry）→ DOM 单例守卫，仍只 1 个 host
{
  const freshMod = factoryExports(def) // 独立闭包
  const { ctx } = makeCtx()
  const d2 = freshMod.apply(ctx)
  freshMod.apply(ctx)
  freshMod.apply(ctx)
  check('重复 apply 只产生一个仪表盘元素（DOM 单例守卫）', countHosts() === 1, `hosts=${countHosts()}`)
  check('重复 apply 返回 no-op disposer', typeof d2 === 'function', `type=${typeof d2}`)
}

// 5) 调用首次 apply 的 disposer → 清理 DOM（host/样式移除）
disposer()
check('disposer 清理后 host 移除', countHosts() === 0, `hosts=${countHosts()}`)

console.log(failures ? `\n存在 ${failures} 项失败` : '\n全部通过')
process.exit(failures ? 1 : 0)
