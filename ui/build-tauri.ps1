$env:CARGO_HOME = 'D:\.cargo'
$env:RUSTUP_HOME = 'D:\.rustup'
$mingw = 'C:\Users\xiach\scoop\apps\mingw\current\bin'
$sc = 'D:\.rustup\toolchains\stable-x86_64-pc-windows-gnu\lib\rustlib\x86_64-pc-windows-gnu\bin\self-contained'
$env:PATH = 'D:\.cargo\bin;' + $mingw + ';' + $sc + ';' + $env:PATH

# 1. Build src-tauri (Tauri desktop shell)
$env:CARGO_TARGET_DIR = 'D:\AIRP-State-Protocol\src-tauri\target'
Set-Location 'D:\AIRP-State-Protocol\src-tauri'
Write-Host "=== cargo build (src-tauri) ==="
& 'D:\.cargo\bin\cargo.exe' +stable-x86_64-pc-windows-gnu build 2>&1 | ForEach-Object { Write-Host $_ }
Write-Host "--- build exit $LASTEXITCODE ---"

# 2. cargo test (src-tauri)
Write-Host "=== cargo test (src-tauri) ==="
& 'D:\.cargo\bin\cargo.exe' +stable-x86_64-pc-windows-gnu test 2>&1 | ForEach-Object { Write-Host $_ }
Write-Host "--- test exit $LASTEXITCODE ---"
