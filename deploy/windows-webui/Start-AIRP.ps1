$ErrorActionPreference = 'Stop'

$packageRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$engine = Join-Path $packageRoot 'airp-core.exe'
$webui = Join-Path $packageRoot 'webui'
if (-not (Test-Path -LiteralPath $engine -PathType Leaf)) {
    throw "Missing airp-core.exe in $packageRoot"
}
if (-not (Test-Path -LiteralPath (Join-Path $webui 'index.html') -PathType Leaf)) {
    throw "Missing webui/index.html in $packageRoot"
}

$dataRoot = Join-Path $packageRoot 'data'
$configPath = Join-Path $packageRoot 'config.json'
New-Item -ItemType Directory -Force -Path $dataRoot | Out-Null

$env:AIRP_DATA_DIR = $dataRoot
$env:AIRP_PERSIST_PROVIDER_KEY = 'true'
# The portable browser topology is fixed: loopback, same-origin, and content
# upload only. Do not inherit service/desktop topology privileges from a shell.
$env:AIRP_ALLOW_LOCAL_PATH = 'false'
Remove-Item Env:AIRP_ACCESS_KEY -ErrorAction SilentlyContinue
Remove-Item Env:AIRP_DEPLOYMENT_MODE -ErrorAction SilentlyContinue
Remove-Item Env:AIRP_PUBLIC_ORIGIN -ErrorAction SilentlyContinue
Remove-Item Env:AIRP_CORS_ORIGINS -ErrorAction SilentlyContinue
$origin = 'http://127.0.0.1:8765'
$arguments = @(
    '--config', ('"' + $configPath + '"'),
    'daemon',
    '--host', '127.0.0.1',
    '--port', '8765',
    '--webui-dir', ('"' + $webui + '"')
)

$process = Start-Process -FilePath $engine -ArgumentList $arguments -PassThru -NoNewWindow
try {
    $ready = $false
    for ($attempt = 0; $attempt -lt 80; $attempt++) {
        if ($process.HasExited) {
            throw "AIRP exited during startup with code $($process.ExitCode)."
        }
        try {
            $response = Invoke-WebRequest -UseBasicParsing -Uri "$origin/health" -TimeoutSec 1
            if ($response.StatusCode -eq 200) {
                $ready = $true
                break
            }
        } catch {
            Start-Sleep -Milliseconds 250
        }
    }
    if (-not $ready) {
        throw "AIRP did not become ready at $origin."
    }
    Start-Process $origin
    Write-Host "AIRP WebUI is running at $origin"
    Write-Host "User data: $dataRoot"
    Write-Host 'Close this window or press Ctrl+C to stop AIRP.'
    Wait-Process -Id $process.Id
} finally {
    if (-not $process.HasExited) {
        Stop-Process -Id $process.Id
    }
}
