@echo off
chcp 65001 >nul
cd /d "%~dp0.."

set CARGO=C:\Users\xj\.rustup\toolchains\nightly-2026-07-21-x86_64-pc-windows-msvc\bin\cargo.exe
if not exist "%CARGO%" (
    echo [错误] 找不到 cargo：%CARGO%
    echo 请确认 nightly-2026-07-21 工具链已安装。
    pause
    exit /b 1
)

echo [Craft-Agent Viewer] 启动中（Azalea 客户端路线，无需 mod）...
echo 浏览器打开后访问 http://127.0.0.1:8080，点"启动"连接 MC 局域网服。
echo.
"%CARGO%" run -p craft-agent-viewer -- --goal "探索世界" --steps 0
if %errorlevel% neq 0 (
    echo.
    echo 启动失败。检查：
    echo   1. Rust nightly 工具链已安装
    echo   2. MC 原版 26.2 局域网服已开启（localhost:4444）
    pause
)
