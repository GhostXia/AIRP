param(
    [string]$PackageRoot = (Join-Path $PSScriptRoot '..\..\dist\airp-webui-windows-x64'),
    [int]$Port = 18765
)

$ErrorActionPreference = 'Stop'
$packageSource = (Resolve-Path $PackageRoot).Path
$scratchRoot = $null
$package = $packageSource
$ui = Join-Path $package 'airp-ui.exe'
$engine = Join-Path $package 'airp-core.exe'
$webui = Join-Path $package 'webui'
$data = Join-Path $package 'data'
$lock = Join-Path $data 'engine-instance.lock'
$debugPort = $Port + 1
$restartEvidence = Join-Path ([System.IO.Path]::GetTempPath()) `
    ("airp-desktop-restart-" + [Guid]::NewGuid().ToString('N') + '.json')
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path

if (-not (Test-Path -LiteralPath $ui -PathType Leaf)) {
    throw 'Portable airp-ui.exe is missing.'
}
if (-not (Test-Path -LiteralPath $engine -PathType Leaf)) {
    throw 'Portable airp-core.exe is missing.'
}
if (-not (Test-Path -LiteralPath (Join-Path $webui 'index.html') -PathType Leaf)) {
    throw 'Portable webui/index.html is missing.'
}

function Assert-LockHasNoOwner {
    param([Parameter(Mandatory = $true)][string]$Path)

    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "engine instance lock file is missing: $Path"
    }
    $raw = Get-Content -LiteralPath $Path -Raw
    if ([string]::IsNullOrWhiteSpace($raw)) {
        return
    }
    throw "engine instance lock still contains non-empty content: $Path"
}

function Assert-LiveLockOwner {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][int]$ExpectedShellPid,
        [Parameter(Mandatory = $true)][int]$ExpectedPort,
        [Parameter(Mandatory = $true)][System.Diagnostics.Process]$ExpectedShellProcess
    )

    $lastError = 'owner record was not available'
    for ($attempt = 0; $attempt -lt 20; $attempt++) {
        if ($ExpectedShellProcess.HasExited) {
            throw "AIRP UI shell PID $ExpectedShellPid exited before owner record was verified"
        }
        try {
            if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
                throw "engine instance lock missing under $Path"
            }
            $raw = Get-Content -LiteralPath $Path -Raw
            if ([string]::IsNullOrWhiteSpace($raw)) {
                throw "engine instance lock is empty while shell PID $ExpectedShellPid is running"
            }
            $record = $raw | ConvertFrom-Json
            $shellPid = [int64]$record.shell_pid
            $enginePid = [int64]$record.engine_pid
            $port = [int64]$record.port
            $instanceId = [string]$record.instance_id
            if ($shellPid -ne [int64]$ExpectedShellPid) {
                throw "engine instance lock shell_pid $shellPid does not match UI PID $ExpectedShellPid"
            }
            if ($enginePid -le 0) {
                throw "engine instance lock engine_pid is invalid: $enginePid"
            }
            if ($port -ne [int64]$ExpectedPort) {
                throw "engine instance lock port $port does not match smoke port $ExpectedPort"
            }
            if ([string]::IsNullOrWhiteSpace($instanceId)) {
                throw 'engine instance lock instance_id is empty'
            }
            $null = [Guid]::Parse($instanceId)
            return
        }
        catch {
            $lastError = $_.Exception.Message
            Start-Sleep -Milliseconds 100
        }
    }
    throw "engine instance lock owner verification timed out: $lastError"
}

function Assert-PortAvailable {
    param(
        [Parameter(Mandatory = $true)][int]$PortNumber,
        [Parameter(Mandatory = $true)][string]$Purpose
    )

    if (Get-NetTCPConnection -LocalPort $PortNumber -State Listen -ErrorAction SilentlyContinue) {
        throw "$Purpose port $PortNumber is already in use; choose a free port before desktop smoke."
    }
}

function Get-OwnedEngineProcess {
    param(
        [Parameter(Mandatory = $true)]$LockRecord,
        [Parameter(Mandatory = $true)][string]$ExpectedEnginePath,
        [Parameter(Mandatory = $true)][int]$ExpectedPort,
        [Parameter(Mandatory = $true)][int[]]$AllowedShellPids
    )

    $shellPid = [int]$LockRecord.shell_pid
    $enginePid = [int]$LockRecord.engine_pid
    if ($AllowedShellPids -notcontains $shellPid) {
        throw "lock shell PID $shellPid was not launched by this smoke"
    }
    if ([int]$LockRecord.port -ne $ExpectedPort) {
        throw "lock port $($LockRecord.port) does not match smoke port $ExpectedPort"
    }
    $process = Get-Process -Id $enginePid -ErrorAction Stop
    if (-not [string]::Equals(
        [System.IO.Path]::GetFullPath($process.Path),
        [System.IO.Path]::GetFullPath($ExpectedEnginePath),
        [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "lock PID $enginePid does not run the isolated packaged airp-core.exe"
    }
    $processInfo = Get-CimInstance Win32_Process -Filter "ProcessId = $enginePid"
    if (-not $processInfo -or [int]$processInfo.ParentProcessId -ne $shellPid) {
        throw "lock PID $enginePid is not a child of owned shell PID $shellPid"
    }
    return $process
}

# 便携包体数据共用前提：包内 data/ 不预建。壳在便携模式（包体标记
# airp-core.exe + webui/index.html 齐备）下以 exe 同目录 data/ 为数据根，
# setup 会 create_dir_all 补齐；保持全新解压包状态可顺带回归断言
# "首次桌面启动即共享包内 data/"（审计 P1 修复）。

# Engine 与 WebView2 调试监听都必须由本次 smoke 独占。若任一端口已被
# 占用，直接失败；清理阶段绝不根据端口反向查找并终止未知进程。
Assert-PortAvailable -PortNumber $Port -Purpose 'Engine'
Assert-PortAvailable -PortNumber $debugPort -Purpose 'WebView2 debug'

# Never run this stateful smoke against the supplied package in place. Copy
# package assets into a unique root and exclude any existing portable data/.
$scratchRoot = Join-Path ([System.IO.Path]::GetTempPath()) `
    ("airp-desktop-smoke-" + [Guid]::NewGuid().ToString('N'))
$package = Join-Path $scratchRoot 'package'
New-Item -ItemType Directory -Path $package -Force | Out-Null
Get-ChildItem -LiteralPath $packageSource | Where-Object { $_.Name -ne 'data' } | ForEach-Object {
    Copy-Item -LiteralPath $_.FullName -Destination $package -Recurse
}
$ui = Join-Path $package 'airp-ui.exe'
$engine = Join-Path $package 'airp-core.exe'
$webui = Join-Path $package 'webui'
$data = Join-Path $package 'data'
$lock = Join-Path $data 'engine-instance.lock'

# 清环境干扰：继承的 engine 地址/access key 会让壳跳过捆绑 sidecar 或
# 改变 bearer 通道，破坏"从包体拉起 engine"的验证语义。
Remove-Item Env:AIRP_ENGINE_URL -ErrorAction SilentlyContinue
Remove-Item Env:AIRP_ACCESS_KEY -ErrorAction SilentlyContinue
Remove-Item Env:AIRP_DATA_DIR -ErrorAction SilentlyContinue
Remove-Item Env:AIRP_WEBUI_DIR -ErrorAction SilentlyContinue
$env:AIRP_DAEMON_PORT = "$Port"
$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = "--remote-debugging-port=$debugPort"
$env:AIRP_SMOKE_CDP_URL = "http://127.0.0.1:$debugPort"
$env:AIRP_SMOKE_ORIGIN = "http://127.0.0.1:$Port"
$env:AIRP_SMOKE_RESTART_EVIDENCE_FILE = $restartEvidence

$uiProcess = $null
$secondUiProcess = $null
$launchedShellPids = [System.Collections.Generic.List[int]]::new()
try {
    $uiProcess = Start-Process -FilePath $ui -WorkingDirectory $package -PassThru -WindowStyle Hidden
    $launchedShellPids.Add($uiProcess.Id)

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
    Assert-LiveLockOwner -Path $lock -ExpectedShellPid $uiProcess.Id -ExpectedPort $Port -ExpectedShellProcess $uiProcess
    Write-Host "Desktop shell uses the shared package data folder: $data"

    # 5. Real WebView2 credential/recovery evidence. Attach to the packaged
    # desktop WebView over a loopback-only smoke CDP port, create durable
    # Memory/State through authenticated Engine intents, then force-kill only
    # the owned Engine. The still-running shell must respawn it, exchange a new
    # short-lived token, reconnect the existing WebView, and recover authority.
    & node (Join-Path $repoRoot 'ui\packaged-desktop-restart-smoke.mjs') before
    if ($LASTEXITCODE -ne 0) {
        throw "Packaged desktop pre-restart evidence failed with code $LASTEXITCODE."
    }
    $beforeLock = Get-Content -LiteralPath $lock -Raw | ConvertFrom-Json
    $terminatedEnginePid = [int]$beforeLock.engine_pid
    $terminatedInstanceId = [string]$beforeLock.instance_id
    $terminatedEngine = Get-OwnedEngineProcess -LockRecord $beforeLock `
        -ExpectedEnginePath $engine -ExpectedPort $Port `
        -AllowedShellPids $launchedShellPids.ToArray()
    Stop-Process -InputObject $terminatedEngine -Force

    $recovered = $false
    for ($attempt = 0; $attempt -lt 120; $attempt++) {
        if ($uiProcess.HasExited) {
            throw "AIRP UI exited while recovering Engine PID $terminatedEnginePid"
        }
        try {
            $currentLock = Get-Content -LiteralPath $lock -Raw | ConvertFrom-Json
            $currentEnginePid = [int]$currentLock.engine_pid
            $version = Invoke-RestMethod -Uri "http://127.0.0.1:$Port/version" -TimeoutSec 1
            if ($currentLock.shell_pid -eq $uiProcess.Id -and
                $currentEnginePid -gt 0 -and
                $currentEnginePid -ne $terminatedEnginePid -and
                [string]$currentLock.instance_id -ne $terminatedInstanceId -and
                $version.name -eq 'airp-core') {
                $recovered = $true
                break
            }
        }
        catch { }
        Start-Sleep -Milliseconds 250
    }
    if (-not $recovered) {
        throw "Desktop shell did not recover terminated Engine PID $terminatedEnginePid."
    }
    Assert-LiveLockOwner -Path $lock -ExpectedShellPid $uiProcess.Id -ExpectedPort $Port -ExpectedShellProcess $uiProcess
    & node (Join-Path $repoRoot 'ui\packaged-desktop-restart-smoke.mjs') after
    if ($LASTEXITCODE -ne 0) {
        throw "Packaged desktop post-restart evidence failed with code $LASTEXITCODE."
    }

    # 6. 优雅退出：关窗口 → 壳退出 → sidecar 停止 → 归属锁清理。
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
    Assert-LockHasNoOwner -Path $lock

    # 7. Reopen through the explicit Blueprint entry. The lock inode is
    # retained; this also proves both shell entries share lifecycle semantics.
    Remove-Item Env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS -ErrorAction SilentlyContinue
    $env:AIRP_DESKTOP_UI = 'blueprint'
    $secondUiProcess = Start-Process -FilePath $ui -WorkingDirectory $package -PassThru -WindowStyle Hidden
    $launchedShellPids.Add($secondUiProcess.Id)
    try {
        $secondReady = $false
        for ($attempt = 0; $attempt -lt 60; $attempt++) {
            if ($secondUiProcess.HasExited) {
                throw "AIRP UI second launch exited early with code $($secondUiProcess.ExitCode)"
            }
            try {
                $response = Invoke-RestMethod -Uri "http://127.0.0.1:$Port/version" -TimeoutSec 1
                if ($response.name -eq 'airp-core') { $secondReady = $true; break }
            }
            catch { Start-Sleep -Milliseconds 250 }
        }
        if (-not $secondReady) { throw "bundled engine did not become ready on second launch on port $Port" }
        $desktopRoot = Invoke-WebRequest -UseBasicParsing -Uri "http://127.0.0.1:$Port/desktop/" -TimeoutSec 5
        if ($desktopRoot.StatusCode -ne 200 -or $desktopRoot.Content -notmatch '/desktop/assets/') {
            throw 'Blueprint desktop bundle was not hosted on the explicit second launch'
        }
        Assert-LiveLockOwner -Path $lock -ExpectedShellPid $secondUiProcess.Id -ExpectedPort $Port -ExpectedShellProcess $secondUiProcess
        if (-not $secondUiProcess.CloseMainWindow()) {
            throw 'could not request graceful shutdown for second UI launch'
        }
        if (-not $secondUiProcess.WaitForExit(10000)) {
            throw 'AIRP UI second launch did not exit after window close'
        }
        $secondStopped = $false
        for ($attempt = 0; $attempt -lt 40; $attempt++) {
            try { Invoke-RestMethod -Uri "http://127.0.0.1:$Port/version" -TimeoutSec 1 | Out-Null }
            catch { $secondStopped = $true; break }
            Start-Sleep -Milliseconds 250
        }
        if (-not $secondStopped) { throw 'engine sidecar remained alive after second UI exit' }
        Assert-LockHasNoOwner -Path $lock
    }
    finally {
        if ($secondUiProcess -and -not $secondUiProcess.HasExited) {
            Stop-Process -Id $secondUiProcess.Id -Force
        }
    }
    Write-Host 'Desktop UI smoke passed: readiness, same-origin hosting, shared data folder, graceful exit, lock reuse, and cleanup.'
}
finally {
    Remove-Item Env:AIRP_DESKTOP_UI -ErrorAction SilentlyContinue
    if ($secondUiProcess -and -not $secondUiProcess.HasExited) {
        Stop-Process -Id $secondUiProcess.Id -Force
        $secondUiProcess.WaitForExit(5000) | Out-Null
    }
    if ($uiProcess -and -not $uiProcess.HasExited) {
        Stop-Process -Id $uiProcess.Id -Force
        $uiProcess.WaitForExit(5000) | Out-Null
    }
    # Failure cleanup acts only on the exact isolated executable recorded by a
    # lock whose shell PID was launched by this smoke. Never kill a PID merely
    # because it currently owns the test port.
    if (Test-Path -LiteralPath $lock -PathType Leaf) {
        try {
            $lockRaw = Get-Content -LiteralPath $lock -Raw
            if (-not [string]::IsNullOrWhiteSpace($lockRaw)) {
                $lockJson = $lockRaw | ConvertFrom-Json
                if ($lockJson.engine_pid) {
                    $ownedEngine = Get-OwnedEngineProcess -LockRecord $lockJson `
                        -ExpectedEnginePath $engine -ExpectedPort $Port `
                        -AllowedShellPids $launchedShellPids.ToArray()
                    Stop-Process -InputObject $ownedEngine -Force
                    Write-Host "Cleaned up owned leftover engine process $($lockJson.engine_pid)"
                }
            }
        }
        catch {
            Write-Warning "Failed to clean leftover engine from lock: $_"
        }
    }
    Remove-Item Env:AIRP_DAEMON_PORT -ErrorAction SilentlyContinue
    Remove-Item Env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS -ErrorAction SilentlyContinue
    Remove-Item Env:AIRP_SMOKE_CDP_URL -ErrorAction SilentlyContinue
    Remove-Item Env:AIRP_SMOKE_ORIGIN -ErrorAction SilentlyContinue
    Remove-Item Env:AIRP_SMOKE_RESTART_EVIDENCE_FILE -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $restartEvidence -Force -ErrorAction SilentlyContinue
    if ($scratchRoot -and (Test-Path -LiteralPath $scratchRoot -PathType Container)) {
        Remove-Item -LiteralPath $scratchRoot -Recurse -Force
    }
}
