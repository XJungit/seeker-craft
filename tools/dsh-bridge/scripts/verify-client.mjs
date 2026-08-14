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
          appendChild(c) { el.children.push(c); return c },
          addEventListener() {},
          removeEventListener() {},
          querySelector: () => genericEl(),
          querySelectorAll: () => [],
          closest: () => null,
          textContent: '',
          innerHTML: '',
          className: '',
          parentElement: null,
        }
        return el
      },
      head: { appendChild() {} },
      addEventListener() {},
      removeEventListener() {},
      dispatchEvent() {},
      body: {
        children: bodyChildren,
        contains: (el) => bodyChildren.includes(el),
        appendChild: (el) => { bodyChildren.push(el); return el },
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

// 构造带 sessions 的 ctx（getSnapshot + subscribe 都提供，贴近真实 ObservableSnapshot）
function makeCtx(agentPreset) {
  const subs = []
  const sessions = {
    list: {
      getSnapshot: () => ({
        current: 's1',
        byId: { s1: { agentPreset } },
      }),
      subscribe: (fn) => { subs.push(fn); return () => {} },
    },
  }
  const ctx = {
    sessions,
    get: (name) => (name === 'sessions' ? sessions : undefined),
    slots: {
      inject: () => {},
      register: () => () => {},
    },
    effect: () => {},
  }
  return { ctx, subs }
}

function countHosts() {
  return bodyChildren.filter((c) => c._attrs && Object.prototype.hasOwnProperty.call(c._attrs, 'data-dsh-craft-host')).length
}

// 1) craft-bot 会话 → 构建面板 host（DOM 单例，此时应有 1 个）
{
  const { ctx } = makeCtx('craft-bot')
  clientMod.apply(ctx)
  check('craft-bot 会话构建仪表盘 host', countHosts() === 1, `hosts=${countHosts()}`)
}

// 2) code 预设 → 复用同一 host，不再新建（不出现第二个仪表盘）
{
  const { ctx } = makeCtx('code')
  clientMod.apply(ctx)
  check('code 预设会话不新建仪表盘（仍只 1 个）', countHosts() === 1, `hosts=${countHosts()}`)
}

// 3) 无 preset 字段 → 同样复用，不新建
{
  const { ctx } = makeCtx(undefined)
  clientMod.apply(ctx)
  check('无 agentPreset 的普通会话不新建仪表盘', countHosts() === 1, `hosts=${countHosts()}`)
}

// 4) 全新模块实例 + 同一 craft-bot ctx 多次 apply（模拟多会话/模块重求值）→ 只存在一个 host
{
  const freshMod = factoryExports(def) // 独立闭包，模拟模块被重新求值
  const { ctx } = makeCtx('craft-bot')
  freshMod.apply(ctx)
  freshMod.apply(ctx)
  freshMod.apply(ctx)
  check('多次 apply（含模块重求值）只产生一个仪表盘元素（DOM 单例）', countHosts() === 1, `hosts=${countHosts()}`)
}

console.log(failures ? `\n存在 ${failures} 项失败` : '\n全部通过')
process.exit(failures ? 1 : 0)
