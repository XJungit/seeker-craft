/**
 * dsh-bridge — craft-bot 预设的 viewer 仪表盘内嵌（client 端 / 浏览器半边）。
 *
 * 让 craft-agent-viewer 的 Web 仪表盘（实时 bot 状态：位置/生命/饱食/背包/附近/
 * 会话流）以 iframe 形式内嵌进 DSH 页面，在对话区“页面旁”实时显示。
 *
 * 关键约束（用户明确要求）：**只有 craft-bot 预设（DSH 控制 Minecraft bot 的会话）
 * 才显示**。判断依据：当前主会话（mainView 保留的会话）的 `agentPreset`。
 * agentPreset 可能藏在 projectionValues 里（projectList 只显式拷贝部分顶层字段，
 * agentPreset 进了 projectionValues），顶层与 projectionValues 两层都查。
 * 其他预设/普通会话里面板保持隐藏，不干扰。
 *
 * 双模式（placement 可配置，见下 PLACEMENT）：
 *   - 'overlay'（默认）：body portal + fixed 右侧停靠（原有行为，保持不动）；
 *   - 'sidebar'：注册官方 sidebar-right tab 类型（kind='craft-bot'，extension 段），
 *     body 走 keyed slot `sidebar.right.pane.tab`（key=TYPE_ID），guide 走
 *     tab 定义的 guide 条目（点选即 openTab）。sidebar 模式下不建 body-portal
 *     host、不碰 grid frame padding、不显示 🎮 启动器——打开位置完全交给官方
 *     sidebar（dock/float/split 都由用户在侧栏里自己排）。
 *   - 'both'：两种同时开。
 *
 * 挂载细节（遵循 DSH 官方插件开发文档 §3.4“选择正确的 UI 接缝：slot 优先，body
 * portal 兜底”，以及 cordis-plugin-development skill 的 practices.md）：
 *   - overlay host 必须是 DOM 单例（`[data-dsh-craft-host]`），`apply` 开头用 DOM
 *     守卫：已存在则返回 no-op disposer（参考 whale-girl 的 `[data-whale-girl]`
 *     守卫）——无论插件被挂载几次，页面中永远只存在一个仪表盘，根治“多仪表盘”。
 *   - DSH 的 client bundle 是一个 cordis 插件 entry（一个 plugin 包 = 一个 loader
 *     entry = 一次 apply），`apply` 返回的函数即 cordis disposer，在插件卸载/HMR
 *     时清理订阅、监听、让位与 DOM。
 *   - 通过 `ctx.sessions.list.subscribe()` 订阅会话变化来显隐（正经做法，替代轮询）。
 *   - overlay 打开时给 DSH 三列布局的 grid frame 加右侧 padding（JS 动态让位）→
 *     真正“页面旁”，而非遮挡对话。稳定锚点是 layout 的 `[data-shell-overlay]`
 *     的父元素（即 grid frame），不依赖任何哈希类名/易变选择器。
 *   - 面板状态（userOpened / iframeLoaded / 当前是否 craft-bot）放在 window 上，
 *     函数每次重查 DOM，插件生命周期内始终拿到最新状态。
 *   - sidebar body 是 React 组件（slot 体系只渲染 React）：React 取自浏览器模块
 *     表 `require('react')`（官方 ui-plugin.md：禁止自带 React/CDN/UMD）；iframe
 *     用原生 <iframe> 元素，tab 卸载即销毁（keepMounted 省略=默认 false）。
 *   - 不 require 任何 `@deepseek-ai/*` client 包（practices.md 明令禁止：无类型
 *     检查、随时变更、抛错会 blank 整个 slot entry）。只用 ctx 注入的服务
 *     （sessions/slots/sidebarRight/sidebarRightTabs）与 slot 传给 body 的标准
 *     props（useSessions/useTabInfo）。
 *
 * 主会话判定（sessions.list 快照实测结论）：
 *   - `SessionListState = { ids, byId, phase, projectionsBySession }` —— **没有
 *     `current` 字段**（见 service.d.ts:43-52）。旧代码读 `snap.current` 永远
 *     undefined，是“预设下没有仪表盘”的直接原因之一。
 *   - 主视图会话 = `byId` 里 `retainedBy.mainView > 0` 的行（ui-session 的
 *     publishMain 就是这么找的，见 client.js:279-290）。本插件沿用同一规则。
 *   - 兜底链：mainView 行 → 无 mainView 行时若 `ids` 仅一项则取该项 → 否则判
 *     非 craft（两侧都隐藏，不误伤其他预设）。
 *
 * viewer 地址：默认 http://127.0.0.1:8080，可用 localStorage 覆盖。
 * client 端不直接 fetch viewer（跨域），一律走 host 的 /craft/api/* 同源代理。
 *
 * @module dsh-bridge/client
 */

window.__ModuleLoader__.load({
  id: 'dsh-bridge',
  factory: (require) => {
    var module = { exports: {} }
    var exports = module.exports
    Object.defineProperty(exports, Symbol.toStringTag, { value: 'Module' })
    // React 取自浏览器模块表（shell 预置的 seed，见 web 前端 WS()：react /
    // react/jsx-runtime / react-dom 都在表里）。plain-JS 手写 createElement，
    // 无 JSX/TS 构建步骤。注意：只 require 'react'，不 require 'react-dom' 与
    // 任何 @deepseek-ai/* 包（practices.md 禁止事项）。
    var React = require('react')

    // ── 常量 ────────────────────────────────────────────────────────────────
    // 本部署里 Craft-Agent 脑会话可能挂在多个 preset id 下（会话头实测出现过
    // 'craft-bot' 与 'code'——后者 header 写 code 却装载 craft-bot persona），
    // 用名单匹配而非单一 id，避免换个预设名就静默失联。
    var W = typeof window !== 'undefined' ? window : {}
    var PRESET_IDS = ['craft-bot', 'code']
    // 浏览器指纹：控制台读 window.__dshCraftBuild 即可确认载入的是哪一版 bundle，
    // 改一次 client.js 就 bump 一次（字母递增），排障时先对指纹再谈逻辑。
    // 'h' = 默认 placement 改为 both（侧栏 tab + 浮层并存），并修正两处失实注释
    W.__dshCraftBuild = '2026-09-05-h'
    var VIEWER_DEFAULT = 'http://127.0.0.1:8080'
    var HOST_ATTR = 'data-dsh-craft-host'
    var OPEN_ATTR = 'data-dsh-craft-open' // 挂在 documentElement，驱动对话列让位
    var PANEL_CLS = 'dsh-craft-panel'
    var LAUNCHER_CLS = 'dsh-craft-launcher'
    // sidebar tab 类型标识：kind 是 openTab 用的名字（全局唯一，取 craft-bot）；
    // id 是实现标识（keyed slot 的 dispatch key，用包名前缀避免与官方 id 碰撞）。
    var TAB_KIND = 'craft-bot'
    var TYPE_ID = 'dsh-preset-craft-bot/craft-bot'
    // placement 配置键（localStorage）：'sidebar' | 'both' | 'overlay'，默认 'both'
    var PLACEMENT_KEY = 'dsh-bridge.placement'

    // ── placement（双模式开关）──────────────────────────────────────────────
    // 存 localStorage，用户在控制台改即可生效，无需重装 bundle。
    // 改法（本插件不注册 settings 卡片，只有这两个入口）：
    //   localStorage.setItem('dsh-bridge.placement','sidebar'); window.__dshCraftRefresh()
    //   （跨 tab 时 storage 事件会自动生效，无需手动 refresh）
    // 默认 'both'：侧边栏出 tab（进 craft 会话自动 openTab）+ overlay 浮层
    // （用 🎮 启动器手动开）两种都在，用户可按需切到单一模式：
    //   localStorage.setItem('dsh-bridge.placement','sidebar')  → 只要侧栏
    //   localStorage.setItem('dsh-bridge.placement','overlay')  → 只要浮层
    // 为什么默认不是单模式：sidebar 是官方 UI 接缝（可 dock/float/split、由用户
    // 自己排布），overlay 是不依赖任何 sidebar 服务的兜底；两者同时在时互不干扰
    // （overlay 显隐仍受 userOpened 门控，不会自己弹出来抢注意力）。
    // 非法值同样回落 'both'，保证行为可预期。
    function placement() {
      try {
        var saved = localStorage.getItem(PLACEMENT_KEY)
        if (saved === 'sidebar' || saved === 'both' || saved === 'overlay') return saved
      } catch (e) { /* localStorage 不可用时忽略 */ }
      return 'both'
    }
    function wantOverlay() { var p = placement(); return p === 'overlay' || p === 'both' }
    function wantSidebar() { var p = placement(); return p === 'sidebar' || p === 'both' }

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
      '.' + LAUNCHER_CLS + '{position:fixed;top:calc(50% + 72px);right:0;transform:translateY(-50%);z-index:41;display:none;' +
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

    // ── 主会话判定 ──────────────────────────────────────────────────────────
    // sessions.list 快照无 current（service.d.ts:43-52）；主视图会话 = byId 里
    // retainedBy.mainView > 0 的行（与 ui-session publishMain 同规则）。
    function mainRow(snap) {
      if (!snap || !snap.byId) return undefined
      var byId = snap.byId
      var main = undefined
      var ids = Array.isArray(snap.ids) ? snap.ids : Object.keys(byId)
      for (var i = 0; i < ids.length; i++) {
        var row = byId[ids[i]]
        if (row && row.retainedBy && (row.retainedBy.mainView || 0) > 0) { main = row; break }
      }
      // 兜底：没有 mainView 行、但列表仅一项 → 取该项（单会话页常见）。
      if (main === undefined && ids.length === 1 && byId[ids[0]]) main = byId[ids[0]]
      // 兼容 items 数组形态：某些版本快照用 items 列表而非 byId 字典
      if (main === undefined && snap && Object.prototype.toString.call(snap.items) === '[object Array]') {
        for (var bi = 0; bi < snap.items.length; bi++) {
          var it = snap.items[bi]
          if (it && it.retainedBy && (it.retainedBy.mainView || 0) > 0) { main = it; break }
        }
        if (main === undefined && snap.items.length === 1) main = snap.items[0]
      }
      return main
    }
    // agentPreset 等同于挂载的 composition id，本部署里 Craft 脑挂 'craft-bot'
    // 或 'code'（见上名单）。不做 startsWith/包含匹配——短名包含极易跨预设误命中；
    // 日后新增挂载位时只改上名单。
    // agentPreset 可能藏在 projectionValues 里（新版 projectList 只显式拷贝
    // 部分顶层字段，agentPreset 进了 projectionValues），两层都查。
    function presetOf(row) {
      if (!row) return undefined
      if (row.agentPreset !== undefined) return row.agentPreset
      var pv = row.projectionValues || null
      if (pv && pv.agentPreset !== undefined) return pv.agentPreset
      return undefined
    }

    // ── overlay 面板 DOM 构建（仅首次，之后复用同一 host）────────────────────
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
      // 注意：sandbox 不能同时给 allow-scripts + allow-same-origin（沙箱逃逸警告）。
      // viewer 是同源直连（http://127.0.0.1:8080），跨域靠 viewer 侧 CORS 头
      // （Access-Control-Allow-Origin: *，不透明源 origin null 照样放行）解决，
      // 所以只保留 allow-scripts + allow-forms；故意不加 allow-same-origin，iframe
      // 以不透明源运行，即使其中脚本有漏洞也无法逃逸沙箱。
      iframe.setAttribute('sandbox', 'allow-scripts allow-forms')
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

    // ── overlay 显隐渲染（纯 DOM 驱动，重求值安全）───────────────────────────
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
      // overlay 模式被关掉时：面板强制隐藏（sidebar 接管显示），让位恢复。
      if (!wantOverlay()) open = false
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
          if (isCraft && !W.__dshCraftUserOpened && wantOverlay()) p.launcher.setAttribute('data-show', '')
          else p.launcher.removeAttribute('data-show')
        }
      }
    }

    function renderCurrent() {
      setOpen(true, !!W.__dshCraftIsCraft) // setOpen 内部会按 userOpened 决定最终态（手动打开才显示）
    }

    // ── sidebar tab body（React，slot 体系只渲染 React 组件）─────────────────
    // props 说明（按 slots.d.ts 的 SidebarRightTabInjected + SessionStandardProps，
    // 由 ui-session / sidebar-right 声明合并后注入——运行时只认存在性，不认类型）：
    //   useSessions(select) —— 全局席：会话列表快照（含 byId/retainedBy.mainView）；
    //   useTabInfo() —— tab  occurrence 信息（本 body 不需要 tab 动作，仅取 refresh 信号位）。
    // 预设门控同样走主会话判定：非 craft-bot 时 body 渲染提示行（tab 本体由 openTab
    // 调用方控制开关，见下 syncSidebar；body 内二次门控防止“别的会话恢复出 tab”）。
    function CraftSidebarBody(props) {
      var useSessions = props && props.useSessions
      var preset = null
      try {
        if (typeof useSessions === 'function') {
          preset = useSessions(function (state) {
            var byId = (state && state.byId) || {}
            var ids = (state && state.ids) || Object.keys(byId)
            var main = null
            for (var i = 0; i < ids.length; i++) {
              var row = byId[ids[i]]
              if (row && row.retainedBy && (row.retainedBy.mainView || 0) > 0) { main = row; break }
            }
            if (!main && ids.length === 1) main = byId[ids[0]]
            if (!main) return null
            if (main.agentPreset !== undefined) return main.agentPreset
            var pv = main.projectionValues || null
            return (pv && pv.agentPreset !== undefined) ? pv.agentPreset : null
          })
        }
      } catch (e) { preset = null }
      var isCraft = !!(preset && PRESET_IDS.indexOf(preset) !== -1)
      if (!isCraft) {
        return React.createElement('div', { style: { padding: '24px 16px', color: 'var(--dsw-alias-label-tertiary,#888)', fontSize: '12px' } },
          'Craft Bot 仪表盘仅在 craft-bot 预设会话中可用。')
      }
      return React.createElement('iframe', {
        title: 'Craft-Agent Viewer',
        src: viewerUrl() + '/?compact=1',
        sandbox: 'allow-scripts allow-forms',
        referrerPolicy: 'no-referrer',
        style: { width: '100%', height: '100%', border: '0', background: '#0f1419', flex: '1' },
      })
    }

    function CraftSidebarTitle() {
      return React.createElement('span', null, '🎮 Craft Bot')
    }

    // ── 插件 apply（client 半边）────────────────────────────────────────────
    /**
     * DSH 会把 client bundle 当作一个 cordis 插件 entry 挂载：apply 只被调用
     * 一次（每个 plugin 包一个 entry/fiber），且返回的函数就是 cordis 的
     * disposer（插件卸载/HMR 时被调用）。参考优秀实现 whale-girl：重复挂载用
     * DOM 单例守卫直接返回 no-op，杜绝多面板。
     *
     * inject 是两个不同平面，别写反（见文件末尾 exports.inject 处的长注释）：
     *   · package.json dsh.client.inject = **包名**（boot 图到达顺序）
     *     api-session-controller（sessions）+ ui-renderer（slots）+
     *     ui-sidebar-right（sidebarRight/sidebarRightTabs）；
     *   · exports.inject = **服务名**（fiber 激活门）。
     * 本处 apply 用服务名取 ctx（ctx.sessions / ctx.slots / ctx.sidebarRight /
     * ctx.sidebarRightTabs），sidebar placement 关掉时不触碰 sidebar 服务、
     * 仅订阅 sessions。
     *
     * @param {import('@deepseek-ai/dsh-client-runtime/client').ClientContext} ctx
     * @returns {() => void} disposer
     */
    function applyInner(ctx) {
      // DOM 单例守卫：无论插件被挂载几次（如全局行 + craft-bot 预设行同时存在），
      // 页面中永远只允许一个仪表盘 host；重复挂载直接返回 no-op disposer。
      if (typeof document !== 'undefined' && document.querySelector('[' + HOST_ATTR + ']') !== null) {
        return function noopDisposer() { /* 已有实例，跳过重复挂载 */ }
      }

      // 兼容两种注入形态：ctx.sessions（声明式）或 ctx.get('sessions')（旧式）。
      var sessions = ctx && (ctx.sessions || (typeof ctx.get === 'function' && ctx.get('sessions')))
      var slots = ctx && (ctx.slots || (typeof ctx.get === 'function' && ctx.get('slots')))
      var sidebarRight = ctx && (ctx.sidebarRight || (typeof ctx.get === 'function' && ctx.get('sidebarRight')))
      var sidebarRightTabs = ctx && (ctx.sidebarRightTabs || (typeof ctx.get === 'function' && ctx.get('sidebarRightTabs')))

      // overlay host：sidebar-only  placement 下不建（sidebar 接管显示），避免
      // body 里多一个永远隐藏的 fixed 节点。
      if (wantOverlay()) buildHost()

      var unsub = null
      var disposers = []
      function track(d) { if (typeof d === 'function') disposers.push(d) }

      // ── sidebar tab 类型注册（仅 wantSidebar，且服务齐备才注册）───────────
      // kind='craft-bot' 全局唯一（官方 terminal/browser/files 均用短名，无碰撞）；
      // priority 省略即 'extension'（tab-registry.d.ts:102：不声明就是 extension 段，
      // 与 builtin 不冲突）；multiple 省略=每 pane 一页（够用，不开多实例）；
      // guide 条目让用户在侧栏 guide 页点选打开；title 走定义 title()，另注册
      // keyed title 组件做实时标题。
      var sidebarOn = false
      if (wantSidebar() && slots && sidebarRightTabs) {
        try {
          track(ctx.effect(function () {
            var disposeType = null
            var disposeBody = null
            var disposeTitle = null
            try {
              disposeType = sidebarRightTabs.register({
                id: TYPE_ID,
                kind: TAB_KIND,
                title: function () { return 'Craft Bot' },
                guide: [{
                  id: 'open',
                  order: 20,
                  title: function () { return 'Craft Bot 仪表盘' },
                  description: function () { return 'Minecraft bot 实时状态（viewer）' },
                }],
              })
            } catch (e) { disposeType = null }
            try {
              disposeBody = slots.inject('sidebar.right.pane.tab', function () {
                return slots.register({ name: 'sidebar.right.pane.tab', key: TYPE_ID }, CraftSidebarBody)
              })
            } catch (e) { disposeBody = null }
            try {
              disposeTitle = slots.inject('sidebar.right.pane.tab.title', function () {
                return slots.register({ name: 'sidebar.right.pane.tab.title', key: TYPE_ID }, CraftSidebarTitle)
              })
            } catch (e) { disposeTitle = null }
            sidebarOn = !!(disposeType || disposeBody || disposeTitle)
            return function () {
              sidebarOn = false
              try { if (disposeTitle) disposeTitle() } catch (e) { /* noop */ }
              try { if (disposeBody) disposeBody() } catch (e) { /* noop */ }
              try { if (disposeType) disposeType() } catch (e) { /* noop */ }
            }
          }, 'dsh-bridge: sidebar tab'))
        } catch (e) { sidebarOn = false }
      }

      // 订阅会话列表（ObservableSnapshot.subscribe）→ 当前主会话切到/离开
      // craft-bot 时：overlay 显隐 + sidebar openTab（正经做法，替代轮询）。
      // sidebar 的 open 由订阅驱动：进 craft 主会话 → openTab（幂等，去重由
      // sidebar 自己做，pages always deduplicate）；离 craft → 不自动关（用户
      // 的 tab 归用户，关闭走 tab 自身 ×；body 内二次门控防“恢复出 tab”）。
      function sync() {
        var snap = null
        try { if (sessions && sessions.list) snap = sessions.list.getSnapshot() } catch (e) { snap = null }
        var row = mainRow(snap)
        var ap = presetOf(row)
        var isCraft = !!(ap && PRESET_IDS.indexOf(ap) !== -1)
        // 一次性诊断探针：把原始快照形状写到 window，用户控制台
        // JSON.stringify(window.__dshCraftDbg) 一次即可看到全部真相。
        try {
          var pv = (row && row.projectionValues) || null
          var mainId = (row && (row.id || row.sessionId)) || null
          W.__dshCraftDbg = {
            build: W.__dshCraftBuild || null,
            mainId: mainId === undefined ? null : (mainId === null ? null : String(mainId)),
            keys: row ? Object.keys(row).slice(0, 24) : [],
            agentPreset: ap === undefined ? null : ap,
            pvKeys: pv ? Object.keys(pv).slice(0, 24) : [],
            pvPreset: (pv && pv.agentPreset !== undefined) ? pv.agentPreset : null,
            retainedBy: (row && row.retainedBy) || null,
            cwd: (row && row.cwd !== undefined) ? String(row.cwd).slice(0, 80) : null,
            placement: placement(),
            sidebarOn: !!sidebarOn,
            ids: (snap && snap.ids) ? snap.ids.slice(0, 8) : null
          }
        } catch (dbgE) { /* 诊断失败不影响主逻辑 */ }
        W.__dshCraftIsCraft = isCraft
        // overlay：显隐完全交给 setOpen：仅在用户手动打开（userOpened）且处于
        // craft-bot 时显示，进入会话不再自动打开；非 craft-bot 时隐藏并移除启动器。
        setOpen(true, isCraft)
        // sidebar：进 craft 主会话且 sidebar 服务齐备 → openTab（幂等）。
        if (isCraft && sidebarOn && sidebarRight) {
          try { sidebarRight.openTab(TAB_KIND) } catch (e) { /* tab 未挂载/无 seat 时下次订阅重试 */ }
        }
      }

      try { if (sessions && sessions.list && typeof sessions.list.subscribe === 'function') unsub = sessions.list.subscribe(sync) } catch (e) { unsub = null }
      sync()

      // placement 切换监听：storage 事件只在**其他 tab** 改动 localStorage 时触发，
      // 同 tab 内改完需自己调 window.__dshCraftRefresh()（见上方 placement 注释）。
      function onStorage(ev) {
        if (ev && ev.key === PLACEMENT_KEY) { renderCurrent(); sync() }
      }
      if (typeof window !== 'undefined' && window.addEventListener) window.addEventListener('storage', onStorage)
      // 手动刷新入口：改完 placement 后调它立刻生效（同 tab 场景）。
      W.__dshCraftRefresh = function () { renderCurrent(); sync() }

      // 窗口缩放时重算让位宽度（46vw 随视口变化）
      var onResize = function () { renderCurrent() }
      if (typeof window !== 'undefined' && window.addEventListener) window.addEventListener('resize', onResize)

      // cordis disposer：插件卸载/HMR 时清理订阅、监听、让位与 DOM。
      return function disposer() {
        try { if (unsub) unsub() } catch (e) { /* noop */ }
        try { if (typeof window !== 'undefined' && window.removeEventListener) window.removeEventListener('resize', onResize) } catch (e) { /* noop */ }
        try { if (typeof window !== 'undefined' && window.removeEventListener) window.removeEventListener('storage', onStorage) } catch (e) { /* noop */ }
        try { if (W.__dshCraftRefresh) delete W.__dshCraftRefresh } catch (e) { /* noop */ }
        for (var i = disposers.length - 1; i >= 0; i--) {
          try { disposers[i]() } catch (e) { /* noop */ }
        }
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

    // ── apply 的**故障隔离外壳**（2026-09-26 事故的第二次加固）──────────────
    // 为什么必须有：cordis 的 Fiber 只要 apply 抛异常就把状态置为 failed
    // （cordis/lib/index.js:1290 `if (this._error) return 3`），而 web boot 的
    // 激活审计对 failed 与 pending 一视同仁地抛错：
    //   `web boot: 1 entry did not activate\n<name>: failed`
    // （已在 verify-cordis-service-gate.mjs 里用真实 cordis 复现 pending 分支；
    //   failed 分支同一处代码 `I7[s.fiber.state] !== "active"` 必然入列）
    // → 面板里任何一个小 bug（选择器、localStorage 被策略禁用、DOM 结构变了）
    //   都能把整个桌面端启动拖垮。这个仪表盘是**可选 UI**，绝不该有这个权力：
    //   boot 必须成功，面板最坏情况只是不出现 + console 报错。
    // 所以：apply 本体整段 try/catch（原 apply 改名为 applyInner），出错时打印
    // 诊断并返回 no-op disposer，让 fiber 稳定落在 active。
    function apply(ctx) {
      try {
        return applyInner(ctx)
      } catch (error) {
        try {
          console.error('[dsh-bridge] 面板初始化失败（已隔离，不影响 DSH 启动）:', error)
          W.__dshCraftError = {
            build: W.__dshCraftBuild || null,
            message: error && error.message ? String(error.message) : String(error),
            stack: error && error.stack ? String(error.stack).slice(0, 1200) : null,
          }
        } catch (e) { /* 诊断本身也失败就彻底静默 */ }
        return function noopDisposer() { /* 初始化失败，无副作用可清 */ }
      }
    }

    // ── exports.inject = 服务名（fiber 激活门），绝不能写包名！─────────────
    // 这里和 package.json 的 dsh.client.inject 是**两个不同的平面**，写反了会
    // 让整个 web boot 挂掉（桌面端起不来）：
    //   · package.json dsh.client.inject = 包名 → boot 图的“到达顺序”依赖
    //     （client-modules 的 arriveGraphRow 预载这些包的行，未知名字静默跳过）；
    //   · exports.inject（本处）= ctx 服务名 → client runner 的**服务门**
    //     （dsh-cordis-client-runner L590 waitingFor 按 Object.keys(fiber.inject)
    //     查 ctx.get(name)，查不到就一直 pending）。
    // 2026-09-26 事故：把包名写进这里后，载体行会永远等一个名为
    // '@deepseek-ai/dsh-client-ui-renderer' 的服务 → “1 entry did not activate”
    // → 桌面端 web boot 失败。服务名必须与 apply 里真正访问的属性一一对应。
    exports.inject = ['sessions', 'slots', 'sidebarRight', 'sidebarRightTabs']
    exports.name = 'dsh-bridge'
    exports.apply = apply
    return module.exports
  },
})
