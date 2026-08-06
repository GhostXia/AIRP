# C-P0：把仓库根的 webui/ 暂存拷贝到 ui/src-tauri/webui-bundle/，供 Tauri
# bundle.resources 打入桌面安装包（Tauri 的 resources 必须位于 src-tauri 项目
# 目录内，不允许越界 glob，故需要此暂存步）。
# 开发模式（tauri dev）不依赖本脚本——壳会从可执行文件向上回溯找到仓库内 webui/。
$ErrorActionPreference = "Stop"

$source = Join-Path $PSScriptRoot "..\webui"
$dest = Join-Path $PSScriptRoot "src-tauri\webui-bundle"

if (-not (Test-Path (Join-Path $source "index.html"))) {
    throw "webui source not found at $source (index.html missing)"
}
if (Test-Path $dest) { Remove-Item -Recurse -Force $dest }
New-Item -ItemType Directory -Force -Path $dest | Out-Null
Copy-Item -Recurse -Path (Join-Path $source "*") -Destination $dest
Write-Host "webui staged for desktop bundle -> $dest"
