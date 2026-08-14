/**
 * dsh-bridge — craft-bot 预设的 viewer 仪表盘内嵌（client 端 / 浏览器半边）。
 *
 * 让 craft-agent-viewer 的 Web 仪表盘（实时 bot 状态：位置/生命/饱食/背包/附近/
 * 会话流）以 iframe 形式内嵌进 DSH 页面，在对话区旁边实时显示。
 *
 * 关键约束（用户明确要求）：**只有 craft-bot 预设（DSH 控制 Minecraft bot 的会话）
 * 才显示入口**。判断依据：`ctx.sessions.list.getSnapshot()` 当前会话的
 * `agentPreset === 'craft-bot'`。其他预设/普通会话完全不注册 UI，不干扰。
 *
 * 挂载点：
 *   - 入口按钮 → `sidebar.footer.action`（list 插槽，remote-web-ui 验证过可用；
 *     兜底 DOM 注入侧边栏底部）
 *   - 面板 → DOM 级挂载到中心列（`[data-pane="conversation"]` 尾部），iframe 嵌入
 *     viewer（无 X-Frame-Options，可直接嵌）；用 html data 属性做开合切换，
 *     不动 conversation 的 React 子树（task-board 同款思路）。
 *
 * viewer 地址：默认 http://127.0.0.1:8080，可用 localStorage 覆盖（settings 卡片）。
 * client 端不直接 fetch viewer（跨域），一律走 host 的 /craft/api/* 同源代理。
 *
 * 实现全部用纯 DOM（无 react-dom 依赖）——DSH client 环境只保证 require('react')，
 * 不保证 react-dom/client；纯 DOM 与 shell 的 React 协调互不干扰（task-board
 * sidebar-entry / remote-web-ui 同款取舍）。
 *
 * @module dsh-bridge/client
 */

window.__ModuleLoader__.load({
  id: 'dsh-bridge',
  factory: (require) => {
    var module = { exports: {} }
    var exports = module.exports
    Object.defineProperty(exports, Symbol.toStringTag, { value: 'Module' })
    // react 仅用于组件化入口（若插槽要求 React 组件）；纯 DOM 路径不强制。
    var react = require('react')
    var createElement = react.createElement

    // ── 常量 ────────────────────────────────────────────────────────────────
    var PRESET_ID = 'craft-bot'
    var VIEWER_DEFAULT = 'http://127.0.0.1:8080'
    var ACTIVE_ATTR = 'data-dsh-craft-active'
    var OTHER_ACTIVE_ATTRS = ['data-dsh-taskboard-active', 'data-dsh-ssh-active']
    var ACTIVATE_EVENT = 'dsh-panel-activate'
    var PANEL_NAME = 'craft-viewer'
    var PANEL_ROOT_ID = 'dsh-craft-panel-root'
    // 用户在 craft-bot 会话内手动关闭后，reconcile 不再强制重开（尊重用户）；
    // 离开 craft-bot 再回来会重置该标志（见 reconcile）。模块可能被 DSH 按会话重新求值，
    // 故屏蔹态仅作会话内参考，跨 apply 的唯一真源是 document 中的 #dsh-craft-panel-root 元素。
    var userClosed = false
    // 面板开合时同步所有入口按钮的 active 态（多个 craft-bot 会话各有一个按钮，共享同一面板）
    function syncEntryActive(v) {
      var els = document.querySelectorAll('[data-dsh-craft-entry]')
      for (var i = 0; i < els.length; i++) {
        if (v) els[i].setAttribute('data-active', 'true')
        else els[i].removeAttribute('data-active')
      }
    }
    function onPanelState(v) {
      if (!v) userClosed = true
      syncEntryActive(v)
    }

    // ── CSS（内联注入，避免额外构建）─────────────────────────────────────────
    var css =
      '.dsh-craft-entry{display:flex;align-items:center;gap:8px;width:100%;padding:8px 12px;' +
      'border:1px solid rgba(128,128,128,.3);border-radius:10px;background:transparent;color:inherit;' +
      'font:inherit;font-size:13px;cursor:pointer;transition:background .15s}' +
      '.dsh-craft-entry:hover{background:rgba(128,128,128,.12)}' +
      '.dsh-craft-entry[data-active]{background:rgba(74,163,255,.15);border-color:rgba(74,163,255,.5)}' +
      '.dsh-craft-entry .dsh-craft-ico{width:14px;height:14px;flex:none}' +
      '.dsh-craft-entry .dsh-craft-label{white-space:nowrap;overflow:hidden;text-overflow:ellipsis}' +
      '.dsh-craft-panel{position:fixed;top:0;right:0;bottom:0;width:min(720px,46vw);z-index:30;' +
      'display:flex;flex-direction:column;background:var(--dsw-alias-bg-base,#0f1419);' +
      'box-shadow:-2px 0 14px rgba(0,0,0,.4);border-left:1px solid rgba(128,128,128,.25)}' +
      '.dsh-craft-panel[data-hidden]{display:none}' +
      '.dsh-craft-panel-bar{display:flex;align-items:center;gap:8px;padding:6px 12px;' +
      'border-bottom:1px solid rgba(128,128,128,.2);font-size:12px;color:var(--dsw-alias-label-secondary,#888)}' +
      '.dsh-craft-panel-bar b{color:inherit;font-weight:600}' +
      '.dsh-craft-panel-bar .spacer{flex:1}' +
      '.dsh-craft-close{cursor:pointer;font-size:12px;padding:2px 10px;border:1px solid rgba(128,128,128,.4);' +
      'border-radius:6px;background:transparent;color:inherit}' +
      '.dsh-craft-panel iframe{flex:1;width:100%;border:0;background:#0f1419}'

    if (typeof document !== 'undefined' && document.querySelector('style[data-plugin-css="dsh-bridge/panel"]') === null) {
      var tag = document.createElement('style')
      tag.dataset.plugin = 'dsh-bridge'
      tag.dataset.pluginCss = 'dsh-bridge/panel'
      tag.textContent = css
      document.head.appendChild(tag)
    }

    // ── viewer 地址解析 ─────────────────────────────────────────────────────
    function viewerUrl() {
      try {
        var saved = localStorage.getItem('dsh-bridge.viewerUrl')
        if (saved && saved.trim().length > 0) return saved.trim()
      } catch (e) { /* localStorage 不可用时忽略 */ }
      return VIEWER_DEFAULT
    }

    // ── 会话预设判断（核心约束：仅 craft-bot 显示）──────────────────────────
    function isCraftBotSession(ctx) {
      try {
        var sessions = ctx.get('sessions') || ctx.sessions
        if (!sessions || typeof sessions.list === 'undefined') return false
        var snap = sessions.list.getSnapshot()
        var currentId = snap && snap.current
        var current = currentId !== undefined && snap.byId ? snap.byId[currentId] : undefined
        if (current && current.agentPreset === PRESET_ID) return true
        // 兜底：没有 current（空列表/加载中）时按可见会话判断——若所有会话
        // 都是 craft-bot 也视为 craft-bot（多会话场景）。
        if (snap && snap.byId) {
          var ids = Object.keys(snap.byId)
          if (ids.length > 0 && ids.every(function (id) {
            var e = snap.byId[id]
            return e && e.agentPreset === PRESET_ID
          })) return true
        }
        return false
      } catch (e) {
        return false
      }
    }

    // ── 入口按钮（React 组件，供插槽使用）────────────────────────────────────
    function CraftEntry(props) {
      return createElement('button', {
        type: 'button',
        className: 'dsh-craft-entry',
        'data-dsh-craft-entry': '',
        'data-active': props.active ? 'true' : undefined,
        onClick: props.onClick,
        title: '打开 Craft Bot 仪表盘',
      },
        createElement('svg', {
          className: 'dsh-craft-ico',
          viewBox: '0 0 16 16',
          fill: 'none',
          stroke: 'currentColor',
          strokeWidth: '1.3',
          strokeLinecap: 'round',
          strokeLinejoin: 'round',
          'aria-hidden': 'true',
        },
          createElement('rect', { x: '1.5', y: '3', width: '13', height: '10', rx: '1.5' }),
          createElement('path', { d: 'M4.5 6h3M4.5 8.5h5M4.5 11h2' }),
        ),
        createElement('span', { className: 'dsh-craft-label' }, props.wide === false ? 'MC' : 'Craft Bot 仪表盘'),
      )
    }

    // ── 面板控制器（纯 DOM 挂载到中心列）────────────────────────────────────
    function createPanelController(ctx, onStateChange) {
      var panelEl = null
      var iframeEl = null
      var visible = false

      function ensureRoot() {
        // 跨多次 apply（多会话）复用同一面板元素，避免重复仪表盘
        var existing = typeof document !== 'undefined' ? document.getElementById(PANEL_ROOT_ID) : null
        if (existing !== null) {
          panelEl = existing
          iframeEl = existing.querySelector('iframe')
          return true
        }
        if (panelEl !== null && document.body.contains(panelEl)) return true
        // 面板容器：固定右侧停靠（"页面旁"），不扰动对话列 React 子树
        panelEl = document.createElement('div')
        panelEl.id = PANEL_ROOT_ID
        panelEl.className = 'dsh-craft-panel'
        panelEl.setAttribute('data-hidden', '')
        panelEl.innerHTML =
          '<div class="dsh-craft-panel-bar">' +
          '<b>Craft Bot 仪表盘</b>' +
          '<span class="dsh-craft-url"></span>' +
          '<span class="spacer"></span>' +
          '<button type="button" class="dsh-craft-close">关闭 ✕</button>' +
          '</div>'
        iframeEl = document.createElement('iframe')
        iframeEl.title = 'Craft-Agent Viewer'
        iframeEl.setAttribute('sandbox', 'allow-scripts allow-same-origin allow-forms')
        iframeEl.setAttribute('referrerPolicy', 'no-referrer')
        panelEl.appendChild(iframeEl)
        panelEl.querySelector('.dsh-craft-close').addEventListener('click', function () { close() })
        document.body.appendChild(panelEl)
        return true
      }

      function open() {
        visible = true
        if (!ensureRoot()) return
        iframeEl.src = viewerUrl() + '/'
        var urlSpan = panelEl.querySelector('.dsh-craft-url')
        if (urlSpan) urlSpan.textContent = viewerUrl()
        panelEl.removeAttribute('data-hidden')
        // 单占用右侧栏：打开本面板驱逐兄弟面板（task-board / ssh）
        OTHER_ACTIVE_ATTRS.forEach(function (a) { document.documentElement.removeAttribute(a) })
        document.documentElement.setAttribute(ACTIVE_ATTR, '')
        document.dispatchEvent(new CustomEvent(ACTIVATE_EVENT, { detail: PANEL_NAME }))
        onStateChange && onStateChange(true)
      }

      function close() {
        visible = false
        if (panelEl !== null) panelEl.setAttribute('data-hidden', '')
        document.documentElement.removeAttribute(ACTIVE_ATTR)
        onStateChange && onStateChange(false)
      }

      function toggle() {
        if (visible) close()
        else open()
      }

      // 兄弟面板激活时关闭本面板
      function onOtherActivate(e) {
        if (e.detail !== PANEL_NAME && visible) close()
      }
      document.addEventListener(ACTIVATE_EVENT, onOtherActivate)

      // 侧边栏点击会话/工作区时关面板（把右侧栏还给对话）
      var SIDEBAR_ROW = '[class*="sessionRow"], [class*="projectRow"], [class*="newSession"]'
      function onClickSidebar(e) {
        if (!visible) return
        var t = e.target
        if (t && t.closest && t.closest(SIDEBAR_ROW) !== null) close()
      }
      document.addEventListener('click', onClickSidebar, true)

      ensureRoot()

      return {
        toggle: toggle,
        open: open,
        close: close,
        isOpen: function () { return visible },
        dispose: function () {
          document.removeEventListener(ACTIVATE_EVENT, onOtherActivate)
          document.removeEventListener('click', onClickSidebar, true)
          if (panelEl !== null && panelEl.parentElement) panelEl.parentElement.removeChild(panelEl)
          document.documentElement.removeAttribute(ACTIVE_ATTR)
        },
      }
    }

    // ── 插件 apply（client 半边）────────────────────────────────────────────
    /**
     * @param {import('@deepseek-ai/dsh-client-runtime/client').ClientContext} ctx
     */
    function apply(ctx) {
      // 仅在 craft-bot 预设会话注册 UI；其他预设/普通会话直接返回（不控制 bot 时不显示）。
      if (!isCraftBotSession(ctx)) return

      // 面板：DOM 级单例——无论 apply 被调用多少次、模块是否被按会话重新求值，
      // document 中只存在一个 #dsh-craft-panel-root；createPanelController 内的 ensureRoot
      // 会复用已存在元素，从根本上杜绝"开 N 个会话出现 N 个仪表盘"。
      var panel = createPanelController(ctx, onPanelState)
      panel.open() // 自动打开（"页面旁"实时显示 bot 状态）

      // reconcile：用 window 句柄做单例，跨模块重求值/多次 apply 只跑一个定时器；
      // 离开 craft-bot 收起、切回自动重开（除非用户在当前 craft-bot 会话手动关闭过）。
      if (typeof window !== 'undefined') {
        if (!window.__dshCraftReconcile) {
          window.__dshCraftReconcile = setInterval(function () {
            var isCraft = isCraftBotSession(ctx)
            if (!isCraft) {
              userClosed = false
              if (panel.isOpen()) panel.close()
            } else if (!userClosed && !panel.isOpen()) {
              panel.open()
            }
          }, 1000)
        }
      }

      // 入口按钮：每个 craft-bot 会话注册一个（各自侧边栏）；点击开合共享面板。
      var footerRegistered = false
      try {
        ctx.slots.inject('sidebar.footer.action', function () {
          return ctx.slots.register({
            name: 'sidebar.footer.action',
            id: 'dsh-bridge',
            order: 80,
            locale: 'dsh-bridge',
            inject: function () { return {} },
          }, function FooterEntry(props) {
            return createElement(CraftEntry, {
              wide: props && props.wide !== false,
              active: panel.isOpen(),
              onClick: function () {
                if (panel.isOpen()) { userClosed = true; panel.close() }
                else { userClosed = false; panel.open() }
              },
            })
          })
        })
        footerRegistered = true
      } catch (e) {
        footerRegistered = false
      }

      // 兜底：插槽未声明时 DOM 注入到侧边栏底部（带单例判断，避免重复按钮）
      if (!footerRegistered && document.querySelector('[data-dsh-craft-entry]') === null) {
        var entryEl = null
        var placed = false
        var entryObserver = null
        function tryPlace() {
          if (placed && document.body.contains(entryEl)) return
          var column = document.querySelector('[data-pane="sidebar"], [class*="sidebarCol"]')
          if (column === null) return
          var root = column.querySelector('[class*="logoRow"]')?.parentElement || column.firstElementChild
          if (root === undefined || root === null) return
          if (entryEl === null) {
            entryEl = document.createElement('button')
            entryEl.type = 'button'
            entryEl.className = 'dsh-craft-entry'
            entryEl.dataset.dshCraftEntry = ''
            entryEl.innerHTML = '<span class="dsh-craft-label">Craft Bot 仪表盘</span>'
            entryEl.addEventListener('click', function () {
              if (panel.isOpen()) { userClosed = true; panel.close() }
              else { userClosed = false; panel.open() }
            })
          }
          if (entryEl.parentElement !== root) root.appendChild(entryEl)
          placed = true
        }
        entryObserver = new MutationObserver(function () { tryPlace() })
        entryObserver.observe(document.body, { childList: true, subtree: true })
        tryPlace()
        if (typeof ctx.effect === 'function') ctx.effect(function () { entryObserver.disconnect() })
      }
    }

    exports.apply = apply
    return module.exports
  },
})
