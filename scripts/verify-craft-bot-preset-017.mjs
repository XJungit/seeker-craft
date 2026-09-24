// 往返一致性：模板 → 生成物 是否可由生成器完全复现（byte-exact）
// 用法: node preset-017-roundtrip.mjs <repoRoot> <dshPkgRoot>
import { readFileSync, writeFileSync, mkdtempSync } from 'node:fs'
import { join } from 'node:path'
import { tmpdir } from 'node:os'
import { execFileSync } from 'node:child_process'

const [repoRoot, dshPkgRoot] = process.argv.slice(2)
const live = join(repoRoot, 'data', 'dsh', 'craft-bot-preset-017', 'cordis.patch.yml')
const tmp = mkdtempSync(join(tmpdir(), 'preset-rt-'))
const fresh = join(tmp, 'cordis.patch.yml')

execFileSync(process.execPath, [
  join(repoRoot, 'scripts', 'gen-craft-bot-preset-017.mjs'),
  '--project-root', repoRoot,
  '--dsh-pkg-root', dshPkgRoot,
  '--out', fresh,
], { stdio: 'pipe' })

const a = readFileSync(live, 'utf8')
const b = readFileSync(fresh, 'utf8')

if (a === b) {
  console.log(`✅ 往返一致（${a.split('\n').length} 行，byte-exact）`)
  process.exit(0)
}
const la = a.split('\n'), lb = b.split('\n')
console.log(`❌ 不一致：live=${la.length} 行, fresh=${lb.length} 行`)
let shown = 0
for (let i = 0; i < Math.max(la.length, lb.length) && shown < 10; i++) {
  if (la[i] !== lb[i]) {
    console.log(`  L${i + 1}:`)
    console.log(`    live : ${JSON.stringify(la[i])}`)
    console.log(`    fresh: ${JSON.stringify(lb[i])}`)
    shown++
  }
}
process.exit(1)
