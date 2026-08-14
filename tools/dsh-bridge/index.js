/**
 * dsh-bridge — craft-bot 预设的 viewer 桥插件（DSH 侧）。
 *
 * 让 DSH（DeepSeek Harness）作为 Minecraft bot 的唯一大脑，经 craft-agent-viewer
 * 的 HTTP API 驱动 live bot。三个工具：
 *
 *   - game_state()               GET  /api/game-state  读取实时世界状态（结构化 + 中文摘要）
 *   - bot_tool(name, args)       POST /api/bot_tool    执行 53 个 Minecraft 工具之一（含 P100/P101/P102/P132 自动修正）
 *   - set_goal(text)             POST /api/goal        设置 bot 的运营目标
 *
 * viewer 地址默认 http://127.0.0.1:8080，可用环境变量 DSH_CRAFT_VIEWER_URL 覆盖。
 * 工具名是 craft-bot 预设 persona 声明的稳定契约，不要改名。
 *
 * @module dsh-bridge
 */

import z from '@deepseek-ai/schemastery'
import { defineTool } from '@deepseek-ai/dsh-tools'

/** Cordis 插件契约：插件名。 */
export const name = 'dsh-bridge'

/** Cordis 插件契约：注入工具注册表。 */
export const inject = ['tools']

/** Cordis 插件契约：可选配置（保留空 schema，未来可加 viewer 地址等）。 */
export const Config = z.object({})

function viewerUrl() {
  const fromEnv = process.env.DSH_CRAFT_VIEWER_URL
  if (fromEnv !== undefined && fromEnv.trim().length > 0) return fromEnv.trim()
  return 'http://127.0.0.1:8080'
}

/** 带超时的 fetch 封装：返回 { ok, status, body }，网络错误转为抛错。 */
async function viewerFetch(path, options = {}, timeoutMs = 60000) {
  const controller = new AbortController()
  const timer = setTimeout(() => controller.abort(), timeoutMs)
  try {
    const res = await fetch(viewerUrl() + path, {
      ...options,
      signal: controller.signal,
      headers: options.body ? { 'content-type': 'application/json', ...(options.headers ?? {}) } : options.headers,
    })
    let body
    try {
      body = await res.json()
    } catch {
      body = null
    }
    return { ok: res.ok, status: res.status, body }
  } finally {
    clearTimeout(timer)
  }
}

/**
 * 渲染一个工具结果：优先透传 message，错误时返回可读诊断。
 * @param {{ok: boolean, message?: string, error?: string, images?: unknown[]}} payload
 * @returns {string}
 */
function renderResult(payload) {
  if (payload.ok) {
    const msg = payload.message ?? '(ok)'
    return msg
  }
  const err = payload.error ?? payload.message ?? 'unknown bridge error'
  return `[bot_tool 失败] ${err}`
}

/** Cordis 插件契约：注册三个桥工具。 */
export function apply(ctx) {
  ctx.tools.register(defineTool({
    name: 'game_state',
    description: '读取 live bot 的实时世界状态（位置/生命/饱食/维度/群系/附近方块与实体/背包/hotbar/装备/世界记忆/警示）。' +
      '每次行动前先调用它感知。返回结构化 JSON（scene_desc 为中文摘要，其余为机器可读字段）。',
    parameters: {},
    output: {
      schema: {
        type: 'object',
        additionalProperties: false,
        properties: {
          scene_desc: { type: 'string', required: true },
          status: { type: 'string', required: true },
        },
      },
      render: (_args, value) => [{ type: 'text', text: value.scene_desc ?? value.status ?? '(no state)' }],
    },
    isConcurrencySafe: () => true,
    async execute() {
      const { ok, status, body } = await viewerFetch('/api/game-state')
      if (!ok || body === null || body === undefined || body.status === 'not_connected') {
        throw new Error(`game_state: viewer 未返回实时状态（HTTP ${status}，body=${JSON.stringify(body ?? null)}）。` +
          '请确认 viewer 已启动且 bot 已连接（craft-agent-ctl status）。')
      }
      return {
        scene_desc: body.scene_desc ?? JSON.stringify(body),
        status: 'ok',
      }
    },
  }))

  ctx.tools.register(defineTool({
    name: 'bot_tool',
    description: '对 live bot 执行一个 Minecraft 工具（name 为 53 个已注册工具之一，见 persona 清单；args 为该工具参数对象）。' +
      '返回 { ok, message }。message 是工具的真实结果；自动修正（挖空气→最近实心、交互→自动靠近≤2.5m）已在 GameTool::execute 内应用，直接传意图目标即可。',
    parameters: {
      name: {
        type: 'string',
        required: true,
        description: '要执行的工具名，如 "perceive"、"goto"、"craft"、"mine"、"equip"、"attack"、"set_mode"。',
      },
      args: {
        type: 'json',
        description: '工具参数对象（如 {"x":1,"y":64,"z":2,"item":"iron_ore"}）。无参数可省略。',
      },
    },
    output: {
      schema: {
        type: 'object',
        additionalProperties: false,
        properties: {
          ok: { type: 'boolean', required: true },
          message: { type: 'string', required: true },
        },
      },
      render: (_args, value) => [{ type: 'text', text: renderResult(value) }],
    },
    async execute(args) {
      const { ok, status, body } = await viewerFetch('/api/bot_tool', {
        method: 'POST',
        body: JSON.stringify({ name: args.name, args: args.args ?? {} }),
      })
      if (!ok) {
        throw new Error(`bot_tool(${args.name}): HTTP ${status}，body=${JSON.stringify(body ?? null)}`)
      }
      return {
        ok: body?.ok === true,
        message: body?.message ?? body?.error ?? '(no message)',
      }
    },
  }))

  ctx.tools.register(defineTool({
    name: 'set_goal',
    description: '设置 live bot 的运营目标（POST /api/goal）。目标会显示在 viewer 状态中，作为 bot 的当前方向。',
    parameters: {
      goal: {
        type: 'string',
        required: true,
        description: '目标文本，如 "收集 24 个铁矿并熔炼成铁锭"。',
      },
    },
    output: {
      schema: {
        type: 'object',
        additionalProperties: false,
        properties: {
          ok: { type: 'boolean', required: true },
          message: { type: 'string', required: true },
        },
      },
      render: (_args, value) => [{ type: 'text', text: value.message }],
    },
    async execute(args) {
      const { ok, status, body } = await viewerFetch('/api/goal', {
        method: 'POST',
        body: JSON.stringify({ goal: args.goal }),
      })
      if (!ok) {
        throw new Error(`set_goal: HTTP ${status}，body=${JSON.stringify(body ?? null)}`)
      }
      return {
        ok: body?.ok === true,
        message: body?.ok ? `目标已设置: ${args.goal}` : `设置失败: ${JSON.stringify(body)}`,
      }
    },
  }))
}
