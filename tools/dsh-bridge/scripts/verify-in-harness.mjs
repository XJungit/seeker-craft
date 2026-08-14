// 模拟 DSH loader 的 harness-base 解析：验证 dsh-bridge 在 DSH 模块图内可加载。
// 用法: node tools/dsh-bridge/scripts/verify-in-harness.mjs
import { createRequire } from 'node:module'
import { pathToFileURL } from 'node:url'

const npxRoot = process.env.DSH_NPX_ROOT || 'C:/Users/xj/AppData/Local/npm-cache/_npx/1e7f6d9597241db0/node_modules'

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

// 3) apply 注册三个工具 + 三个占位符变量
const registered = []
const variables = []
let effectFn = null
const fakeCtx = {
  tools: { register: (def) => registered.push(def.name) },
  systemPrompt: { variable: (name, provider) => { variables.push({ name, provider }) } },
  effect: (fn) => { effectFn = fn },
}
mod.apply(fakeCtx)
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

console.log('\n全部通过：dsh-bridge 加载 + 3 工具 + 3 占位符变量（同步 provider）')
