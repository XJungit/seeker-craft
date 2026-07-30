<#
.SYNOPSIS
  Scan craft-agent session jsonl and output a structured diagnostic report.

.DESCRIPTION
  Reads sessions/mc_run.jsonl written by the viewer, detects 8 issue patterns:
    1. pseudo_call         - assistant content contains fake patterns like [Tool Call]/[Tool Exec]/tool(...) etc.
    2. pseudo_tool_name    - tool_calls[].name not in known 37-tool set
    3. high_failure_rate   - ToolResult.is_error=true aggregated by tool
    4. repeated_call       - 4+ consecutive identical (name, arguments) signatures (dead loop)
    5. plain_text_reply    - assistant has no tool_calls and is not the final turn (missed call)
    6. position_stuck      - extract position sequence from perceive results, 5+ consecutive unchanged with goto calls
    7. step_timeout        - adjacent MessageEntry timestamp gap > 60s
    8. token_anomaly       - single turn input_tokens > context_window * 0.8 (compaction may have failed)

  Capability boundary (important):
    - This script only looks at LLM-side jsonl; can verify consistency of "LLM output vs perceive real state sequence"
    - Cannot verify "tool result lying" (e.g. polling bug returns success before command executes).
      That requires adding BotEvent logs on the azalea handler side for cross-comparison; mid-term enhancement.

.PARAMETER SessionPath
  jsonl file path. Default sessions/mc_run.jsonl

.PARAMETER OutFile
  Optional: write report to a markdown file

.PARAMETER BotTracePath
  Optional: bot real-state log sessions/bot_trace.jsonl sampled by auto_diag.ps1.
  When provided, enables cross-comparison: LLM perceive sequence vs bot real position/health sequence,
  can detect "tool result lying", "perception drift", "execution layer no response" type bugs.

.EXAMPLE
  .\tools\scan_run.ps1
  .\tools\scan_run.ps1 -SessionPath sessions\mc_run.jsonl -OutFile report.md
  .\tools\scan_run.ps1 -BotTracePath sessions/bot_trace.jsonl
#>
[CmdletBinding()]
param(
    [string]$SessionPath = "sessions/mc_run.jsonl",
    [string]$OutFile = "",
    [int]$ContextWindow = 200000,
    [string]$BotTracePath = ""
)

# Force UTF-8 output (avoid GBK garbling on Chinese Windows)
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$OutputEncoding = [System.Text.Encoding]::UTF8

$ErrorActionPreference = "Stop"
if (-not (Test-Path $SessionPath)) {
    Write-Host "[FAIL] Session file not found: $SessionPath" -ForegroundColor Red
    Write-Host "   Run auto_diag.ps1 or viewer first to generate a session."
    exit 1
}

# Known 38 tools (aligned with tools_azalea.rs::create_mc_azalea_tools_full, mine_above added)
$KnownTools = @(
    'perceive','memory','goto','mine','mine_below','mine_above','interact_block',
    'attack','defend','craft','craft_3x3','smelt','auto_craft','enchant',
    'gather','place','open','pickup','chest_view','chest_withdraw','chest_deposit',
    'equip','discard','consume','interact_entity','trade','chat',
    'set_goal','pause_goal','resume_goal',
    'build','build_blueprint','list_blueprints',
    'run_plan','run_script','new_action','list_actions','search_wiki'
)

# Pseudo call text regex (these patterns in assistant content = LLM writing pseudo calls instead of real tool_calls)
# Uses \uXXXX escapes to keep source pure ASCII while still matching CJK pseudo-call markers emitted by the LLM.
#   U+3010 = left black lenticular bracket  [
#   U+3011 = right black lenticular bracket ]
#   U+5DE5 U+5177 U+8C03 U+7528  =      (tool call)
#   U+5DE5 U+5177 U+6267 U+884C  =      (tool exec)
#   U+5DE5 U+5177 U+7D50 U+679C  =      / (tool result)
#   U+2192 = rightwards arrow
$PseudoCallPatterns = @(
    '\u3010\u5DE5\u5177\u8C03\u7528\u3011',
    '\u3010\u5DE5\u5177\u6267\u884C\u3011',
    '\u3010\u5DE5\u5177\u7D50\u679C\u3011',
    '\*\*(?:gather|mine|goto|craft|attack|consume|equip|place|build|smelt|enchant|chew|discard|chest_view|chest_withdraw|chest_deposit|interact_entity|trade|chat|set_goal|pause_goal|resume_goal|build_blueprint|list_blueprints|run_plan|run_script|new_action|list_actions|search_wiki|memory|perceive|pickup|open|interact_block|mine_below|mine_above|defend|auto_craft)\*\*\s*\(',
    '(?m)^\s*(?:gather|mine|goto|craft|attack|consume|equip|place|build|smelt)\s*\([^)]*\)\s*(?:\u2192|->|=>)',
    '(?m)^\s*-\s*(?:gather|mine|goto|craft|attack|consume)\s*\('
)

# -- Parse jsonl --
$lines = Get-Content -Path $SessionPath -Encoding UTF8
if ($lines.Count -eq 0) {
    Write-Host "[FAIL] Session file empty: $SessionPath" -ForegroundColor Red
    exit 1
}

$header = $lines[0] | ConvertFrom-Json
$entries = @()
for ($i = 1; $i -lt $lines.Count; $i++) {
    $t = $lines[$i].Trim()
    if (-not $t) { continue }
    try {
        $entries += ($t | ConvertFrom-Json)
    } catch {
        # Skip corrupt lines
    }
}

# -- Extract message sequence (only care about type=message) --
$messages = @()
$stepNo = 0
foreach ($e in $entries) {
    if ($e.type -ne 'message') { continue }
    $msg = $e.message
    if (-not $msg) { continue }
    $role = $msg.role
    if ($role -eq 'user') {
        $messages += [pscustomobject]@{
            kind='user'; step=$stepNo; id=$e.id; ts=$e.timestamp
            content=$msg.content; calls=@(); usage=$null; is_error=$false; tool_name=$null
        }
    } elseif ($role -eq 'assistant') {
        $stepNo++
        $calls = @()
        if ($msg.tool_calls) {
            foreach ($c in $msg.tool_calls) {
                $calls += [pscustomobject]@{ id=$c.id; name=$c.name; arguments=$c.arguments }
            }
        }
        $messages += [pscustomobject]@{
            kind='assistant'; step=$stepNo; id=$e.id; ts=$e.timestamp
            content=$msg.content; calls=$calls; usage=$msg.usage; is_error=$false; tool_name=$null
        }
    } elseif ($role -eq 'tool' -or $role -eq 'toolresult') {
        # Internal serialization uses toolresult, OpenAI API uses tool; match both
        $messages += [pscustomobject]@{
            kind='tool'; step=$stepNo; id=$e.id; ts=$e.timestamp
            content=$msg.content; calls=@(); usage=$null
            is_error=($msg.is_error -eq $true); tool_name=$msg.tool_name
            tool_call_id=$msg.tool_call_id
        }
    }
}

$totalSteps = $stepNo
if ($totalSteps -eq 0) {
    Write-Host "[!] No assistant steps in session; agent may not have started." -ForegroundColor Yellow
    exit 0
}

# -- Stats overview --
$assistantMsgs = $messages | Where-Object { $_.kind -eq 'assistant' }
$toolResults = $messages | Where-Object { $_.kind -eq 'tool' }
$allCalls = $assistantMsgs | ForEach-Object { $_.calls } | Where-Object { $_ }
$totalCalls = ($allCalls | Measure-Object).Count
$totalErrors = ($toolResults | Where-Object { $_.is_error } | Measure-Object).Count
$totalInputTokens = ($assistantMsgs | ForEach-Object { $_.usage.input_tokens } | Measure-Object -Sum).Sum
$totalOutputTokens = ($assistantMsgs | ForEach-Object { $_.usage.output_tokens } | Measure-Object -Sum).Sum

# by_tool distribution
$byTool = @{}
foreach ($c in $allCalls) {
    if (-not $byTool.ContainsKey($c.name)) {
        $byTool[$c.name] = [pscustomobject]@{ calls=0; errors=0; errorSamples=@() }
    }
    $byTool[$c.name].calls++
}
foreach ($tr in $toolResults) {
    if ($byTool.ContainsKey($tr.tool_name) -and $tr.is_error) {
        $byTool[$tr.tool_name].errors++
        # Keep the actual failure text (deduplicated). Without this the report only
        # says "gather 3/3 failed" and every investigation has to re-dig the jsonl
        # by hand to find out *why* -- which is where most debugging time went.
        $txt = ($tr.content -replace '\s+', ' ').Trim()
        if ($txt.Length -gt 400) { $txt = $txt.Substring(0, 400) + ' ...' }
        if ($txt -and ($byTool[$tr.tool_name].errorSamples -notcontains $txt)) {
            $byTool[$tr.tool_name].errorSamples += $txt
        }
    }
}

# -- Issue detection --
$issues = @()
# Fix suggestion table (category -> suggested fix file + suggestion)
$FixSuggestions = @{
    "pseudo_call" = @{
        "fix_file" = "crates/craft-agent-model/src/decision.rs"
        "fix_desc" = "Check if fold_tool_history was reintroduced, or strengthen the 'must use function calling to output tool calls' instruction in prompt"
    }
    "pseudo_tool_name" = @{
        "fix_file" = "crates/craft-agent-minecraft/src/tools_azalea.rs"
        "fix_desc" = "Check if LLM is fabricating tool names; emphasize in prompt that only the 38 registered tools may be used"
    }
    "high_failure_rate" = @{
        "fix_file" = "crates/craft-agent-minecraft/src/azalea/"
        "fix_desc" = "Check timeout, pathfinding, server sync issues for the corresponding tool, or add retry logic"
    }
    "repeated_call" = @{
        "fix_file" = "crates/craft-agent/src/agent/mod.rs"
        "fix_desc" = "Check recent_calls dead loop detection logic, normalization rules, or nudge injection position"
    }
    "plain_text_reply" = @{
        "fix_file" = "crates/craft-agent/src/agent/mod.rs"
        "fix_desc" = "Check SelfPrompter forced execution logic or plain text nudge injection strategy"
    }
    "position_stuck" = @{
        "fix_file" = "crates/craft-agent-minecraft/src/adapter_azalea.rs"
        "fix_desc" = "Check stuck_since time-based stuck detection, or goto timeout setting (currently 3s/60 ticks)"
    }
    "step_timeout" = @{
        "fix_file" = "crates/craft-agent-model/src/decision.rs"
        "fix_desc" = "Check LLM retry timeout config (RetryConfig) or provider response time"
    }
    "token_anomaly" = @{
        "fix_file" = "crates/craft-agent/src/agent/compaction.rs"
        "fix_desc" = "Check compaction threshold or token estimation logic"
    }
    "perception_drift" = @{
        "fix_file" = "crates/craft-agent-minecraft/src/adapter_azalea.rs"
        "fix_desc" = "Check perceive output format or bot state sync logic"
    }
    "exec_no_response" = @{
        "fix_file" = "crates/craft-agent-minecraft/src/azalea/mod.rs"
        "fix_desc" = "Check goto timeout (3s/60 ticks) or Pathfinder pathfinding failure handling"
    }
    "health_drop_no_response" = @{
        "fix_file" = "crates/craft-agent-minecraft/src/azalea/mod.rs"
        "fix_desc" = "Check self_defense mode trigger conditions (distance<=4 + !is_busy()) or consume food logic"
    }
}

function Add-Issue($cat, $severity, $step, $msg) {
    $fix = $FixSuggestions[$cat]
    $script:issues += [pscustomobject]@{
        category=$cat; severity=$severity; step=$step; detail=$msg
        fix_file = $(if ($fix) { $fix.fix_file } else { "" })
        fix_desc = $(if ($fix) { $fix.fix_desc } else { "" })
    }
}

# 1. pseudo_call
foreach ($a in $assistantMsgs) {
    if (-not $a.content) { continue }
    foreach ($pat in $PseudoCallPatterns) {
        if ($a.content -match $pat) {
            $snippet = $a.content.Substring(0, [Math]::Min(120, $a.content.Length))
            Add-Issue "pseudo_call" "CRITICAL" $a.step "step $($a.step): pattern matched | content: $snippet..."
            break
        }
    }
}

# 2. pseudo_tool_name
foreach ($c in $allCalls) {
    if ($c.name -notin $KnownTools) {
        Add-Issue "pseudo_tool_name" "CRITICAL" 0 "Tool name '$($c.name)' not in known 37-tool set (LLM may be fabricating)"
    }
}

# 3. high_failure_rate (per tool; report only if failure rate > 30% or absolute failures >= 3)
foreach ($k in $byTool.Keys) {
    $v = $byTool[$k]
    if ($v.calls -ge 2 -and $v.errors -gt 0) {
        $rate = [Math]::Round($v.errors / $v.calls * 100, 1)
        if ($rate -ge 30 -or $v.errors -ge 3) {
            $sev = if ($rate -ge 50) { "HIGH" } else { "MEDIUM" }
            # Include the real error text so the report is actionable on its own.
            $detail = "$k : $($v.errors)/$($v.calls) failed ($rate%)"
            $n = 0
            foreach ($s in $v.errorSamples) {
                $n++
                if ($n -gt 3) { $detail += "`n        ... and $($v.errorSamples.Count - 3) more distinct error(s)"; break }
                $detail += "`n        why[$n]: $s"
            }
            Add-Issue "high_failure_rate" $sev 0 $detail
        }
    }
}

# 4. repeated_call (4+ consecutive identical signatures)
$callSignatures = @()
foreach ($a in $assistantMsgs) {
    foreach ($c in $a.calls) {
        $sig = "$($c.name)|$($c.arguments | ConvertTo-Json -Compress -Depth 5)"
        $callSignatures += [pscustomobject]@{ step=$a.step; sig=$sig }
    }
}
$consecutive = 1
for ($i = 1; $i -lt $callSignatures.Count; $i++) {
    if ($callSignatures[$i].sig -eq $callSignatures[$i-1].sig) {
        $consecutive++
        if ($consecutive -eq 4) {
            Add-Issue "repeated_call" "HIGH" $callSignatures[$i].step "4+ consecutive identical calls: $($callSignatures[$i].sig.Substring(0, [Math]::Min(80, $callSignatures[$i].sig.Length)))"
        }
    } else {
        $consecutive = 1
    }
}

# 5. plain_text_reply (missed call)
foreach ($a in $assistantMsgs) {
    $hasCalls = ($a.calls | Measure-Object).Count -gt 0
    if (-not $hasCalls -and $a.content -and $a.content.Trim().Length -gt 0) {
        # Exclude last turn (may be a wrap-up)
        if ($a.step -lt $totalSteps) {
            $snippet = $a.content.Substring(0, [Math]::Min(80, $a.content.Length))
            Add-Issue "plain_text_reply" "MEDIUM" $a.step "step $($a.step): no tool_calls, content: $snippet..."
        }
    }
}

# 6. position_stuck (extract position sequence from perceive results)
# perceive output (perception.rs) contains labels like "coord: (x,y,z)" / "position: (x,y,z)" /
#   (coord) / (position). Use \uXXXX escapes to keep source pure ASCII while still
#   matching the CJK labels the Rust side emits.
#   U+5750 U+6807 =  (coord)
#   U+4F4D U+7F6E =  (position)
$positionSeq = @()
foreach ($tr in $toolResults) {
    if ($tr.tool_name -ne 'perceive') { continue }
    if (-not $tr.content) { continue }
    if ($tr.content -match '(?:\u5750\u6807|\u4F4D\u7F6E|position|coord)[^\d-]*\(\s*(-?\d+)\s*,\s*(-?\d+)\s*,\s*(-?\d+)\s*\)') {
        $positionSeq += [pscustomobject]@{ step=$tr.step; x=[int]$Matches[1]; y=[int]$Matches[2]; z=[int]$Matches[3] }
    }
}
if ($positionSeq.Count -ge 5) {
    $streak = 1
    for ($i = 1; $i -lt $positionSeq.Count; $i++) {
        if ($positionSeq[$i].x -eq $positionSeq[$i-1].x -and
            $positionSeq[$i].y -eq $positionSeq[$i-1].y -and
            $positionSeq[$i].z -eq $positionSeq[$i-1].z) {
            $streak++
            if ($streak -eq 5) {
                $p = $positionSeq[$i]
                Add-Issue "position_stuck" "HIGH" $p.step "5+ consecutive turns position unchanged: ($($p.x), $($p.y), $($p.z))"
            }
        } else {
            $streak = 1
        }
    }
}

# 7. step_timeout (adjacent assistant timestamp gap > 60s)
$tsList = $assistantMsgs | Where-Object { $_.ts } | ForEach-Object {
    try { [datetime]::Parse($_.ts) } catch { $null }
} | Where-Object { $_ }
for ($i = 1; $i -lt $tsList.Count; $i++) {
    $gap = ($tsList[$i] - $tsList[$i-1]).TotalSeconds
    if ($gap -gt 60) {
        Add-Issue "step_timeout" "MEDIUM" ($i+1) "step $($i+1): gap from previous turn $([Math]::Round($gap,1))s (LLM may be stuck or retrying)"
    }
}

# 8. token_anomaly
foreach ($a in $assistantMsgs) {
    if ($a.usage -and $a.usage.input_tokens) {
        $ratio = $a.usage.input_tokens / $ContextWindow
        if ($ratio -gt 0.8) {
            Add-Issue "token_anomaly" "MEDIUM" $a.step "step $($a.step): input_tokens=$($a.usage.input_tokens) occupies $([Math]::Round($ratio*100,1))% of window (compaction may have failed)"
        }
    }
}

# -- 9. Cross-comparison: LLM perceive sequence vs bot_trace real state sequence --
$botTrace = @()
$crossIssues = @()
if ($BotTracePath -and (Test-Path $BotTracePath)) {
    $traceLines = Get-Content -Path $BotTracePath -Encoding UTF8
    foreach ($l in $traceLines) {
        $t = $l.Trim()
        if (-not $t) { continue }
        try { $botTrace += ($t | ConvertFrom-Json) } catch {}
    }
    Write-Host "  Loaded bot_trace: $($botTrace.Count) sample points" -ForegroundColor DarkGray

    if ($botTrace.Count -ge 2 -and $positionSeq.Count -ge 1) {
        # Compare 1: each perceive position vs same-time bot_trace position
        # Drift > 3 blocks = perception layer bug
        foreach ($p in $positionSeq) {
            # Find nearest bot_trace sample point (by step)
            $closest = $botTrace | Sort-Object { [Math]::Abs($_.step - $p.step) } | Select-Object -First 1
            if (-not $closest -or -not $closest.position) { continue }
            $btPos = $closest.position
            if ($btPos.Count -lt 3) { continue }
            $dx = [Math]::Abs($p.x - $btPos[0])
            $dy = [Math]::Abs($p.y - $btPos[1])
            $dz = [Math]::Abs($p.z - $btPos[2])
            $dist = [Math]::Sqrt($dx*$dx + $dy*$dy + $dz*$dz)
            if ($dist -gt 3) {
                $crossIssues += [pscustomobject]@{
                    category="perception_drift";
                    severity="HIGH"; step=$p.step
                    detail="step $($p.step): perceive says ($($p.x),$($p.y),$($p.z)) but bot_trace real ($([Math]::Round($btPos[0],1)),$([Math]::Round($btPos[1],1)),$([Math]::Round($btPos[2],1))) drift $([Math]::Round($dist,1))m"
                }
            }
        }

        # Compare 2: after goto call, does bot_trace position change (did execution layer respond)
        # Find all goto calls, check if position changes in 2-3 samples after the call
        $gotoCalls = $allCalls | Where-Object { $_.name -eq 'goto' }
        foreach ($g in $gotoCalls) {
            # Parse goto params x,y,z (arguments is PSCustomObject with fields x/y/z)
            $gargs = $g.arguments
            if (-not $gargs) { continue }
            try {
                $gx = [int]$gargs.x
                $gy = [int]$gargs.y
                $gz = [int]$gargs.z
            } catch { continue }

            # Find bot_trace points before/after the call
            $beforePt = $botTrace | Where-Object { $_.step -le $g.step } | Select-Object -Last 1
            $afterPts = $botTrace | Where-Object { $_.step -ge $g.step } | Select-Object -First 3
            if (-not $beforePt -or $afterPts.Count -eq 0) { continue }
            if (-not $beforePt.position -or -not $afterPts[0].position) { continue }

            $bPos = $beforePt.position
            $aPos = $afterPts[-1].position
            if ($bPos.Count -lt 3 -or $aPos.Count -lt 3) { continue }

            $moved = [Math]::Sqrt(
                [Math]::Pow($bPos[0]-$aPos[0], 2) +
                [Math]::Pow($bPos[1]-$aPos[1], 2) +
                [Math]::Pow($bPos[2]-$aPos[2], 2)
            )
            if ($moved -lt 1.0) {
                # Position barely changed, but LLM called goto; execution layer may not have responded
                $crossIssues += [pscustomobject]@{
                    category="exec_no_response";
                    severity="MEDIUM"; step=$g.step
                    detail="step $($g.step): goto($gx,$gy,$gz) called but bot_trace moved only $([Math]::Round($moved,2))m (may be stuck/timeout/pathfinding failed)"
                }
            }
        }

        # Compare 3: health continuously dropping but LLM not reacting
        $healthSeq = $botTrace | Where-Object { $_.health -ne $null } | ForEach-Object {
            [pscustomobject]@{ step=$_.step; health=[float]$_.health }
        }
        if ($healthSeq.Count -ge 3) {
            for ($i = 2; $i -lt $healthSeq.Count; $i++) {
                if ($healthSeq[$i].health -lt $healthSeq[$i-2].health - 5) {
                    # Lost 5+ health within 3 sample points
                    $crossIssues += [pscustomobject]@{
                        category="health_drop_no_response";
                        severity="MEDIUM"; step=$healthSeq[$i].step
                        detail="step $($healthSeq[$i].step): bot_trace shows health dropped from $($healthSeq[$i-2].health) to $($healthSeq[$i].health); is LLM defending/fleeing/eating?"
                    }
                    break  # Report only once
                }
            }
        }
    }
} elseif ($BotTracePath) {
    Write-Host "  [!] bot_trace file not found: $BotTracePath (skipping cross-comparison)" -ForegroundColor Yellow
}

# Merge cross-comparison issues into issues
$issues += $crossIssues

# -- Output report --
$report = New-Object System.Text.StringBuilder
function W($line="") { [void]$script:report.AppendLine($line); Write-Host $line }
function WC($line, $color) { [void]$script:report.AppendLine($line); Write-Host $line -ForegroundColor $color }

W "============================================================"
W "  Craft-Agent Session diagnostic report"
W "  File: $SessionPath"
W "  Generated: $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')"
W "============================================================"
W ""
W "[Overview]"
W "  Total steps:       $totalSteps"
W "  Total tool calls:  $totalCalls"
W "  Tool failures:     $totalErrors"
W "  Failure rate:      $(if($totalCalls -gt 0){[Math]::Round($totalErrors/$totalCalls*100,1)}else{0})%"
W "  input tokens:      $totalInputTokens"
W "  output tokens:     $totalOutputTokens"
W ""

W "[Tool call distribution]"
$byTool.GetEnumerator() | Sort-Object { $_.Value.calls } -Descending | ForEach-Object {
    $v = $_.Value
    $errRate = if ($v.calls -gt 0) { [Math]::Round($v.errors/$v.calls*100, 1) } else { 0 }
    $errMark = if ($errRate -ge 50) { " [!]" } else { "" }
    W ("  {0,-18} calls={1,-4} errors={2,-3} ({3}%){4}" -f $_.Key, $v.calls, $v.errors, $errRate, $errMark)
}
W ""

if ($issues.Count -eq 0) {
    WC "[Issue list] [OK] No issue patterns detected" "Green"
} else {
    W "[Issue list] Total $($issues.Count) issue(s)"
    $issues | Group-Object category | ForEach-Object {
        $cat = $_.Name
        $sev = ($_.Group | Select-Object -First 1).severity
        $color = switch ($sev) {
            "CRITICAL" { "Red" }
            "HIGH"     { "Yellow" }
            default    { "Cyan" }
        }
        WC "`n  [$sev] $cat ($($_.Count) item(s)):" $color
        foreach ($iss in $_.Group) {
            W "    - $($iss.detail)"
            if ($iss.fix_file) {
                W "      Suggested fix: $($iss.fix_file) -- $($iss.fix_desc)"
            }
        }
    }
    W ""

    # Severity stats
    $critical = ($issues | Where-Object { $_.severity -eq 'CRITICAL' } | Measure-Object).Count
    $high     = ($issues | Where-Object { $_.severity -eq 'HIGH' } | Measure-Object).Count
    $medium   = ($issues | Where-Object { $_.severity -eq 'MEDIUM' } | Measure-Object).Count
    W "[Severity stats]"
    WC "  CRITICAL: $critical  (pseudo call/pseudo tool name -- must fix)" $(if($critical -gt 0){"Red"}else{"Gray"})
    WC "  HIGH:     $high      (high failure/dead loop/position stuck -- strongly recommend fix)" $(if($high -gt 0){"Yellow"}else{"Gray"})
    WC "  MEDIUM:   $medium    (missed call/timeout/token anomaly -- worth checking)" $(if($medium -gt 0){"Cyan"}else{"Gray"})
}

W ""
W "[position sequence (extracted from perceive)]"
if ($positionSeq.Count -eq 0) {
    W "  (no perceive calls or no coordinates extracted)"
} else {
    $positionSeq | Select-Object -First 20 | ForEach-Object {
        W "  step $($_.step): ($($_.x), $($_.y), $($_.z))"
    }
    if ($positionSeq.Count -gt 20) {
        W "  ... total $($positionSeq.Count) coordinate points"
    }
}

W ""
W "============================================================"

if ($OutFile) {
    $report.ToString() | Out-File -FilePath $OutFile -Encoding UTF8
    Write-Host ""
    Write-Host "[REPORT] Report written to: $OutFile" -ForegroundColor Green
}

# Exit code: 2 if CRITICAL, 1 if HIGH, otherwise 0
$exitCode = 0
if (($issues | Where-Object { $_.severity -eq 'CRITICAL' } | Measure-Object).Count -gt 0) { $exitCode = 2 }
elseif (($issues | Where-Object { $_.severity -eq 'HIGH' } | Measure-Object).Count -gt 0) { $exitCode = 1 }
exit $exitCode
