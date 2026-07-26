<#
.SYNOPSIS
  打印 mc_run.jsonl 中所有 assistant 消息（content + tool_calls）
#>
param(
    [string]$SessionPath = "sessions/mc_run.jsonl"
)
$lines = Get-Content -Path $SessionPath -Encoding UTF8
$assistant = @()
foreach ($l in $lines) {
    $t = $l.Trim()
    if (-not $t) { continue }
    try {
        $e = $t | ConvertFrom-Json
        if ($e.type -eq 'message' -and $e.message.role -eq 'assistant') {
            $assistant += $e
        }
    } catch {}
}
$idx = 0
foreach ($a in $assistant) {
    $idx++
    Write-Host ""
    Write-Host ("--- assistant #" + $idx + " ts=" + $a.timestamp + " ---") -ForegroundColor Cyan
    if ($a.message.content) {
        Write-Host ("content: " + $a.message.content)
    }
    if ($a.message.tool_calls) {
        foreach ($c in $a.message.tool_calls) {
            $argsJson = $c.arguments | ConvertTo-Json -Compress -Depth 5
            Write-Host ("  tool: " + $c.name + " args: " + $argsJson) -ForegroundColor Yellow
        }
    }
}
Write-Host ""
Write-Host ("Total assistant messages: " + $assistant.Count)
