# C-P0：把仓库根的 webui/ 暂存拷贝到 ui/src-tauri/webui-bundle/，供 Tauri
# bundle.resources 打入桌面安装包（Tauri 的 resources 必须位于 src-tauri 项目
# 目录内，不允许越界 glob，故需要此暂存步）。
# 开发模式（tauri dev）不依赖本脚本——壳会从可执行文件向上回溯找到仓库内 webui/。
$ErrorActionPreference = "Stop"

$source = Join-Path $PSScriptRoot "..\webui"
$dest = Join-Path $PSScriptRoot "src-tauri\webui-bundle"
$desktopSource = Join-Path $PSScriptRoot "dist"

if (-not (Test-Path (Join-Path $source "index.html"))) {
    throw "webui source not found at $source (index.html missing)"
}
if (Test-Path $dest) { Remove-Item -Recurse -Force $dest }
New-Item -ItemType Directory -Force -Path $dest | Out-Null
Copy-Item -Recurse -Path (Join-Path $source "*") -Destination $dest

Push-Location $PSScriptRoot
try {
    & npm run build
    if ($LASTEXITCODE -ne 0) { throw "Vue desktop build failed with exit code $LASTEXITCODE" }
} finally {
    Pop-Location
}
if (-not (Test-Path (Join-Path $desktopSource "index.html"))) {
    throw "Vue desktop output not found at $desktopSource"
}
$desktopDest = Join-Path $dest "desktop"
New-Item -ItemType Directory -Force -Path $desktopDest | Out-Null
Copy-Item -Recurse -Path (Join-Path $desktopSource "*") -Destination $desktopDest
Write-Host "legacy WebUI + Vue /desktop/ staged -> $dest"
