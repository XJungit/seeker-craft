<#
.SYNOPSIS
  SeekerCraft (Craft-Agent) 1.0 一键安装/配置脚本（幂等，可重复运行）。
.DESCRIPTION
  自动完成：
    1. 前置检查（Rust nightly / Git / Node.js / pnpm；MC 服务器与 DeepSeek Harness 提示用户自备）
    2. 构建项目（cargo build --workspace）
    3. 配置 DSH 桥插件（dsh-bridge）：注册到 ~/.dsh profile + 链接依赖 + pnpm install
    4. 生成 craft-bot 预设（~/.dsh/.agent-presets/craft-bot），替换本机路径占位符
    5. 复制 .env.example -> .env（如不存在）
    6. 运行 DSH 插件验证脚本（verify-in-harness.mjs）
  说明：
    - DeepSeek Harness（DSH）与 Minecraft Java 版服务器需要用户自行下载/启动，本脚本只做提示。
    - MC 版本要求：Java Edition 26.2（vanilla，见 README）。
.NOTES
  Author: SeekerCraft maintainers
  Requires: PowerShell 5.1+ / pwsh 7+
#>
[CmdletBinding()]
param(
    [switch]$SkipBuild,
    [switch]$SkipDsh,
    [string]$ProjectRoot = ""
)

$ErrorActionPreference = 'Stop'
if ([string]::IsNullOrWhiteSpace($ProjectRoot)) {
    # 脚本位于 <repo>/scripts/setup.ps1，项目根 = 脚本目录的上一级
    if ($PSScriptRoot) { $ProjectRoot = Split-Path $PSScriptRoot -Parent }
    if ([string]::IsNullOrWhiteSpace($ProjectRoot)) { $ProjectRoot = (Get-Location).Path }
}
$ProjectRoot = [System.IO.Path]::GetFullPath($ProjectRoot)
$ProjectRootPosix = ($ProjectRoot -replace '\\', '/')

function Read-Text([string]$path) {
    if (-not (Test-Path $path)) { return $null }
    return [System.IO.File]::ReadAllText($path, [System.Text.Encoding]::UTF8)
}
function Write-Text([string]$path, [string]$content) {
    [System.IO.File]::WriteAllText($path, $content, (New-Object System.Text.UTF8Encoding $false))
}
function Write-Step($msg) { Write-Host "`n==> $msg" -ForegroundColor Cyan }
function Write-Ok($msg) { Write-Host "    [OK] $msg" -ForegroundColor Green }
function Write-Warn($msg) { Write-Host "    [!!] $msg" -ForegroundColor Yellow }
function Write-Fail($msg) { Write-Host "    [FAIL] $msg" -ForegroundColor Red }

# ---------------------------------------------------------------- 前置检查
Write-Step "1/6 前置检查"

$missing = @()
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) { $missing += 'cargo (Rust)' }
if (-not (Get-Command git -ErrorAction SilentlyContinue)) { $missing += 'git' }
if (-not (Get-Command node -ErrorAction SilentlyContinue)) { $missing += 'node (Node.js)' }
if (-not (Get-Command pnpm -ErrorAction SilentlyContinue)) { $missing += 'pnpm' }

if ($missing.Count -gt 0) {
    Write-Fail "缺少前置依赖: $($missing -join ', ')"
    Write-Host "  请先安装："
    Write-Host "    - Rust nightly:  https://rustup.rs  (rustup toolchain install nightly-2026-07-21)"
    Write-Host "    - Git:           https://git-scm.com"
    Write-Host "    - Node.js:       https://nodejs.org  (>= 20)"
    Write-Host "    - pnpm:          npm install -g pnpm"
    Write-Host "  然后重新运行本脚本。"
    exit 1
}
Write-Ok "cargo / git / node / pnpm 均可用"

if (-not (Test-Path "$env:USERPROFILE\.dsh")) {
    Write-Warn "未检测到 DeepSeek Harness 配置目录 (~/.dsh)。"
    Write-Host "    本项目不打包 DeepSeek Harness —— 请自行下载安装："
    Write-Host "      https://github.com/deepseek-ai/deepseek-harness"
    Write-Host "    安装完成后重新运行本脚本，或在 -SkipDsh 模式下先完成 Rust 侧构建。"
} else {
    Write-Ok "检测到 ~/.dsh（DeepSeek Harness 已安装）"
}

Write-Host "    MC 服务器: 需要自备 Java 版 26.2 vanilla 服务器（本脚本不下载）。"
Write-Host "    bot 默认连接 localhost:4444（可在 start.ps1 用 -Mc 覆盖）。"

# ---------------------------------------------------------------- 构建项目
Write-Step "2/6 构建项目 (cargo build --workspace)"
if ($SkipBuild) {
    Write-Warn "跳过构建（-SkipBuild）"
} else {
    Push-Location $ProjectRoot
    try {
        cargo build --workspace 2>&1 | ForEach-Object { Write-Host "    $_" }
        if ($LASTEXITCODE -ne 0) { throw "cargo build 失败" }
        Write-Ok "workspace 构建成功"
    } finally { Pop-Location }
}

# ---------------------------------------------------------------- DSH 插件配置
Write-Step "3/6 配置 DSH 桥插件 (dsh-bridge)"
$webDir = "$env:USERPROFILE\.dsh\profiles\web"
$bridgeDir = Join-Path $ProjectRoot 'tools\dsh-bridge'
$pluginNodeModules = Join-Path $bridgeDir 'node_modules\@deepseek-ai'

if ($SkipDsh -or -not (Test-Path $webDir)) {
    Write-Warn "跳过 DSH 插件配置（-SkipDsh 或未检测到 $webDir）"
} else {
    New-Item -ItemType Directory -Force -Path $pluginNodeModules | Out-Null

    # 3a. package.json: 加 dsh-bridge link 依赖 + bundles 条目（幂等）
    $pkgPath = Join-Path $webDir 'package.json'
    try {
        $pkg = Read-Text $pkgPath | ConvertFrom-Json
    } catch {
        Write-Warn "无法解析 $pkgPath（$_）；跳过 package.json 修改"
        $pkg = $null
    }
    if ($pkg) {
        $changed = $false
        if (-not $pkg.dependencies.PSObject.Properties['dsh-bridge']) {
            $pkg.dependencies | Add-Member -NotePropertyName 'dsh-bridge' -NotePropertyValue "link:$ProjectRootPosix/tools/dsh-bridge"
            $changed = $true
            Write-Ok "package.json 添加 dsh-bridge link 依赖"
        }
        if (-not $pkg.dsh.profile.bundles -contains 'dsh-bridge') {
            $pkg.dsh.profile.bundles += 'dsh-bridge'
            $changed = $true
            Write-Ok "package.json bundles 添加 dsh-bridge"
        }
        if ($changed) {
            Write-Text $pkgPath ($pkg | ConvertTo-Json -Depth 10)
            Write-Ok "package.json 已更新"
        } else {
            Write-Ok "package.json 已包含 dsh-bridge（跳过）"
        }
    }

    # 3b. cordis.patch.yml: dsh-bridge 全局配置覆盖（hostTools:false，仅 client 半边）
    $patchPath = Join-Path $webDir 'cordis.patch.yml'
    $patchRaw = Read-Text $patchPath
    if ($patchRaw -ne $null -and $patchRaw -match '(?ms)- id:\s*dsh-bridge' -and $patchRaw -match 'hostTools:\s*false') {
        Write-Ok "cordis.patch.yml 已包含 dsh-bridge 配置覆盖（跳过）"
    } elseif ($patchRaw -ne $null) {
        $block = @"

# --- SeekerCraft dsh-bridge 全局配置覆盖（由 setup.ps1 自动追加）---
# hostTools:false -> 不向其他项目暴露 Minecraft 工具；面板由 client.js 按
# agentPreset === 'craft-bot' 判断显示。craft-bot 预设内另用绝对路径加载。
- id: dsh-bridge
  config:
    hostTools: false
"@
        Write-Text $patchPath ($patchRaw + $block)
        Write-Ok "cordis.patch.yml 追加 dsh-bridge 配置覆盖"
    } else {
        Write-Warn "未找到 $patchPath（跳过）"
    }

    # 3c. 链接 @deepseek-ai/dsh-tools / schemastery 到插件 node_modules
    # 候选来源（由近到远）：profile node_modules / npm 全局根 / DSH 全局包嵌套 node_modules
    $srcRoots = @()
    if (Test-Path (Join-Path $webDir 'node_modules\@deepseek-ai')) { $srcRoots += (Join-Path $webDir 'node_modules\@deepseek-ai') }
    $npmRoot = (npm root -g 2>$null | Select-Object -First 1)
    if ($npmRoot -and (Test-Path (Join-Path $npmRoot '@deepseek-ai'))) { $srcRoots += (Join-Path $npmRoot '@deepseek-ai') }
    # DSH CLI 全局包内嵌依赖（dsh -> node_modules/@deepseek-ai）
    if ($npmRoot) {
        $dshNested = Join-Path $npmRoot "@deepseek-ai\dsh\node_modules\@deepseek-ai"
        if (Test-Path $dshNested) { $srcRoots += $dshNested }
    }
    # 用户级 pnpm 全局
    $pnpmGlobal = "$env:LOCALAPPDATA\pnpm\global\5\node_modules\@deepseek-ai"
    if (Test-Path $pnpmGlobal) { $srcRoots += $pnpmGlobal }

    if ($srcRoots.Count -eq 0) {
        Write-Warn "未定位 @deepseek-ai 依赖根。请先运行 DSH 一次（安装其依赖），或手动链接 tools\dsh-bridge\node_modules\@deepseek-ai。"
    } else {
        foreach ($t in @('dsh-tools', 'schemastery')) {
            $link = Join-Path $pluginNodeModules $t
            if (Test-Path $link) {
                Write-Ok "dsh-bridge/node_modules/@deepseek-ai/$t 已存在（跳过）"
                continue
            }
            $src = $null
            foreach ($r in $srcRoots) {
                if (Test-Path (Join-Path $r $t)) { $src = Join-Path $r $t; break }
            }
            if ($src) {
                New-Item -ItemType Junction -Path $link -Target $src -ErrorAction Stop | Out-Null
                Write-Ok "链接 @deepseek-ai/$t -> $src"
            } else {
                Write-Warn "@deepseek-ai/$t 未找到（可运行 pnpm install 后重试）"
            }
        }
    }

    # 3d. pnpm install
    Push-Location $webDir
    try {
        Write-Ok "运行 pnpm install（$webDir）"
        pnpm install 2>&1 | ForEach-Object { Write-Host "    $_" }
        if ($LASTEXITCODE -ne 0) { Write-Warn "pnpm install 返回非零（可忽略若依赖已就绪）" }
        else { Write-Ok "pnpm install 完成" }
    } finally { Pop-Location }

    # 3e. 运行插件验证
    if (Test-Path (Join-Path $bridgeDir 'scripts\verify-in-harness.mjs')) {
        Push-Location $bridgeDir
        try {
            Write-Ok "运行 verify-in-harness 验证"
            node scripts/verify-in-harness.mjs 2>&1 | ForEach-Object { Write-Host "    $_" }
        } finally { Pop-Location }
    }
}

# ---------------------------------------------------------------- craft-bot 预设
Write-Step "4/6 生成 craft-bot 预设（rc.3 目录格式 + 0.1.7+ bundle 格式）"
$presetDir = "$env:USERPROFILE\.dsh\.agent-presets\craft-bot"
$templateDir = Join-Path $ProjectRoot 'data\dsh\craft-bot-preset'
$preset017Dir = Join-Path $ProjectRoot 'data\dsh\craft-bot-preset-017'

if (-not (Test-Path $templateDir)) {
    Write-Warn "未找到预设模板 $templateDir（跳过）"
} elseif ($SkipDsh -or -not (Test-Path "$env:USERPROFILE\.dsh")) {
    Write-Warn "跳过预设生成（-SkipDsh 或未检测到 DSH）"
} else {
    New-Item -ItemType Directory -Force -Path $presetDir | Out-Null

    # 定位 DSH 包根（用于 {{DSH_PKG_ROOT}} 占位符替换）
    $dshPkgRoot = $null
    foreach ($cand in @(
        (Join-Path $webDir 'node_modules\@deepseek-ai\dsh'),
        "$env:APPDATA\npm\node_modules\@deepseek-ai\dsh",
        "$env:USERPROFILE\AppData\Roaming\npm\node_modules\@deepseek-ai\dsh"
    )) {
        if (Test-Path $cand) { $dshPkgRoot = $cand; break }
    }

    # ── 版本判定（决定用哪种预设格式）──────────────────────────────────────
    #  判据：DSH 自身 package.json 的 dependencies 是否含 `@deepseek-ai/dsh-agent-preset`（单数）。
    #  0.1.7 引入了该包（预设改为声明式注册，且不再扫描 .agent-presets）；
    #  rc.3 只有 `dsh-agent-presets`（复数，目录扫描式）。
    #  用 dependencies 而非目录探测，可避开 npm 全局（rc.3 嵌套 dsh/node_modules）与
    #  npx/pnpm（0.1.7 提升到顶层 node_modules）两种布局差异带来的误判。
    $isV017 = $false
    $dshVersion = $null
    if ($dshPkgRoot) {
        $dshPj = Join-Path $dshPkgRoot 'package.json'
        if (Test-Path $dshPj) {
            try {
                $dshMeta = Read-Text $dshPj | ConvertFrom-Json
                $dshVersion = $dshMeta.version
                $isV017 = ($dshMeta.dependencies.PSObject.Properties.Name -contains '@deepseek-ai/dsh-agent-preset')
            } catch {
                Write-Warn "无法解析 $dshPj（$_），按 rc.3 处理"
            }
        }
    }
    Write-Ok "DSH $dshVersion -> 预设格式：$(if ($isV017) { '0.1.7+ bundle' } else { 'rc.3 目录' })"

    # ── 4a. rc.3 格式：~/.dsh/.agent-presets/craft-bot/{agent.cordis.yml,preset.yml}
    #        仅在 rc.3 系生成，避免在 0.1.7 上留下一个永不被扫描的死目录。
    $template = Read-Text (Join-Path $templateDir 'agent.cordis.yml')
    if (-not $template) {
        Write-Warn "预设模板 agent.cordis.yml 为空（跳过）"
    } elseif ($isV017) {
        Write-Ok "跳过 rc.3 目录格式（当前 DSH 为 0.1.7+，只用 bundle 格式）"
    } else {
        $expanded = $template -replace '\{\{PROJECT_ROOT\}\}', $ProjectRootPosix
        if ($dshPkgRoot) {
            $expanded = $expanded -replace '\{\{DSH_PKG_ROOT\}\}', (($dshPkgRoot -replace '\\', '/'))
        } else {
            Write-Warn "未定位 DSH 包根，{{DSH_PKG_ROOT}} 保留占位符（需手动替换 skills 路径）"
        }
        Write-Text (Join-Path $presetDir 'agent.cordis.yml') $expanded
        Write-Ok "agent.cordis.yml 已生成（rc.3 格式，PROJECT_ROOT=$ProjectRootPosix）"
        if (Test-Path (Join-Path $templateDir 'preset.yml')) {
            Copy-Item (Join-Path $templateDir 'preset.yml') (Join-Path $presetDir 'preset.yml') -Force
            Write-Ok "preset.yml 已复制"
        }
    }

    # ── 4b. 0.1.7+ 格式：data/dsh/craft-bot-preset-017/cordis.patch.yml（由生成器产出）
    #        生成文件本身对 rc.3 无害（不被引用就不会加载），故始终生成，便于仓库内自查；
    #        但只有 0.1.7+ 才注册进 profile bundles（见 4c）。
    $genScript = Join-Path $ProjectRoot 'scripts\gen-craft-bot-preset-017.mjs'
    if (Test-Path $genScript) {
        $genArgs = @($genScript, '--project-root', $ProjectRoot)
        if ($dshPkgRoot) { $genArgs += @('--dsh-pkg-root', $dshPkgRoot) }
        node @genArgs 2>&1 | ForEach-Object { Write-Host "    $_" }
        if ($LASTEXITCODE -eq 0) {
            Write-Ok "cordis.patch.yml 已生成（0.1.7+ bundle 格式）"

            # 4b-2. 校验生成物与模板往返一致（防止两份内容漂移）
            $verifyScript = Join-Path $ProjectRoot 'scripts\verify-craft-bot-preset-017.mjs'
            if ((Test-Path $verifyScript) -and $dshPkgRoot) {
                node $verifyScript $ProjectRoot $dshPkgRoot 2>&1 | ForEach-Object { Write-Host "    $_" }
                if ($LASTEXITCODE -ne 0) { Write-Warn "预设一致性校验未通过（exit=$LASTEXITCODE）" }
            }
        } else {
            Write-Warn "0.1.7+ 预设生成失败（exit=$LASTEXITCODE）"
        }
    } else {
        Write-Warn "未找到 $genScript（跳过 0.1.7+ 预设生成）"
    }

    # ── 4c. 把 0.1.7+ 预设注册为 profile bundle（幂等，且必须版本门控）
    #
    #  ⚠️ 该 bundle 的 cordis.patch.yml 引用了 `@deepseek-ai/dsh-agent-preset`（单数），
    #     此包在 rc.3 中**不存在**。若在 rc.3 上把本 bundle 加进 dsh.profile.bundles，
    #     DSH 启动时直接崩溃（已实测）：
    #       Error: dsh: plugin tree failed to load: failed to apply loader entry include
    #              (cordis:include): failed to import loader entry preset-craft-bot
    #              (@deepseek-ai/dsh-agent-preset): Cannot find package ...
    #              [ERR_MODULE_NOT_FOUND]     -> exit=1
    #     故仅 0.1.7+ 注册；rc.3 继续用 4a 的 .agent-presets 目录格式。
    $cbBundleDir = Join-Path $ProjectRoot 'data\dsh\craft-bot-preset-017'
    $cbPkgPath = Join-Path $webDir 'package.json'
    if (-not $isV017) {
        Write-Ok "跳过 0.1.7+ preset bundle 注册（rc.3 系，注册会导致启动崩溃）"
    } elseif (-not (Test-Path $cbBundleDir)) {
        Write-Warn "未找到 preset bundle 目录 $cbBundleDir（跳过注册）"
    } else {
        # 用官方 CLI 通道注册：`dsh plugin --profile <name> add <bundle-dir>` 会一次性完成
        # 「写入 link 依赖 → 安装落地 → 选中 bundle」三步，且顺序由 DSH 自己保证。
        #
        # 为何不用手写 package.json + pnpm install：那是复现安装步骤（技能明确要求避免），
        # 且极易产生顺序缺陷 —— 历史故障正是先选了 bundle 而依赖未落地，导致启动时
        # 该 bundle 载入失败、预设从会话列表中「消失」。
        $alreadyLinked = Test-Path (Join-Path $webDir 'node_modules\dsh-preset-craft-bot')
        $bundles = @()
        if (Test-Path $cbPkgPath) {
            try { $bundles = @((Read-Text $cbPkgPath | ConvertFrom-Json).dsh.profile.bundles) } catch { }
        }
        $alreadySelected = $bundles -contains 'dsh-preset-craft-bot'

        if ($alreadyLinked -and $alreadySelected) {
            Write-Ok "craft-bot 预设 bundle 已注册（跳过）"
        } else {
            Push-Location $ProjectRoot
            try {
                Write-Ok "注册 craft-bot 预设 bundle：dsh plugin --profile web add $cbBundleDir"
                & dsh plugin --profile web add $cbBundleDir 2>&1 | ForEach-Object { Write-Host "    $_" }
                $addExit = $LASTEXITCODE
            } finally { Pop-Location }

            if ($addExit -eq 0) {
                Write-Ok "craft-bot 预设 bundle 已注册（依赖 + 安装 + bundle 选择）"
            } else {
                Write-Warn "注册失败（exit=$addExit）。请手动执行："
                Write-Warn "  dsh plugin --profile web add `"$cbBundleDir`""
            }
        }
    }
    Write-Ok "craft-bot 预设已就绪（rc.3 目录：$presetDir；0.1.7+ bundle：$preset017Dir）"
}

# ---------------------------------------------------------------- .env
Write-Step "5/6 配置 .env"
$envExample = Join-Path $ProjectRoot '.env.example'
$envFile = Join-Path $ProjectRoot '.env'
if (Test-Path $envFile) {
    Write-Ok ".env 已存在（跳过）"
} elseif (Test-Path $envExample) {
    Copy-Item $envExample $envFile
    Write-Ok "已从 .env.example 复制 .env（请按需填入 API Key）"
} else {
    Write-Warn "未找到 .env.example"
}

# ---------------------------------------------------------------- 完成
Write-Step "6/6 完成"
Write-Host ""
Write-Host "SeekerCraft 安装配置完成！下一步：" -ForegroundColor Green
Write-Host "  1) 启动 Minecraft Java 版 26.2 服务器（bot 默认连接 localhost:4444）"
Write-Host "  2) 运行 .\scripts\start.ps1 启动 viewer 并连接 bot"
Write-Host "  3) 重启 DeepSeek Harness，在 DSH 中选择 craft-bot 预设会话，即可用 game_state / bot_tool / set_goal 驱动 bot"
Write-Host "     注：0.1.7+ 的预设以 bundle 形式装载，必须重启 DSH 才会出现（运行中的进程不会加载新 bundle）"
Write-Host "  详细教程见 README.md（Quick Start / DSH 模式）与 docs/tutorials/getting-started.md"
