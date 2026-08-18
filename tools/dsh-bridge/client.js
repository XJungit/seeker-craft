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
 *   - host 必须是 DOM 单例（`[data-dsh-craft-host]`），`apply` 开头用 DOM 守卫：
 *     已存在则返回 no-op disposer（参考 whale-girl 的 `[data-whale-girl]` 守卫）——
 *     无论插件被挂载几次（如全局行 + craft-bot 预设行同时存在），页面中永远只存在
 *     一个仪表盘，根治“多仪表盘”。
 *   - DSH 的 client bundle 是一个 cordis 插件 entry（见 web/src/boot.tsx：一个 plugin
 *     包 = 一个 loader entry = 一次 apply），`apply` 返回的函数即 cordis disposer，
 *     在插件卸载/HMR 时清理订阅、监听、让位与 DOM。
 *   - 通过 `ctx.sessions.list.subscribe()` 订阅会话变化来显隐（正经做法，替代轮询）。
 *   - 面板打开时给 DSH 三列布局的 grid frame 加右侧 padding（JS 动态让位）→ 真正
 *     “页面旁”，而非遮挡对话。稳定锚点是 layout 的 `[data-shell-overlay]` 的父元素
 *     （即 grid frame），不依赖任何哈希类名/易变选择器（真实 DSH 无 data-phase）。
 *   - 面板状态（userOpened / iframeLoaded / 当前是否 craft-bot）放在 window 上，
 *     函数每次重查 DOM，插件生命周期内始终拿到最新状态。
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
      // 面板打开时隐藏启动器（打开状态下右上角不再显示重开标签）
      'html[' + OPEN_ATTR + '] .' + LAUNCHER_CLS + '{display:none}' +
      '.dsh-craft-bar{display:flex;align-items:center;gap:8px;padding:6px 12px;' +
      'border-bottom:1px solid rgba(128,128,128,.2);font-size:12px;color:var(--dsw-alias-label-secondary,#888)}' +
      '.dsh-craft-bar b{color:inherit;font-weight:600}' +
      '.dsh-craft-bar .spacer{flex:1}' +
      // 底部操作栏（关闭按钮放在面板右下角，避免与 DSH 右上角官方 UI 重叠）
      '.dsh-craft-bar.dsh-craft-bar-bottom{justify-content:flex-end;border-bottom:none;border-top:1px solid rgba(128,128,128,.2)}' +
      '.dsh-craft-close{cursor:pointer;font-size:12px;padding:2px 10px;border:1px solid rgba(128,128,128,.4);' +
      'border-radius:6px;background:transparent;color:inherit}' +
      '.' + PANEL_CLS + ' iframe{flex:1;width:100%;border:0;background:#0f1419}' +
      // 启动器小标签：仅 craft-bot 且用户关闭时才出现，用于重开（其他会话完全不显示）。
      // 位置：右边缘垂直居中（不占右上角，避免与 DSH 官方 Session log 等右上角按钮重叠）
      '.' + LAUNCHER_CLS + '{position:fixed;top:50%;right:0;transform:translateY(-50%);z-index:41;display:none;' +
      'align-items:center;gap:6px;padding:8px 5px;border-radius:8px 0 0 8px;cursor:pointer;' +
      'border:1px solid rgba(74,163,255,.5);border-right:none;background:rgba(74,163,255,.15);color:inherit;font:inherit;font-size:12px;' +
      'writing-mode:vertical-rl;letter-spacing:.12em}' +
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
        // 顶部栏：标题 + viewer 地址（关闭按钮已移到底部右下角）
        '<div class="dsh-craft-bar">' +
        '<b>Craft Bot 仪表盘</b>' +
        '<span class="dsh-craft-url"></span>' +
        '<span class="spacer"></span>' +
        '</div>' +
        // 底部栏：把关闭按钮放在面板右下角，避免与 DSH 右上角官方 UI 重叠
        '<div class="dsh-craft-bar dsh-craft-bar-bottom">' +
        '<button type="button" class="dsh-craft-close">关闭 ✕</button>' +
        '</div>'
      var iframe = document.createElement('iframe')
      iframe.title = 'Craft-Agent Viewer'
      iframe.setAttribute('sandbox', 'allow-scripts allow-same-origin allow-forms')
      iframe.setAttribute('referrerPolicy', 'no-referrer')
      panel.appendChild(iframe)
      // 关闭：记录 userOpened=false（用户手动关闭），按当前 isCraft 重新渲染
      panel.querySelector('.dsh-craft-close').addEventListener('click', function () {
        W.__dshCraftUserOpened = false
        renderCurrent()
      })
      host.appendChild(panel)

      // 启动器：仅在 craft-bot 且面板未手动打开时显示，供用户点击手动打开面板
      var launcher = document.createElement('button')
      launcher.type = 'button'
      launcher.className = LAUNCHER_CLS
      launcher.textContent = '🎮 Craft'
      launcher.addEventListener('click', function () {
        W.__dshCraftUserOpened = true
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
    // 面板宽度：与 CSS 的 width:min(720px,46vw) 保持一致，用于给三列 grid frame 让位
    function panelWidthPx() {
      var vw = (typeof window !== 'undefined' && window.innerWidth) || 0
      return Math.round(Math.min(720, vw * 0.46))
    }
    // 给 DSH 三列布局的 frame 加右侧 padding，把面板宽度让出来（“页面旁”而非遮挡）。
    // 稳定锚点：layout 的 overlay layer 带 data-shell-overlay，其父元素就是 grid frame；
    // 不依赖任何哈希类名/易变选择器。frame 找不到时退化为纯 fixed 停靠（可接受）。
    function applyFramePadding(open) {
      if (typeof document === 'undefined') return
      var overlay = document.querySelector('[data-shell-overlay]')
      var frame = overlay && overlay.parentElement
      if (!frame || !frame.style) return
      frame.style.paddingRight = open ? panelWidthPx() + 'px' : ''
    }

    function setOpen(open, isCraft) {
      var p = queryParts()
      if (p === null) return
      // 仅在用户手动打开（userOpened）且处于 craft-bot 会话时显示；进入会话不再自动打开
      var show = open && isCraft && !!W.__dshCraftUserOpened
      applyFramePadding(show)
      if (show) {
        // iframe 只加载一次（保留 viewer 的 SSE 连接）；切换会话只显隐不重载。
        // ?compact=1 = viewer 紧凑模式：隐藏"对话历史/工具调用"两列，只留 bot 实时状态
        if (!W.__dshCraftIframeLoaded && p.iframe) {
          p.iframe.src = viewerUrl() + '/?compact=1'
          W.__dshCraftIframeLoaded = true
        }
        if (p.urlSpan) p.urlSpan.textContent = viewerUrl()
        p.panel.removeAttribute('data-hidden')
        document.documentElement.setAttribute(OPEN_ATTR, '')
        if (p.launcher) p.launcher.removeAttribute('data-show')
      } else {
        p.panel.setAttribute('data-hidden', '')
        document.documentElement.removeAttribute(OPEN_ATTR)
        // 仅 craft-bot 且面板未手动打开 → 显示启动器标签（点击手动打开）；其余情况隐藏
        if (p.launcher) {
          if (isCraft && !W.__dshCraftUserOpened) p.launcher.setAttribute('data-show', '')
          else p.launcher.removeAttribute('data-show')
        }
      }
    }

    function renderCurrent() {
      setOpen(true, !!W.__dshCraftIsCraft) // setOpen 内部会按 userOpened 决定最终态（手动打开才显示）
    }

    // ── 插件 apply（client 半边）────────────────────────────────────────────
    /**
     * DSH 会把 client bundle 当作一个 cordis 插件 entry 挂载：apply 只被调用
     * 一次（每个 plugin 包一个 entry/fiber，见 web/src/boot.tsx），且返回的
     * 函数就是 cordis 的 disposer（插件卸载/HMR 时被调用）。参考优秀实现
     * whale-girl：重复挂载用 DOM 单例守卫直接返回 no-op，杜绝多面板。
     *
     * @param {import('@deepseek-ai/dsh-client-runtime/client').ClientContext} ctx
     * @returns {() => void} disposer
     */
    function apply(ctx) {
      // DOM 单例守卫：无论插件被挂载几次（如全局行 + craft-bot 预设行同时存在），
      // 页面中永远只允许一个仪表盘 host；重复挂载直接返回 no-op disposer。
      if (typeof document !== 'undefined' && document.querySelector('[' + HOST_ATTR + ']') !== null) {
        return function noopDisposer() { /* 已有实例，跳过重复挂载 */ }
      }

      // 兼容两种注入形态：ctx.sessions（声明式）或 ctx.get('sessions')（旧式）。
      // 缺 sessions 时仍建 host（隐藏），但无法订阅显隐——插件声明了 inject:['sessions']，
      // 正常不会走到。
      var sessions = ctx && (ctx.sessions || (typeof ctx.get === 'function' && ctx.get('sessions')))

      // 构建 host（此时必为空，守卫已保证单例）
      buildHost()

      // 订阅会话列表（ObservableSnapshot.subscribe）→ 当前会话切到/离开 craft-bot 时
      // 自动显隐。正经做法，替代脆弱的 setInterval 轮询。
      function sync() {
        // 依赖 DSH client 的 sessions.list（ObservableSnapshot）契约：
        // getSnapshot() 同步返回 { current, byId }，subscribe(fn) 在变更时回调。
        // 若 DSH API 变更此形状，这里是唯一需要同步调整的消费点。
        var snap = null
        try { if (sessions && sessions.list) snap = sessions.list.getSnapshot() } catch (e) { snap = null }
        var currentId = snap && snap.current
        var current = currentId !== undefined && snap.byId ? snap.byId[currentId] : undefined
        var isCraft = !!(current && current.agentPreset === PRESET_ID)
        W.__dshCraftIsCraft = isCraft
        // 显隐完全交给 setOpen：仅在用户手动打开（userOpened）且处于 craft-bot 时显示，
        // 进入会话不再自动打开；非 craft-bot 时隐藏并移除启动器。
        setOpen(true, isCraft)
      }

      var unsub = null
      try { if (sessions && sessions.list && typeof sessions.list.subscribe === 'function') unsub = sessions.list.subscribe(sync) } catch (e) { unsub = null }
      sync()

      // 窗口缩放时重算让位宽度（46vw 随视口变化）
      var onResize = function () { renderCurrent() }
      if (typeof window !== 'undefined' && window.addEventListener) window.addEventListener('resize', onResize)

      // cordis disposer：插件卸载/HMR 时清理订阅、监听、让位与 DOM。
      return function disposer() {
        try { if (unsub) unsub() } catch (e) { /* noop */ }
        try { if (typeof window !== 'undefined' && window.removeEventListener) window.removeEventListener('resize', onResize) } catch (e) { /* noop */ }
        // 恢复 grid frame 让位
        try {
          var overlay = document.querySelector('[data-shell-overlay]')
          var frame = overlay && overlay.parentElement
          if (frame && frame.style) frame.style.paddingRight = ''
        } catch (e) { /* noop */ }
        // 移除面板 DOM 与样式
        try {
          var host = document.querySelector('[' + HOST_ATTR + ']')
          if (host && host.parentElement) host.parentElement.removeChild(host)
        } catch (e) { /* noop */ }
        try {
          var style = document.querySelector('style[data-plugin-css="dsh-bridge"]')
          if (style && style.parentElement) style.parentElement.removeChild(style)
        } catch (e) { /* noop */ }
        try { document.documentElement.removeAttribute(OPEN_ATTR) } catch (e) { /* noop */ }
      }
    }

    exports.inject = ['sessions']
    exports.name = 'dsh-bridge'
    exports.apply = apply
    return module.exports
  },
})
