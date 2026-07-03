$ErrorActionPreference = "Stop"

$env:RUSTUP_HOME = "D:\.rustup"
$env:CARGO_HOME = "D:\.cargo"
$env:npm_config_prefix = "D:\npm-global"
$env:npm_config_cache = "D:\npm-global\npm-cache"
$env:PATH = "D:\.cargo\bin;D:\msys64\mingw64\bin;D:\nodejs;" + $env:PATH

$repoRoot = Resolve-Path (Join-Path $PSScriptRoot "..")
$cargo = Join-Path $env:CARGO_HOME "bin\cargo.exe"
$npm = "D:\nodejs\npm.cmd"

if (-not (Test-Path -LiteralPath $cargo)) {
    throw "cargo.exe not found at $cargo"
}
if (-not (Test-Path -LiteralPath $npm)) {
    throw "npm.cmd not found at $npm"
}

Push-Location (Join-Path $repoRoot "ui")
try {
    Write-Host "=== npm run tauri -- build ==="
    Write-Host "tauri beforeBuildCommand runs build:engine-sidecar and npm run build"
    & $npm run tauri -- build
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
}
finally {
    Pop-Location
}
