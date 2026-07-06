# WebUI 审计 v2 — 上次报告后的变更复核

> 日期：2026-07-06
> 审计基线：`docs/WEBUI-ANALYSIS-AND-OPTIMIZATION.md`（初版报告）
> 变更范围：PR #54（session polish）、#60（M1 usability）、#62（workbench PR E+F）、#63（M2 polish）
> 审计立场：独立审计（AGENTS.md 守则），不附和 commit message 或开发者注释

---

## 1. 上次报告问题兑现跟踪

### 1.1 webui/ — W-01 到 W-13 逐项

| 编号 | 问题 | 状态 | 说明 |
|------|------|------|------|
| W-01 | 连接状态不持久（bearer 刷新即丢） | ✅ 已修复 | `sessionStorage.setItem('airp_engine_url', bearer)` + 页面加载时恢复。注释正确说明 sessionStorage 不缓解 XSS，只缩短 token 存活窗口。 |
| W-02 | 并发 stream 测试暴露在主 UI | ✅ 已修复 | `<details id="concurrent-test">` + `<summary>` 折叠，默认收起。不再占空间。 |
| W-03 | 左侧面板信息过载 | ✅ 已修复 | Engine/Settings/Models 合并到一个 `<details open>`（连接信息），State 有独立 `<details>`（默认折叠），Import 有独立 `<details>`（默认折叠）。核心只留 Characters + Sessions 展开。 |
| W-04 | Agent Runner / Concurrent Test / Diagnostics 混在 chat 面板 | ✅ 已修复 | 三者全部收进 `<details>` 折叠区，默认收起。chat 区域空间释放。 |
| W-05 | 缺少停止生成按钮 | ✅ 已修复 | `#btn-stop` 存在，`hidden` 属性控制可见性，streaming 开始时显示、结束时隐藏。abort 逻辑正确（含 race-safe 局部引用 `ac` 模式）。 |
| W-06 | Chat 消息没有时间戳 | ❌ 未修 | `appendMsg` 仍未渲染时间。作为 harness 低优，可接受。 |
| W-07 | History 按钮语义不清 | ✅ 已修复 | 按钮改为 `↻ 刷新历史` + `title="从 engine 拉取完整聊天记录"`。 |
| W-08 | Session 选择器只显示前 12 字符 | ❌ 未修 | `id.slice(0, 12)` 仍在。UUID 前 12 位区分度低。低优。 |
| W-09 | 缺少 Settings 写入 UI | ❌ 未修 | 与 WEBUI-BACKEND-PLAN §10 对齐，不修是正确决定。 |
| W-10 | app.js 单文件 1365 行 | 🟡 更严重 | 现已 1402 行，新增 workbench（~280 行）+ M1/M2 polish 让单文件进一步膨胀。但零构建约束下合理，不算 bug。 |
| W-11 | markdown renderer 边界 case | 🟡 改善 | 新增 nonce 占位符防用户输入同形序列误替换（F16/F17 fix），CRLF 统一（F18 fix），块级切分避免 `<p><pre>` 嵌套。不支持列表/链接/表格仍为已知限制。 |
| W-12 | Workbench 拖拽边界问题 | ❌ 未修 | `mouseup` 仍只在 window 上，无 IFrame 内处理。但当前无 IFrame 场景，影响低。 |
| W-13 | serve.js 无压缩 | ❌ 未修 | 正确——保持零依赖。 |

**兑现率**：6/13 已修复，2/13 改善，5/13 不修（低优或有意不修）。短路线图全部完成。

### 1.2 ui/ (Tauri)

自上次报告至今，`ui/` 无任何代码变更（`git diff --stat HEAD~10 -- ui/` 为空）。T-01~T-10 状态不变。

---

## 2. 新增代码审计

### 2.1 Workbench（PR #62, PR E+F）— 角色卡 + 世界书编辑

#### 正面

- **dirty 追踪 + 未保存提示**：`wbDirty` 标志 + `closeWorkbench()` 中 `confirm()` 弹窗，ESC 关闭时跳过 input/textarea 焦点。防止误关丢数据。
- **API 正确**：GET card → 编辑表单 → PUT 写回；GET lorebook → 编辑条目列表 → PUT 写回。404 区分（无世界书 vs 加载失败）。
- **安全**：角色卡编辑走 PUT（`/v1/characters/:id`），世界书编辑走 PUT（`/v1/characters/:id/lorebook`），无 `card_path` 使用。RR-001 护栏完好。
- **ESC 保护**：keydown handler 检查 `e.target.tagName`，textarea/input 内按 ESC 不关闭面板。正确。
- **reextract 确认**：`confirm()` 弹窗警告破坏性操作。

#### 审计发现

**[A-01] Workbench 保存角色卡时 `wbCardData` 被就地修改（mutation risk）**
- `saveWorkbenchCard()` 第 1125 行：`data[f.key] = el.value` 直接修改 `wbCardData` 对象。
- 如果保存失败，`wbCardData` 已被 mutation，用户无法「恢复到上次保存的版本」。
- 严重度：低（harness 场景，用户可 reload）。但如果要严肃，应在保存前深拷贝一份，失败时恢复。

**[A-02] 世界书条目删除后 `renderLoreEntries()` 全量重渲染，丢失已展开/折叠状态**
- `wb-lore-del` 点击后 `splice` + `renderLoreEntries()` 全量重渲染。
- 用户在编辑多个条目时，如果删除了中间一个，所有条目的展开/折叠状态、未保存的 input 值全部丢失。
- 严重度：中——用户编辑多条世界书条目时可能丢失未保存修改。
- 建议：只在删除前 `setWbDirty(true)`（已有），但如果用户正在编辑别的条目，删除一个条目会打断所有其他条目的编辑。可考虑删除时不全量重渲染，或提示「其他条目有未保存修改」。

**[A-03] 世界书条目的 priority 输入无 `step` 属性**
- `<input type="number" min="0">` 缺 `step="1"`。部分浏览器默认 `step="any"`，允许小数输入。
- 严重度：极低——engine 侧可能接受小数 priority，但语义上 priority 应是整数。

**[A-04] Workbench Tab 切换无确认保护**
- 用户在角色卡 tab 编辑了内容（dirty），切到世界书 tab 时没有任何提示。
- 切 tab 不会丢数据（data 在 JS 对象里），但用户可能忘记回来保存。
- 严重度：极低——closeWorkbench 有保护，tab 切换不丢数据。

### 2.2 M1 可用性改进（PR #60）— state/history + auto-load + avatar + markdown

#### 正面

- **state 404 显式区分**：`refreshState()` 对 404 显示「该角色尚无 live.json」，与空对象 `{}` 语义分开。符合 PLAN §2.1 L47。
- **state history limit 输入校验**：`Math.max(1, Math.min(1000, ...))` 钳位，防超大 limit 拖垮 engine。
- **auto-load history**：`connect()` → `refreshAll()` → `loadHistory()` 链路，以及 `charSelect.change` 和 `sessSelect.change` 后自动 `loadHistory()`。减少用户手动操作。
- **clearChatView() 统一**：`abortInFlightStreams()` + `innerHTML = ''`。切角色/切 session/新建 session 统一走此路径。防上一 session 的 SSE chunk 写回已清空的视图（issue #43/#44 D）。
- **avatar blob URL 管理**：`refreshAvatar()` 先 `revokeObjectURL` 旧 URL 再 fetch 新的。防内存泄漏。

#### 审计发现

**[A-05] `loadHistory()` 全量替换 `chatLog.innerHTML = ''`（性能契约 #3 违反）**
- `loadHistory()` 第 598 行：`chatLog.innerHTML = ''` 后逐条 `appendMsg`。
- 如果历史消息量大（数百条），每次 loadHistory 都全量重建 DOM。
- 这在初版报告 §5.4 性能契约表中已标注为 ❌。
- 严重度：低（harness 不要求满足性能契约，且单次会话很少超 500 条）。

**[A-06] `clearChatView()` 在 `charSelect.change` handler 中被调用两次**
- `charSelect.addEventListener('change', ...)` 调用了 `clearChatView()`（第 346 行），然后调用 `loadHistory()`（第 352 行）。`loadHistory()` 内部也调 `chatLog.innerHTML = ''`（第 598 行）。
- `clearChatView()` 的 `innerHTML = ''` 是多余的——`loadHistory()` 会再清一次。
- 严重度：极低——两次 `innerHTML = ''` 无副作用差异，只是冗余。

### 2.3 M2 Polish（PR #63）— session persistence + collapsible panels + stop button

#### 正面

- **sessionStorage restore**：页面加载时从 sessionStorage 恢复 engine URL + bearer。恢复后自动 `connect()`。
- **collapsible panels**：Agent Runner、Concurrent Test、Diagnostics 全部收进 `<details>`，默认折叠。HTML 结构上 summary 内嵌 button 的点击冒泡被 `stopPropagation` 拦截（第 221-223 行）。
- **stop button 样式**：红色按钮 `#btn-stop`，CSS 样式明显区别于普通按钮。

#### 审计发现

**[A-07] sessionStorage `connect()` 自动重连——bearer token 过期时无感知**
- 页面加载恢复 bearer 后自动调用 `connect()`。如果 token 已过期，`/version` 返回 401，UI 显示「连接失败」。
- 但用户可能困惑：为什么上次能用，今天自动连不上？
- 严重度：极低——`connText` 会显示错误原因，用户可手动重输。但考虑在 auto-connect 失败时不自动重试（当前也没有重试，只是静默失败）。

**[A-08] `summary > button` 的 `stopPropagation` 只绑定一次，不覆盖动态新增**
- 第 221-223 行：`document.querySelectorAll('summary > button')` 在 IIFE 顶部一次性绑定。
- Workbench 面板的 summary 内也有按钮（如 `btn-wb-close`），但 Workbench 不在 `<details>` 内（是独立 `<aside>`），所以不受影响。
- 严重度：无——当前所有 `summary > button` 在 HTML 中都是静态的。

### 2.4 serve.js 变更（PR #60）

- 新增 `.mjs` MIME type（`application/javascript`）。
- 正确——ES modules 用 `.mjs` 扩展名时需要正确 MIME。

---

## 3. 综合评估

### 3.1 代码质量

| 维度 | 评分 | 说明 |
|------|------|------|
| 安全 | A | RR-001 完好、escapeHtml + nonce 防注入、bearer sessionStorage 降低暴露、blob URL revoke 防泄漏、serve.js 路径穿越防护 |
| 竞态防护 | A- | abort race 有局部引用模式（`ac`）防旧请求 finally 误清新请求、`abortInFlightStreams()` 统一终止、agent 二次点击先 abort 前一个 |
| UX 一致性 | B+ | 6/13 P0/P1 问题已修、dirty 保护、ESC 安全、destructive confirm、404 区分 |
| 可维护性 | B | 单文件 1402 行持续增长，但 harness 定位下可接受。代码注释充分，命名清晰 |
| 性能 | B- | chat 全量 DOM 重建（harness 可接受）、event log 200 条上限、agent output 500 条上限 |

### 3.2 新发现汇总

| 编号 | 严重度 | 描述 | 建议 |
|------|--------|------|------|
| A-01 | 低 | Workbench 保存角色卡时就地修改 `wbCardData`，保存失败后无法恢复原值 | 保存前深拷贝，失败时恢复 |
| A-02 | 中 | 世界书删除条目后全量重渲染，丢失其他条目的展开/折叠状态和未保存 input 值 | 删除时只移除对应 DOM 节点，不全量重渲染 |
| A-03 | 极低 | 世界书 priority 输入缺 `step="1"` | 加 `step="1"` |
| A-04 | 极低 | Workbench tab 切换无 dirty 提醒 | 不修——closeWorkbench 有保护 |
| A-05 | 低 | `loadHistory()` 全量替换 chat DOM（已知，harness 不要求） | 不修 |
| A-06 | 极低 | `clearChatView()` + `loadHistory()` 双重清 chatLog | 代码洁癖，影响为零 |
| A-07 | 极低 | auto-connect token 过期静默失败 | 不修——当前行为合理 |
| A-08 | 无 | summary > button stopPropagation 只绑定一次 | 不修——当前无动态场景 |

**可操作的发现**：只有 A-02（中）值得修。其余要么是已知限制（A-05），要么是代码洁癖（A-06），要么影响极低。

### 3.3 与审计守则的对齐

按 AGENTS.md 审计 agent 守则，本审计的独立判断：

1. **独立审计**：以上发现全部基于代码阅读 + 上次报告交叉验证，未直接接受 commit message 的结论。
2. **提出自己的想法**：初版报告建议的短路线图（W-01/W-03/W-04/W-05）全部已实现，说明建议合理。但 A-02 指出了一个开发者未注意到的 UX 问题（世界书编辑中删除条目打断其他条目编辑）。
3. **质疑历史决策**：`loadHistory()` 全量 DOM 重建在 M1 PR 中被引入，而 PERF SPIKE 已明确要求 patch 优先（PLAN §2.5 约束 #3）。虽然 harness 不要求满足全部性能契约，但 auto-load history 后这个全量重建会被更频繁触发（每次切角色/切 session 都会），值得在注释中标注。

---

## 4. 建议优先级

| 优先级 | 行动 | 说明 |
|--------|------|------|
| 可选 | 修 A-02 | 世界书删除不全量重渲染，改单条 DOM 移除 |
| 可选 | 清理 A-06 | `loadHistory()` 内已有 `innerHTML = ''`，`clearChatView()` 的清空在 `loadHistory` 路径上是冗余的，可从 charSelect/sessSelect handler 中去掉 |
| 不修 | A-01, A-03~A-08 | harness 场景影响极低 |

**整体评价**：webui 自上次报告后收到了 3 个 PR 的改进（#60 M1、#62 workbench、#63 M2 polish），短路线图 4 项全部完成，代码质量稳步提升。没有发现安全问题或竞态缺陷。唯一值得修的是 A-02（世界书编辑 UX），其余都是已知限制或极低影响项。
