@echo off
REM AIRP WebUI 一键启动脚本（同时起 engine + webui 静态 server）
REM 按_AGENTS.md Windows 工具链规则设环境变量，所有构建产物落在 D: 盘。
REM 双击即可；关闭两个弹出的窗口即停止服务。

setlocal

REM ── 工具链环境（AGENTS.md）─────────────────────────────────────────────
set RUSTUP_HOME=D:\.rustup
set CARGO_HOME=D:\.cargo
set npm_config_prefix=D:\npm-global
set npm_config_cache=D:\npm-global\npm-cache
set PATH=D:\.cargo\bin;D:\msys64\mingw64\bin;D:\nodejs;%PATH%

REM ── 业务配置（按需改）─────────────────────────────────────────────────
REM provider endpoint / model / data dir：未设则 engine 用默认值
REM set AIRP_ENDPOINT=http://127.0.0.1:8889/v1/chat/completions
REM set AIRP_MODEL=gemini-3.1-pro-preview
REM set AIRP_DATA_DIR=d:\AIRP-Dev\target\webui-smoke-data
REM set AIRP_ACCESS_KEY=                       REM 设了 engine 就要求 Bearer

REM ── WebUI server 端口/绑定（serve.js 读）──────────────────────────────
set WEBUI_PORT=9001
set WEBUI_HOST=127.0.0.1                       REM 跨设备访问改 0.0.0.0

cd /d "%~dp0\.."

REM ── 启动 engine（新窗口）──────────────────────────────────────────────
echo Starting engine on http://127.0.0.1:8000 ...
start "AIRP Engine" cmd /k "cargo run -p airp-core -- daemon --port 8000"

REM ── 启动 WebUI 静态 server（新窗口）───────────────────────────────────
echo Starting WebUI on http://%WEBUI_HOST%:%WEBUI_PORT% ...
start "AIRP WebUI" cmd /k "node webui\serve.js"

REM ── 自动打开浏览器 ────────────────────────────────────────────────────
timeout /t 2 /nobreak >nul
start "" http://%WEBUI_HOST%:%WEBUI_PORT%/

echo.
echo Engine:  http://127.0.0.1:8000
echo WebUI:   http://%WEBUI_HOST%:%WEBUI_PORT%
echo.
echo 两个窗口已弹出。关闭窗口即停止对应服务。
echo 本窗口可关闭。
endlocal
