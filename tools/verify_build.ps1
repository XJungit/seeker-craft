<#
.SYNOPSIS
  One-click build verification + test run + regression tests

.DESCRIPTION
  Three-step flow:
  1. cargo build --workspace
  2. cargo test --workspace
  3. regression_* tests

  Run after each code change to confirm nothing is broken.

.PARAMETER NoBuild
  Skip build, only run tests

.PARAMETER Quick
  Quick mode: build + lib tests only (skip integration tests)

.PARAMETER Regression
  Only run regression tests (regression_*)

.PARAMETER Crate
  Only run tests for the specified crate

.EXAMPLE
  .\tools\verify_build.ps1
  .\tools\verify_build.ps1 -Quick
  .\tools\verify_build.ps1 -Regression
  .\tools\verify_build.ps1 -Crate craft-agent
#>
[CmdletBinding()]
param(
    [switch]$NoBuild,
    [switch]$Quick,
    [switch]$Regression,
    [string]$Crate = ""
)

$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot/..

# Force UTF-8 for this script (avoid GBK garbling on Chinese Windows)
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$OutputEncoding = [System.Text.Encoding]::UTF8

$nightlyCargo = "$env:USERPROFILE\.rustup\toolchains\nightly-2026-07-21-x86_64-pc-windows-msvc\bin\cargo.exe"
if (-not (Test-Path $nightlyCargo)) { $nightlyCargo = "cargo" }

# Helper: run cargo command without treating stderr warnings as fatal errors
# Output goes to pipeline; check $script:CargoExitCode after calling
$script:CargoExitCode = 0
function Invoke-CargoSafe {
    param([string[]]$CmdArgs)
    $prev = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    & $nightlyCargo @CmdArgs 2>&1 | ForEach-Object { $_ }
    $script:CargoExitCode = $LASTEXITCODE
    $ErrorActionPreference = $prev
}

$allPassed = $true
$start = Get-Date

Write-Host "==================================================" -ForegroundColor Cyan
Write-Host "  Craft-Agent Build Verify + Test" -ForegroundColor Cyan
Write-Host "==================================================" -ForegroundColor Cyan
Write-Host ""

# -- Step 1: Build --
if (-not $NoBuild) {
    Write-Host "=== [1/3] Build ===" -ForegroundColor Green
    $buildStart = Get-Date
    if ($Quick) {
        Write-Host "  Quick mode: lib only ..." -NoNewline
        Invoke-CargoSafe @("build", "--workspace") | Out-Null
    } elseif ($Crate) {
        Write-Host "  Build $Crate ..." -NoNewline
        Invoke-CargoSafe @("build", "-p", $Crate) | Out-Null
    } else {
        Write-Host "  Full build ..." -NoNewline
        Invoke-CargoSafe @("build", "--workspace") | Out-Null
    }
    $buildElapsed = [int]((Get-Date) - $buildStart).TotalSeconds
    if ($script:CargoExitCode -eq 0) {
        Write-Host " OK (${buildElapsed}s)" -ForegroundColor Green
    } else {
        Write-Host " FAIL (${buildElapsed}s)" -ForegroundColor Red
        $allPassed = $false
    }
    Write-Host ""
} else {
    Write-Host "=== [1/3] Build (skipped) ===" -ForegroundColor Gray
    Write-Host ""
}

# -- Step 2: Test --
Write-Host "=== [2/3] Test ===" -ForegroundColor Green
$testStart = Get-Date
$testArgs = @("test", "--no-fail-fast", "--color=never")

if ($Quick) {
    $testArgs += "--workspace"
    $testArgs += "--lib"
    Write-Host "  Quick mode: lib tests only"
} elseif ($Regression) {
    Write-Host "  Regression mode"
    $testArgs += "--workspace"
    $testArgs += "regression_"
} elseif ($Crate) {
    Write-Host "  Crate: $Crate"
    $testArgs += "-p"
    $testArgs += $Crate
} else {
    Write-Host "  Full test ..."
    $testArgs += "--workspace"
}

$output = Invoke-CargoSafe $testArgs
$testElapsed = [int]((Get-Date) - $testStart).TotalSeconds
$exitCode = $script:CargoExitCode

# Parse results
$passed = 0; $failed = 0; $ignored = 0
$failedTests = @()
foreach ($line in $output) {
    $lineStr = "$line"
    if ($lineStr -match '^test .+ \.\.\. ok$') { $passed++ }
    elseif ($lineStr -match '^test .+ \.\.\. FAILED$') {
        $failed++
        $testName = ($lineStr -replace '^test ', '') -replace ' \.\.\. FAILED$', ''
        $failedTests += $testName
    }
    elseif ($lineStr -match '^test .+ \.\.\. ignored$') { $ignored++ }
}

# Fallback: parse from summary line
if ($passed -eq 0 -and $failed -eq 0) {
    foreach ($line in $output) {
        $lineStr = "$line"
        if ($lineStr -match '(\d+) passed.*?(\d+) failed') {
            $passed = [int]$Matches[1]
            $failed = [int]$Matches[2]
        }
        if ($lineStr -match '(\d+) ignored') {
            $ignored = [int]$Matches[1]
        }
    }
}

$total = $passed + $failed + $ignored
if ($failed -eq 0) {
    Write-Host ""
    Write-Host "[OK] All $total tests passed (${testElapsed}s)" -ForegroundColor Green
    if ($ignored -gt 0) {
        Write-Host "   ($ignored ignored)" -ForegroundColor Gray
    }
} else {
    Write-Host ""
    Write-Host "[FAIL] $passed / $total passed, $failed failed (${testElapsed}s)" -ForegroundColor Red
    foreach ($t in $failedTests) {
        Write-Host "   - $t" -ForegroundColor Red
    }
    $allPassed = $false
}

Write-Host ""

# -- Step 3: Regression tests (if not regression mode, list them separately) --
if (-not $Regression -and -not $Quick) {
    Write-Host "=== [3/3] Regression tests (regression_*) ===" -ForegroundColor Green
    Write-Host "  Running ..." -NoNewline
    $regOutput = Invoke-CargoSafe @("test", "--workspace", "--no-fail-fast", "--color=never", "regression_")
    $regExitCode = $script:CargoExitCode
    $regPassed = 0; $regFailed = 0
    foreach ($line in $regOutput) {
        $lineStr = "$line"
        if ($lineStr -match '^test .+ \.\.\. ok$') { $regPassed++ }
        elseif ($lineStr -match '^test .+ \.\.\. FAILED$') { $regFailed++ }
    }
    if ($regFailed -eq 0) {
        Write-Host " OK ($regPassed passed)" -ForegroundColor Green
    } else {
        Write-Host " FAIL ($regPassed passed, $regFailed failed)" -ForegroundColor Red
        $allPassed = $false
    }
}

$totalElapsed = [int]((Get-Date) - $start).TotalSeconds
Write-Host ""
Write-Host "=== Total: ${totalElapsed}s ===" -ForegroundColor Cyan
exit $(if ($allPassed) { 0 } else { 1 })
