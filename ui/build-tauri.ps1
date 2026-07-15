$ErrorActionPreference = "Stop"

$repoRoot = Resolve-Path (Join-Path $PSScriptRoot "..")
$cargo = $null
if ($env:CARGO_HOME) {
    $candidate = Join-Path $env:CARGO_HOME "bin\cargo.exe"
    if (Test-Path -LiteralPath $candidate) { $cargo = $candidate }
}
if (-not $cargo) {
    $cmd = Get-Command cargo -ErrorAction SilentlyContinue
    if ($cmd) { $cargo = $cmd.Source }
}
$npm = Get-Command npm.cmd -ErrorAction SilentlyContinue
if (-not $npm) { $npm = Get-Command npm -ErrorAction SilentlyContinue }
if (-not $cargo) { throw "cargo not found; configure CARGO_HOME or add cargo to PATH" }
if (-not $npm) { throw "npm not found; add npm to PATH" }
$npm = $npm.Source

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
