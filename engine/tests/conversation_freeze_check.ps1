# Conversation freeze guard (N3, PR #384 审计).
#
# E-P0-2 决策 B 冻结 Conversation 功能对称扩张：仅允许安全修复、既有合同
# bugfix、文档/测试诚实性维护。本脚本检测 conversation 源文件的公开 API 面
# 是否超出已归档基线，若新增 pub 项则 CI 失败。
#
# 用法：
#   pwsh conversation_freeze_check.ps1                # 校验模式（CI 用）
#   pwsh conversation_freeze_check.ps1 -RegenBaseline # 重新生成基线（审计确认后）

param([switch]$RegenBaseline)

$ErrorActionPreference = 'Stop'

# 合同 §6 适用范围（不含 conversation_compat.rs）。
$files = @(
  "engine/src/conversation.rs",
  "engine/src/conversation_context.rs",
  "engine/src/conversation_observability.rs",
  "engine/src/conversation_policy.rs",
  "engine/src/conversation_projection.rs",
  "engine/src/conversation_turn.rs"
)

$baselinePath = "engine/tests/conversation_pub_surface.baseline.txt"

# 提取 pub 项声明，排除 `mod tests` 块内的内容与 pub(crate)/pub(super)/pub(self)。
function Extract-PubSurface {
  $surface = @()
  foreach ($file in $files) {
    $inTests = $false
    $depth = 0
    $lines = Get-Content $file
    for ($i = 0; $i -lt $lines.Count; $i++) {
      $line = $lines[$i]
      # 跟踪 mod tests 块边界。
      if ($line -match '^\s*mod\s+tests\s*\{') {
        $inTests = $true
        $opens = ($line.ToCharArray() | Where-Object { $_ -eq '{' }).Count
        $closes = ($line.ToCharArray() | Where-Object { $_ -eq '}' }).Count
        $depth = $opens - $closes
        continue
      }
      if ($inTests) {
        $opens = ($line.ToCharArray() | Where-Object { $_ -eq '{' }).Count
        $closes = ($line.ToCharArray() | Where-Object { $_ -eq '}' }).Count
        $depth += $opens - $closes
        if ($depth -le 0) { $inTests = $false }
        continue
      }
      # 匹配 pub 项声明（排除受限可见性）。
      if ($line -match '^\s*pub (fn|async fn|struct|enum|trait|type|const|static|use)\b' `
          -and $line -notmatch 'pub\((crate|super|self)\)') {
        $sig = $line.Trim()
        # use 语句取到分号为止；fn/struct 等取首行签名。
        $surface += "$file`t$sig"
      }
    }
  }
  return $surface | Sort-Object -Unique
}

$current = Extract-PubSurface

if ($RegenBaseline) {
  $current | Set-Content -Encoding UTF8 $baselinePath
  Write-Host "Baseline regenerated: $($current.Count) pub items -> $baselinePath"
  exit 0
}

if (-not (Test-Path $baselinePath)) {
  Write-Error "Baseline file not found: $baselinePath. Run with -RegenBaseline to create it."
  exit 1
}

$baseline = @(Get-Content $baselinePath | Where-Object { $_.Trim() -ne '' })

# 仅检测扩张（current 中有但 baseline 中没有的项）。收缩允许。
$newItems = $current | Where-Object { $baseline -notcontains $_ }

if ($newItems) {
  Write-Host "Conversation freeze violation (E-P0-2 决策 B): new public API items detected."
  Write-Host ""
  Write-Host "New items:"
  foreach ($item in $newItems) { Write-Host "  + $item" }
  Write-Host ""
  Write-Host "若此扩张经审计批准为安全修复/bugfix 例外，请更新基线："
  Write-Host "  pwsh engine/tests/conversation_freeze_check.ps1 -RegenBaseline"
  Write-Host "并在 PR 审计报告中记录例外理由。"
  exit 1
}

Write-Host "Conversation freeze guard: OK ($($current.Count) pub items, no expansion detected)"
