# AIRP 桌面壳（WebView2/Tauri）发布前截屏验收脚本 —— 骨架占位（Task #12 交付物 5）。
#
# 目标：在桌面宿主（Tauri + WebView2）里复用 webui 截图验证机制（webui-screenshot-suite.mjs
# 的 token 断言 + CSP/pageerror 门禁），确保桌面壳渲染与浏览器基线一致。
#
# 当前状态：占位实现（立桩）。仅校验 Tauri 发布产物存在性，并说明如何接 harness 截图。
# 桌面壳侧的 harness 注入改造（把 agent-test-harness / 截图通道接进 WebView2）是后续任务，
# 届时把本脚本的 TODO 段替换为真实截图与 golden 对比逻辑（第二阶段引入 pixelmatch/pngjs）。
#
# 用法：
#   pwsh -NoProfile -File ui/smoke-desktop-screenshots.ps1
#
# 退出码：0 = 产物存在性检查通过；1 = 缺少产物或检查失败。

$ErrorActionPreference = "Stop"

$repoRoot = Resolve-Path (Join-Path $PSScriptRoot "..")

# ── 1. 校验 Tauri 发布产物存在性 ────────────────────────────────────────────
# 与 ui/smoke-windows-installer.ps1 对齐：NSIS 安装包位于 <repo>/target/release/bundle/nsis。
$bundleDir = Join-Path $repoRoot "target\release\bundle\nsis"
$installer = $null
if (Test-Path -LiteralPath $bundleDir) {
    $installer = Get-ChildItem -LiteralPath $bundleDir -Filter "*.exe" -File |
        Sort-Object LastWriteTime -Descending |
        Select-Object -First 1
}

# 裸可执行（未打包 NSIS 时的调试宿主）也在检查范围内。
$hostExe = Join-Path $repoRoot "target\release\airp-ui.exe"

if (-not $installer -and -not (Test-Path -LiteralPath $hostExe)) {
    Write-Error "未找到桌面发布产物。请先运行 ui/build-tauri.ps1 生成 NSIS 安装包（期望位于 $bundleDir），或构建 target/release/airp-ui.exe。"
    exit 1
}

if ($installer) {
    Write-Host "[ok] 发现 NSIS 安装包: $($installer.FullName)"
} else {
    Write-Host "[ok] 未发现 NSIS 安装包，但找到裸宿主可执行: $hostExe"
}

# ── 2. 如何接 harness 截图（后续任务实现，此处为说明占位）──────────────────
# 截图链路设计（与 webui-screenshot-suite.mjs 复用同一套令牌/CSP 门禁）：
#   a) 桌面壳加载 webui 时注入 agent-test-harness.js（参考 webui/assets/agent-test-harness.js，
#      通过 ?airp_agent_test=1 或宿主启动参数启用），暴露页内截图/DOM 快照 API。
#   b) 宿主内置 sidecar（airp-core daemon）就绪后，复用 ui/webui-screenshot-suite.mjs 的
#      逐屏遍历 + tokens.css 精确值断言逻辑，对 WebView2 渲染结果截屏。
#   c) 与 webui/baseline-screenshots/ 的 golden 基线做像素对比（第二阶段引入
#      pixelmatch/pngjs；本任务不加运行时依赖，因此当前不做真实对比）。
# TODO(桌面壳改造任务)：
#   - 启动安装后的 airp-ui.exe（参考 smoke-windows-installer.ps1 的启动/就绪等待模式）
#   - 通过 harness 逐屏截图并落盘
#   - 与 baseline 对比，超阈值即失败
Write-Host "[stub] 桌面 harness 截图与 golden 对比为后续任务，本脚本当前仅立桩校验产物存在性。"

Write-Host "desktop screenshot smoke skeleton passed (artifact presence only)"
exit 0
