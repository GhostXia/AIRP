@echo off
setlocal

set "AIRP_ROOT=%~dp0"
set "AIRP_DATA_DIR=%AIRP_ROOT%data"
set "AIRP_PERSIST_PROVIDER_KEY=true"
set "AIRP_ALLOW_LOCAL_PATH=false"
set "AIRP_ACCESS_KEY="
set "AIRP_DEPLOYMENT_MODE="
set "AIRP_PUBLIC_ORIGIN="
set "AIRP_CORS_ORIGINS="
set "AIRP_BROWSER_ARG=--open-browser"
if /I "%AIRP_LAUNCHER_SMOKE%"=="1" set "AIRP_BROWSER_ARG="

if not exist "%AIRP_ROOT%airp-core.exe" (
  echo Missing airp-core.exe in "%AIRP_ROOT%"
  pause
  exit /b 1
)
if not exist "%AIRP_ROOT%webui\index.html" (
  echo Missing webui\index.html in %AIRP_ROOT%
  pause
  exit /b 1
)
if not exist "%AIRP_DATA_DIR%" mkdir "%AIRP_DATA_DIR%"

REM 防双开预检（审计 N-01）：端口 8765 已被占用（桌面壳 airp-ui.exe 或
REM 上一轮残留 engine）时直接退出，避免两个 engine 实例共用同一 data/
REM 导致会话状态混乱。用 netstat + findstr（纯 batch，不依赖外部脚本
REM 运行器——打包 smoke 断言 Start-AIRP.cmd 不得调用它们）。
netstat -ano | findstr "LISTENING" | findstr /C:":8765 " >nul 2>&1
if not errorlevel 1 (
  echo AIRP is already running on port 8765 ^(maybe via airp-ui.exe^).
  echo Close the existing AIRP window first. If no window is visible,
  echo a leftover engine may be holding the port — run:
  echo   taskkill /F /IM airp-core.exe
  echo then try again.
  pause
  exit /b 1
)

echo Starting AIRP WebUI at http://127.0.0.1:8765
echo User data stays in "%AIRP_DATA_DIR%"
echo Close this window or press Ctrl+C to stop AIRP.
echo.

"%AIRP_ROOT%airp-core.exe" --config "%AIRP_ROOT%config.json" daemon --host 127.0.0.1 --port 8765 --webui-dir "%AIRP_ROOT%webui" %AIRP_BROWSER_ARG%
set "AIRP_EXIT_CODE=%ERRORLEVEL%"
if not "%AIRP_EXIT_CODE%"=="0" (
  echo.
  echo AIRP stopped with an error.
  if /I not "%AIRP_LAUNCHER_SMOKE%"=="1" pause
)
exit /b %AIRP_EXIT_CODE%
