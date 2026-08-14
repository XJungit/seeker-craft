<#
.SYNOPSIS
  SeekerCraft 一键停止：停止 viewer / autopilot（经 craft-agent-ctl stop）。
.DESCRIPTION
  停止 viewer 与相关 bot 进程。不影响 Minecraft 服务器与 DSH。
.NOTES
  Requires: PowerShell 5.1+ / pwsh 7+
#>
[CmdletBinding()]
param(
    [string]$ProjectRoot = ""
)

$ErrorActionPreference = 'Stop'
if ([string]::IsNullOrWhiteSpace($ProjectRoot)) {
    if ($PSScriptRoot) { $ProjectRoot = Split-Path $PSScriptRoot -Parent }
    if ([string]::IsNullOrWhiteSpace($ProjectRoot)) { $ProjectRoot = (Get-Location).Path }
}
$ProjectRoot = [System.IO.Path]::GetFullPath($ProjectRoot)

Write-Host "==> 停止 SeekerCraft viewer/autopilot ..." -ForegroundColor Cyan
Push-Location $ProjectRoot
try {
    $ctl = Join-Path $ProjectRoot 'target\debug\craft-agent-ctl.exe'
    if (-not (Test-Path $ctl)) {
        cargo build -p craft-agent-ctl 2>&1 | ForEach-Object { Write-Host "    $_" }
    }
    & $ctl stop
    Write-Host "    已停止。" -ForegroundColor Green
} finally { Pop-Location }
