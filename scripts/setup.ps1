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
Write-Step "4/6 生成 craft-bot 预设 (~/.dsh/.agent-presets/craft-bot)"
$presetDir = "$env:USERPROFILE\.dsh\.agent-presets\craft-bot"
$templateDir = Join-Path $ProjectRoot 'data\dsh\craft-bot-preset'

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

    $template = Read-Text (Join-Path $templateDir 'agent.cordis.yml')
    if ($template) {
        $template = $template -replace '\{\{PROJECT_ROOT\}\}', $ProjectRootPosix
        if ($dshPkgRoot) {
            $template = $template -replace '\{\{DSH_PKG_ROOT\}\}', (($dshPkgRoot -replace '\\', '/'))
        } else {
            Write-Warn "未定位 DSH 包根，{{DSH_PKG_ROOT}} 保留占位符（需手动替换 skills 路径）"
        }
        Write-Text (Join-Path $presetDir 'agent.cordis.yml') $template
        Write-Ok "agent.cordis.yml 已生成（PROJECT_ROOT=$ProjectRootPosix）"
    } else {
        Write-Warn "预设模板 agent.cordis.yml 为空（跳过）"
    }
    if (Test-Path (Join-Path $templateDir 'preset.yml')) {
        Copy-Item (Join-Path $templateDir 'preset.yml') (Join-Path $presetDir 'preset.yml') -Force
        Write-Ok "preset.yml 已复制"
    }
    Write-Ok "craft-bot 预设位于 $presetDir"
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
Write-Host "  3) 启动 DeepSeek Harness，在 DSH 中选择 craft-bot 预设会话，即可用 game_state / bot_tool / set_goal 驱动 bot"
Write-Host "  详细教程见 README.md（Quick Start / DSH 模式）与 docs/tutorials/getting-started.md"
