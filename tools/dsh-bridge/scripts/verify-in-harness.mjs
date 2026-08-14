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

// 3) apply 注册三个工具
const registered = []
const fakeCtx = { tools: { register: (def) => registered.push(def.name) } }
mod.apply(fakeCtx)
console.log('✅ registered tools =', JSON.stringify(registered))
if (registered.join(',') !== 'game_state,bot_tool,set_goal') {
  console.error('❌ expected game_state,bot_tool,set_goal')
  process.exit(1)
}
console.log('\n全部通过：dsh-bridge 在 DSH 模块图内可加载且注册 3 工具')
