$cargo = "$env:USERPROFILE\.rustup\toolchains\nightly-2026-07-21-x86_64-pc-windows-msvc\bin\cargo.exe"
$proc = Start-Process -FilePath $cargo -ArgumentList @("run", "-p", "craft-agent-autopilot") -RedirectStandardOutput "tools/autopilot_out.log" -RedirectStandardError "tools/autopilot_err.log" -WindowStyle Hidden -PassThru -WorkingDirectory "D:\Craft-Agent"
Write-Host "Started PID: $($proc.Id)"
