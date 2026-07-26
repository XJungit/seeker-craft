<# peek_calls.ps1 - List assistant tool_calls + matching tool results in session #>
param(
    [string]$SessionPath = "sessions/mc_run.jsonl",
    [int]$MaxLen = 400,
    [string]$FilterName = ""
)
$ErrorActionPreference = "Stop"
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8

$lines = Get-Content -Path $SessionPath -Encoding UTF8

# Index all message entries by id
$byId = @{}
$entries = @()
foreach ($l in $lines) {
    $t = $l.Trim()
    if (-not $t) { continue }
    try { $o = $t | ConvertFrom-Json } catch { continue }
    if ($o.type -ne 'message') { continue }
    $entries += $o
    if ($o.id) { $byId[$o.id] = $o }
}

# Build tool_call_id -> result map
$toolResults = @{}
foreach ($e in $entries) {
    $m = $e.message
    if (-not $m) { continue }
    if ($m.role -ne 'tool' -and $m.role -ne 'toolresult') { continue }
    $tcid = $m.tool_call_id
    if (-not $tcid -and $e.tool_call_id) { $tcid = $e.tool_call_id }
    if ($tcid) {
        $toolResults[$tcid] = $e
    }
}

$stepNo = 0
foreach ($e in $entries) {
    $m = $e.message
    if (-not $m) { continue }
    if ($m.role -ne 'assistant') { continue }
    $stepNo++
    if (-not $m.tool_calls) { continue }
    foreach ($tc in $m.tool_calls) {
        # arguments may be a string (OpenAI) or object (Anthropic). Normalize.
        $argsStr = ""
        if ($tc.arguments) {
            if ($tc.arguments -is [string]) {
                $argsStr = $tc.arguments
            } else {
                $argsStr = ($tc.arguments | ConvertTo-Json -Compress -Depth 10)
            }
        }
        if ($FilterName -and $tc.name -ne $FilterName) { continue }
        $result = ""
        $isErr = $false
        if ($tc.id -and $toolResults.ContainsKey($tc.id)) {
            $tr = $toolResults[$tc.id]
            $result = $tr.message.content
            if (-not $result) { $result = $tr.message.tool_result }
            $isErr = ($tr.message.is_error -eq $true)
        }
        $mark = if ($isErr) { "[ERR]" } else { "[OK ]" }
        $truncR = if ($result) {
            $len = [Math]::Min($MaxLen, $result.Length)
            $result.Substring(0, $len)
        } else { "(no result)" }
        Write-Host "step $stepNo $($tc.name)($argsStr) $mark"
        Write-Host "  -> $truncR"
    }
}
