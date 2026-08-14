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
    var CONVERSATION_COLUMN_SELECTOR = '[data-pane="conversation"]'
    var PANEL_ROOT_ID = 'dsh-craft-panel-root'

    // ── CSS（内联注入，避免额外构建）─────────────────────────────────────────
    var css =
      '.dsh-craft-entry{display:flex;align-items:center;gap:8px;width:100%;padding:8px 12px;' +
      'border:1px solid rgba(128,128,128,.3);border-radius:10px;background:transparent;color:inherit;' +
      'font:inherit;font-size:13px;cursor:pointer;transition:background .15s}' +
      '.dsh-craft-entry:hover{background:rgba(128,128,128,.12)}' +
      '.dsh-craft-entry[data-active]{background:rgba(74,163,255,.15);border-color:rgba(74,163,255,.5)}' +
      '.dsh-craft-entry .dsh-craft-ico{width:14px;height:14px;flex:none}' +
      '.dsh-craft-entry .dsh-craft-label{white-space:nowrap;overflow:hidden;text-overflow:ellipsis}' +
      '.dsh-craft-panel{position:absolute;inset:0;z-index:30;display:flex;flex-direction:column;' +
      'background:var(--dsw-alias-bg-base,#0f1419)}' +
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
      var open = false
      var column = null
      var columnObserver = null

      function ensureColumn() {
        if (column !== null && document.body.contains(column)) return true
        column = document.querySelector(CONVERSATION_COLUMN_SELECTOR)
        if (column === null) return false
        // 面板容器：绝对定位覆盖整个中心列（会话切换时由 DOM 保留，React 不管）
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
        // 中心列需要 position:relative 才能让 absolute 面板覆盖；找不到就 fallback 固定定位。
        var cs = window.getComputedStyle(column)
        if (cs.position === 'static') column.style.position = 'relative'
        column.appendChild(panelEl)
        return true
      }

      function open() {
        open = true
        if (!ensureColumn()) return
        iframeEl.src = viewerUrl() + '/'
        var urlSpan = panelEl.querySelector('.dsh-craft-url')
        if (urlSpan) urlSpan.textContent = viewerUrl()
        panelEl.removeAttribute('data-hidden')
        // 单占用中心列：打开本面板要驱逐兄弟面板（task-board / ssh）
        OTHER_ACTIVE_ATTRS.forEach(function (a) { document.documentElement.removeAttribute(a) })
        document.documentElement.setAttribute(ACTIVE_ATTR, '')
        document.dispatchEvent(new CustomEvent(ACTIVATE_EVENT, { detail: PANEL_NAME }))
        onStateChange && onStateChange(true)
      }

      function close() {
        open = false
        if (panelEl !== null) panelEl.setAttribute('data-hidden', '')
        document.documentElement.removeAttribute(ACTIVE_ATTR)
        onStateChange && onStateChange(false)
      }

      function toggle() {
        if (open) close()
        else open()
      }

      // 等待中心列出现（boot 后 frame 挂载）
      columnObserver = new MutationObserver(function () { if (open) ensureColumn() })
      columnObserver.observe(document.body, { childList: true, subtree: true })

      // 兄弟面板激活时关闭本面板
      function onOtherActivate(e) {
        if (e.detail !== PANEL_NAME && open) close()
      }
      document.addEventListener(ACTIVATE_EVENT, onOtherActivate)

      // 侧边栏点击会话/工作区时关面板（把中心列还给对话）
      var SIDEBAR_ROW = '[class*="sessionRow"], [class*="projectRow"], [class*="newSession"]'
      function onClickSidebar(e) {
        if (!open) return
        var t = e.target
        if (t && t.closest && t.closest(SIDEBAR_ROW) !== null) close()
      }
      document.addEventListener('click', onClickSidebar, true)

      ensureColumn()

      return {
        toggle: toggle,
        open: open,
        close: close,
        isOpen: function () { return open },
        dispose: function () {
          document.removeEventListener(ACTIVATE_EVENT, onOtherActivate)
          document.removeEventListener('click', onClickSidebar, true)
          if (columnObserver !== null) columnObserver.disconnect()
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
      // 仅在 craft-bot 预设会话显示：非 craft-bot 直接返回，不注册任何 UI。
      if (!isCraftBotSession(ctx)) return

      var panel = null
      var entryActive = false

      function setEntryActive(v) {
        entryActive = v
        var els = document.querySelectorAll('[data-dsh-craft-entry]')
        for (var i = 0; i < els.length; i++) {
          if (v) els[i].setAttribute('data-active', 'true')
          else els[i].removeAttribute('data-active')
        }
      }

      function ensurePanel() {
        if (panel === null) panel = createPanelController(ctx, setEntryActive)
        return panel
      }

      // 入口按钮：优先 sidebar.footer.action（list 插槽）。
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
              active: entryActive,
              onClick: function () {
                ensurePanel().toggle()
                setEntryActive(panel.isOpen())
              },
            })
          })
        })
        footerRegistered = true
      } catch (e) {
        footerRegistered = false
      }

      // 兜底：插槽未声明时 DOM 注入到侧边栏底部（task-board sidebar-entry 思路）
      if (!footerRegistered) {
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
              ensurePanel().toggle()
              setEntryActive(panel.isOpen())
            })
          }
          if (entryEl.parentElement !== root) root.appendChild(entryEl)
          placed = true
        }
        entryObserver = new MutationObserver(function () { tryPlace() })
        entryObserver.observe(document.body, { childList: true, subtree: true })
        tryPlace()
        ctx.effect(function () { entryObserver.disconnect() })
      }
    }

    exports.apply = apply
    return module.exports
  },
})
