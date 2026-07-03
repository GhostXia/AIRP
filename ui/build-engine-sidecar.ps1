# Build the airp-core engine binary and place it where Tauri's sidecar
# bundling expects it: ui/src-tauri/binaries/airp-core-$TARGET_TRIPLE[.exe].
# Run before `npm run tauri -- build` (or build-tauri.ps1 will call this).
# 首要目标 (DEV-GUIDE §0): make the packaged .exe self-contained.
$ErrorActionPreference = "Stop"

$env:RUSTUP_HOME = "D:\.rustup"
$env:CARGO_HOME = "D:\.cargo"
$env:PATH = "D:\.cargo\bin;D:\msys64\mingw64\bin;D:\nodejs;" + $env:PATH

$repoRoot = Resolve-Path (Join-Path $PSScriptRoot "..")
$cargo = Join-Path $env:CARGO_HOME "bin\cargo.exe"
if (-not (Test-Path -LiteralPath $cargo)) { throw "cargo.exe not found at $cargo" }

# Host target triple (e.g. x86_64-pc-windows-gnu). Tauri sidecar requires the
# binary be named airp-core-$TRIPLE.exe on Windows.
$triple = & $cargo -vV 2>$null | Select-String -Pattern "^host:" | ForEach-Object { ($_ -split ":\s*")[1].Trim() }
if (-not $triple) { throw "could not detect host target triple" }
Write-Host "host triple: $triple"

Write-Host "=== cargo build --release -p airp-core --bin airp-core ==="
& $cargo build --release -p airp-core --bin airp-core
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$src = Join-Path $repoRoot "target\release\airp-core.exe"
if (-not (Test-Path -LiteralPath $src)) { throw "engine binary not found at $src" }

$binDir = Join-Path $PSScriptRoot "src-tauri\binaries"
New-Item -ItemType Directory -Force -Path $binDir | Out-Null
$dst = Join-Path $binDir "airp-core-$triple.exe"
Copy-Item -LiteralPath $src -Destination $dst -Force
Write-Host "engine sidecar placed: $dst"
