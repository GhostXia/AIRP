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
