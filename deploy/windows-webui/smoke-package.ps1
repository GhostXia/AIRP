param(
    [string]$PackageRoot = (Join-Path $PSScriptRoot '..\..\dist\airp-webui-windows-x64'),
    [int]$Port = 8765,
    [switch]$BrowserSmoke
)

$ErrorActionPreference = 'Stop'
$package = (Resolve-Path $PackageRoot).Path
$engine = Join-Path $package 'airp-core.exe'
$webui = Join-Path $package 'webui'
$data = Join-Path $package 'data'
$config = Join-Path $package 'config.json'
$origin = "http://127.0.0.1:$Port"
$env:AIRP_DATA_DIR = $data
$env:AIRP_PERSIST_PROVIDER_KEY = 'true'
$env:AIRP_ALLOW_LOCAL_PATH = 'false'
Remove-Item Env:AIRP_ACCESS_KEY -ErrorAction SilentlyContinue
Remove-Item Env:AIRP_DEPLOYMENT_MODE -ErrorAction SilentlyContinue
Remove-Item Env:AIRP_PUBLIC_ORIGIN -ErrorAction SilentlyContinue
Remove-Item Env:AIRP_CORS_ORIGINS -ErrorAction SilentlyContinue
$arguments = @(
    '--config', ('"' + $config + '"'),
    'daemon', '--host', '127.0.0.1', '--port', $Port,
    '--webui-dir', ('"' + $webui + '"')
)

$process = Start-Process -FilePath $engine -ArgumentList $arguments -PassThru -WindowStyle Hidden
try {
    $ready = $false
    for ($attempt = 0; $attempt -lt 80; $attempt++) {
        try {
            $health = Invoke-WebRequest -UseBasicParsing -Uri "$origin/health" -TimeoutSec 1
            if ($health.StatusCode -eq 200) { $ready = $true; break }
        } catch {
            if ($process.HasExited) {
                throw "Packaged AIRP exited with code $($process.ExitCode)."
            }
            Start-Sleep -Milliseconds 250
        }
    }
    if (-not $ready) { throw "Packaged AIRP did not become ready at $origin." }

    $root = Invoke-WebRequest -UseBasicParsing -Uri "$origin/"
    $runtime = Invoke-WebRequest -UseBasicParsing -Uri "$origin/runtime-config.js"
    if ($root.StatusCode -ne 200) { throw "WebUI returned $($root.StatusCode)." }
    if ($runtime.Content -notmatch "mode: 'local'") { throw 'Local runtime mode was not injected.' }
    if ($root.Headers['Cache-Control'] -ne 'no-store') { throw 'WebUI cache policy is not no-store.' }
    if ($root.Headers['Content-Security-Policy'] -notmatch "script-src 'self'") {
        throw 'WebUI CSP is missing the same-origin script boundary.'
    }
    if (-not (Test-Path -LiteralPath $data -PathType Container)) {
        throw 'Portable data directory was not created inside the package.'
    }
    if (-not (Test-Path -LiteralPath $config -PathType Leaf)) {
        throw 'Portable config.json was not created inside the package.'
    }
    Write-Host "Packaged WebUI smoke passed at $origin"
    Write-Host "Portable data boundary: $data"
    if ($BrowserSmoke) {
        $chrome = if ($env:AIRP_CHROME_PATH) {
            $env:AIRP_CHROME_PATH
        } else {
            'C:\Program Files\Google\Chrome\Application\chrome.exe'
        }
        if (-not (Test-Path -LiteralPath $chrome -PathType Leaf)) {
            throw "Chrome not found: $chrome"
        }
        $repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
        $env:AIRP_SMOKE_ORIGIN = $origin
        $env:AIRP_CHROME_PATH = $chrome
        & node (Join-Path $repoRoot 'ui\local-webui-browser-smoke.mjs')
        if ($LASTEXITCODE -ne 0) { throw "Browser smoke failed with code $LASTEXITCODE." }
    }

    $smokeKey = 'airp-smoke-provider-key-never-use'
    $settingsBody = @{
        provider = 'OpenAI'
        endpoint = 'https://provider.invalid/v1/chat/completions'
        api_key = $smokeKey
        model = 'smoke-model'
    } | ConvertTo-Json
    Invoke-WebRequest -UseBasicParsing -Uri "$origin/v1/settings" -Method Post `
        -ContentType 'application/json' -Body $settingsBody | Out-Null
    $secretFile = Join-Path $data 'secrets.json'
    if (-not (Test-Path -LiteralPath $secretFile -PathType Leaf)) {
        throw 'Provider secrets.json was not created.'
    }
    $secretState = Get-Content -LiteralPath $secretFile -Raw | ConvertFrom-Json
    if ($secretState.version -ne 1 -or $secretState.provider_api_key -ne $smokeKey) {
        throw 'Provider secrets.json does not match the versioned single-key contract.'
    }
    foreach ($nonSecretFile in @($config, (Join-Path $data 'settings.json'))) {
        if ((Get-Content -LiteralPath $nonSecretFile -Raw).Contains($smokeKey)) {
            throw "Provider key leaked into non-secret file: $nonSecretFile"
        }
    }

    if (-not $process.HasExited) {
        Stop-Process -Id $process.Id
    }
    if (-not $process.WaitForExit(5000)) {
        throw 'Packaged AIRP did not stop before the provider-key restart check.'
    }
    $process = Start-Process -FilePath $engine -ArgumentList $arguments -PassThru -WindowStyle Hidden
    $restartReady = $false
    for ($attempt = 0; $attempt -lt 80; $attempt++) {
        try {
            $restartHealth = Invoke-WebRequest -UseBasicParsing -Uri "$origin/health" -TimeoutSec 1
            $restartState = $restartHealth.Content | ConvertFrom-Json
            if ($restartHealth.StatusCode -eq 200 -and $restartState.provider_configured) {
                $restartReady = $true
                break
            }
        } catch {
            if ($process.HasExited) {
                throw "Packaged AIRP exited after provider-key restart with code $($process.ExitCode)."
            }
            Start-Sleep -Milliseconds 250
        }
    }
    if (-not $restartReady) {
        throw 'Provider key was not restored from secrets.json after engine restart.'
    }
    Write-Host "Provider-key restart smoke passed: $secretFile"
} finally {
    if ($process -and -not $process.HasExited) {
        Stop-Process -Id $process.Id
    }
    Remove-Item Env:AIRP_DATA_DIR -ErrorAction SilentlyContinue
    Remove-Item Env:AIRP_SMOKE_ORIGIN -ErrorAction SilentlyContinue
}
