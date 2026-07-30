@echo off
cd /d D:\Craft-Agent
start "" /B C:\Users\xj\.rustup\toolchains\nightly-2026-07-21-x86_64-pc-windows-msvc\bin\cargo.exe run -p craft-agent-autopilot > tools\autopilot_out.log 2>&1
