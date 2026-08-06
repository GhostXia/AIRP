param(
    [switch]$SkipBuild,
    [switch]$IncludeDesktop
)

$ErrorActionPreference = 'Stop'
$deployRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = (Resolve-Path (Join-Path $deployRoot '..\..')).Path
$distRoot = Join-Path $repoRoot 'dist'
$packageRoot = Join-Path $distRoot 'airp-webui-windows-x64'
$archive = Join-Path $distRoot 'airp-webui-windows-x64.zip'

if (-not $packageRoot.StartsWith($distRoot, [StringComparison]::OrdinalIgnoreCase)) {
    throw "Refusing to stage outside $distRoot"
}
if (-not $SkipBuild) {
    Push-Location $repoRoot
    try {
        cargo build -p airp-core --bin airp-core --release --locked
    } finally {
        Pop-Location
    }
}

$engine = Join-Path $repoRoot 'target\release\airp-core.exe'
if (-not (Test-Path -LiteralPath $engine -PathType Leaf)) {
    throw "Missing release engine: $engine"
}

if (Test-Path -LiteralPath $packageRoot) {
    Remove-Item -LiteralPath $packageRoot -Recurse -Force
}
New-Item -ItemType Directory -Force -Path (Join-Path $packageRoot 'webui') | Out-Null
Copy-Item -LiteralPath $engine -Destination $packageRoot
Copy-Item -LiteralPath (Join-Path $repoRoot 'webui\index.html') -Destination (Join-Path $packageRoot 'webui')
Copy-Item -LiteralPath (Join-Path $repoRoot 'webui\assets') -Destination (Join-Path $packageRoot 'webui') -Recurse
Copy-Item -LiteralPath (Join-Path $repoRoot 'webui\screens') -Destination (Join-Path $packageRoot 'webui') -Recurse
foreach ($file in @('Start-AIRP.cmd', 'README.txt')) {
    Copy-Item -LiteralPath (Join-Path $deployRoot $file) -Destination $packageRoot
}
Copy-Item -LiteralPath (Join-Path $repoRoot 'LICENSE-MIT') -Destination $packageRoot
Copy-Item -LiteralPath (Join-Path $repoRoot 'LICENSE-APACHE') -Destination $packageRoot

if ($IncludeDesktop) {
    # 桌面旗舰 UI（v0.0.4）：在 webui 便携包体基础上增加 airp-ui.exe，
    # 与 airp-core.exe、webui\ 共用同一目录结构（不另起炉灶）。
    # 先产出 Tauri externalBin 源文件（ui/src-tauri/binaries/airp-core-<triple>.exe），
    # 再编译壳本身（tauri-build 校验 externalBin 存在）。
    & (Join-Path $repoRoot 'ui\build-engine-sidecar.ps1')
    if ($LASTEXITCODE -ne 0) {
        throw "build-engine-sidecar.ps1 failed with exit code $LASTEXITCODE"
    }
    Push-Location $repoRoot
    try {
        cargo build -p airp-ui --release --locked
    } finally {
        Pop-Location
    }
    if ($LASTEXITCODE -ne 0) {
        throw "cargo build -p airp-ui failed with exit code $LASTEXITCODE"
    }
    $ui = Join-Path $repoRoot 'target\release\airp-ui.exe'
    if (-not (Test-Path -LiteralPath $ui -PathType Leaf)) {
        throw "Missing desktop UI binary: $ui"
    }
    Copy-Item -LiteralPath $ui -Destination $packageRoot
    Write-Host "Desktop UI added to package: airp-ui.exe"
}

if (Test-Path -LiteralPath $archive) {
    Remove-Item -LiteralPath $archive -Force
}
Compress-Archive -LiteralPath $packageRoot -DestinationPath $archive -CompressionLevel Optimal
Write-Host "Created $archive"
