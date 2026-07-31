# Conversation freeze guard (N3, PR #384 审计).
#
# E-P0-2 决策 B 冻结 Conversation 功能对称扩张：仅允许安全修复、既有合同
# bugfix、文档/测试诚实性维护。本脚本检测 conversation 源文件的公开 API 面
# 是否超出已归档基线，若新增 pub 项则 CI 失败。
#
# 覆盖范围（CR-new, CodeRabbit 2026-07-31 复审）：
# - 顶层 pub 声明：fn / async fn / unsafe fn / unsafe async fn / extern fn /
#   struct / enum / union / trait / type / const / static / use / mod
# - 成员级 pub 项（仅在 pub struct/enum/trait/union 块内）：
#   * struct 的 pub fields
#   * enum 的 variants（继承 enum 可见性）
#   * trait 的 fn 方法（默认公开）
# - 排除受限可见性 pub(crate) / pub(super) / pub(self) 与 `mod tests` 块内内容。
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

# 统计一行中指定字符的出现次数。
function Count-Char([string]$s, [char]$c) {
  $n = 0
  foreach ($ch in $s.ToCharArray()) { if ($ch -eq $c) { $n = $n + 1 } }
  return $n
}

# 提取 pub 项声明，排除 `mod tests` 块内的内容与 pub(crate)/pub(super)/pub(self)。
# 覆盖顶层声明与 pub struct/enum/trait/union 块内的成员级公开项。
function Extract-PubSurface {
  $surface = @()
  foreach ($file in $files) {
    $lines = Get-Content $file
    $braceDepth = 0
    $inTests = $false
    $testsExitDepth = -1
    # 上下文栈：每项 = @{ Kind = "struct"|"enum"|"trait"|"union"; ExitDepth = int }
    # ExitDepth = 进入该块前的 braceDepth；当 braceDepth 回落到 <= ExitDepth 时弹出。
    $ctxStack = New-Object System.Collections.Stack

    for ($i = 0; $i -lt $lines.Count; $i++) {
      $line = $lines[$i]
      $trimmed = $line.Trim()

      $opens = Count-Char $line '{'
      $closes = Count-Char $line '}'

      # 跟踪 mod tests 块边界。
      if (-not $inTests -and $line -match '^\s*mod\s+tests\s*\{') {
        $inTests = $true
        $testsExitDepth = $braceDepth
        $braceDepth += $opens - $closes
        if ($braceDepth -le $testsExitDepth) { $inTests = $false }
        continue
      }
      if ($inTests) {
        $braceDepth += $opens - $closes
        if ($braceDepth -le $testsExitDepth) { $inTests = $false }
        continue
      }

      $beforeDepth = $braceDepth
      $braceDepth += $opens - $closes

      # 弹出已结束的 pub 块上下文。
      while ($ctxStack.Count -gt 0 -and $braceDepth -le $ctxStack.Peek().ExitDepth) {
        $ctxStack.Pop() | Out-Null
      }
      $currentCtx = if ($ctxStack.Count -gt 0) { $ctxStack.Peek().Kind } else { '' }

      # 顶层 pub 声明（排除受限可见性）。
      # 关键字扩展（CR-new）：fn / async fn / unsafe fn / unsafe async fn /
      # extern fn / extern "ABI" fn / struct / enum / union / trait / type /
      # const / static / use / mod。
      if ($line -match '^\s*pub\s+' -and $line -notmatch 'pub\((crate|super|self)\)') {
        if ($line -match '^\s*pub\s+((unsafe\s+)?(async\s+)?(unsafe\s+)?fn\b)|(extern(\s+"[^"]+")?\s+fn\b)|(struct|enum|union|trait|type|const|static|use|mod)\b') {
          $surface += "$file`t$trimmed"
        }
        # 检测 pub struct/enum/trait/union 块开始（本行含 `{` 才算进入块）。
        if ($line -match '^\s*pub\s+(struct|enum|trait|union)\b' -and $opens -gt 0) {
          $kind = $Matches[1]
          $ctxStack.Push(@{ Kind = $kind; ExitDepth = $beforeDepth }) | Out-Null
        }
      }

      # 成员级公开项（仅在 pub 块内）。
      if ($currentCtx -ne '') {
        if ($currentCtx -eq 'struct') {
          # pub field: `pub field: Type`（排除 pub fn/const/static 等嵌套项）。
          if ($line -match '^\s*pub\s+\w+' -and $line -notmatch 'pub\((crate|super|self)\)' `
              -and $line -notmatch '^\s*pub\s+(fn|async|unsafe|const|static|struct|enum|trait|union|type|use|mod)\b') {
            $surface += "$file`t[struct-field] $trimmed"
          }
        } elseif ($currentCtx -eq 'enum') {
          # enum variant：非空、非注释、非属性、非 `}` 行。
          if ($trimmed -ne '' -and $trimmed -ne '}' `
              -and -not $trimmed.StartsWith('//') -and -not $trimmed.StartsWith('#[')) {
            $surface += "$file`t[enum-variant] $trimmed"
          }
        } elseif ($currentCtx -eq 'trait') {
          # trait 方法：fn（pub 可省略，trait 方法默认公开）。
          if ($line -match '^\s*(pub\s+)?(unsafe\s+)?(async\s+)?fn\b') {
            $surface += "$file`t[trait-method] $trimmed"
          }
        }
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
