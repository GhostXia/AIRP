# PR #63 审计报告

> 审计源 LLM：GLM-5.2（智谱 AI, 2026）
> 审计日期：2026-07-06
> 审计范围：`webui/app.js`、`webui/index.html`、`webui/style.css`
> 审计轮次：v1 (PASS, 修复后) → v2 (本轮独立复审，发现 3 处原审计漏掉的问题)

---

## 1. 审计结论

**PASS（修复后，v2）**。二次独立审计发现 1 个 P0 异步竞态、1 个 P0 样式回归、1 个 P1 注释不准确，已在本分支修复并推送。CodeRabbit 的 async-race 提示与 Gemini Code Assist 的 state-history-limit 提示均成立，已采纳并修复。

---

## 2. 功能与实现审查

### 2.1 W-01：sessionStorage 持久化

- `connect()` 中持久化 Engine URL + Bearer token 到 `sessionStorage`。
- 页面加载时从 `sessionStorage` 恢复并自动连接。
- key 带 `airp_` 前缀，降低命名空间冲突风险。
- 所有 storage 访问包在 `try/catch` 中，避免隐私模式/禁用 storage 时崩溃。
- **评估**：实现正确。R3 修复后注释明确"缩短泄漏 token 的存活窗口"，不再误称"XSS 缓解"。

### 2.2 W-05：停止生成按钮（R1 修复后）

- `doSend` 创建局部 `ac = new AbortController()`，fetch 绑 `ac.signal`。
- 全局 `abortController = ac`，按钮显示。
- `finally` 中仅在 `abortController === ac` 时清理全局与按钮可见性。
- **评估**：R1 修复后异步竞态消除，连续点击发送不会让旧请求的 finally 误清新请求状态。

### 2.3 W-03/W-04：折叠面板

- 左侧面板用原生 `<details>`/`<summary>` 分组，连接信息/Characters/Sessions 默认展开，State/Import 默认折叠。
- 工具箱（agent-runner/concurrent-test/diagnostics）改为 `<details>` 默认折叠。
- summary 内的 button 加了 `stopPropagation`，避免点击刷新/新建按钮时意外 toggle。
- **评估**：DOM 改动合理，CSS 已限定在 `#left-panel details` 与 `#center-panel > details`，不影响 `.ev-raw summary`。

---

## 3. 发现的问题与修复

### v1 轮（首次独立审计）

#### U1（已修复）：通用 `summary` 样式覆盖 `.ev-raw summary`

- **修复提交**：`32e8ed0` — 折叠区样式限定到 `#left-panel` / `#center-panel > details`。

### v2 轮（二次独立审计，本轮）

#### R1（P0，已修复）：doSend 异步竞态

- **问题**：原 `finally { abortController = null; btnStop.hidden = true; }` 无条件清理全局状态。用户在流式期间连续点发送时，旧请求的 finally 可能在新请求已开始后被调度执行，误清新请求的 `abortController` 与 `btnStop` 可见性，导致新流式无法被停止、按钮闪烁。
- **修复提交**：`bdae533` — 改用局部 `ac` 引用，fetch 绑 `ac.signal`，finally 中加 `if (abortController === ac)` 守卫。

#### R2（P0，已修复）：`#state-history-limit` input 失去专属样式

- **问题**：DOM 从 `<section>` 改 `<details>`、子节点从 `<h3 class="subhead">` 改 `<div class="hint">` 后，原 CSS 选择器 `#state-section .subhead input` 不再匹配 `id="state-history-limit"` 的 input。input 回退到浏览器默认样式，深色面板上视觉异常。
- **修复提交**：`bdae533` — 新增 `#state-history-limit` 专属规则，复用原色板与尺寸（width 60px、深色背景、11px 字体）。

#### R3（P1，已修复）：sessionStorage 注释不准确

- **问题**：原注释"W-01: 持久化到 sessionStorage（关 tab 即清，降 XSS 持久风险）"暗示该选择能缓解 XSS，但 sessionStorage 在同 tab 内仍可被任意脚本读取，XSS 利用面未缩小。
- **修复提交**：`bdae533` — 改为"缩短泄漏 token 的存活窗口"，并加显式注："不缓解 XSS——同 tab 任意脚本仍可读"。

#### R4（P2，不修）：全局 `details` 选择器

- CodeRabbit 建议用 `.collapsible` class 替代全局 `details` 选择器，便于未来添加不同样式的 details 而不互相影响。
- **当前判断**：CSS 已限定到 `#left-panel details` 与 `#center-panel > details`，不会影响其他位置的 details（包括 agent-output 内的 `.ev-raw`）。核心风险已规避，引入新 class 收益不明显，不修。
- **未来预留**：如新增非折叠区 details，优先考虑 `.collapsible` 模式以保持一致性。

---

## 4. 安全评估

| 项目 | 评估 |
|------|------|
| Bearer token 持久化 | sessionStorage 是合理折中，缩短泄漏 token 跨会话复用窗口；不缓解 XSS（R3 注释已澄清） |
| stopPropagation 使用 | 正确，仅阻止 summary 内 button 的冒泡，不影响 button 自身点击 |
| 异步竞态（R1） | 修复后连续点击发送安全，停止按钮不会被旧请求 finally 误隐藏 |
| 无外部依赖引入 | 零构建约束保持 |
| 无新网络请求 | 纯 UI 行为改动 |

---

## 5. 验证记录

- `node --check webui/app.js` ✓
- `node --check webui/serve.js` ✓
- 神圣不变式 `subagent_context_has_no_orchestrator_noise` + `subagent_prepared_pipeline_has_no_orchestrator_noise` 2/2 ✓
- `cargo build --bin airp-core` ✓
- engine smoke：`GET /version`、`GET /v1/characters` 正常返回
- WebUI HTML/CSS 渲染验证：
  - `btn-stop` 默认 `hidden`
  - 左侧面板 `details open` 结构正确
  - 工具箱 `details` 默认折叠
  - `.ev-raw summary` 样式不再被通用折叠区覆盖
  - `#state-history-limit` 专属样式恢复

---

## 6. 合并建议

**v2 轮建议合并到 `main`**。

---

## 7. 审计源声明

本审计由 GLM-5.2（智谱 AI, 2026）作为审计源 LLM 独立完成。CodeRabbit 与 Gemini Code Assist 的自动 review 作为交叉参考。
