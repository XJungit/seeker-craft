$action = {
    Set-Location "D:\Craft-Agent"
    $cargo = "$env:USERPROFILE\.rustup\toolchains\nightly-2026-07-21-x86_64-pc-windows-msvc\bin\cargo.exe"
    Start-Process -WindowStyle Hidden -FilePath $cargo -ArgumentList @("run", "-p", "craft-agent-autopilot") -RedirectStandardOutput "D:\Craft-Agent\tools\autopilot_out.log" -RedirectStandardError "D:\Craft-Agent\tools\autopilot_err.log"
}
Invoke-Command -ScriptBlock $action
