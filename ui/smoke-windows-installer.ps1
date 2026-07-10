$ErrorActionPreference = "Stop"

$bundleDir = Join-Path $PSScriptRoot "..\target\release\bundle\nsis"
$installer = Get-ChildItem -LiteralPath $bundleDir -Filter "*.exe" -File |
    Sort-Object LastWriteTime -Descending |
    Select-Object -First 1
if (-not $installer) { throw "NSIS installer not found under $bundleDir" }

$root = Join-Path $env:RUNNER_TEMP "airp-installer-smoke"
$installDir = Join-Path $root "app"
$port = 18765
$uiProcess = $null

New-Item -ItemType Directory -Force -Path $installDir | Out-Null
try {
    $install = Start-Process -FilePath $installer.FullName -ArgumentList @("/S", "/D=$installDir") -Wait -PassThru -WindowStyle Hidden
    if ($install.ExitCode -ne 0) { throw "installer exited with $($install.ExitCode)" }

    $appExe = Get-ChildItem -LiteralPath $installDir -Recurse -Filter "airp-ui.exe" -File |
        Select-Object -First 1
    if (-not $appExe) { throw "installed airp-ui.exe not found under $installDir" }

    $env:AIRP_DAEMON_PORT = "$port"
    $uiProcess = Start-Process -FilePath $appExe.FullName -PassThru -WindowStyle Hidden
    $ready = $false
    for ($attempt = 0; $attempt -lt 60; $attempt++) {
        if ($uiProcess.HasExited) { throw "AIRP UI exited before sidecar readiness" }
        try {
            $response = Invoke-RestMethod -Uri "http://127.0.0.1:$port/version" -TimeoutSec 1
            if ($response.name -eq "airp-core") { $ready = $true; break }
        }
        catch { Start-Sleep -Milliseconds 250 }
    }
    if (-not $ready) { throw "bundled engine did not become ready on port $port" }

    if (-not $uiProcess.CloseMainWindow()) { throw "could not request a graceful UI shutdown" }
    if (-not $uiProcess.WaitForExit(10000)) { throw "AIRP UI did not exit after window close" }
    $stopped = $false
    for ($attempt = 0; $attempt -lt 40; $attempt++) {
        try { Invoke-RestMethod -Uri "http://127.0.0.1:$port/version" -TimeoutSec 1 | Out-Null }
        catch { $stopped = $true; break }
        Start-Sleep -Milliseconds 250
    }
    if (-not $stopped) { throw "engine sidecar remained alive after UI exit" }
    Write-Host "Installer smoke passed: install, launch, readiness, and sidecar shutdown."
}
finally {
    if ($uiProcess -and -not $uiProcess.HasExited) { Stop-Process -Id $uiProcess.Id -Force }
}
