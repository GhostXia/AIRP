$ErrorActionPreference = "Stop"

if (-not $env:RUSTUP_HOME) { $env:RUSTUP_HOME = "D:\.rustup" }
if (-not $env:CARGO_HOME) { $env:CARGO_HOME = "D:\.cargo" }
if (-not $env:npm_config_prefix) { $env:npm_config_prefix = "D:\npm-global" }
if (-not $env:npm_config_cache) { $env:npm_config_cache = "D:\npm-global\npm-cache" }
if (Test-Path -LiteralPath "D:\msys64\mingw64\bin") {
    $env:PATH = "D:\msys64\mingw64\bin;" + $env:PATH
}
if (Test-Path -LiteralPath "D:\nodejs") {
    $env:PATH = "D:\nodejs;" + $env:PATH
}
if (Test-Path -LiteralPath (Join-Path $env:CARGO_HOME "bin")) {
    $env:PATH = (Join-Path $env:CARGO_HOME "bin") + ";" + $env:PATH
}

$repoRoot = Resolve-Path (Join-Path $PSScriptRoot "..")
$cargo = Join-Path $env:CARGO_HOME "bin\cargo.exe"
$npm = "D:\nodejs\npm.cmd"

if (-not (Test-Path -LiteralPath $cargo)) {
    $cmd = Get-Command cargo -ErrorAction SilentlyContinue
    if ($cmd) { $cargo = $cmd.Source }
}
if (-not (Test-Path -LiteralPath $npm)) {
    $cmd = Get-Command npm -ErrorAction SilentlyContinue
    if ($cmd) { $npm = $cmd.Source }
}
if (-not (Test-Path -LiteralPath $cargo)) { throw "cargo.exe not found" }
if (-not (Test-Path -LiteralPath $npm)) { throw "npm.cmd not found" }

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
