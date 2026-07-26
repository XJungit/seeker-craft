<#
  读取 session jsonl，打印指定 step 范围的消息详情。
  用法: .\tools\peek_step.ps1 -From 3 -To 8 [-SessionPath sessions/mc_run.jsonl]
#>
[CmdletBinding()]
param(
    [int]$From = 1,
    [int]$To = 10,
    [string]$SessionPath = "sessions/mc_run.jsonl"
)

$ErrorActionPreference = "Stop"
if (-not (Test-Path $SessionPath)) {
    Write-Host "❌ 文件不存在: $SessionPath" -ForegroundColor Red
    exit 1
}

$lines = Get-Content -Path $SessionPath -Encoding UTF8
$stepNo = 0
for ($i = 1; $i -lt $lines.Count; $i++) {
    $t = $lines[$i].Trim()
    if (-not $t) { continue }
    try { $e = $t | ConvertFrom-Json } catch { continue }
    if ($e.type -ne 'message') { continue }
    $m = $e.message
    if (-not $m) { continue }
    if ($m.role -eq 'assistant') { $stepNo++ }

    if ($stepNo -ge $From -and $stepNo -le $To) {
        $role = $m.role
        $toolName = $m.tool_name
        Write-Host ""
        Write-Host ("[step=" + $stepNo + " role=" + $role + $(if ($toolName) { " tool=" + $toolName } else { "" }) + "]") -ForegroundColor Cyan

        if ($m.content) {
            $preview = $m.content.Substring(0, [Math]::Min(500, $m.content.Length))
            Write-Host "  content:"
            foreach ($ln in $preview -split "`n") {
                Write-Host ("    " + $ln)
            }
        }
        if ($m.tool_calls) {
            foreach ($c in $m.tool_calls) {
                $argsJson = $c.arguments | ConvertTo-Json -Compress -Depth 5
                Write-Host ("  call: " + $c.name + " args=" + $argsJson) -ForegroundColor Yellow
            }
        }
    }
}
