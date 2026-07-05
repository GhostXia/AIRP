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
REM set AIRP_ACCESS_KEY=

REM ── WebUI server 端口/绑定（serve.js 读）──────────────────────────────
set WEBUI_PORT=9001
REM 跨设备访问改 0.0.0.0
set WEBUI_HOST=127.0.0.1

cd /d "%~dp0\.."

REM ── 清旧进程（防止端口被上一次未关干净的 engine 占用）─────────────────
echo Cleaning stale engine processes ...
taskkill /F /IM airp-core.exe >nul 2>&1
REM 释放 8000 端口给新 engine
REM 注意：node serve.js 用 9001，不与 engine 冲突，无需 kill node

REM ── 启动 engine（新窗口，强制有头：失败时窗口停留显示错误）────────────
REM cmd /k 链：先跑 cargo，无论成败都 echo + pause。
REM 注意：cmd /k 内 %errorlevel% 是退出时实时值（cmd /k 不像 /c 提前展开）。
echo Starting engine on http://127.0.0.1:8000 ...
start "AIRP Engine" cmd /k "cargo run -p airp-core -- daemon --port 8000 & echo. & echo [engine exited, code %errorlevel%] & pause"

REM ── 启动 WebUI 静态 server（新窗口）───────────────────────────────────
echo Starting WebUI on http://%WEBUI_HOST%:%WEBUI_PORT% ...
start "AIRP WebUI" cmd /k "node webui\serve.js & echo. & echo [webui exited, code %errorlevel%] & pause"

REM ── 自动打开浏览器（等 engine 编译 + 启动）─────────────────────────────
REM cargo build 增量约 3-5s，首次 cold build 可能 60s+。等 5s 后开浏览器，
REM 若 engine 还在编译，浏览器会显示 connection refused，用户刷新即可。
timeout /t 5 /nobreak >nul
start "" http://%WEBUI_HOST%:%WEBUI_PORT%/

echo.
echo Engine:  http://127.0.0.1:8000
echo WebUI:   http://%WEBUI_HOST%:%WEBUI_PORT%
echo.
echo 两个窗口已弹出。关闭窗口即停止对应服务。
echo Engine 窗口在编译/运行失败时会停留显示错误（强制有头）。
echo 本窗口可关闭。
endlocal
