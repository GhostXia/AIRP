param(
    [string]$PackageRoot = (Join-Path $PSScriptRoot '..\..\dist\airp-webui-windows-x64'),
    [int]$Port = 18765
)

$ErrorActionPreference = 'Stop'
$package = (Resolve-Path $PackageRoot).Path
$ui = Join-Path $package 'airp-ui.exe'
$engine = Join-Path $package 'airp-core.exe'
$webui = Join-Path $package 'webui'
$data = Join-Path $package 'data'
$lock = Join-Path $data 'engine-instance.lock'

if (-not (Test-Path -LiteralPath $ui -PathType Leaf)) {
    throw 'Portable airp-ui.exe is missing.'
}
if (-not (Test-Path -LiteralPath $engine -PathType Leaf)) {
    throw 'Portable airp-core.exe is missing.'
}
if (-not (Test-Path -LiteralPath (Join-Path $webui 'index.html') -PathType Leaf)) {
    throw 'Portable webui/index.html is missing.'
}

# 便携包体数据共用前提：包内 data/ 目录存在（webui 用户运行过
# Start-AIRP.cmd 即已创建；桌面壳便携模式以 exe 同目录 data/ 为数据根，
# 未运行过时由壳 setup 自动创建，这里模拟已存在场景）。
New-Item -ItemType Directory -Force -Path $data | Out-Null

# 端口必须空闲：若残留 engine 占用（壳会走 ReuseExternalHosting 分支，
# 退出不 kill 外部进程，后面"退出即停止"断言会误判），直接失败提示。
try {
    $probe = Invoke-WebRequest -UseBasicParsing -Uri "http://127.0.0.1:$Port/version" -TimeoutSec 1
    if ($probe.StatusCode -eq 200) {
        throw "Port $Port is already serving an engine; clean up the leftover process before desktop smoke."
    }
}
catch {
    # 连接失败 = 端口空闲，继续
}

# 清环境干扰：继承的 engine 地址/access key 会让壳跳过捆绑 sidecar 或
# 改变 bearer 通道，破坏"从包体拉起 engine"的验证语义。
Remove-Item Env:AIRP_ENGINE_URL -ErrorAction SilentlyContinue
Remove-Item Env:AIRP_ACCESS_KEY -ErrorAction SilentlyContinue
Remove-Item Env:AIRP_DATA_DIR -ErrorAction SilentlyContinue
Remove-Item Env:AIRP_WEBUI_DIR -ErrorAction SilentlyContinue
$env:AIRP_DAEMON_PORT = "$Port"

$uiProcess = $null
try {
    $uiProcess = Start-Process -FilePath $ui -WorkingDirectory $package -PassThru -WindowStyle Hidden

    # 1. 就绪：壳自启的捆绑 engine 在指定端口响应 /version。
    $ready = $false
    for ($attempt = 0; $attempt -lt 60; $attempt++) {
        if ($uiProcess.HasExited) {
            throw "AIRP UI exited early with code $($uiProcess.ExitCode)"
        }
        try {
            $response = Invoke-RestMethod -Uri "http://127.0.0.1:$Port/version" -TimeoutSec 1
            if ($response.name -eq 'airp-core') { $ready = $true; break }
        }
        catch { Start-Sleep -Milliseconds 250 }
    }
    if (-not $ready) { throw "bundled engine did not become ready on port $Port" }

    # 2. 同源承载：engine 以 desktop router 承载 webui（与 webui 便携包
    #    同一份 webui/ 资产，AIRP_DESKTOP_WEBUI_DIR 指向包体 webui）。
    $runtimeConfig = Invoke-WebRequest -UseBasicParsing -Uri "http://127.0.0.1:$Port/runtime-config.js" -TimeoutSec 5
    if ($runtimeConfig.StatusCode -ne 200) {
        throw "engine is not hosting the WebUI (runtime-config.js -> $($runtimeConfig.StatusCode))"
    }
    if ($runtimeConfig.Content -notmatch "mode: 'desktop'") {
        throw "runtime-config.js does not report desktop mode: $($runtimeConfig.Content)"
    }

    # 3. 包体 webui/ 资产可访问（壳解析 resource_dir = exe 同目录 → webui）。
    $root = Invoke-WebRequest -UseBasicParsing -Uri "http://127.0.0.1:$Port/" -TimeoutSec 5
    $rolePage = Invoke-WebRequest -UseBasicParsing -Uri "http://127.0.0.1:$Port/screens/01-role-list.html" -TimeoutSec 5
    if ($root.StatusCode -ne 200) { throw "WebUI root returned $($root.StatusCode)" }
    if ($rolePage.StatusCode -ne 200) { throw "Bundled webui screens were not hosted" }

    # 4. 数据共用：壳与 webui 便携包共用包内 data/（锁文件落在包内证明
    #    便携数据根生效，而非 %APPDATA%）。
    if (-not (Test-Path -LiteralPath $lock -PathType Leaf)) {
        throw "engine instance lock not found under $lock; desktop shell did not use the package data\ folder"
    }
    Write-Host "Desktop shell uses the shared package data folder: $data"

    # 5. 优雅退出：关窗口 → 壳退出 → sidecar 停止 → 归属锁清理。
    if (-not $uiProcess.CloseMainWindow()) {
        throw 'could not request a graceful UI shutdown'
    }
    if (-not $uiProcess.WaitForExit(10000)) {
        throw 'AIRP UI did not exit after window close'
    }
    $stopped = $false
    for ($attempt = 0; $attempt -lt 40; $attempt++) {
        try { Invoke-RestMethod -Uri "http://127.0.0.1:$Port/version" -TimeoutSec 1 | Out-Null }
        catch { $stopped = $true; break }
        Start-Sleep -Milliseconds 250
    }
    if (-not $stopped) { throw 'engine sidecar remained alive after UI exit' }
    if (Test-Path -LiteralPath $lock -PathType Leaf) {
        throw 'engine instance lock was not cleaned up after UI exit'
    }
    Write-Host 'Desktop UI smoke passed: readiness, same-origin hosting, shared data folder, graceful exit, and lock cleanup.'
}
finally {
    if ($uiProcess -and -not $uiProcess.HasExited) {
        Stop-Process -Id $uiProcess.Id -Force
    }
    Remove-Item Env:AIRP_DAEMON_PORT -ErrorAction SilentlyContinue
}
