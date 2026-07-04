# Build the airp-core engine binary and place it where Tauri's sidecar
# bundling expects it: ui/src-tauri/binaries/airp-core-$TARGET_TRIPLE[.exe].
# Run before `npm run tauri -- build` (or build-tauri.ps1 will call this).
# 首要目标 (DEV-GUIDE §0): make the packaged .exe self-contained.
# WINDOWS-ONLY: this script builds the Windows `.exe` sidecar for the host
# target triple reported by Cargo. Cross-target packaging needs a matching
# sidecar builder for that target.
$ErrorActionPreference = "Stop"

if (-not $env:RUSTUP_HOME) { $env:RUSTUP_HOME = "D:\.rustup" }
if (-not $env:CARGO_HOME) { $env:CARGO_HOME = "D:\.cargo" }
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
if (-not (Test-Path -LiteralPath $cargo)) {
    $cmd = Get-Command cargo -ErrorAction SilentlyContinue
    if ($cmd) { $cargo = $cmd.Source }
}
if (-not (Test-Path -LiteralPath $cargo)) { throw "cargo.exe not found" }

# Host target triple (e.g. x86_64-pc-windows-gnu). Tauri sidecar requires a
# platform-suffixed name, but the Tauri CLI may resolve the Windows suffix as
# x86_64-pc-windows-msvc even when this repo builds Rust with the GNU toolchain.
# The produced Windows executable is still runnable; write both common suffixes
# so bundling does not depend on the CLI/toolchain naming mismatch.
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
$triples = [System.Collections.Generic.List[string]]::new()
$triples.Add($triple)
if ($triple -eq "x86_64-pc-windows-gnu") {
    $triples.Add("x86_64-pc-windows-msvc")
}

foreach ($targetTriple in ($triples | Select-Object -Unique)) {
    $dst = Join-Path $binDir "airp-core-$targetTriple.exe"
    Copy-Item -LiteralPath $src -Destination $dst -Force
    Write-Host "engine sidecar placed: $dst"
}
