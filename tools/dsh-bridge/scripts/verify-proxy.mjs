#!/usr/bin/env node
/**
 * dsh-bridge 仪表盘代理（/craft/api/*）单元验证。
 *
 * 不依赖真实 viewer/DSH：用 mock webServer 捕获 craftProxy handler，
 * 再用 Node 的 http 服务器模拟 viewer，验证：
 *   1. GET  /craft/api/game-state → 转发到 viewer /api/game-state 并透传 body
 *   2. POST /craft/api/goal       → 转发 POST + JSON body
 *   3. 非 /craft/api 路径 → 404
 *   4. viewer 不可达 → 502 + JSON 错误
 *
 * 用法: node tools/dsh-bridge/scripts/verify-proxy.mjs
 */
import { createServer, IncomingMessage } from 'node:http'
import { pathToFileURL } from 'node:url'

// 用随机空闲端口模拟 viewer
const viewer = createServer((req, res) => {
  const chunks = []
  req.on('data', (c) => chunks.push(c))
  req.on('end', () => {
    const body = Buffer.concat(chunks).toString('utf8')
    if (req.url === '/api/game-state') {
      res.writeHead(200, { 'Content-Type': 'application/json' })
      res.end(JSON.stringify({ ok: true, status: 'connected', scene_desc: '测试状态', echoed: body || null }))
    } else if (req.url === '/api/goal') {
      res.writeHead(200, { 'Content-Type': 'application/json' })
      res.end(JSON.stringify({ ok: true, goal: JSON.parse(body || '{}').goal }))
    } else {
      res.writeHead(404)
      res.end('not found')
    }
  })
})

function listen(server) {
  return new Promise((resolve) => {
    server.listen(0, '127.0.0.1', () => resolve(server.address().port))
  })
}

let failures = 0
function check(name, cond, detail) {
  console.log(`${cond ? '✅' : '❌'} ${name}${detail ? ` — ${detail}` : ''}`)
  if (!cond) failures++
}

// 加载 index.js 并捕获 proxy handler
process.env.DSH_CRAFT_VIEWER_URL = '' // 用默认
const bridgeUrl = pathToFileURL(new URL('../index.js', import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, '$1')).href
const mod = await import(bridgeUrl)

let capturedHandler = null
const registeredWebServer = []
const fakeCtx = {
  tools: { register: () => {} },
  systemPrompt: { variable: () => {} },
  webServer: {
    register: (opts) => {
      registeredWebServer.push(opts)
      if (opts.path === '/craft/api') capturedHandler = opts.handler
      return 'dispose-handle'
    },
  },
  effect: () => {},
  get: () => undefined,
}
mod.apply(fakeCtx, { hostTools: false, proxy: true }) // 全局行语义：无工具但挂代理

if (!capturedHandler) {
  console.error('❌ 未捕获 /craft/api 代理 handler（webServer.register 未被调用）')
  process.exit(1)
}

const viewerPort = await listen(viewer)
// 覆盖 viewerUrl 指向 mock（通过 DSH_CRAFT_VIEWER_URL 环境变量在 import 前设置无效——
// 因为 index.js 的 viewerUrl() 每次调用都读环境变量，所以运行时设置即可）
process.env.DSH_CRAFT_VIEWER_URL = `http://127.0.0.1:${viewerPort}`

// 构造最小 req/res
function makeReq(method, url, body) {
  const req = new IncomingMessage(null)
  req.method = method
  req.url = url
  req.headers = { 'content-type': 'application/json' }
  // 注入可迭代 body
  if (body !== undefined) {
    const buf = Buffer.from(body)
    req[Symbol.asyncIterator] = async function* () { yield buf }
  } else {
    req[Symbol.asyncIterator] = async function* () {}
  }
  return req
}
function makeRes() {
  const chunks = []
  const res = {
    status: 0,
    headers: {},
    writeHead: function (code, h) { this.status = code; Object.assign(this.headers, h) },
    end: function (text) { this.body = text ?? '' },
  }
  return res
}

// 1) GET /craft/api/game-state
{
  const res = makeRes()
  await capturedHandler(makeReq('GET', '/craft/api/game-state'), res)
  const parsed = JSON.parse(res.body)
  check('GET /craft/api/game-state 转发成功', res.status === 200 && parsed.ok === true && parsed.status === 'connected',
    `status=${res.status}, body=${res.body.slice(0, 80)}`)
}

// 2) POST /craft/api/goal
{
  const res = makeRes()
  await capturedHandler(makeReq('POST', '/craft/api/goal', JSON.stringify({ goal: '测试目标' })), res)
  const parsed = JSON.parse(res.body)
  check('POST /craft/api/goal 转发 body', res.status === 200 && parsed.ok === true && parsed.goal === '测试目标',
    `status=${res.status}, goal=${parsed.goal}`)
}

// 3) 非 /craft/api 路径 → 404
{
  const res = makeRes()
  await capturedHandler(makeReq('GET', '/other/path'), res)
  check('非 /craft/api 路径 → 404', res.status === 404, `status=${res.status}`)
}

// 4) viewer 不可达 → 502
{
  process.env.DSH_CRAFT_VIEWER_URL = 'http://127.0.0.1:1' // 不可达端口
  const res = makeRes()
  await capturedHandler(makeReq('GET', '/craft/api/status'), res)
  check('viewer 不可达 → 502 JSON', res.status === 502 && res.body.includes('ok":false'),
    `status=${res.status}, body=${res.body.slice(0, 80)}`)
}

viewer.closeAllConnections()
viewer.close()
// 等 close 完成再退出
await new Promise((resolve) => setTimeout(resolve, 50))
console.log(failures ? `\n存在 ${failures} 项失败` : '\n全部通过')
process.exit(failures ? 1 : 0)
