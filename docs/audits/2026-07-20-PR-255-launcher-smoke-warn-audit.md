# PR #255 独立审计报告 — launcher smoke warn + defensive env cleanup comment

- **审计源模型**：GLM-5.2（Trae IDE，按根 `AGENTS.md` §Audit Agent Charter 独立执行）
- **审计日期**：2026-07-20
- **审计 charter**：根 `AGENTS.md` §Audit Agent Charter（独立审计 / 可提己见 / 可质疑历史并查证）
- **PR**：[#255 chore(deploy/windows-webui): clarify launcher smoke skip warning + document defensive env cleanup](https://github.com/GhostXia/AIRP/pull/255)
- **分支**：`chore/244-launcher-smoke-warn-artifact-align`
- **commit**：`812e646 chore(deploy/windows-webui): clarify launcher smoke skip warning + document defensive env cleanup`

## 1. 范围与背景

PR #255 关闭 #244 中 L2、L3 两条遗留项（PR #243 audit leftover）：

- **L3**：`smoke-package.ps1:95` 的 warning 文本晦涩，没说明"用户传了非 8765 的 -Port"
- **L2**：`AIRP_ACCESS_KEY` 等变量在 first finally 已清理、second setup 又清理一次的冗余

PR 改动只有 1 个 commit，diff 9 行：
1. `Write-Warning` 文本改为带 `$Port` 插值，单引号→双引号
2. 加 5 行注释解释为什么 first finally + second setup 两处清理都保留（防御性 + 无条件）

## 2. 独立证据

### 2.1 改动静态读

```diff
-    Write-Warning 'Skipping Start-AIRP.cmd process smoke because the launcher contract uses port 8765.'
+    Write-Warning "Skipping Start-AIRP.cmd process smoke because -Port was $Port but the launcher is hardcoded to 8765."
```

PowerShell 双引号字符串在 `"$Port"` 处插值，`$Port` 是脚本参数（`param([int]$Port = 8765)`，L3）。
插值方向正确；旧文本 `'...port 8765.'` 与新文本 `"...-Port was $Port but the launcher is hardcoded to 8765."` 都描述同一个事实，但新文本把"用户实际传了什么"显式化，对维护者排障更直接。

```diff
+# Defensive cleanup: these are also cleared in the launcher-smoke finally
+# block above, but that block only runs on the launcher-smoke path. This
+# cleanup is unconditional and also covers env leaked from a previous
+# smoke invocation when the launcher-smoke branch was skipped. Keep both.
 Remove-Item Env:AIRP_ACCESS_KEY -ErrorAction SilentlyContinue
 Remove-Item Env:AIRP_DEPLOYMENT_MODE -ErrorAction SilentlyContinue
 Remove-Item Env:AIRP_PUBLIC_ORIGIN -ErrorAction SilentlyContinue
 Remove-Item Env:AIRP_CORS_ORIGINS -ErrorAction SilentlyContinue
```

注释解释了为什么 L88 first finally 与 L101 second setup 两处清理都保留：
- first finally 只在 launcher-smoke 路径走（`if ($Port -eq 8765)`）
- second setup 无条件执行，覆盖 launcher-smoke 被跳过时残留的 env

这个解释准确。`if ($Port -eq 8765) {...} else { Write-Warning ... }` 分支外（L97 之后）
的 cleanup 确实是无条件执行；如果 `$Port` 不是 8765，first finally 不跑，second
setup 就是唯一清理点。注释没有夸大。

### 2.2 PowerShell 语法验证

```
$ $null = [System.Management.Automation.PSParser]::Tokenize(
    (Get-Content -Raw deploy/windows-webui/smoke-package.ps1), [ref]$null)
syntax OK
```

PowerShell tokenizer 通过，无解析错误。

### 2.3 行为不变性

- `Write-Warning` 文本变化不影响进程退出码、不抛异常
- 注释是 PowerShell `#` 行注释，不参与执行
- 9 行 diff 全部为字符串/注释改动，无控制流、变量、参数、调用顺序变化

CI 实跑：`Portable Windows WebUI` job 通过，与 main 基线一致。

### 2.4 L2 注释是否充分

#244 L2 的建议是"移除 second setup 中的重复清理（或保留作为防御性冗余，但加注释说明）"。
PR 选择保留 + 加注释，与 issue 给出的 alternative 一致。注释明确说明 first finally 是
launcher-smoke 路径专属、second setup 无条件覆盖 skip 路径残留，达到 issue 要求。

## 3. 阻塞意见

无。

## 4. 非阻塞 / 可后续

| # | 项 | 严重度 | 建议时机 |
|---|----|--------|---------|
| N-1 | #244 L2 与本 PR 注释解释一致，但 first finally 已经覆盖 launcher-smoke 路径、second setup 覆盖 skip 路径；理论上两处 cleanup 实际上"非重叠"。注释说"defensive redundancy"略误导——更准确是"two disjoint-path cleanups"。但保留现有措辞也可接受，不必跟进 | 极低 | 不跟进 |
| N-2 | #244 L1/L4/L5/L6/L7 不在本 PR 范围，#244 关闭时应明确"仅 L2/L3 实现，L1/L4/L5/L6/L7 维持 open" | 低 | PR 合并后跟进 #244 |

## 5. 神圣不变式

- 本 PR 不触 engine、webui http 边界、subagent context、normalizer 或神圣提示词不变式。✓

## 6. 结论

**通过**。

- L3 warning 文本：$Port 插值正确，文本更可读，PowerShell tokenizer 通过。
- L2 注释：准确解释两处 cleanup 的不同覆盖路径，与 #244 alternative 一致。
- 9 行 diff 全部为字符串/注释，无行为变化；CI 全绿。
- 无阻塞意见。N-1 不跟进、N-2 留 PR 合并后跟进 #244。
