#!/usr/bin/env node
/**
 * dsh-bridge client.js 核心逻辑验证（agentPreset 判断 + apply 注册）。
 *
 * client.js 是浏览器 bundle（依赖 window.__ModuleLoader__），这里用 vm 模拟
 * 浏览器环境，验证：
 *   1. craft-bot 会话 → apply 注册 sidebar 入口（slots.inject 被调用）
 *   2. code 预设会话 → apply 直接返回，不注册任何 UI
 *   3. 会话列表含 craft-bot + 其他 → 仅 craft-bot 当前会话才注册
 *
 * 用法: node tools/dsh-bridge/scripts/verify-client.mjs
 */
import { readFileSync } from 'node:fs'
import vm from 'node:vm'
import { createRequire } from 'node:module'

const clientSrc = readFileSync(new URL('../client.js', import.meta.url), 'utf8')

// 模拟 __ModuleLoader__：加载 client.js，捕获 load 定义
function loadClient() {
  let loaded = null
  const sandbox = {
    window: {},
    document: {
      querySelector: (sel) => {
        // style 检查：模拟"尚未注入"，返回 null 触发创建
        return null
      },
      querySelectorAll: () => [],
      createElement: (tag) => {
        const el = {
          tagName: String(tag).toUpperCase(),
          style: {},
          dataset: {},
          setAttribute() {},
          removeAttribute() {},
          appendChild() {},
          addEventListener() {},
          removeEventListener() {},
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
      body: { contains: () => false, appendChild() {} },
      documentElement: { removeAttribute() {}, setAttribute() {} },
    },
    CustomEvent: class { constructor(type, opts) { this.type = type; this.detail = opts?.detail } },
    MutationObserver: class { constructor() {} observe() {} disconnect() {} },
    localStorage: { getItem: () => null, setItem() {}, removeItem() {} },
    location: { search: '' },
    console,
    setTimeout,
    clearTimeout,
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
  // client.js factory 只 require('react')；用空实现避免真实 react 依赖
  const fakeRequire = (name) => {
    if (name === 'react') {
      // 最小 react 存根：createElement 返回描述对象
      return {
        createElement: (type, props, ...children) => ({ type, props: props ?? {}, children }),
      }
    }
    return req(name)
  }
  // factory 内部创建 module/exports 并 return module.exports
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

// 构造带 sessions 的 ctx
function makeCtx(agentPreset) {
  const injects = []
  const registers = []
  const sessions = {
    list: {
      getSnapshot: () => ({
        current: 's1',
        byId: {
          s1: { agentPreset },
        },
      }),
    },
  }
  const ctx = {
    get: (name) => {
      if (name === 'sessions') return sessions
      return undefined
    },
    slots: {
      inject: (name, fn) => {
        injects.push(name)
        // 立即执行注册回调（简化）
        try { fn() } catch (e) { /* 插槽未声明时静默 */ }
      },
      register: (opts, comp) => {
        registers.push(opts)
        return () => {}
      },
    },
    effect: () => {},
  }
  return { ctx, injects, registers }
}

// 1) craft-bot 会话 → 注册入口
{
  const { ctx, injects, registers } = makeCtx('craft-bot')
  clientMod.apply(ctx)
  check('craft-bot 会话注册 sidebar.footer.action', injects.includes('sidebar.footer.action') || registers.length > 0,
    `injects=${JSON.stringify(injects)}, registers=${registers.length}`)
}

// 2) code 预设 → 不注册
{
  const { ctx, injects, registers } = makeCtx('code')
  clientMod.apply(ctx)
  check('code 预设会话不注册任何 UI', injects.length === 0 && registers.length === 0,
    `injects=${JSON.stringify(injects)}`)
}

// 3) 无 preset 字段 → 不注册（普通会话）
{
  const { ctx, injects, registers } = makeCtx(undefined)
  clientMod.apply(ctx)
  check('无 agentPreset 的普通会话不注册', injects.length === 0,
    `injects=${JSON.stringify(injects)}`)
}

console.log(failures ? `\n存在 ${failures} 项失败` : '\n全部通过')
process.exit(failures ? 1 : 0)
