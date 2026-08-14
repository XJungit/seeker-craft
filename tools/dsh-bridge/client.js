/**
 * dsh-bridge — craft-bot 预设的 viewer 仪表盘内嵌（client 端 / 浏览器半边）。
 *
 * 让 craft-agent-viewer 的 Web 仪表盘（实时 bot 状态：位置/生命/饱食/背包/附近/
 * 会话流）以 iframe 形式内嵌进 DSH 页面，在对话区“页面旁”实时显示。
 *
 * 关键约束（用户明确要求）：**只有 craft-bot 预设（DSH 控制 Minecraft bot 的会话）
 * 才显示**。判断依据：`ctx.sessions.list`（ObservableSnapshot）当前会话的
 * `agentPreset === 'craft-bot'`。其他预设/普通会话里面板保持隐藏，不干扰。
 *
 * 挂载方式（遵循 DSH 官方插件开发文档 §3.4“选择正确的 UI 接缝：slot 优先，body
 * portal 兜底”）：本面板是“跨会话、固定在 shell 角落”的全局面板，故用 body portal +
 * fixed 定位（而非塞进某个语义 slot）。要点：
 *   - host 必须是 DOM 单例（data 属性标记），无论 apply 被调用多少次、模块是否被按
 *     会话重新求值，document 中只存在一个 `#dsh-craft-host`；这是杜绝“开 N 个会话出现
 *     N 个仪表盘”的根本手段。
 *   - 通过 `ctx.sessions.list.subscribe()` 订阅会话变化来显隐（正经做法，替代轮询）。
 *   - 面板打开时把 open 状态广播到 `document.documentElement[data-dsh-craft-open]`，
 *     用 CSS 把对话列 `[data-phase='active']` 右移让位 → 真正“页面旁”，而非遮挡对话。
 *   - 所有跨重求值状态（userClosed / iframeLoaded / 当前是否 craft-bot）都放在 window
 *     上，函数每次重查 DOM，使插件在 DSH 按会话重求值模块时也安全。
 *
 * viewer 地址：默认 http://127.0.0.1:8080，可用 localStorage 覆盖（settings 卡片）。
 * client 端不直接 fetch viewer（跨域），一律走 host 的 /craft/api/* 同源代理。
 *
 * 实现用纯 DOM（无 react-dom 依赖）：DSH client 环境保证 require('react')，但不保证
 * react-dom/client；纯 DOM 与 shell 的 React 子树互不干扰（官方文档同款取舍）。
 *
 * @module dsh-bridge/client
 */

window.__ModuleLoader__.load({
  id: 'dsh-bridge',
  factory: (require) => {
    var module = { exports: {} }
    var exports = module.exports
    Object.defineProperty(exports, Symbol.toStringTag, { value: 'Module' })
    // react 仅作环境兼容占位（DSH client 保证可 require），本插件纯 DOM 渲染不依赖它。
    require('react')

    // ── 常量 ────────────────────────────────────────────────────────────────
    var PRESET_ID = 'craft-bot'
    var VIEWER_DEFAULT = 'http://127.0.0.1:8080'
    var HOST_ATTR = 'data-dsh-craft-host'
    var OPEN_ATTR = 'data-dsh-craft-open' // 挂在 documentElement，驱动对话列让位
    var PANEL_CLS = 'dsh-craft-panel'
    var LAUNCHER_CLS = 'dsh-craft-launcher'
    var W = typeof window !== 'undefined' ? window : {}

    // ── CSS（内联注入，避免额外构建）─────────────────────────────────────────
    // 面板固定右侧停靠 = 真正的“页面旁”；打开时对话列右移让位，不遮挡对话。
    var css =
      '.' + PANEL_CLS + '{position:fixed;top:0;right:0;bottom:0;width:min(720px,46vw);z-index:40;' +
      'display:flex;flex-direction:column;background:var(--dsw-alias-bg-base,#0f1419);' +
      'box-shadow:-2px 0 14px rgba(0,0,0,.4);border-left:1px solid rgba(128,128,128,.25)}' +
      '.' + PANEL_CLS + '[data-hidden]{display:none}' +
      // 对话列让位（“页面旁”而非遮挡）；两条选择器兼容不同 DSH 版本
      'html[' + OPEN_ATTR + '] [data-phase="active"],' +
      'html[' + OPEN_ATTR + '] [data-pane="conversation"]{padding-right:min(720px,46vw);transition:padding-right .18s ease}' +
      '.dsh-craft-bar{display:flex;align-items:center;gap:8px;padding:6px 12px;' +
      'border-bottom:1px solid rgba(128,128,128,.2);font-size:12px;color:var(--dsw-alias-label-secondary,#888)}' +
      '.dsh-craft-bar b{color:inherit;font-weight:600}' +
      '.dsh-craft-bar .spacer{flex:1}' +
      '.dsh-craft-close{cursor:pointer;font-size:12px;padding:2px 10px;border:1px solid rgba(128,128,128,.4);' +
      'border-radius:6px;background:transparent;color:inherit}' +
      '.' + PANEL_CLS + ' iframe{flex:1;width:100%;border:0;background:#0f1419}' +
      // 启动器小标签：仅 craft-bot 且用户关闭时才出现，用于重开（其他会话完全不显示）
      '.' + LAUNCHER_CLS + '{position:fixed;top:12px;right:12px;z-index:41;display:none;' +
      'align-items:center;gap:6px;padding:6px 10px;border-radius:10px;cursor:pointer;' +
      'border:1px solid rgba(74,163,255,.5);background:rgba(74,163,255,.15);color:inherit;font:inherit;font-size:12px}' +
      'html[' + OPEN_ATTR + '] .' + LAUNCHER_CLS + '{display:none}' +
      '.' + LAUNCHER_CLS + '[data-show]{display:flex}'

    if (typeof document !== 'undefined' && document.querySelector('style[data-plugin-css="dsh-bridge"]') === null) {
      var styleTag = document.createElement('style')
      styleTag.dataset.plugin = 'dsh-bridge'
      styleTag.dataset.pluginCss = 'dsh-bridge'
      styleTag.textContent = css
      document.head.appendChild(styleTag)
    }

    // ── viewer 地址解析 ─────────────────────────────────────────────────────
    function viewerUrl() {
      try {
        var saved = localStorage.getItem('dsh-bridge.viewerUrl')
        if (saved && saved.trim().length > 0) return saved.trim()
      } catch (e) { /* localStorage 不可用时忽略 */ }
      return VIEWER_DEFAULT
    }

    // ── 面板 DOM 构建（仅首次，之后复用同一 host）────────────────────────────
    function buildHost() {
      var host = document.createElement('div')
      host.setAttribute(HOST_ATTR, '')

      var panel = document.createElement('div')
      panel.className = PANEL_CLS
      panel.setAttribute('data-hidden', '')
      panel.innerHTML =
        '<div class="dsh-craft-bar">' +
        '<b>Craft Bot 仪表盘</b>' +
        '<span class="dsh-craft-url"></span>' +
        '<span class="spacer"></span>' +
        '<button type="button" class="dsh-craft-close">关闭 ✕</button>' +
        '</div>'
      var iframe = document.createElement('iframe')
      iframe.title = 'Craft-Agent Viewer'
      iframe.setAttribute('sandbox', 'allow-scripts allow-same-origin allow-forms')
      iframe.setAttribute('referrerPolicy', 'no-referrer')
      panel.appendChild(iframe)
      // 关闭：记录 userClosed（跨重求值存 window），按当前 isCraft 重新渲染
      panel.querySelector('.dsh-craft-close').addEventListener('click', function () {
        W.__dshCraftUserClosed = true
        renderCurrent()
      })
      host.appendChild(panel)

      // 启动器：用户关闭后用于重开（仅在 craft-bot 且已关闭时显示）
      var launcher = document.createElement('button')
      launcher.type = 'button'
      launcher.className = LAUNCHER_CLS
      launcher.textContent = '🎮 Craft Bot 仪表盘'
      launcher.addEventListener('click', function () {
        W.__dshCraftUserClosed = false
        renderCurrent()
      })
      host.appendChild(launcher)

      document.body.appendChild(host)
      return host
    }

    // 取已存在的面板/iframe/启动器（模块重求值时也按 DOM 重查，绝不用闭包内旧引用）
    function queryParts() {
      var host = typeof document !== 'undefined' ? document.querySelector('[' + HOST_ATTR + ']') : null
      if (host === null) return null
      var panel = host.querySelector('.' + PANEL_CLS)
      if (panel === null) return null
      return {
        host: host,
        panel: panel,
        iframe: panel.querySelector('iframe'),
        urlSpan: panel.querySelector('.dsh-craft-url'),
        launcher: host.querySelector('.' + LAUNCHER_CLS),
      }
    }

    // ── 显隐渲染（纯 DOM 驱动，重求值安全）──────────────────────────────────
    function setOpen(open, isCraft) {
      var p = queryParts()
      if (p === null) return
      if (open && isCraft && !W.__dshCraftUserClosed) {
        // iframe 只加载一次（保留 viewer 的 SSE 连接）；切换会话只显隐不重载
        if (!W.__dshCraftIframeLoaded && p.iframe) {
          p.iframe.src = viewerUrl() + '/'
          W.__dshCraftIframeLoaded = true
        }
        if (p.urlSpan) p.urlSpan.textContent = viewerUrl()
        p.panel.removeAttribute('data-hidden')
        document.documentElement.setAttribute(OPEN_ATTR, '')
        if (p.launcher) p.launcher.removeAttribute('data-show')
      } else {
        p.panel.setAttribute('data-hidden', '')
        document.documentElement.removeAttribute(OPEN_ATTR)
        // 仅 craft-bot 且用户手动关闭 → 显示启动器以便重开；其余情况（非 craft-bot）什么都不显示
        if (p.launcher) {
          if (isCraft && W.__dshCraftUserClosed) p.launcher.setAttribute('data-show', '')
          else p.launcher.removeAttribute('data-show')
        }
      }
    }

    function renderCurrent() {
      setOpen(true, !!W.__dshCraftIsCraft) // setOpen 内部会按 userClosed 决定最终态
    }

    // ── 插件 apply（client 半边）────────────────────────────────────────────
    /**
     * @param {import('@deepseek-ai/dsh-client-runtime/client').ClientContext} ctx
     */
    function apply(ctx) {
      if (!ctx || !ctx.sessions || !ctx.sessions.list) return

      // DOM 单例 host：已存在则复用（多会话/多次 apply 只一个仪表盘），
      // 不存在才构建；之后所有引用都按 DOM 重查，安全对抗模块重求值。
      var existing = typeof document !== 'undefined' ? document.querySelector('[' + HOST_ATTR + ']') : null
      if (existing === null) buildHost()

      // 订阅会话列表（ObservableSnapshot.subscribe）→ 当前会话切到/离开 craft-bot 时
      // 自动显隐。正经做法，替代脆弱的 setInterval 轮询。
      function sync() {
        var snap
        try { snap = ctx.sessions.list.getSnapshot() } catch (e) { snap = null }
        var currentId = snap && snap.current
        var current = currentId !== undefined && snap.byId ? snap.byId[currentId] : undefined
        var isCraft = !!(current && current.agentPreset === PRESET_ID)
        W.__dshCraftIsCraft = isCraft
        // 离开 craft-bot → 重置手动关闭，下次进入自动重开
        if (!isCraft) W.__dshCraftUserClosed = false
        setOpen(isCraft && !W.__dshCraftUserClosed, isCraft)
      }

      var unsub = null
      try { unsub = ctx.sessions.list.subscribe(sync) } catch (e) { unsub = null }
      sync()

      // 清理本 apply 的订阅（不移除共享 host；面板随页面生命周期存在，符合“页面旁实时显示”）。
      if (typeof ctx.effect === 'function') {
        ctx.effect(function () { if (unsub) { try { unsub() } catch (e) { /* noop */ } } }, 'dsh-bridge: craft session sub')
      }
    }

    exports.inject = ['sessions']
    exports.name = 'dsh-bridge'
    exports.apply = apply
    return module.exports
  },
})
