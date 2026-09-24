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
 * 除缩进/包装外，本脚本还处理四处 0.1.7 的破坏性变更：
 *   1. `@deepseek-ai/dsh-workflow-worker-thread` → `@deepseek-ai/dsh-workflow-ptc`（包改名）；
 *   2. 复用官方预设技能目录的表达式改为 `createRequire(baseUrl)` 版本 —— 0.1.7 里
 *      `dsh-agent-presets`（复数）包已不存在，绝对路径写法必然失效；
 *   3. `{{PROJECT_ROOT_URL}}` → `file:///<PROJECT_ROOT>`：0.1.7 的 loader 把非 `.`
 *      开头的 name 直接交给 Node `import()`，无 scheme 的 Windows 绝对路径会被拒
 *      （ERR_UNSUPPORTED_ESM_URL_SCHEME），dsh-bridge 行必须写文件 URL；
 *   4. 注入 `tool-cordis` + `tool-plugin-manager`（自进化能力）—— 见下方 (3) 的说明，
 *      模板出于 rc.3 的 process-global 约束刻意不含这两行，仅在 0.1.7 输出中注入。
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
// 0.1.7 的 loader 把非 `.` 开头的 name 直接交给 Node `import()`，而无 scheme 的
// Windows 绝对路径会被拒绝（ERR_UNSUPPORTED_ESM_URL_SCHEME），故 dsh-bridge 行
// 必须写成 file:// URL。
const projectRootUrl = 'file:///' + projectRootPosix
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
  // {{PROJECT_ROOT_URL}} 必须先于 {{PROJECT_ROOT}} 替换，否则会被后者截断前缀。
  line = line.replace(/\{\{PROJECT_ROOT_URL\}\}/g, projectRootUrl)
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

// ── (3) 0.1.7 专属：注入 cordis 自引用能力（self-modification 段）─────────────
//  模板本身是 rc.3 / 0.1.7 共用源，而这一段的行为在两版本下**相反**：
//    rc.3   ：cordisInspect 是 process-global，多预设各注册一次会碰撞 → 刻意不注册；
//    0.1.7+ ：provider 由 host 的 `cordis-inspect-providers` 每进程注册一次，
//             预设行只做 ctx.tools.register → 可安全注册，且 json 里 persona 明确
//             把「改预设 / 装插件」列为自进化使命，故对齐官方 cordis（创造模式）预设
//             一并启用 tool-plugin-manager。
//  因此不在模板里写这两行，而由生成器只在 0.1.7 输出中注入。
const SELF_MOD_ROWS = [
  '',
  '# 以下两行由生成器为 0.1.7+ 注入（rc.3 模板中刻意省略，原因见上）。',
  "- id: tool-cordis",
  "  name: '@deepseek-ai/dsh-tool-cordis'",
  '',
  "# profile 层（dsh-web-app）把该行全局置为 disabled: true；官方 cordis 预设用",
  "# `!!js \"!ctx.get('profileContext')\"` 在有 profile 的会话中启用。此表达式与官方一致。",
  '# 注意：plugin_manager 相关调用需 danger-full-access 或逐次批准。',
  "- id: tool-plugin-manager",
  "  name: '@deepseek-ai/dsh-plugin-manager/tools'",
  '  disabled: !!js "!ctx.get(\'profileContext\')"',
]

let selfModInjected = false
{
  const marker = body.findIndex((l) => /─+\s*self-modification/.test(l))
  if (marker >= 0) {
    // 找到该注释段的最后一行（连续注释块），在其后插入
    let last = marker
    for (let i = marker; i < body.length; i++) if (/^\s*#/.test(body[i])) last = i
    body.splice(last + 1, 0, ...SELF_MOD_ROWS)
    selfModInjected = true
  }
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
const outText = out.join('\n')
const problems = []
if (!skillsReplaced) problems.push('未替换技能目录表达式（模板可能已改）')
if (/dsh-agent-presets[\/\\]presets/.test(outText)) problems.push('输出中仍残留旧 `dsh-agent-presets/presets` 路径')
if (/dsh-workflow-worker-thread/.test(outText)) problems.push('输出中仍残留 `dsh-workflow-worker-thread`')
if (!/workflow-ptc/.test(outText)) problems.push('输出中未见 `workflow-ptc`')
if (!selfModInjected) problems.push('未找到 self-modification 注释段，cordis 自引用行未注入')
if (!/^ {10}- id: tool-cordis$/m.test(outText)) problems.push('输出中未见 tool-cordis 行')
if (!/^ {10}- id: tool-plugin-manager$/m.test(outText)) problems.push('输出中未见 tool-plugin-manager 行')

// dsh-bridge 必须以 file:// URL 加载：无 scheme 的 Windows 绝对路径会被 Node ESM 拒绝
if (/^\s*name:\s*[A-Za-z]:[\\/]/.test(outText)) problems.push('输出中存在无 scheme 的 Windows 绝对路径 name（0.1.7 loader 会拒绝）')
if (!/^\s*name:\s*file:\/\/\/.*\/tools\/dsh-bridge\/index\.js$/m.test(outText)) problems.push('dsh-bridge 行未写成 file:// URL')
if (/\{\{PROJECT_ROOT(_URL)?\}\}/.test(outText)) problems.push('输出中残留未替换的 PROJECT_ROOT 占位符')

// 与官方 standard/ptc/cordis 的基线对齐（v1.6.1 修复项）——防止回退：
if (!/^\s*modelSelectionSettings: true$/m.test(outText)) problems.push('tool-subagent 缺少 `modelSelectionSettings: true`（未对齐官方基线）')
if (/enableRunInBackground/.test(outText)) problems.push('输出中仍有旧式键 `enableRunInBackground`（应为 `backgroundMode`）')
if (!/^ {10}- id: present$/m.test(outText)) problems.push('present 行 id 未对齐官方（应为 `present`）')
if (/^ {10}- id: tool-present$/m.test(outText)) problems.push('输出中仍有旧行 id `tool-present`')
if (!/^ {14}- id: tool-ralph$\n(?:.*\n)*?^ {16}disabled: true$/m.test(outText)) problems.push('tool-ralph 未禁用（官方三预设均禁用）')

console.log(`[OK] 已生成 0.1.7 预设：${outPath}`)
console.log(`     行数 ${out.length} ｜ 技能表达式替换 ${skillsReplaced ? '是' : '否'} ｜ workflow 改名 ${workflowRenamed} 处 ｜ 自引用行注入 ${selfModInjected ? '是' : '否'}`)
if (problems.length) {
  console.error('[FAIL] 自检未通过：')
  for (const p of problems) console.error('   - ' + p)
  process.exit(2)
}
