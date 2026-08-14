/**
 * dsh-toolscan — 诊断 + provider 边界 schema 归一化插件（craft-bot 预设专用，绝对路径加载）。
 *
 * 拦截 harness 实际发往 DeepSeek / codebuddy 的 chat completion 请求，在出网前：
 *   1. 把请求体里的 tools 数组 dump 出来扫描 DeepSeek 会拒绝的 schema：
 *        - 属性节点无 `type` 且无 `oneOf`/`anyOf`   （无约束 JSON，DeepSeek 拒）
 *        - 含 `oneOf` / `anyOf`                       （union 类型，DeepSeek 拒）
 *        - `type` 为数组（如 ["object","string"]）      （type 数组，DeepSeek 拒）
 *   2. 归一化每个工具的 parameters：把 anyOf/oneOf 压成 DeepSeek 接受的单一类型
 *      （可空标量变非空 type；多 object 分支合并 properties），并修补裸无 type 节点。
 *      【只改 tool schema，绝不碰 messages 或其他字段。】
 *   3. 改写后重新扫描，确认 0 问题才把改写后的 body 发出去。
 *
 * 机制：monkey-patch globalThis.fetch（harness 发 HTTP 用全局 fetch）。不改 harness
 * 源码、不改 MCP client、不改各预设。诊断/修复完成后删除 agent.cordis.yml 里本插件
 * 的挂载行 + 删除本文件即可完全还原。
 *
 * @module dsh-toolscan
 */

const fs = await import('node:fs')
const path = await import('node:path')

function homeDir() {
  return process.env.DSH_HOME || (process.env.USERPROFILE
    ? path.join(process.env.USERPROFILE, '.dsh') : '.')
}
let reqSeq = 0

function logPath() {
  return path.join(homeDir(), '_toolscan.log')
}

function scanSchema(node, loc, problems) {
  if (!node || typeof node !== 'object') return
  if (Array.isArray(node)) {
    for (let i = 0; i < node.length; i++) scanSchema(node[i], `${loc}[${i}]`, problems)
    return
  }
  if ('properties' in node || 'items' in node || 'type' in node || 'oneOf' in node || 'anyOf' in node) {
    if ('oneOf' in node) problems.push(`${loc}: uses oneOf (DeepSeek-rejected)`)
    if ('anyOf' in node) problems.push(`${loc}: uses anyOf (DeepSeek-rejected)`)
    if ('type' in node && Array.isArray(node.type)) {
      problems.push(`${loc}: type is an array ${JSON.stringify(node.type)} (DeepSeek-rejected)`)
    }
    if ('properties' in node) {
      for (const [k, v] of Object.entries(node.properties)) {
        const childLoc = `${loc}.properties.${k}`
        if (v && typeof v === 'object' && !('type' in v) && !('oneOf' in v) && !('anyOf' in v)) {
          problems.push(`${childLoc}: property has NO type and NO oneOf/anyOf (DeepSeek-rejected)`)
        }
        scanSchema(v, childLoc, problems)
      }
    }
    if ('items' in node && node.items) scanSchema(node.items, `${loc}.items`, problems)
  }
}

/**
 * Normalize one schema node in place so DeepSeek accepts it.
 * - anyOf/oneOf containing a {type:null} branch -> collapse to the non-null branch
 * - anyOf/oneOf of object branches -> merge properties + additionalProperties:true
 * - anyOf/oneOf of scalar branches -> take the first non-null branch
 * - bare node with no type/oneOf/anyOf -> treat as open object (type:object, additionalProperties:true)
 * Recurses into properties/items. Returns nothing (mutates).
 */
function normalizeSchema(node) {
  if (!node || typeof node !== 'object') return
  if (Array.isArray(node)) {
    for (const item of node) normalizeSchema(item)
    return
  }
  const unionKey = 'anyOf' in node ? 'anyOf' : ('oneOf' in node ? 'oneOf' : null)
  if (unionKey) {
    const branches = node[unionKey]
    delete node[unionKey]
    const nonNull = branches.filter(b => b && typeof b === 'object' && b.type !== 'null')
    const hasNull = branches.some(b => b && typeof b === 'object' && b.type === 'null')
    const chosen = nonNull.length ? nonNull : branches
    if (chosen.length === 1) {
      const b = chosen[0]
      for (const [k, v] of Object.entries(b)) {
        if (k === 'type' && Array.isArray(v)) {
          node.type = v.find(t => t !== 'null') || v[0]
        } else {
          node[k] = v
        }
      }
    } else {
      const merged = { type: 'object', additionalProperties: true, properties: {} }
      for (const b of chosen) {
        if (b && typeof b === 'object' && b.properties) {
          Object.assign(merged.properties, b.properties)
        }
      }
      if (Object.keys(merged.properties).length === 0) {
        merged.additionalProperties = true
        delete merged.properties
      }
      Object.assign(node, merged)
    }
    if (hasNull && node.type) node.type = Array.isArray(node.type) ? node.type.filter(t => t !== 'null') : node.type
  }
  if (!('type' in node) && !('oneOf' in node) && !('anyOf' in node) && ('properties' in node || 'description' in node)) {
    node.type = 'object'
    if (!('additionalProperties' in node)) node.additionalProperties = true
  }
  if ('properties' in node && node.properties) {
    for (const v of Object.values(node.properties)) normalizeSchema(v)
  }
  if ('items' in node && node.items) normalizeSchema(node.items)
}

function normalizeToolParams(t) {
  const params = getToolParams(t)
  if (params && typeof params === 'object') normalizeSchema(params)
}

function getToolName(t) {
  if (!t || typeof t !== 'object') return '(non-object)'
  if (typeof t.name === 'string') return t.name
  if (t.function && typeof t.function.name === 'string') return t.function.name
  if (t.function && typeof t.function === 'object') return '(fn:' + Object.keys(t.function).join('|') + ')'
  return '(keys:' + Object.keys(t).join('|') + ')'
}

function getToolParams(t) {
  if (!t || typeof t !== 'object') return undefined
  if (t.parameters && typeof t.parameters === 'object') return t.parameters
  if (t.function && t.function.parameters && typeof t.function.parameters === 'object') return t.function.parameters
  return undefined
}

function scanTools(tools, tag) {
  const problems = []
  if (!Array.isArray(tools)) {
    problems.push(`${tag}: tools is not an array (${typeof tools})`)
  } else {
    for (let i = 0; i < tools.length; i++) {
      const t = tools[i]
      const name = getToolName(t)
      const params = getToolParams(t)
      if (params) {
        scanSchema(params, `tools[${name}].parameters`, problems)
      }
      if (t && typeof t === 'object' && t.function && typeof t.function === 'object') {
        scanSchema(t.function, `tools[${name}].function`, problems)
      }
    }
  }
  const lines = []
  lines.push(`\n========== ${new Date().toISOString()} [${tag}] ==========`)
  lines.push(`tools count: ${Array.isArray(tools) ? tools.length : 'n/a'}`)
  lines.push(`tool names: ${Array.isArray(tools) ? tools.map((t, i) => `${i}:${getToolName(t)}`).join(', ') : '(n/a)'}`)
  if (Array.isArray(tools) && tools.length > 0) {
    lines.push(`--- first tool raw (first 400 chars) ---`)
    lines.push(JSON.stringify(tools[0]).slice(0, 400))
  }
  if (problems.length === 0) {
    lines.push('SCAN: CLEAN — no DeepSeek-rejected schema nodes found among harness tools')
  } else {
    lines.push(`SCAN: ${problems.length} PROBLEM(S):`)
    for (const p of problems) lines.push('  - ' + p)
  }
  try {
    fs.appendFileSync(logPath(), lines.join('\n') + '\n', 'utf8')
  } catch (e) {
    // best-effort; never throw from a diagnostic probe
  }
  return problems
}

// Only patch once.
if (!globalThis.__dsh_toolscan_patched) {
  globalThis.__dsh_toolscan_patched = true
  const origFetch = globalThis.fetch?.bind(globalThis)
  if (origFetch) {
    globalThis.fetch = async function patchedFetch(input, init = {}) {
      const url = typeof input === 'string' ? input : (input?.url || '')
      const isLLM = /deepseek|codebuddy|chat\/completions|openai/i.test(url)
      const seq = ++reqSeq
      let toolsCached = null
      let nToolsCached = 0
      // Expand the match: include localhost proxy too, and detect the target message.
      const isLLMReq = /chat\/completions|deepseek|codebuddy|openai|localhost:20128/i.test(url)
      const TARGET = '777888666'
      if (isLLMReq && (init?.method || 'GET').toUpperCase() === 'POST') {
        const bodyKind = init?.body == null ? 'null' : (typeof init.body)
        fs.appendFileSync(logPath(), `\n[req #${seq}] url=${url} bodyKind=${bodyKind}\n`, 'utf8')
        if (bodyKind === 'string') {
          try {
            const parsed = JSON.parse(init.body)
            const hasTools = Array.isArray(parsed?.tools)
            const msgBlob = JSON.stringify(parsed?.messages || '')
            const isTarget = msgBlob.includes(TARGET)
            fs.appendFileSync(logPath(),
              `[req #${seq}] hasTools=${hasTools} nTools=${hasTools ? parsed.tools.length : '-'} isTarget=${isTarget}\n`, 'utf8')
            if (hasTools) {
              // Normalize every tool's parameters in place.
              for (const t of parsed.tools) {
                normalizeToolParams(t)
                if (t && typeof t === 'object' && t.function) normalizeSchema(t.function)
              }
              const remaining = scanTools(parsed.tools, `LLM_REQUEST_NORMALIZED #${seq}`)
              toolsCached = parsed.tools
              nToolsCached = parsed.tools.length
              // Dump the FULL normalized tools for the target request (untruncated).
              if (isTarget) {
                try {
                  fs.writeFileSync(path.join(homeDir(), `_toolscan_TARGET_req${String(seq).padStart(2,'0')}_n${nToolsCached}.json`),
                    JSON.stringify(parsed.tools, null, 2), 'utf8')
                  fs.appendFileSync(logPath(), `[req #${seq}] [TARGET] full tools dumped\n`, 'utf8')
                } catch (e) {
                  fs.appendFileSync(logPath(), `[req #${seq}] [TARGET] dump-fail ${String(e).slice(0,80)}\n`, 'utf8')
                }
              }
              if (remaining.length === 0) {
                init = { ...init, body: JSON.stringify(parsed) }
              } else {
                fs.appendFileSync(logPath(),
                  `[req #${seq}] NOT swapped (${remaining.length} problems); sent ORIGINAL\n`, 'utf8')
              }
            }
          } catch (e) {
            fs.appendFileSync(logPath(), `[req #${seq}] parse-fail: ${String(e).slice(0,80)}\n`, 'utf8')
          }
        }
      }
      const resp = await origFetch(input, init)
      if (isLLMReq) {
        try {
          const txt = await resp.text()
          fs.appendFileSync(logPath(),
            `\n[resp #${seq}] status=${resp.status}\n${txt.slice(0, 1500)}\n`, 'utf8')
          try {
            const finalName = path.join(homeDir(),
              `_toolscan_req${String(seq).padStart(2, '0')}_n${nToolsCached}_resp${resp.status}.json`)
            fs.writeFileSync(finalName, JSON.stringify({ status: resp.status, response: txt, tools: toolsCached }, null, 2), 'utf8')
          } catch {}
          return new Response(txt, {
            status: resp.status,
            statusText: resp.statusText,
            headers: resp.headers,
          })
        } catch (e) {
          fs.appendFileSync(logPath(), `\n[resp #${seq}] read-fail: ${String(e)} ===\n`, 'utf8')
        }
      }
      return resp
    }
  }
}

export const name = 'dsh-toolscan'
export const inject = []
export function apply() {
  // nothing to register; the fetch patch above runs at import time.
}
