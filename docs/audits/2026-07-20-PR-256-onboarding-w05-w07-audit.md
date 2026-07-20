# PR #256 独立审计报告 — onboarding W-05 timer + W-07 preset_id + disposed guard

- **审计源模型**：GLM-5.2（Trae IDE，按根 `AGENTS.md` §Audit Agent Charter 独立执行）
- **审计日期**：2026-07-20
- **审计 charter**：根 `AGENTS.md` §Audit Agent Charter（独立审计 / 可提己见 / 可质疑历史并查证）
- **PR**：[#256 chore(webui/onboarding): clear Stage 1 prod timer on cleanup + harden preset_id fallback (W-05, W-07)](https://github.com/GhostXia/AIRP/pull/256)
- **分支**：`chore/213-onboarding-w05-w07-presetid-timeout`
- **commits**：
  - `ea76d79 chore(webui/onboarding): clear Stage 1 prod timer on cleanup + harden preset_id fallback (W-05, W-07)`（PR 主改动）
  - `a120118 fix(onboarding): guard runHealthCheck with disposed flag + pad preset_id random suffix`（CodeRabbit actionable 跟进）

## 1. 范围与背景

PR #256 关闭 #213 中 W-05、W-07 两条遗留项（PR #212 audit leftover）：

- **W-05**：Stage 1 prod 模式 `setTimeout(..., 0)` 未清理，cleanup 在回调触发前调用存在 stale callback 风险
- **W-07**：`preset_id` fallback 用 `'onb-' + Date.now()`，快速双击/快速导入两次可能得到相同时间戳

第二个 commit `a120118` 是 CodeRabbit PR #256 review 的两条 actionable 跟进：
- (a) cleanup 后 pending `/version`/`/health` continuation 仍可能更新已卸载 DOM → 添加 `disposed` 标志
- (b) `Math.random().toString(36).slice(2, 6)` 在极小值时返回 <4 字符 → 添加 `padEnd(4, '0')`

## 2. 独立证据

### 2.1 W-05 主改动 — 静态读

`webui/onboarding.js` 改动：

```diff
-      setTimeout(() => { safeAsync(() => runHealthCheck(box), 'stage1-health-check-prod'); }, 0);
+      stage1ProdTimer = setTimeout(() => {
+        stage1ProdTimer = null;
+        safeAsync(() => runHealthCheck(box), 'stage1-health-check-prod');
+      }, 0);
```

```diff
   return function cleanup() {
+    disposed = true;                              // a120118 加
     if (sseAbort) { try { sseAbort.abort(); } catch {} sseAbort = null; }
+    if (stage1ProdTimer) { try { clearTimeout(stage1ProdTimer); } catch {} stage1ProdTimer = null; }
```

W-05 修复正确：
- timer id 保存到 `stage1ProdTimer`
- 回调内 `stage1ProdTimer = null` 让 cleanup 的 `if` 判空避免无害但仍多余的 `clearTimeout`
- cleanup 中 `clearTimeout` + 置 null 是标准 timer cleanup 模式

### 2.2 W-07 主改动 — 静态读

```diff
-      || 'onb-' + Date.now();
+      || 'onb-' + Date.now() + '-' + Math.random().toString(36).slice(2, 6).padEnd(4, '0');
```

W-07 修复正确：
- 加 `Math.random().toString(36).slice(2, 6)` 提供 4 字符随机后缀，快速双击得到相同 Date.now() 时仍有随机熵区分
- `padEnd(4, '0')` 处理 `Math.random()` 极小值时 toString(36) 分数部分不足 4 字符的边界
- 前缀 `onb-` + `Date.now()` + `-` + 4 字符后缀，格式与 #149 WB-01 的"诊断 provenance"原则一致

### 2.3 disposed guard 改动 — 静态读 + race 分析

`runHealthCheck` 在每个 await 之后 DOM 更新之前检查 `disposed`：

```js
async function runHealthCheck(box) {
  const loading = el('p', 'onb-hint', '正在检查…');
  box.appendChild(loading);
  try {
    const vr = await callApi('/version');
    if (disposed) return;                          // 新增
    if (!vr.ok) { ... showError ... return; }
    const hr = await callApi('/health');
    if (disposed) return;                          // 新增
    box.removeChild(loading);
    ...
  } catch (err) {
    if (disposed) return;                          // 新增
    if (loading.parentNode) box.removeChild(loading);
    showError(box, ...);
  }
}
```

Race 分析（JS 单线程）：

| 时序 | 事件 | disposed | 行为 |
|------|------|----------|------|
| T1 | setTimeout 触发，runHealthCheck 开始 | false | 正常 |
| T2 | `await callApi('/version')` pending | false | - |
| T3 | cleanup() 调用 | true | stage1ProdTimer=null（已 fire），无 timer 可清 |
| T4 | `/version` resolve | true | `if (disposed) return` 提前退出，无 DOM mutation |

正确。`disposed = true` 在 cleanup 入口同步设置，JS 单线程下任何后续 `if (disposed)` 检查都能看到最新值。

catch 块也加 `if (disposed) return` 是好的：callApi reject 后不应再向已卸载 DOM 写错误。

### 2.4 实跑证据

```
$ node --test webui/tests/onboarding.test.mjs
ℹ tests 31
ℹ pass 31
ℹ fail 0
ℹ duration_ms 174.0631
```

31 个现有测试全绿。原有 cleanup / mountOnboarding / Stage 1 渲染分支测试覆盖未受影响。

### 2.5 测试覆盖检查

`webui/tests/onboarding.test.mjs` 现有 31 个测试覆盖：
- cleanup 函数存在性、不抛异常、清空 container（L135-153, L197-202）
- Stage 1 dev/prod 分支渲染（待 grep 确认）
- F4 崩溃后 cleanup 安全

未覆盖：
- cleanup 在 runHealthCheck pending 期间调用的 race 路径（W-05 + disposed guard）
- preset_id 在两次连续 import 时的冲突避免（W-07）

行为正确性目前靠静态读 + JS 单线程语义论证。无回归测试守护。这是非阻塞 finding。

## 3. 阻塞意见

无。

## 4. 非阻塞 / 可后续

| # | 项 | 严重度 | 建议时机 |
|---|----|--------|---------|
| N-1 | W-05 + disposed guard 缺 race 路径的 L2 集成测试。#213 W-06 已要求补 6-stage happy path L2 集成；W-05/disposed 的回归测试可以与 W-06 同 PR 补，mock fetcher 慢响应 + 中途 cleanup 断言无 DOM mutation | 低 | 与 W-06 同 PR |
| N-2 | W-07 preset_id 随机后缀缺单元测试。可在 onboarding.test.mjs 加测试：mock Date.now 固定值 + Math.random 固定值，断言 preset_id 格式与唯一性 | 低 | 下次 onboarding 测试修订 |
| N-3 | `padEnd(4, '0')` 处理了 Math.random 极小值，但 `Math.random().toString(36)` 理论上可能返回 `"0"` (虽然概率 ~0)；slice(2, 6) 后是空字符串，padEnd 后是 `"0000"`。这是预期行为（4 字符零），不是 bug，仅记录 | 极低 | 不跟进 |
| N-4 | #213 W-01/W-02/W-03/W-04/W-06 不在本 PR 范围，#213 关闭时应明确"仅 W-05/W-07 实现，W-01/W-02/W-03/W-04/W-06 维持 open" | 低 | PR 合并后跟进 #213 |

## 5. 神圣不变式

- 本 PR 不触 engine、http 边界、subagent context、normalizer 或神圣提示词不变式。✓
- WebUI CSP / 路径穿越：本 PR 不触 onboarding 的 http 调用边界，callApi 不变。✓
- 不使用 innerHTML 注入未受信数据：本 PR 改动用 `el()` 与 `box.removeChild`，不触 innerHTML。✓

## 6. 结论

**通过**。

- W-05 timer cleanup：标准 setTimeout cleanup 模式，正确。
- W-07 preset_id 随机后缀：4 字符随机 + padEnd 边界处理，格式与 #149 诊断 provenance 原则一致。
- disposed guard：3 处 `if (disposed) return` 覆盖 await 之后 DOM mutation 与 catch 路径，JS 单线程下 race 分析正确。
- 31 个现有测试全绿；race 与随机后缀缺回归测试（N-1/N-2），不阻塞合并。
- 无阻塞意见。N-3 不跟进、N-4 留 PR 合并后跟进 #213。
