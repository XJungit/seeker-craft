#!/usr/bin/env node
/**
 * gen-craft-bot-preset-017.mjs
 *
 * 把本仓库的 craft-bot 预设模板（rc.3 格式，`data/dsh/craft-bot-preset/agent.cordis.yml`）
 * 转换为 DeepSeek Harness 0.1.7+ 的「单条 `@deepseek-ai/dsh-agent-preset` 声明」格式。
 *
 * 两种格式的关系（已与官方 preset 逐条比对验证）：
 *   rc.3   ：`~/.dsh/.agent-presets/<id>/agent.cordis.yml` —— 文件顶层就是一个插件条目数组，
 *            同目录 `preset.yml` 提供 name / description / order。
 *   0.1.7+ ：一个 YAML 文档，形如
 *              - insert:
 *                  - id: preset-<id>
 *                    name: '@deepseek-ai/dsh-agent-preset'
 *                    config: { id, name, description, order, plugins: [...] }
 *            其中 `plugins` 正是 rc.3 那份数组，整体缩进 +10 后再缩进到 `plugins:` 之下。
 *            0.1.7 不再扫描 `.agent-presets`，预设必须由 bundle 的 `dsh.bundle.patch` 提供。
 *
 * 除缩进/包装外，本脚本还处理两处 0.1.7 的破坏性变更：
 *   1. `@deepseek-ai/dsh-workflow-worker-thread` → `@deepseek-ai/dsh-workflow-ptc`（包改名）；
 *   2. 复用官方预设技能目录的表达式改为 `createRequire(baseUrl)` 版本 —— 0.1.7 里
 *      `dsh-agent-presets`（复数）包已不存在，绝对路径写法必然失效。
 *
 * 用法：
 *   node scripts/gen-craft-bot-preset-017.mjs [--project-root <path>] [--dsh-pkg-root <path>] [--out <file>]
 * 默认：
 *   --project-root  脚本上两级目录（仓库根）
 *   --out           <project-root>/data/dsh/craft-bot-preset-017/cordis.patch.yml
 * `--dsh-pkg-root` 仅供 rc.3 模板占位符兼容，0.1.7 输出不再使用绝对技能路径。
 */

import { readFileSync, writeFileSync, mkdirSync, existsSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const HERE = dirname(fileURLToPath(import.meta.url))
const REPO_ROOT = resolve(HERE, '..')

// ── 参数解析 ────────────────────────────────────────────────────────────────
const argv = process.argv.slice(2)
const arg = (name, fallback) => {
  const i = argv.indexOf(name)
  return (i >= 0 && argv[i + 1]) ? argv[i + 1] : fallback
}
const projectRoot = resolve(arg('--project-root', REPO_ROOT))
const projectRootPosix = projectRoot.replace(/\\/g, '/')
const templatePath = join(projectRoot, 'data', 'dsh', 'craft-bot-preset', 'agent.cordis.yml')
const metaPath = join(projectRoot, 'data', 'dsh', 'craft-bot-preset', 'preset.yml')
const outPath = resolve(arg('--out', join(projectRoot, 'data', 'dsh', 'craft-bot-preset-017', 'cordis.patch.yml')))
const dshPkgRoot = arg('--dsh-pkg-root', '')

if (!existsSync(templatePath)) {
  console.error(`[FAIL] 未找到预设模板：${templatePath}`)
  process.exit(1)
}

// ── 读取 rc.3 模板与元数据 ─────────────────────────────────────────────────
const template = readFileSync(templatePath, 'utf8')

/** 极简 YAML 顶层标量读取（只需 name / description / order 三行）。 */
function readFlatYaml(text) {
  const out = {}
  for (const line of text.split(/\r?\n/)) {
    const m = /^([A-Za-z_][\w-]*):\s*(.*)$/.exec(line)
    if (m) out[m[1]] = m[2].trim()
  }
  return out
}
const meta = existsSync(metaPath) ? readFlatYaml(readFileSync(metaPath, 'utf8')) : {}

const PRESET_ID = 'craft-bot'
const PRESET_NAME = meta.name || 'Craft Bot'
const PRESET_ORDER = meta.order || '10'
const PRESET_DESC = meta.description || ''

// ── 转换 ────────────────────────────────────────────────────────────────────
// 0.1.7 官方预设里复用 dsh-agent-preset 自带技能目录的写法：不依赖任何绝对路径。
const SKILLS_017 =
  "- !!js process.getBuiltinModule('node:path').join(process.getBuiltinModule('node:path').dirname(" +
  "process.getBuiltinModule('node:module').createRequire(baseUrl).resolve('@deepseek-ai/dsh-agent-preset/package.json')), 'skills')"

const lines = template.split(/\r?\n/)
const body = []
let skillsReplaced = false
let workflowRenamed = 0

for (const raw of lines) {
  let line = raw

  // 占位符替换（0.1.7 输出保留 {{model}} / {{cwd}} / {{tool_list}} / {{viewer_url}} 这类运行时变量）
  line = line.replace(/\{\{PROJECT_ROOT\}\}/g, projectRootPosix)
  if (dshPkgRoot) line = line.replace(/\{\{DSH_PKG_ROOT\}\}/g, dshPkgRoot.replace(/\\/g, '/'))

  // (1) 技能目录表达式 → 官方 baseUrl 写法（整行替换，连带处理残留的 {{DSH_PKG_ROOT}}）
  if (line.includes('customSkillDirs') || /dsh-agent-presets[\/\\]presets/.test(line) || line.includes('{{DSH_PKG_ROOT}}')) {
    if (/^\s*-\s*!!js/.test(line) && !line.includes('process.platform')) {
      const indent = /^(\s*)/.exec(line)[1]
      body.push(indent + SKILLS_017)
      skillsReplaced = true
      continue
    }
    // 注释里提到旧包名/占位符 —— 改写注释，避免误导
    line = line.replace(/dsh-agent-presets\/presets\/cordis/g, 'dsh-agent-preset/skills')
  }

  // (2) workflow 引擎包改名（0.1.7 起 dsh-workflow-worker-thread → dsh-workflow-ptc）
  if (/dsh-workflow-worker-thread/.test(line)) {
    line = line.replace(/dsh-workflow-worker-thread/g, 'dsh-workflow-ptc')
    workflowRenamed++
  }
  // 条目 id 同步改名（仅裸 id 行，避免误伤注释中的说明）
  if (/^\s*-?\s*id:\s*workflow-worker-thread\s*$/.test(line)) {
    line = line.replace(/workflow-worker-thread/, 'workflow-ptc')
  }

  body.push(line)
}

// ── 包装为 0.1.7 的单条声明 ─────────────────────────────────────────────────
const head = [
  `# Craft-Agent 的 craft-bot 预设 —— DeepSeek Harness 0.1.7+ 格式。`,
  `#`,
  `# ⚠️ 本文件由 scripts/gen-craft-bot-preset-017.mjs 生成，请勿手工编辑。`,
  `# 源模板：data/dsh/craft-bot-preset/agent.cordis.yml（rc.3 格式，两者共用同一份内容）。`,
  `# 该文件包含本机绝对路径，已加入 .gitignore。`,
  `- insert:`,
  `    - id: preset-${PRESET_ID}`,
  `      name: '@deepseek-ai/dsh-agent-preset'`,
  `      config:`,
  `        id: ${PRESET_ID}`,
  PRESET_NAME ? `        name: ${JSON.stringify(PRESET_NAME)}` : '',
  PRESET_DESC ? `        description: ${JSON.stringify(PRESET_DESC)}` : '',
  `        order: ${PRESET_ORDER}`,
  `        plugins:`,
].filter(Boolean)

const out = []
for (const line of head) out.push(line)
for (const line of body) {
  if (line.trim() === '') { out.push(''); continue }
  out.push('          ' + line)   // 缩进到 `plugins:` 的条目层级
}

mkdirSync(dirname(outPath), { recursive: true })
writeFileSync(outPath, out.join('\n') + '\n', 'utf8')

// ── 自检 ────────────────────────────────────────────────────────────────────
const problems = []
if (!skillsReplaced) problems.push('未替换技能目录表达式（模板可能已改）')
if (/dsh-agent-presets[\/\\]presets/.test(out.join('\n'))) problems.push('输出中仍残留旧 `dsh-agent-presets/presets` 路径')
if (/dsh-workflow-worker-thread/.test(out.join('\n'))) problems.push('输出中仍残留 `dsh-workflow-worker-thread`')
if (!/workflow-ptc/.test(out.join('\n'))) problems.push('输出中未见 `workflow-ptc`')

console.log(`[OK] 已生成 0.1.7 预设：${outPath}`)
console.log(`     行数 ${out.length} ｜ 技能表达式替换 ${skillsReplaced ? '是' : '否'} ｜ workflow 改名 ${workflowRenamed} 处`)
if (problems.length) {
  console.error('[FAIL] 自检未通过：')
  for (const p of problems) console.error('   - ' + p)
  process.exit(2)
}
