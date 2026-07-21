param([switch]$SkipBuild)

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
foreach ($asset in @(
    'index.html', 'style.css', 'runtime-config.js', 'app.js', 'shared.js',
    'onboarding.js', 'persona-utils.js', 'lorebook-utils.js',
    'assembly-utils.js', 'history-utils.js', 'smooth-streamer-utils.js'
)) {
    Copy-Item -LiteralPath (Join-Path $repoRoot "webui\$asset") -Destination (Join-Path $packageRoot 'webui')
}
foreach ($file in @('Start-AIRP.cmd', 'README.txt')) {
    Copy-Item -LiteralPath (Join-Path $deployRoot $file) -Destination $packageRoot
}
Copy-Item -LiteralPath (Join-Path $repoRoot 'LICENSE-MIT') -Destination $packageRoot
Copy-Item -LiteralPath (Join-Path $repoRoot 'LICENSE-APACHE') -Destination $packageRoot

if (Test-Path -LiteralPath $archive) {
    Remove-Item -LiteralPath $archive -Force
}
Compress-Archive -LiteralPath $packageRoot -DestinationPath $archive -CompressionLevel Optimal
Write-Host "Created $archive"
