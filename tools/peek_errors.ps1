<# peek_errors.ps1 — 列出 session 中所有 tool 角色消息（前 N 个）#>
param(
    [string]$SessionPath = "sessions/mc_run.jsonl",
    [int]$Show = 30,
    [int]$MaxLen = 280
)
$lines = Get-Content -Path $SessionPath -Encoding UTF8
$count = 0
for ($i = 1; $i -lt $lines.Count; $i++) {
    $t = $lines[$i].Trim()
    if (-not $t) { continue }
    try {
        $o = $t | ConvertFrom-Json
    } catch { continue }
    if ($o.type -ne 'message') { continue }
    $m = $o.message
    if (-not $m) { continue }
    if ($m.role -ne 'tool') { continue }
    $c = $m.content
    if (-not $c) { continue }
    $count++
    if ($count -gt $Show) { break }
    $trunc = $c.Substring(0, [Math]::Min($MaxLen, $c.Length))
    Write-Host "── tool #$count ($($c.Length) chars) ──"
    Write-Host $trunc
    Write-Host ""
}
Write-Host "Total tool messages: $count (showing first $Show)"
