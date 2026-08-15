/**
 * 验证 bot_state 惰性刷新修复：
 * 1. context 同步返回 string（DSH 契约）
 * 2. 缓存过期后，context 求值触发异步刷新（fire-and-forget）
 * 3. 刷新失败时 .catch 兜底，不产生 unhandled rejection（不崩 DSH）
 * 4. setInterval 与惰性刷新互补
 */
import z from '@deepseek-ai/schemastery'
import { defineTool } from '@deepseek-ai/dsh-tools'

// 最小 mock viewer：模拟 game-state 返回，可切换可达/不可达
let viewerUp = true
let sceneValue = 'scene-v1'
const mockFetch = async (url, options = {}) => {
  if (!viewerUp) throw new Error('viewer down')
  return { ok: true, status: 200, json: async () => ({ scene_desc: sceneValue, status: 'ok' }) }
}

// 复制插件核心逻辑（botStateCache + refreshBotState + context text）
let botStateCache = { at: 0, text: null }
const BOT_STATE_TTL_MS = 30000
let fetchFn = mockFetch

async function refreshBotState() {
  try {
    const res = await fetchFn('/api/game-state', {}, 5000)
    const body = res.ok ? await res.json() : null
    if (res.ok && body && body.status !== 'not_connected') {
      botStateCache = { at: Date.now(), text: body.scene_desc ?? JSON.stringify(body) }
      return true
    }
  } catch {
    // 保留旧缓存
  }
  return false
}

// context text（模拟插件 L158-163 的新逻辑）
function contextText() {
  const cached = botStateCache.text ?? '(bot 状态加载中…)'
  if (Date.now() - botStateCache.at > BOT_STATE_TTL_MS) {
    refreshBotState().catch(() => { /* 静默 */ })
  }
  return `【当前游戏状态】\n${cached}`
}

let pass = 0, fail = 0
function check(name, cond, detail = '') {
  if (cond) { pass++; console.log(`✅ ${name}`) }
  else { fail++; console.log(`❌ ${name} ${detail}`) }
}

// 测试1：初始无缓存 → 同步返回占位符（不崩）
const r1 = contextText()
check('初始返回 string 占位符', typeof r1 === 'string' && r1.includes('加载中'), r1)

// 测试2：viewer 可达 → 手动刷新一次（模拟 setInterval）
await refreshBotState()
const r2 = contextText()
check('刷新后返回 scene-v1', r2.includes('scene-v1'), r2)

// 测试3：缓存未过期（at 是刚刷新的）→ 不触发刷新，直接返回缓存
const before = botStateCache.at
contextText()
check('未过期不触发刷新', botStateCache.at === before)

// 测试4：缓存过期 → context 求值触发异步刷新，下一轮拿到新值
botStateCache.at = 0 // 强制过期
sceneValue = 'scene-v2'
const r4a = contextText() // 这轮可能还是旧值（异步未完成），但触发刷新
await new Promise(r => setTimeout(r, 50)) // 等异步刷新完成
const r4b = contextText()
check('过期后惰性刷新拿到 scene-v2', r4b.includes('scene-v2'), r4b)

// 测试5：viewer 不可达 → refresh 静默失败，.catch 兜底，无 unhandled rejection
viewerUp = false
botStateCache.at = 0
const r5 = contextText() // 触发刷新，但 viewer down
await new Promise(r => setTimeout(r, 50))
check('viewer 不可达时返回旧缓存不崩', typeof r5 === 'string' && r5.includes('scene-v2'), r5)

// 测试6：恢复 viewer → 惰性刷新恢复
viewerUp = true
sceneValue = 'scene-v3'
botStateCache.at = 0
contextText()
await new Promise(r => setTimeout(r, 50))
const r6 = contextText()
check('viewer 恢复后惰性刷新拿到 scene-v3', r6.includes('scene-v3'), r6)

console.log(`\n结果: ${pass} 通过, ${fail} 失败`)
process.exit(fail === 0 ? 0 : 1)
