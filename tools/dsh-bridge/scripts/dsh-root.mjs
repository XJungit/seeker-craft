#!/usr/bin/env node
/**
 * DSH 安装根解析（三个校验脚本共用）。
 *
 * 为什么需要它：0.1.x 时代 DSH 装在 npm 全局
 * （<roaming>/npm/node_modules/@deepseek-ai/dsh/node_modules/@deepseek-ai），
 * 校验脚本把这个路径写死了。2026-09 桌面端升级到 0.2.0-rc.2 后改为**自带运行时**，
 * @deepseek-ai/* 打进 `D:\DeepSeek Harness\resources\app.asar`，旧的 npm 全局安装
 * 被卸载 → 写死路径全部失效，三个脚本集体误报「包读不到 / provider 读不到」，
 * 看着像兼容性炸了，其实只是探测路径过期。
 *
 * 解析策略：按「谁真的在」逐个探测，返回第一个含 sidebar-right 的目录；
 * 找到 asar 时额外标注 needsAsar（调用方用 scripts/extract-asar.mjs 读）。
 */
import { existsSync } from 'node:fs'
import { join } from 'node:path'
import { execFileSync } from 'node:child_process'

const HOME = process.env.USERPROFILE || process.env.HOME || ''

/** 仓库根（scripts/ → dsh-bridge/ → tools/ → 仓库根）。 */
export const REPO = new URL('../../../', import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, '$1')

/** asar 解包器脚本路径。 */
const EXTRACT = join(REPO, 'scripts', 'extract-asar.mjs')

/**
 * 本地兼容性缓存：从 asar 抠出的 0.2.0-rc.2 官方包（gitignore 掉，不入库）。
 * 之所以要缓存：cordis 的包内依赖（cosmokit / @standard-schema/spec）必须按
 * node_modules 布局摆放，Node 的 ESM 解析才找得到。
 */
export const COMPAT_DIR = join(REPO, 'data', 'dsh', 'compat-020')

const COMPAT_PKGS = [
  ['dsh/node_modules/@deepseek-ai/cordis/', '@deepseek-ai/cordis'],
  ['dsh/node_modules/@deepseek-ai/cosmokit/', '@deepseek-ai/cosmokit'],
  ['dsh/node_modules/@standard-schema/spec/', '@standard-schema/spec'],
  ['dsh/node_modules/@deepseek-ai/dsh-client-ui-sidebar-right/', '@deepseek-ai/dsh-client-ui-sidebar-right'],
  ['dsh/node_modules/@deepseek-ai/dsh-client-ui-renderer/', '@deepseek-ai/dsh-client-ui-renderer'],
  ['dsh/node_modules/@deepseek-ai/dsh-api-session-controller/', '@deepseek-ai/dsh-api-session-controller'],
]

/**
 * 确保 compat 缓存存在（缺什么补什么），返回 cordis 的 lib/index.js 路径；不可用返回 null。
 * 这样三个校验脚本不再依赖"磁盘上恰好有一份可读的 DSH"。
 */
export function ensureCompatCache({ quiet = true } = {}) {
  const asar = resolveDshAsar()
  if (!asar) return null
  const cordis = join(COMPAT_DIR, 'node_modules', '@deepseek-ai', 'cordis', 'lib', 'index.js')
  if (existsSync(cordis)) return cordis
  for (const [inner, dest] of COMPAT_PKGS) {
    try {
      execFileSync(process.execPath, [EXTRACT, asar, 'extract', inner, join(COMPAT_DIR, 'node_modules', dest)], {
        stdio: quiet ? 'ignore' : 'inherit',
      })
    } catch { /* 单个包失败不致命：后面按缺什么报什么 */ }
  }
  return existsSync(cordis) ? cordis : null
}

/** 候选安装根，按可信度排序。 */
export function dshRootCandidates() {
  const out = []
  if (process.env.DSH_PKG_ROOT) out.push({ root: process.env.DSH_PKG_ROOT, kind: 'explicit' })
  // 桌面端 0.2.0+：自带运行时（asar 打包，磁盘无散包）
  for (const asar of [
    'D:/DeepSeek Harness/resources/app.asar',
    `${HOME}/AppData/Local/Programs/DeepSeek Harness/resources/app.asar`,
    '/Applications/DeepSeek Harness.app/Contents/Resources/app.asar',
  ]) {
    if (existsSync(asar)) out.push({ root: asar, kind: 'asar' })
  }
  // asar 解包目录（部分文件会落地）
  out.push({
    root: 'D:/DeepSeek Harness/resources/app.asar.unpacked/dsh/node_modules/@deepseek-ai',
    kind: 'dir',
  })
  // 本地兼容性缓存（ensureCompatCache 产出）
  out.push({ root: join(COMPAT_DIR, 'node_modules', '@deepseek-ai'), kind: 'dir' })
  // npm 全局（0.1.x 及更早）
  out.push({
    root: join(HOME, 'AppData/Roaming/npm/node_modules/@deepseek-ai/dsh/node_modules/@deepseek-ai'),
    kind: 'dir',
  })
  out.push({
    root: join(HOME, 'AppData/Roaming/npm/node_modules/@deepseek-ai'),
    kind: 'dir',
  })
  // pnpm 全局
  out.push({ root: join(HOME, 'AppData/Local/pnpm/global/5/node_modules/@deepseek-ai'), kind: 'dir' })
  // 本地开发 checkout
  out.push({ root: 'D:/DeepSeekHarness/packages', kind: 'dir' })
  return out
}

/** 找一个真实可读的 @deepseek-ai 目录（只认 dir 型候选）。 */
export function resolveDshDir() {
  for (const c of dshRootCandidates()) {
    if (c.kind !== 'dir') continue
    if (existsSync(join(c.root, 'dsh-client-ui-sidebar-right/package.json'))) return c.root
  }
  return null
}

/** 找 asar（用 extract-asar.mjs 读）。 */
export function resolveDshAsar() {
  for (const c of dshRootCandidates()) {
    if (c.kind === 'asar') return c.root
  }
  return null
}

/**
 * 读一个包内文件的文本：优先磁盘目录，其次 asar。
 * @returns {string|null}
 */
export function readDshPackageFile(pkgName, relPath) {
  const dir = resolveDshDir()
  if (dir) {
    const p = join(dir, pkgName, relPath)
    if (existsSync(p)) return readFileSync(p, 'utf8')
  }
  const asar = resolveDshAsar()
  if (asar) {
    try {
      return execFileSync(process.execPath, [EXTRACT, asar, 'cat', `dsh/node_modules/@deepseek-ai/${pkgName}/${relPath}`], {
        encoding: 'utf8',
        maxBuffer: 64 * 1024 * 1024,
      })
    } catch {
      return null
    }
  }
  return null
}
