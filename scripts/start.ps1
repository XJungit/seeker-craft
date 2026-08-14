<#
.SYNOPSIS
  SeekerCraft 一键启动：构建 viewer -> 启动 viewer -> 连接 bot（经 craft-agent-ctl）。
.DESCRIPTION
  前置：已运行 scripts/setup.ps1 完成配置；Minecraft 26.2 服务器已在 localhost:4444 运行
  （或通过 -Mc 指定其他地址）。
  说明：viewer 只是提供 HTTP 桥（/api/connect + /api/bot_tool + /api/game-state），
  大脑是 DeepSeek Harness（DSH）——本脚本不启动 DSH，请自行打开 DSH 并选择 craft-bot 预设。
.NOTES
  Requires: PowerShell 5.1+ / pwsh 7+
#>
[CmdletBinding()]
param(
    [string]$Goal = "探索世界并逐步推进：挖矿 -> 合成工具 -> 装备 -> 向末影龙推进",
    [int]$Steps = 0,
    [int]$Port = 8080,
    [string]$Mc = "localhost:4444",
    [string]$Username = "CraftAgent",
    [string]$ProjectRoot = ""
)

$ErrorActionPreference = 'Stop'
if ([string]::IsNullOrWhiteSpace($ProjectRoot)) {
    if ($PSScriptRoot) { $ProjectRoot = Split-Path $PSScriptRoot -Parent }
    if ([string]::IsNullOrWhiteSpace($ProjectRoot)) { $ProjectRoot = (Get-Location).Path }
}
$ProjectRoot = [System.IO.Path]::GetFullPath($ProjectRoot)

function Write-Step($msg) { Write-Host "`n==> $msg" -ForegroundColor Cyan }
function Write-Ok($msg) { Write-Host "    [OK] $msg" -ForegroundColor Green }
function Write-Warn($msg) { Write-Host "    [!!] $msg" -ForegroundColor Yellow }
function Write-Fail($msg) { Write-Host "    [FAIL] $msg" -ForegroundColor Red }

Write-Step "1/3 检查 viewer 是否已在运行"
try {
    $existing = Invoke-RestMethod -Uri "http://127.0.0.1:$Port/api/game-state" -TimeoutSec 5 -ErrorAction SilentlyContinue
    if ($existing) {
        Write-Ok "viewer 已在 $Port 运行（跳过启动）"
        $viewerRunning = $true
    }
} catch { $viewerRunning = $false }

if (-not $viewerRunning) {
    Write-Step "2/3 启动 viewer（craft-agent-ctl viewer）"
    # 用 ctl viewer 启动（Rust Command::args 逐参传递，避免 PowerShell 参数拼接问题）
    Push-Location $ProjectRoot
    try {
        $ctl = Join-Path $ProjectRoot 'target\debug\craft-agent-ctl.exe'
        if (-not (Test-Path $ctl)) {
            Write-Host "    构建 craft-agent-ctl ..."
            cargo build -p craft-agent-ctl 2>&1 | ForEach-Object { Write-Host "    $_" }
        }
        # ctl viewer 是阻塞式后台进程；用 Start-Process 分离启动
        $logDir = "$env:TEMP\opencode"
        New-Item -ItemType Directory -Force -Path $logDir | Out-Null
        $p = Start-Process -FilePath $ctl -ArgumentList @('viewer', "`"$Goal`"", "$Steps") `
            -RedirectStandardOutput "$logDir\viewer_run.log" `
            -RedirectStandardError "$logDir\viewer_run.err.log" -PassThru
        Write-Ok "viewer 已启动 (PID $($p.Id))，日志: $logDir\viewer_run.log"
        # 等待 viewer API 可响应（最多 60s，覆盖冷启动/编译）
        $apiReady = $false
        for ($i = 0; $i -lt 20; $i++) {
            Start-Sleep -Seconds 3
            try {
                Invoke-RestMethod -Uri "http://127.0.0.1:$Port/api/game-state" -TimeoutSec 5 -ErrorAction Stop | Out-Null
                $apiReady = $true
                break
            } catch {
                Write-Host "    等待 viewer API 就绪（$($i + 1)/20）..."
            }
        }
        if (-not $apiReady) {
            Write-Fail "viewer API 未在 $Port 就绪。日志: $logDir\viewer_run.log / viewer_run.err.log"
            exit 1
        }
        Write-Ok "viewer API 就绪"
    } finally { Pop-Location }
}

Write-Step "3/3 连接 bot（POST /api/connect）"
try {
    $conn = Invoke-RestMethod -Uri "http://127.0.0.1:$Port/api/connect" -Method Post -TimeoutSec 20
    Write-Ok "连接结果: $($conn | ConvertTo-Json -Compress)"
} catch {
    Write-Fail "连接失败: $_"
    Write-Host "  请确认：1) Minecraft 26.2 服务器在 $Mc 运行；2) viewer 日志 $env:TEMP\opencode\viewer_run.log"
    exit 1
}

# 等待 bot 状态就绪（轮询重试最多 90s，覆盖加入世界/同步延迟）
$ready = $false
for ($i = 0; $i -lt 30; $i++) {
    Start-Sleep -Seconds 3
    try {
        $state = Invoke-RestMethod -Uri "http://127.0.0.1:$Port/api/game-state" -TimeoutSec 10
        if ($state.scene_desc) {
            $summary = (($state.scene_desc -split "`n") | Select-Object -First 2) -join ' / '
            Write-Ok "bot 已就绪：$summary"
            $ready = $true
            break
        }
    } catch {
        Write-Host "    等待 bot 加入世界（$($i + 1)/30）... ($($_.Exception.Message))"
        continue
    }
    Write-Host "    等待 bot 加入世界（$($i + 1)/30）..."
}
if (-not $ready) {
    Write-Warn "bot 状态暂不可读。若 MC 服务器已启动，可稍后重试 .\scripts\start.ps1，或检查 viewer 日志。"
}

Write-Host "`n启动完成！"
Write-Host "  1) 打开 DSH（DeepSeek Harness），新建/进入 craft-bot 预设会话"
Write-Host "  2) 在会话中调用 game_state() 感知 -> bot_tool(name, args) 执行 -> set_goal(text) 设目标"
Write-Host "  3) 停止：.\scripts\stop.ps1"
