<#
.SYNOPSIS
  Auto diagnostic + fix loop: build verify -> run tests -> analyze issues -> auto fix -> rerun

.DESCRIPTION
  Fully automated test toolchain entry point. Two modes:
  1. Quick diagnostic (default): build -> run tests -> analyze session -> output report + fix suggestions
  2. Auto fix loop (-AutoFix): detect issues, search source, apply known fixes, rerun

  Mode 1 suits daily dev verification. Mode 2 suits unattended fully automated testing.

  Flow:
    1. cargo build (build verification)
    2. cargo test (run tests)
    3. Analyze session (if exists)
    4. Match known issue table
    5. Generate fix plan
    6. (-AutoFix) auto search source -> output fix hints

.PARAMETER Goal
  Agent goal. Default: "Explore the world, gather wood and craft a crafting table"

.PARAMETER Steps
  How many steps to run. Default 60.

.PARAMETER Port
  Viewer port. Default 8080.

.PARAMETER McAddr
  MC server address. Default localhost:4444

.PARAMETER Profile
  Optional profile name.

.PARAMETER TimeoutMin
  Maximum runtime in minutes. Default 30.

.PARAMETER AutoFix
  Enable auto fix loop (detect issues -> search source -> hint fixes -> rerun).

.PARAMETER MaxFixIterations
  Max iterations for auto fix loop. Default 3.

.PARAMETER ScanOnly
  Skip running agent; only analyze existing sessions/mc_run.jsonl.

.PARAMETER NoBuild
  Skip cargo build.

.PARAMETER TestOnly
  Only run cargo test and session analysis; do not start viewer.

.EXAMPLE
  .\tools\auto_diag.ps1
  .\tools\auto_diag.ps1 -AutoFix -MaxFixIterations 5
  .\tools\auto_diag.ps1 -TestOnly
  .\tools\auto_diag.ps1 -ScanOnly
#>
[CmdletBinding()]
param(
    [string]$Goal = "Explore the world, gather wood and craft a crafting table",
    # 0 = 自动根据 goal 复杂度估算步数（推荐）；>0 = 用户指定
    [int]$Steps = 0,
    [int]$Port = 8080,
    [string]$McAddr = "localhost:4444",
    [string]$Profile = "",
    [int]$TimeoutMin = 30,
    [switch]$AutoFix,
    [int]$MaxFixIterations = 3,
    [switch]$ScanOnly,
    [switch]$NoBuild,
    [switch]$TestOnly,
    # 归档策略：auto = 自动归档（默认），append = 接着上一轮写，archive_only = 仅归档不跑
    [ValidateSet("auto","append","archive_only")]
    [string]$SessionPolicy = "auto"
)

# Force UTF-8 output (avoid GBK garbling on Chinese Windows)
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$OutputEncoding = [System.Text.Encoding]::UTF8

$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot/..

$SessionPath = "sessions/mc_run.jsonl"
$BotTracePath = "sessions/bot_trace.jsonl"
$ViewerExe = "target/debug/craft-agent-viewer.exe"
$ScriptDir = $PSScriptRoot
$ReportDir = "sessions/reports"
if (-not (Test-Path $ReportDir)) { New-Item -ItemType Directory -Path $ReportDir -Force | Out-Null }

# Global nightly cargo path (set once, used by all functions)
$script:NightlyCargo = "$env:USERPROFILE\.rustup\toolchains\nightly-2026-07-21-x86_64-pc-windows-msvc\bin\cargo.exe"
if (-not (Test-Path $script:NightlyCargo)) { $script:NightlyCargo = "cargo" }

# Helper: run cargo without stderr warnings causing fatal errors
$script:CargoExitCode = 0
function Invoke-CargoSafe {
    param([string[]]$CmdArgs)
    $prev = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    & $script:NightlyCargo @CmdArgs 2>&1 | ForEach-Object { $_ }
    $script:CargoExitCode = $LASTEXITCODE
    $ErrorActionPreference = $prev
}

# ============================================================
# Helper functions
# ============================================================

function Write-Section($msg) {
    Write-Host ""
    Write-Host "============================================" -ForegroundColor Cyan
    Write-Host "  $msg" -ForegroundColor Cyan
    Write-Host "============================================" -ForegroundColor Cyan
}

function Write-Result($ok, $msg) {
    if ($ok) {
        Write-Host "  [OK] $msg" -ForegroundColor Green
    } else {
        Write-Host "  [FAIL] $msg" -ForegroundColor Red
    }
}

function Get-Timestamp() {
    return Get-Date -Format "yyyy-MM-dd HH:mm:ss"
}

# ============================================================
# Dynamic step estimator
#   根据 goal 复杂度自动估算需要的步数。
#   规则：基础 20 步 + 每个识别到的子任务难度分。
#   用户也可以 -Steps N 显式覆盖。
# ============================================================
function Estimate-Steps {
    param([string]$GoalText)
    $g = $GoalText.ToLower()
    $steps = 20  # 基础步数（探索 + 1 个简单动作）

    # ── 采集/合成链 ──
    if ($g -match 'wood|log|原木|木头|oak')             { $steps += 8  }   # 砍树 + 合成木板
    if ($g -match 'planks|木板')                          { $steps += 4  }
    if ($g -match 'stick|木棍')                           { $steps += 4  }
    if ($g -match 'crafting_table|工作台')                { $steps += 6  }
    if ($g -match 'torch|火把')                           { $steps += 10 }   # 需要 coal + stick
    if ($g -match 'wooden_pickaxe|木镐')                  { $steps += 8  }
    if ($g -match 'stone_pickaxe|石镐')                   { $steps += 15 }   # 需要先有木镐挖石头
    if ($g -match 'iron_pickaxe|铁镐')                    { $steps += 30 }   # 需要石镐挖铁矿 + 熔炼
    if ($g -match 'diamond|钻石')                         { $steps += 50 }
    if ($g -match 'furnace|熔炉')                         { $steps += 12 }
    if ($g -match 'sword|axe|hoe|shovel|剑|斧|锄|铲')    { $steps += 8  }

    # ── 战斗 / 防御 ──
    if ($g -match 'attack|kill|hunt|战斗|击杀|狩猎')      { $steps += 15 }
    if ($g -match 'dragon|末影龙|ender')                  { $steps += 100 }

    # ── 探索 / 维度 ──
    if ($g -match 'explore|探索|find|look')               { $steps += 15 }
    if ($g -match 'nether|下界|黑曜石|obsidian')          { $steps += 60 }
    if ($g -match 'end|末地|stronghold|要塞')             { $steps += 100 }
    if ($g -match 'village|村庄|librarian|图书管理员')    { $steps += 30 }

    # ── 建造 ──
    if ($g -match 'build|建造|house|shelter|房子')        { $steps += 25 }
    if ($g -match 'farm|农场')                            { $steps += 20 }

    # ── 多任务串联（goal 里有多个独立目标）──
    $taskCount = 0
    foreach ($kw in @('craft','合成','make','做','gather','采集','mine','挖','smelt','熔炼','build','建造','find','找')) {
        $matches2 = [regex]::Matches($g, $kw)
        $taskCount += $matches2.Count
    }
    if ($taskCount -gt 1) { $steps += ($taskCount - 1) * 5 }

    # ── 上限 / 下限 ──
    if ($steps -lt 20) { $steps = 20 }
    if ($steps -gt 300) { $steps = 300 }

    return $steps
}

# ============================================================
# Cross-run bug tracker
#   对比 sessions/reports/ 下历史 scan 报告，找出反复出现的 bug。
#   返回反复 bug 列表（用于提示 AI 必须联网学习开源项目）。
# ============================================================
function Find-RecurringBugs {
    param([string]$CurrentReportDir = $ReportDir)

    # 找最近 5 份 scan_*.md 报告
    $reports = Get-ChildItem -Path $CurrentReportDir -Filter "scan_*.md" |
               Sort-Object LastWriteTime -Descending |
               Select-Object -First 5
    if ($reports.Count -lt 2) { return @() }

    # 对每份报告提取 high_failure_rate 的工具名
    $toolFailureHistory = @{}  # tool -> list of (timestamp, error_rate)
    foreach ($r in $reports) {
        $content = Get-Content $r.FullName -Raw -ErrorAction SilentlyContinue
        if (-not $content) { continue }
        $ts = $r.BaseName -replace 'scan_', ''
        # 匹配 "  tool_name         calls=N   errors=M   (P%) [!]"
        $matches3 = [regex]::Matches($content, '^\s+(\w+)\s+calls=(\d+)\s+errors=(\d+)\s+\((\d+)%\)\s*(\[\!\])?',
                       [System.Text.RegularExpressions.RegexOptions]::Multiline)
        foreach ($m in $matches3) {
            $tool = $m.Groups[1].Value
            $errRate = [int]$m.Groups[4].Value
            if ($errRate -ge 50) {  # 只追踪 ≥50% 失败的工具
                if (-not $toolFailureHistory.ContainsKey($tool)) {
                    $toolFailureHistory[$tool] = @()
                }
                $toolFailureHistory[$tool] += [pscustomobject]@{
                    timestamp = $ts
                    error_rate = $errRate
                }
            }
        }
    }

    # 找出连续 2+ 轮失败的 bug
    $recurring = @()
    foreach ($tool in $toolFailureHistory.Keys) {
        $hist = $toolFailureHistory[$tool]
        if ($hist.Count -ge 2) {
            $recurring += [pscustomobject]@{
                tool = $tool
                consecutive_runs = $hist.Count
                error_rates = ($hist | ForEach-Object { "$($_.error_rate)%" }) -join ' -> '
                latest_rate = $hist[0].error_rate
            }
        }
    }
    return $recurring
}

# ============================================================
# Known issues from tools/known_issues.json
# ============================================================

function Get-KnownIssues {
    <#
    .SYNOPSIS
      Load known issues from tools/known_issues.json
    .DESCRIPTION
      Returns a list of known issues with symptoms, root causes, fix files, and descriptions.
      Used to cross-reference with scan results for auto-diagnosis.
    #>
    $jsonPath = "$PSScriptRoot/../tools/known_issues.json"
    if (-not (Test-Path $jsonPath)) {
        Write-Host "  [WARN] known_issues.json not found at $jsonPath" -ForegroundColor Yellow
        return @()
    }
    try {
        $issues = Get-Content $jsonPath -Raw | ConvertFrom-Json
        Write-Host "  [INFO] Loaded $($issues.Count) known issues from known_issues.json" -ForegroundColor Gray
        return $issues
    } catch {
        Write-Host "  [WARN] Failed to parse known_issues.json: $_" -ForegroundColor Yellow
        return @()
    }
}

function Match-KnownIssues {
    param(
        [string]$ReportFile = ""
    )
    $knownIssues = Get-KnownIssues
    if ($knownIssues.Count -eq 0) { return @() }
    if (-not (Test-Path $ReportFile)) { return @() }
    $reportContent = Get-Content $ReportFile -Raw
    $matches = @()
    foreach ($issue in $knownIssues) {
        foreach ($symptom in $issue.symptom) {
            if ($reportContent -match [regex]::Escape($symptom)) {
                $matches += [pscustomobject]@{
                    symptom = $symptom
                    root_cause = $issue.root_cause
                    fix_files = $issue.fix_files -join ", "
                    fix_description = $issue.fix_description
                }
                break
            }
        }
    }
    return $matches
}

# ============================================================
# Step 1: Build verification
# ============================================================

function Step-Build {
    Write-Section "1. Build verification"
    $start = Get-Date
    if ($NoBuild) {
        Write-Host "  [SKIP] Skipping build (-NoBuild)"
        return $true
    }

    Write-Host "  Building ..." -NoNewline
    $build = Invoke-CargoSafe @("build", "--workspace")
    $elapsed = [int]((Get-Date) - $start).TotalSeconds
    $exitCode = $script:CargoExitCode

    if ($exitCode -eq 0) {
        Write-Host " [OK] (${elapsed}s)" -ForegroundColor Green
        return $true
    } else {
        Write-Host " [FAIL] (${elapsed}s)" -ForegroundColor Red
        # Extract error lines (first 30)
        $errors = $build | Where-Object { "$_" -match 'error' } | Select-Object -First 30
        foreach ($e in $errors) {
            Write-Host "    $e" -ForegroundColor Red
        }
        return $false
    }
}

# ============================================================
# Step 2: Run tests
# ============================================================

function Step-Test {
    param([string]$Filter = "")

    Write-Section "2. Run tests"
    $start = Get-Date

    if ($Filter) {
        Write-Host "  Filter: $Filter"
        $output = Invoke-CargoSafe @("test", "--workspace", "--no-fail-fast", "--", $Filter)
    } else {
        Write-Host "  Running full test suite ..."
        $output = Invoke-CargoSafe @("test", "--workspace", "--no-fail-fast")
    }

    $elapsed = [int]((Get-Date) - $start).TotalSeconds
    $exitCode = $script:CargoExitCode

    # Parse results
    $passed = 0; $failed = 0
    $failedTests = @()
    foreach ($line in $output) {
        $lineStr = "$line"
        if ($lineStr -match '^test .+ \.\.\. ok$') { $passed++ }
        elseif ($lineStr -match '^test .+ \.\.\. FAILED$') {
            $failed++
            $testName = ($lineStr -replace '^test ', '') -replace ' \.\.\. FAILED$', ''
            $failedTests += $testName
        }
    }

    # If parsing failed, look for summary at end of output
    if ($passed -eq 0 -and $failed -eq 0) {
        foreach ($line in $output) {
            $lineStr = "$line"
            if ($lineStr -match '(\d+) passed.*?(\d+) failed') {
                $passed = [int]$Matches[1]
                $failed = [int]$Matches[2]
            }
        }
    }

    $total = $passed + $failed
    Write-Host "  Result: $passed / $total passed, $failed failed (${elapsed}s)"

    if ($failed -gt 0) {
        Write-Host "  Failed tests:" -ForegroundColor Yellow
        foreach ($t in $failedTests) {
            Write-Host "    [FAIL] $t" -ForegroundColor Red
        }
        # Extract failure details
        $inFailure = $false
        $failureLines = @()
        foreach ($line in $output) {
            $lineStr = "$line"
            if ($lineStr -match '^---- .+ ----$') {
                $inFailure = $true
                $failureLines = @()
            }
            elseif ($lineStr -match '^(test |running |$)') {
                if ($inFailure -and $failureLines.Count -gt 0) {
                    $failureHead = $failureLines -join '; '
                    if ($failureHead.Length -gt 200) { $failureHead = $failureHead.Substring(0, 200) + '...' }
                    Write-Host "    [MSG] $failureHead" -ForegroundColor DarkGray
                }
                $inFailure = $false
            }
            elseif ($inFailure) {
                $failureLines += $lineStr
            }
        }
    }

    return [pscustomobject]@{
        passed = $passed
        failed = $failed
        total = $total
        elapsed_sec = $elapsed
        failed_tests = $failedTests
        exit_code = $exitCode
    }
}

# ============================================================
# Step 3: Analyze Session
# ============================================================

function Step-AnalyzeSession {
    Write-Section "3. Analyze session"
    if (-not (Test-Path $SessionPath)) {
        Write-Host "  [!] Session file not found: $SessionPath (skipping analysis)" -ForegroundColor Yellow
        return $null
    }

    # Call scan_run.ps1 to analyze
    $reportFile = "$ReportDir/scan_$(Get-Date -Format 'yyyyMMdd_HHmmss').md"
    & "$ScriptDir/scan_run.ps1" -SessionPath $SessionPath -BotTracePath $BotTracePath -OutFile $reportFile
    $scanExit = $LASTEXITCODE

    # Read summary
    $critical = 0; $high = 0; $medium = 0
    if (Test-Path $reportFile) {
        $reportContent = Get-Content $reportFile -Raw
        if ($reportContent -match 'CRITICAL: (\d+)') { $critical = [int]$Matches[1] }
        if ($reportContent -match 'HIGH:\s+(\d+)') { $high = [int]$Matches[1] }
        if ($reportContent -match 'MEDIUM:\s+(\d+)') { $medium = [int]$Matches[1] }

        Write-Host "  Report written to: $reportFile" -ForegroundColor Gray
    }

    Write-Host "  Severity: CRITICAL=$critical HIGH=$high MEDIUM=$medium"
    return [pscustomobject]@{
        critical = $critical
        high = $high
        medium = $medium
        report_file = $reportFile
        exit_code = $scanExit
    }
}

# ============================================================
# Step 4: Generate consolidated report
# ============================================================

function Step-Report {
    param($buildOk, $testResult, $sessionResult, $iteration = 0)

    Write-Section "4. Consolidated report"
    $report = @"
================================================================================
  Craft-Agent auto diagnostic report
  Time: $(Get-Timestamp)
  Iteration: $iteration / $MaxFixIterations
================================================================================

[Build] $(
    if ($buildOk) { "[OK] passed" } else { "[FAIL] failed" }
)

[Test] $(
    if ($testResult) { "$($testResult.passed)/$($testResult.total) passed, $($testResult.failed) failed ($($testResult.elapsed_sec)s)" }
    else { "not run" }
)

"@

    if ($sessionResult) {
        $report += @"
[Session] Severity: CRITICAL=$($sessionResult.critical) HIGH=$($sessionResult.high) MEDIUM=$($sessionResult.medium)
  Report: $($sessionResult.report_file)
"@
    }

    $report += @"

[Fix suggestions]
"@

    if ($testResult -and $testResult.failed -gt 0) {
        $report += "  - $($testResult.failed) test failure(s) need fixing`n"
        foreach ($t in $testResult.failed_tests) {
            $report += "    - $t`n"
        }
    }

    if ($sessionResult -and $sessionResult.critical -gt 0) {
        $report += "  - CRITICAL level issue(s) detected; fix first`n"
    }
    if ($sessionResult -and $sessionResult.high -gt 0) {
        $report += "  - HIGH level issue(s) detected; recommend fixing`n"
    }

    if ($testResult.failed -eq 0 -and (-not $sessionResult -or ($sessionResult.critical -eq 0 -and $sessionResult.high -eq 0))) {
        $report += "  [OK] No issues detected requiring fixes`n"
    }

    $report += @"
================================================================================
"@

    Write-Host $report
    return $report
}

# ============================================================
# Step 5: Auto fix loop (search source + locate issues)
# ============================================================

function Step-AutoFix {
    param($testResult, $sessionResult, $iteration)

    Write-Section "5. Auto fix (iteration $iteration)"

    $foundIssues = @()

    # 5a: Locate from test failures
    if ($testResult -and $testResult.failed -gt 0) {
        foreach ($t in $testResult.failed_tests) {
            # Guess source file from test name
            $sourceFile = Guess-TestSourceFile $t
            $foundIssues += [pscustomobject]@{
                type = "test_failure"
                name = $t
                source = $sourceFile
                fix = ""
            }
            Write-Host "  [LOC] Test failure '$t' -> possible source: $sourceFile" -ForegroundColor Yellow
        }
    }

    # 5b: Locate from session analysis result
    if ($sessionResult -and $sessionResult.report_file -and (Test-Path $sessionResult.report_file)) {
        $reportContent = Get-Content $sessionResult.report_file -Raw

        # pseudo_call -> decision.rs
        if ($reportContent -match 'pseudo_call') {
            $foundIssues += [pscustomobject]@{
                type = "pseudo_call"
                name = "pseudo call text"
                source = "crates/craft-agent-model/src/decision.rs"
                fix = "Check if fold_tool_history was reintroduced, or strengthen the 'must use function calling to output tool calls' instruction in prompt"
            }
            Write-Host "  [LOC] Pseudo call detected -> check decision.rs" -ForegroundColor Yellow
        }

        # position_stuck -> mod.rs / adapter_azalea.rs
        if ($reportContent -match 'position_stuck') {
            $foundIssues += [pscustomobject]@{
                type = "stuck"
                name = "position stuck"
                source = "crates/craft-agent-minecraft/src/azalea/mod.rs"
                fix = "Check goto timeout setting (currently 3s/60 ticks) or stuck_since time-based detection"
            }
            Write-Host "  [LOC] Position stuck detected -> check mod.rs goto timeout" -ForegroundColor Yellow
        }

        # dead loop -> mod.rs
        if ($reportContent -match 'repeated_call') {
            $foundIssues += [pscustomobject]@{
                type = "dead_loop"
                name = "dead loop"
                source = "crates/craft-agent/src/agent/mod.rs"
                fix = "Check recent_calls dead loop detection logic or nudge injection position"
            }
            Write-Host "  [LOC] Dead loop detected -> check mod.rs" -ForegroundColor Yellow
        }

        # high tool failure rate -> tool implementations
        if ($reportContent -match 'high_failure_rate') {
            $foundIssues += [pscustomobject]@{
                type = "high_failure"
                name = "high tool failure rate"
                source = "crates/craft-agent-minecraft/src/azalea/"
                fix = "Check timeout, pathfinding, server sync issues for each tool"
            }
            Write-Host "  [LOC] High tool failure rate -> check azalea tool implementations" -ForegroundColor Yellow
        }
    }

    if ($foundIssues.Count -eq 0) {
        Write-Host "  [OK] No auto-fixable issues detected" -ForegroundColor Green
        return $foundIssues
    }

    # Output fix summary
    Write-Host "  Detected $($foundIssues.Count) fixable issue(s):" -ForegroundColor Cyan
    foreach ($issue in $foundIssues) {
        Write-Host "    [FIX] [$($issue.type)] $($issue.name)" -ForegroundColor Yellow
        Write-Host "        Source file: $($issue.source)" -ForegroundColor Gray
        if ($issue.fix) {
            Write-Host "        Fix: $($issue.fix)" -ForegroundColor Gray
        }
    }

    return $foundIssues
}

# Guess source file from test name
function Guess-TestSourceFile($testName) {
    $testName = $testName.ToLower()

    $mapping = @(
        @{ pattern = 'regression_system_prompt'; file = 'crates/craft-agent/src/agent/mod.rs' }
        @{ pattern = 'regression_compact'; file = 'crates/craft-agent/src/agent/compaction.rs' }
        @{ pattern = 'estimate_tokens'; file = 'crates/craft-agent/src/agent/compaction.rs' }
        @{ pattern = 'dead_loop'; file = 'crates/craft-agent/src/agent/mod.rs' }
        @{ pattern = 'volatile_injection'; file = 'crates/craft-agent/src/agent/mod.rs' }
        @{ pattern = 'prompt_state'; file = 'crates/craft-agent/src/agent/mod.rs' }
        @{ pattern = 'auto_perceive'; file = 'crates/craft-agent/src/agent/mod.rs' }
        @{ pattern = 'self_prompt'; file = 'crates/craft-agent/src/agent/mod.rs' }
        @{ pattern = 'memory_injected'; file = 'crates/craft-agent/src/agent/mod.rs' }
        @{ pattern = 'serialize_msg'; file = 'crates/craft-agent/src/agent/mod.rs' }
        @{ pattern = 'json_byte_len'; file = 'crates/craft-agent/src/agent/mod.rs' }
        @{ pattern = 'is_obs_tool'; file = 'crates/craft-agent/src/agent/compaction.rs' }
        @{ pattern = 'integration_run'; file = 'crates/craft-agent/tests/regression.rs' }
        @{ pattern = 'compaction_calls'; file = 'crates/craft-agent-viewer/tests/compaction_agnes.rs' }
    )

    foreach ($m in $mapping) {
        if ($testName -match $m.pattern) {
            return $m.file
        }
    }

    # Default: infer from crate name
    if ($testName -match 'craft_agent_minecraft') {
        return "crates/craft-agent-minecraft/src/"
    }
    if ($testName -match 'craft_agent_model') {
        return "crates/craft-agent-model/src/"
    }
    if ($testName -match 'craft_agent_viewer') {
        return "crates/craft-agent-viewer/src/"
    }
    return "crates/craft-agent/src/"
}

# ============================================================
# Step 6: Run viewer (end-to-end test)
# ============================================================

function Step-RunViewer {
    param([int]$iteration)

    Write-Section "6. Run end-to-end test (iteration $iteration)"

    # Session 归档策略：
    #   auto   = 默认；每轮测试前归档上一轮的 mc_run.jsonl（避免叠加）
    #   append = 不归档，下一轮接着写（用于跨多轮连续观察同一 bot）
    if (-not (Test-Path "sessions")) { New-Item -ItemType Directory -Path "sessions" | Out-Null }
    if ($SessionPolicy -eq "auto") {
        if (Test-Path $SessionPath) {
            $ts = Get-Date -Format "yyyyMMdd_HHmmss"
            $backup = "sessions/archive/mc_run.$ts.jsonl"
            if (-not (Test-Path "sessions/archive")) { New-Item -ItemType Directory -Path "sessions/archive" | Out-Null }
            Move-Item $SessionPath $backup -Force
            Write-Host "  Old session backed up -> $backup"
        }
        if (Test-Path $BotTracePath) { Remove-Item $BotTracePath -Force }
    } else {
        # append 模式：保留 session，但提示用户
        if (Test-Path $SessionPath) {
            $size = (Get-Item $SessionPath).Length
            Write-Host "  [Append] 保留旧 session (size=$size bytes)，新一轮将追加"
        }
    }

    # Check MC server
    $mcHost, $mcPortStr = $McAddr -split ':'
    $mcPort = [int]$mcPortStr
    $tcp = New-Object Net.Sockets.TcpClient
    try {
        $iar = $tcp.BeginConnect($mcHost, $mcPort, $null, $null)
        $ok = $iar.AsyncWaitHandle.WaitOne(3000)
        if (-not $ok -or -not $tcp.Connected) {
            Write-Host "  [FAIL] MC server $McAddr unreachable. Open MC LAN first (port 4444)." -ForegroundColor Red
            return $false
        }
        $tcp.EndConnect($iar)
        Write-Host "  [OK] MC server reachable" -ForegroundColor Green
    } catch {
        Write-Host "  [FAIL] MC server connection failed: $_" -ForegroundColor Red
        return $false
    } finally {
        $tcp.Close()
    }

    # Start viewer
    $viewerArgs = @("--goal", $Goal, "--steps", $Steps, "--port", $Port, "--mc", $McAddr, "--session", $SessionPath)
    if ($Profile) { $viewerArgs += @("--profile", $Profile) }

    $useExe = Test-Path $ViewerExe
    if ($useExe) {
        $viewerProc = Start-Process -FilePath $ViewerExe -ArgumentList $viewerArgs -PassThru -WindowStyle Hidden -RedirectStandardOutput "tools/viewer_output.log" -RedirectStandardError "tools/viewer_err.log"
        Write-Host "  Using exe: $ViewerExe (PID: $($viewerProc.Id))"
    } else {
        Write-Host "  exe not found, fallback to cargo run ..." -ForegroundColor Yellow
        $cargoExe = "$env:USERPROFILE\.rustup\toolchains\nightly-2026-07-21-x86_64-pc-windows-msvc\bin\cargo.exe"
        if (-not (Test-Path $cargoExe)) { $cargoExe = "cargo" }
        $cargoArgs = @("run", "-p", "craft-agent-viewer", "--") + $viewerArgs
        $viewerProc = Start-Process -FilePath $cargoExe -ArgumentList $cargoArgs -PassThru -WindowStyle Hidden -RedirectStandardOutput "tools/viewer_output.log" -RedirectStandardError "tools/viewer_err.log"
        Write-Host "  cargo run (PID: $($viewerProc.Id))"
    }

    # Wait for viewer ready
    Write-Host "  Waiting for viewer ready ..." -NoNewline
    $ready = $false
    for ($i = 0; $i -lt 30; $i++) {
        Start-Sleep -Seconds 1
        try {
            $resp = Invoke-RestMethod -Uri "http://127.0.0.1:$Port/api/status" -TimeoutSec 2 -ErrorAction Stop
            $ready = $true
            Write-Host " [OK] (${i}s)" -ForegroundColor Green
            break
        } catch {}
    }
    if (-not $ready) {
        Write-Host " [FAIL]" -ForegroundColor Red
        Write-Host "  viewer not ready within 30s, see tools/viewer_err.log" -ForegroundColor Red
        if (-not $viewerProc.HasExited) { Stop-Process -Id $viewerProc.Id -Force }
        return $false
    }

    # Start agent
    try {
        $resp = Invoke-RestMethod -Uri "http://127.0.0.1:$Port/api/start" -Method Post -TimeoutSec 5 -ErrorAction Stop
        if ($resp.ok) {
            Write-Host "  [OK] Agent started" -ForegroundColor Green
        } else {
            Write-Host "  [FAIL] Agent start failed: $($resp.error)" -ForegroundColor Red
            if (-not $viewerProc.HasExited) { Stop-Process -Id $viewerProc.Id -Force }
            return $false
        }
    } catch {
        Write-Host "  [FAIL] POST /api/start failed: $_" -ForegroundColor Red
        if (-not $viewerProc.HasExited) { Stop-Process -Id $viewerProc.Id -Force }
        return $false
    }

    # Poll + sample
    $deadline = (Get-Date).AddMinutes($TimeoutMin)
    $lastStep = 0
    $sampleCount = 0
    try {
        while ($true) {
            if ($viewerProc.HasExited) {
                Write-Host "  [!] viewer process exited" -ForegroundColor Yellow
                break
            }

            # Sample bot state
            try {
                $gs = Invoke-RestMethod -Uri "http://127.0.0.1:$Port/api/game-state" -TimeoutSec 3 -ErrorAction Stop
                if ($gs -and $gs.position) {
                    $sample = [pscustomobject]@{
                        ts = (Get-Date).ToString("o")
                        step = $lastStep
                        position = $gs.position
                        health = $gs.health
                        hunger = $gs.hunger
                        held_item = $gs.held_item
                        selected_slot = $gs.selected_slot
                    }
                    $sample | ConvertTo-Json -Compress -Depth 5 | Out-File -FilePath $BotTracePath -Append -Encoding UTF8
                    $sampleCount++
                }
            } catch {}

            # Check status
            try {
                $st = Invoke-RestMethod -Uri "http://127.0.0.1:$Port/api/status" -TimeoutSec 2 -ErrorAction Stop
                if ($st.step -gt $lastStep) {
                    $lastStep = $st.step
                    if ($lastStep % 10 -eq 0) {
                        Write-Host "    step $lastStep / $Steps" -ForegroundColor DarkGray
                    }
                }
                if (-not $st.running) {
                    Write-Host "  [OK] Agent finished (step=$($st.step))" -ForegroundColor Green
                    break
                }
            } catch {}

            if ((Get-Date) -gt $deadline) {
                Write-Host "  [!] Timeout ${TimeoutMin}min, force stopping" -ForegroundColor Yellow
                try { Invoke-RestMethod -Uri "http://127.0.0.1:$Port/api/stop" -Method Post -TimeoutSec 3 -ErrorAction Stop | Out-Null } catch {}
                Start-Sleep -Seconds 2
                break
            }

            Start-Sleep -Seconds 5
        }
    } finally {
        # Cleanup viewer
        if (-not $viewerProc.HasExited) {
            try { Stop-Process -Id $viewerProc.Id -Force } catch {}
            Write-Host "  [OK] viewer stopped"
        }
    }

    Write-Host "  bot_trace samples: $sampleCount points"
    return $true
}

# ============================================================
# Main flow
# ============================================================

Write-Host @"
==================================================
|       Craft-Agent fully automated diagnostic toolchain
|       $(Get-Timestamp)
==================================================
"@ -ForegroundColor Cyan

# 动态步数：-Steps 0 时根据 goal 复杂度估算
if ($Steps -eq 0) {
    $Steps = Estimate-Steps -GoalText $Goal
    Write-Host "[Goal] $Goal"
    Write-Host "[Steps] 动态估算 = $Steps （用 -Steps N 显式覆盖）"
} else {
    Write-Host "[Goal] $Goal"
    Write-Host "[Steps] 用户指定 = $Steps"
}

# 归档策略：archive_only 模式仅归档不跑
if ($SessionPolicy -eq "archive_only") {
    Write-Host "[SessionPolicy] archive_only: 仅归档当前 mc_run.jsonl 后退出"
    if (Test-Path $SessionPath) {
        $ts = Get-Date -Format "yyyyMMdd_HHmmss"
        if (-not (Test-Path "sessions/archive")) { New-Item -ItemType Directory -Path "sessions/archive" | Out-Null }
        Move-Item $SessionPath "sessions/archive/mc_run.$ts.jsonl" -Force
        Write-Host "  [OK] Archived -> sessions/archive/mc_run.$ts.jsonl"
    }
    exit 0
}

$allReports = @()
$allOk = $true

for ($iter = 0; $iter -lt $MaxFixIterations; $iter++) {
    Write-Host ""
    Write-Host "============================================" -ForegroundColor Magenta
    Write-Host "  Iteration $($iter+1) / $MaxFixIterations" -ForegroundColor Magenta
    Write-Host "============================================" -ForegroundColor Magenta

    # 1. Build
    $buildOk = Step-Build
    if (-not $buildOk) {
        # Build failed: try searching source to locate issue
        if ($AutoFix) {
            Write-Host "  [SEARCH] Build failed, searching source ..." -ForegroundColor Yellow
            $buildErrors = Get-Content "tools/viewer_err.log" -ErrorAction SilentlyContinue
            # Output build error summary
        }
        $allOk = $false
        # Continue running tests even if build failed (some tests may still run)
    }

    # 2. Tests
    if ($TestOnly -or $ScanOnly) {
        $testResult = Step-Test
    } else {
        $testResult = Step-Test
    }

    # 3. Analyze session
    $sessionResult = Step-AnalyzeSession

    # 4. Report
    $report = Step-Report -buildOk $buildOk -testResult $testResult -sessionResult $sessionResult -iteration ($iter+1)
    $allReports += $report

    # 5. Run end-to-end test (unless -TestOnly or -ScanOnly)
    if (-not $TestOnly -and -not $ScanOnly -and $buildOk -and $testResult.failed -eq 0) {
        $viewerOk = Step-RunViewer -iteration ($iter+1)
        if ($viewerOk) {
            # Re-analyze the new session from the viewer run
            Write-Host ""
            Write-Host "  Re-analyzing new session from viewer run ..." -ForegroundColor Cyan
            $sessionResult = Step-AnalyzeSession
        }
    }

    # 6. Auto fix (if enabled)
    if ($AutoFix) {
        $issues = Step-AutoFix -testResult $testResult -sessionResult $sessionResult -iteration ($iter+1)
        if ($issues.Count -eq 0) {
            Write-Host ""
            Write-Host "  [OK] All issues fixed, exiting loop" -ForegroundColor Green
            break
        }

        # 6.1 跨轮 bug 追踪：找出连续 2+ 轮未修的 bug，强制要求联网学习
        $recurring = Find-RecurringBugs
        if ($recurring.Count -gt 0) {
            Write-Host ""
            Write-Host "  [RECURRING] 以下工具连续多轮失败，必须联网学习开源项目（mindcraft/azalea）后修复：" -ForegroundColor Red
            foreach ($r in $recurring) {
                Write-Host "    - $($r.tool): 连续 $($r.consecutive_runs) 轮失败 [$($r.error_rates)]" -ForegroundColor Yellow
            }
            Write-Host "  >> AI 必须执行：" -ForegroundColor Cyan
            Write-Host "     1. WebSearch 搜索 'mindcraft <tool> implementation' 或 'azalea-rs <tool>'" -ForegroundColor Cyan
            Write-Host "     2. WebFetch 抓取 mindcraft 源码对应文件" -ForegroundColor Cyan
            Write-Host "     3. 对比本项目 crates/craft-agent-minecraft/src/azalea/<tool>.rs 实现" -ForegroundColor Cyan
            Write-Host "     4. 找出本质差异并重写（不是缝缝补补）" -ForegroundColor Cyan
            Write-Host "     5. 改完 cargo build + cargo test 验证再重跑" -ForegroundColor Cyan
        }
    } else {
        # Non-AutoFix mode: 仍然输出 recurring bug 提示
        $recurring = Find-RecurringBugs
        if ($recurring.Count -gt 0) {
            Write-Host ""
            Write-Host "  [RECURRING] 跨轮反复 bug（建议 -AutoFix 让 AI 主动修复）：" -ForegroundColor Yellow
            foreach ($r in $recurring) {
                Write-Host "    - $($r.tool): 连续 $($r.consecutive_runs) 轮 [$($r.error_rates)]" -ForegroundColor Yellow
            }
        }
        # Non-AutoFix mode: exit after one iteration
        if ($iter -eq 0) {
            break
        }
    }

    # Check if all passed
    if ($testResult -and $testResult.failed -eq 0 -and $buildOk) {
        $allOk = $true
        if ($AutoFix) {
            Write-Host ""
            Write-Host "  [OK] All passed! Exiting fix loop" -ForegroundColor Green
            break
        }
    } else {
        $allOk = $false
    }
}

# Save consolidated report
$reportFile = "$ReportDir/diag_$(Get-Date -Format 'yyyyMMdd_HHmmss').md"
$allReports -join "`n---`n" | Out-File -FilePath $reportFile -Encoding UTF8
Write-Host ""
Write-Host "[REPORT] Consolidated report: $reportFile" -ForegroundColor Green

exit $(if ($allOk) { 0 } else { 1 })
