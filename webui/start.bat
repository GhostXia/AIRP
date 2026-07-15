@echo off
REM AIRP WebUI 一键启动脚本（同时起 engine + webui 静态 server）
REM 使用调用者已配置的 Rust/Node 工具链；请确保 cargo 和 node 可从 PATH 找到。
REM 双击即可；关闭两个弹出的窗口即停止服务。

setlocal

REM ── 业务配置（按需改）─────────────────────────────────────────────────
REM 默认启用零密钥 mock provider（PR B 验收用；要换真 provider 改这里）
set AIRP_ENDPOINT=http://127.0.0.1:8889/v1/chat/completions
set AIRP_MODEL=airp-mock-1
set AIRP_API_KEY=mock-key-not-checked
REM 验收用临时 data root，避免污染个人 data/，且让"全新 data root 上完成闭环"判据可复现
set "AIRP_DATA_DIR=%~dp0..\target\webui-smoke-data"
if exist "%AIRP_DATA_DIR%" rmdir /S /Q "%AIRP_DATA_DIR%"
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

REM ── 启动零密钥 mock provider（新窗口；PR B 验收用，真 provider 时可注释掉）──
echo Starting mock provider on http://127.0.0.1:8889 ...
start "AIRP Mock Provider" cmd /V:ON /k "node webui\mock-provider.js & echo. & echo [mock exited, code !errorlevel!] & pause"

REM ── 启动 engine（新窗口，强制有头：失败时窗口停留显示错误）────────────
REM cmd /k 链：先跑 cargo，无论成败都 echo + pause。
REM 注意：cmd /k 内 %errorlevel% 是退出时实时值（cmd /k 不像 /c 提前展开）。
echo Starting engine on http://127.0.0.1:8000 ...
start "AIRP Engine" cmd /V:ON /k "cargo run -p airp-core -- daemon --port 8000 & echo. & echo [engine exited, code !errorlevel!] & pause"

REM ── 启动 WebUI 静态 server（新窗口）───────────────────────────────────
echo Starting WebUI on http://%WEBUI_HOST%:%WEBUI_PORT% ...
start "AIRP WebUI" cmd /V:ON /k "node webui\serve.js & echo. & echo [webui exited, code !errorlevel!] & pause"

REM ── 自动打开浏览器（等 engine 编译 + 启动）─────────────────────────────
REM cargo build 增量约 3-5s，首次 cold build 可能 60s+。等 5s 后开浏览器，
REM 若 engine 还在编译，浏览器会显示 connection refused，用户刷新即可。
timeout /t 5 /nobreak >nul
start "" http://%WEBUI_HOST%:%WEBUI_PORT%/

echo.
echo Mock:    http://127.0.0.1:8889   (零密钥；真 provider 时注释掉 mock 启动块)
echo Engine:  http://127.0.0.1:8000
echo WebUI:   http://%WEBUI_HOST%:%WEBUI_PORT%
echo.
echo 三个窗口已弹出。关闭窗口即停止对应服务。
echo Engine 窗口在编译/运行失败时会停留显示错误（强制有头）。
echo 本窗口可关闭。
endlocal
