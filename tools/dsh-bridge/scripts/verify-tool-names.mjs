// 纯工具清单比对：index.js::TOOL_NAMES vs tools_azalea.rs::ALL_TOOL_NAMES。
// 不依赖 DSH node_modules（与 verify-in-harness.mjs 不同，本脚本零依赖、秒级、任何环境可跑）。
// 这是「新增能力纪律」第 5 条（文档同步点）的自动化防线：
//   Rust 侧增删工具（ALL_TOOL_NAMES 权威清单）时，dsh-bridge 的 TOOL_NAMES 静态镜像必须同步。
//
// 用法: node tools/dsh-bridge/scripts/verify-tool-names.mjs
// 退出码: 0=一致（绿），1=失配（红）
import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { dirname, join } from 'node:path'

const here = dirname(fileURLToPath(import.meta.url))
const bridgeRoot = join(here, '..')
const repoRoot = join(bridgeRoot, '..', '..')

// 从 index.js 提取 TOOL_NAMES 数组（const TOOL_NAMES = [...]）
const indexSrc = readFileSync(join(bridgeRoot, 'index.js'), 'utf8')
const toolNamesMatch = indexSrc.match(/const TOOL_NAMES = \[([\s\S]*?)\]/)
if (!toolNamesMatch) {
  console.error('❌ 无法从 index.js 提取 TOOL_NAMES 数组')
  process.exit(1)
}
const bridgeTools = [...toolNamesMatch[1].matchAll(/'([^']+)'/g)].map((m) => m[1])

// 从 tools_azalea.rs 提取 ALL_TOOL_NAMES 数组（pub const ALL_TOOL_NAMES: &[&str] = &[...]）
const rsSrc = readFileSync(join(repoRoot, 'crates', 'craft-agent-minecraft', 'src', 'tools_azalea.rs'), 'utf8')
const allNamesMatch = rsSrc.match(/pub const ALL_TOOL_NAMES: &\[&str\] = &\[([\s\S]*?)\];/)
if (!allNamesMatch) {
  console.error('❌ 无法从 tools_azalea.rs 提取 ALL_TOOL_NAMES 数组')
  process.exit(1)
}
const rustTools = [...allNamesMatch[1].matchAll(/"([^"]+)"/g)].map((m) => m[1])

const missing = rustTools.filter((t) => !bridgeTools.includes(t))
const extra = bridgeTools.filter((t) => !rustTools.includes(t))
const dupes = bridgeTools.filter((t, i) => bridgeTools.indexOf(t) !== i)

console.log(`🔧 bridge TOOL_NAMES: ${bridgeTools.length} 个`)
console.log(`🔧 rust  ALL_TOOL_NAMES: ${rustTools.length} 个`)

if (missing.length || extra.length || dupes.length) {
  console.error(`❌ 工具清单失配！rust=${rustTools.length} bridge=${bridgeTools.length}`)
  if (missing.length) console.error(`   Rust 有但 bridge 缺: ${missing.join(', ')}`)
  if (extra.length) console.error(`   bridge 有但 Rust 无: ${extra.join(', ')}`)
  if (dupes.length) console.error(`   bridge 重复: ${dupes.join(', ')}`)
  console.error('   请按「新增能力纪律」同步 tools/dsh-bridge/index.js::TOOL_NAMES')
  process.exit(1)
}
console.log(`✅ 工具清单比对一致：${rustTools.length} 个（rust ALL_TOOL_NAMES == bridge TOOL_NAMES）`)
process.exit(0)
