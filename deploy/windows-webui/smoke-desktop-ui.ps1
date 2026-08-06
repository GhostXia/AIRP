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

# 便携包体数据共用前提：包内 data/ 不预建。壳在便携模式（包体标记
# airp-core.exe + webui/index.html 齐备）下以 exe 同目录 data/ 为数据根，
# setup 会 create_dir_all 补齐；保持全新解压包状态可顺带回归断言
# "首次桌面启动即共享包内 data/"（审计 P1 修复）。

# 端口必须空闲：若残留 engine 占用（壳会走 ReuseExternalHosting 分支，
# 退出不 kill 外部进程，后面"退出即停止"断言会误判），直接失败提示。
# 判定"任何 HTTP 响应即视为占用"：IWR 成功（2xx）走 try，非 2xx 抛异常
# 且 Exception.Response 非 null；连接失败（端口空闲）抛异常但 Response 为 null。
# 必须用标志位 + try 外 throw：脚本自身 throw 抛 RuntimeException（无 Response），
# 若放在 try 内会被 catch 吞掉（审计 B-01 实测复现）。
$portOccupied = $false
try {
    $null = Invoke-WebRequest -UseBasicParsing -Uri "http://127.0.0.1:$Port/version" -TimeoutSec 1
    $portOccupied = $true
}
catch {
    if ($_.Exception.Response) {
        $portOccupied = $true
    }
    # 连接失败且无 HTTP 响应 = 端口空闲，继续
}
if ($portOccupied) {
    throw "Port $Port is already serving an engine; clean up the leftover process before desktop smoke."
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
    # 失败路径清理：壳被强杀时 sidecar 可能存活（锁文件还在），先按归属
    # 锁的 engine_pid 清理残留 engine，再强杀壳，避免占端口/挡下次 smoke。
    if (Test-Path -LiteralPath $lock -PathType Leaf) {
        try {
            $lockJson = Get-Content -LiteralPath $lock -Raw | ConvertFrom-Json
            if ($lockJson.engine_pid) {
                if (Get-Process -Id $lockJson.engine_pid -ErrorAction SilentlyContinue) {
                    Stop-Process -Id $lockJson.engine_pid -Force
                    Write-Host "Cleaned up leftover engine process $($lockJson.engine_pid)"
                }
            }
        }
        catch {
            Write-Warning "Failed to clean leftover engine from lock: $_"
        }
    }
    if ($uiProcess -and -not $uiProcess.HasExited) {
        Stop-Process -Id $uiProcess.Id -Force
    }
    Remove-Item Env:AIRP_DAEMON_PORT -ErrorAction SilentlyContinue
}
