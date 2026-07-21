@echo off
chcp 65001 >nul
cd /d "%~dp0"
echo [Craft-Agent Viewer] Starting...
echo Make sure MC is running with mod loaded (port 25567)
echo.
cargo run -p craft-agent-viewer -- --goal "Explore the world" --steps 0
if %errorlevel% neq 0 (
    echo.
    echo Failed. Check:
    echo   1. Rust toolchain installed
    echo   2. MC is running with mod loaded
    pause
)
