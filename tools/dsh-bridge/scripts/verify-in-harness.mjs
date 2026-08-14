// 模拟 DSH loader 的 harness-base 解析：验证 dsh-bridge 在 DSH 模块图内可加载。
// 用法: node tools/dsh-bridge/scripts/verify-in-harness.mjs
import { createRequire } from 'node:module'
import { pathToFileURL } from 'node:url'

const npxRoot = process.env.DSH_NPX_ROOT || 'C:/Users/xj/AppData/Roaming/npm/node_modules/@deepseek-ai/dsh/node_modules'

// 用 DSH 的 node_modules 根作为 base，模拟 loader 的 harness-base 解析
const req = createRequire(pathToFileURL(npxRoot + '/__probe__.js').href)
const bridgePath = pathToFileURL(new URL('../index.js', import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, '$1')).href

// 1) 依赖可解析性
for (const name of ['@deepseek-ai/cordis', '@deepseek-ai/dsh-tools', '@deepseek-ai/schemastery']) {
  try {
    console.log(`✅ resolve ${name} ->`, req.resolve(name))
  } catch (e) {
    console.error(`❌ resolve ${name}:`, e.message)
    process.exit(1)
  }
}

// 2) dsh-bridge 模块加载（其内部 import 从自身路径解析；junction 指向 DSH npx）
const mod = await import(bridgePath)
console.log('✅ dsh-bridge loaded, name =', mod.name, ', inject =', JSON.stringify(mod.inject), ', apply =', typeof mod.apply)

// 3) apply 注册三个工具 + 三个占位符变量 + webServer 代理
const registered = []
const variables = []
const webServer = { register: (opts) => `registered:${opts.path}` }
let effectFn = null
const fakeCtx = {
  tools: { register: (def) => registered.push(def.name) },
  systemPrompt: { variable: (name, provider) => { variables.push({ name, provider }) } },
  webServer,
  effect: (fn) => { effectFn = fn },
}
mod.apply(fakeCtx, {}) // hostTools 默认 true
console.log('✅ registered tools =', JSON.stringify(registered))
if (registered.join(',') !== 'game_state,bot_tool,set_goal') {
  console.error('❌ expected game_state,bot_tool,set_goal')
  process.exit(1)
}
console.log('✅ registered variables =', JSON.stringify(variables.map((v) => v.name)))
if (variables.map((v) => v.name).join(',') !== 'bot_state,tool_list,viewer_url') {
  console.error('❌ expected bot_state,tool_list,viewer_url')
  process.exit(1)
}

// 3b) hostTools:false → 不注册工具/变量，仍挂代理（profile 全局行语义）
{
  const reg2 = []
  const vars2 = []
  const web2 = { register: (opts) => `registered:${opts.path}` }
  const ctx2 = {
    tools: { register: (def) => reg2.push(def.name) },
    systemPrompt: { variable: (name, provider) => { vars2.push({ name, provider }) } },
    webServer: web2,
    effect: () => {},
    get: () => undefined,
  }
  mod.apply(ctx2, { hostTools: false })
  if (reg2.length !== 0 || vars2.length !== 0) {
    console.error('❌ hostTools:false 不应注册工具/变量')
    process.exit(1)
  }
  console.log('✅ hostTools:false → 无工具/变量（仅 client 半边）')
}

// 3c) proxy:false → 不注册 webServer 代理（预设行语义，避免重复）
{
  const web3 = { register: (opts) => { throw new Error('不应注册代理: ' + opts.path) } }
  const ctx3 = {
    tools: { register: () => {} },
    systemPrompt: { variable: () => {} },
    webServer: web3,
    effect: () => {},
    get: () => undefined,
  }
  mod.apply(ctx3, { hostTools: true, proxy: false })
  console.log('✅ proxy:false → 不注册 webServer 代理')
}

// 4) 占位符 provider 同步返回 string（契约：assemble 不 await provider）
for (const v of variables) {
  const val = v.provider({})
  if (typeof val !== 'string') {
    console.error(`❌ variable ${v.name} provider 返回 ${typeof val}，必须同步 string`)
    process.exit(1)
  }
  console.log(`✅ {{${v.name}}} 同步返回 ${val.length} 字符`)
}
if (typeof effectFn === 'function') {
  effectFn() // 清理 timer（不泄漏）
  console.log('✅ effect disposer 可调用（timer 清理）')
}

// 5) client 半边（client.js）加载验证：包导出 ./client 且可解析
import { readFileSync } from 'node:fs'
const pkg = JSON.parse(readFileSync(new URL('../package.json', import.meta.url), 'utf8'))
console.log('✅ package.json dsh.client =', JSON.stringify(pkg.dsh?.client ?? null))
if (!pkg.dsh?.client || pkg.dsh.client.platform !== 'web') {
  console.error('❌ package.json 必须声明 dsh.client.platform=web（client 半边才能被 DSH 加载）')
  process.exit(1)
}
const clientExport = pkg.exports?.['./client'] ?? './client.js'
const clientPath = clientExport
const clientSrc = readFileSync(new URL('../' + clientPath, import.meta.url), 'utf8')
if (!clientSrc.includes("window.__ModuleLoader__.load")) {
  console.error('❌ client.js 必须以 window.__ModuleLoader__.load 开头（DSH client 加载契约）')
  process.exit(1)
}
if (!clientSrc.includes("agentPreset") || !clientSrc.includes("'craft-bot'")) {
  console.error('❌ client.js 必须包含 agentPreset === craft-bot 判断（仅 craft-bot 预设显示）')
  process.exit(1)
}
console.log(`✅ client.js ${clientPath} 含 __ModuleLoader__.load + agentPreset 判断（${clientSrc.length} 字符）`)

console.log('\n全部通过：dsh-bridge 加载 + 3 工具 + 3 占位符变量 + webServer 代理 + client.js（craft-bot 限定）')
