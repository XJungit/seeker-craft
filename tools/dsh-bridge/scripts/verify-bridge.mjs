#!/usr/bin/env node
/**
 * dsh-bridge 连通性验证脚本（独立于 DSH 运行，直接测 viewer API）。
 *
 * 用法：node scripts/verify-bridge.mjs [viewerUrl]
 * 默认 viewerUrl = http://127.0.0.1:8080
 */

const url = (process.argv[2] ?? process.env.DSH_CRAFT_VIEWER_URL ?? 'http://127.0.0.1:8080').replace(/\/$/, '')

async function call(path, options = {}, timeoutMs = 30000) {
  const controller = new AbortController()
  const timer = setTimeout(() => controller.abort(), timeoutMs)
  try {
    const res = await fetch(url + path, {
      ...options,
      signal: controller.signal,
      headers: options.body ? { 'content-type': 'application/json' } : undefined,
    })
    const body = await res.json().catch(() => null)
    return { status: res.status, ok: res.ok, body }
  } finally {
    clearTimeout(timer)
  }
}

function check(name, cond, detail) {
  console.log(`${cond ? '✅' : '❌'} ${name}${detail ? ` — ${detail}` : ''}`)
  if (!cond) process.exitCode = 1
}

const state = await call('/api/game-state')
check('GET /api/game-state', state.ok && state.body && state.body.status !== 'not_connected',
  state.ok ? `scene_desc 前 60 字: ${(state.body?.scene_desc ?? '').slice(0, 60).replace(/\n/g, ' ')}` : `HTTP ${state.status}`)

// 无害工具：perceive（只读感知，不改变世界）
const perceive = await call('/api/bot_tool', {
  method: 'POST',
  body: JSON.stringify({ name: 'perceive', args: {} }),
})
check('POST /api/bot_tool perceive', perceive.ok && perceive.body?.ok === true,
  perceive.body?.message ? `message 前 60 字: ${String(perceive.body.message).slice(0, 60).replace(/\n/g, ' ')}` : `HTTP ${perceive.status}`)

const unknown = await call('/api/bot_tool', {
  method: 'POST',
  body: JSON.stringify({ name: 'definitely_not_a_tool', args: {} }),
})
check('未知工具返回错误', unknown.ok && unknown.body?.ok === false,
  unknown.body?.error ?? unknown.body?.message ?? `HTTP ${unknown.status}`)

const goal = await call('/api/goal', {
  method: 'POST',
  body: JSON.stringify({ goal: 'DSH 桥接验证目标' }),
})
check('POST /api/goal', goal.ok && goal.body?.ok === true, goal.body?.goal ?? `HTTP ${goal.status}`)

console.log(process.exitCode ? '\n存在失败项' : '\n全部通过')
