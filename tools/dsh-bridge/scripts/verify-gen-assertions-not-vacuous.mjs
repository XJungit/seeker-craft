#!/usr/bin/env node
/**
 * 变异测试：证明 gen-craft-bot-preset-017.mjs 里新增的「apply 故障隔离外壳」
 * 三条自检**真的会响**（不是永远为真的空断言）。
 *
 * 做法：读真实 client.js 源文件 → 用同样的正则跑一遍（必须 0 问题）→
 * 人为破坏（去掉 try/catch、去掉 applyInner、去掉 noopDisposer）→
 * 断言对应自检确实报出问题。若破坏后仍 0 问题，说明断言是空转的，这里会失败。
 *
 * 用法: node tools/dsh-bridge/scripts/verify-gen-assertions-not-vacuous.mjs
 */
import { readFileSync } from 'node:fs'
import { join } from 'node:path'

let failures = 0
function check(name, cond, detail) {
  console.log(`${cond ? '✅' : '❌'} ${name}${detail ? ` — ${detail}` : ''}`)
  if (!cond) failures++
}

const SRC = join(process.cwd(), 'tools', 'dsh-bridge', 'client.js')

// 与生成器自检同口径的断言（保持同步；生成器那边改了这三处正则，这里也要改）。
// 关键：**切片到 apply 外壳段内**再断言，否则 noopDisposer 在 DOM 单例守卫里也出现
// 一次，断言会退化成永远为真——本脚本的变异 C 就是专门抓这个空转的。
function auditIsolationShell(text) {
  const problems = []
  if (!/function applyInner\(ctx\)/.test(text)) problems.push('缺少 applyInner')
  const shellStart = text.indexOf('function apply(ctx) {')
  const shellEnd = shellStart >= 0 ? text.indexOf('exports.name', shellStart) : -1
  const shell = shellStart >= 0 && shellEnd > shellStart ? text.slice(shellStart, shellEnd) : ''
  if (shell === '') {
    problems.push('找不到 apply(ctx) 外壳段')
  } else {
    if (!/return\s+applyInner\(ctx\)/.test(shell)) problems.push('外壳未调用 applyInner')
    if (!/try\s*\{[\s\S]*?return\s+applyInner\(ctx\)[\s\S]*?\}\s*catch/.test(shell)) {
      problems.push('外壳未把 applyInner 包进 try/catch')
    }
    if (!/catch\s*\(\s*error\s*\)[\s\S]*?return\s+function/.test(shell)) {
      problems.push('catch 分支未返回 disposer 函数')
    }
  }
  return problems
}

const original = readFileSync(SRC, 'utf8')

// 1) 真实源文件必须通过（否则说明断言口径与源码不符）
const cleanProblems = auditIsolationShell(original)
check('真实 client.js 通过隔离外壳自检（0 问题）', cleanProblems.length === 0,
  cleanProblems.length ? cleanProblems.join('; ') : 'ok')

// 2) 变异 A：去掉 try/catch（把 apply 还原成直接调用本体）
{
  const mutated = original.replace(
    /try\s*\{\s*\n\s*return applyInner\(ctx\)\s*\n\s*\}\s*catch[\s\S]*?return function noopDisposer\(\)[^\n]*\n\s*\}/,
    'return applyInner(ctx)',
  )
  check('变异 A 确实改动了源码（不是替换失败造成的假阳性）', mutated !== original,
    mutated === original ? '替换未命中！' : '已去掉 try/catch')
  const problems = auditIsolationShell(mutated)
  check('变异 A（去掉 try/catch）→ 自检报「未把 applyInner 包进 try/catch」',
    problems.some((p) => p.includes('try/catch')), problems.join('; ') || '（无问题=空断言！）')
}

// 3) 变异 B：applyInner 改名（本体不再可隔离）
{
  const mutated = original.replace(/function applyInner\(ctx\)/, 'function applyBody(ctx)')
  check('变异 B 确实改动了源码', mutated !== original)
  const problems = auditIsolationShell(mutated)
  check('变异 B（applyInner 改名）→ 自检报「缺少 applyInner」',
    problems.some((p) => p.includes('applyInner')), problems.join('; ') || '（无问题=空断言！）')
}

// 4) 变异 C：失败分支返回 undefined（违反 cordis「必须返回 disposer」契约）
{
  const mutated = original.replace(
    /return function noopDisposer\(\) \{ \/\* 初始化失败，无副作用可清 \*\/ \}/,
    'return undefined',
  )
  check('变异 C 确实改动了源码', mutated !== original)
  const problems = auditIsolationShell(mutated)
  check('变异 C（失败分支返回 undefined）→ 自检报「catch 分支未返回 disposer」',
    problems.some((p) => p.includes('未返回 disposer')), problems.join('; ') || '（无问题=空断言！）')
}

console.log(failures ? `\n存在 ${failures} 项失败` : '\n全部通过（自检非空转，真会响）')
process.exit(failures ? 1 : 0)
