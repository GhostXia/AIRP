# PR #72 审计报告

> **审计源 LLM**: `GLM-5.2`（Trae IDE 托管实例，2026-07-06 生成）
> **审计对象**: PR #72 `feat(webui): usability polish (W-06/W-08/agent-input/C3)`（分支 `webui-pr-g-usability-polish`，commit `674d486`）
> **审计员立场**: 独立审计（参 [AGENTS.md §审计 agent 守则](../../AGENTS.md)）。不附和开发文档与既有代码的结论；以"我会不会这样写"为准。
> **审计日期**: 2026-07-06

---

## 0. 总结

### 总体判断

4 项改动**方向正确、实现质量中等偏上**，解决了声称要解决的问题，无明显阻塞性回归。可以合并，但合并前建议处理 1 项 `low` 级遗留（C3 阈值边界精度），其余为 `info`/`low`/`medium` 级改进建议，可后续迭代。

### 是否建议合并

**建议合并**。无 `blocking` / `high` 级问题。所有发现均为边界精度、UX 一致性、文档措辞层面，不阻塞当前 PR。

### 4 项改动一句话评价

| 项 | 评价 |
|---|---|
| W-08（完整 UUID） | 正确修复，无回归风险 |
| W-06（消息时间戳） | 方案合理但有 UX 不一致隐患 |
| agent-input（placeholder） | 纯改善，无风险 |
| C3（import size gate） | 思路对，阈值计算有 ~18 字节边界误差 |

---

## 1. 逐项审计

### 1.1 W-08: session 选择器显示完整 UUID

**位置**: [webui/app.js:234-236](../../webui/app.js#L234-L236)

**改动**: 去掉 `id.slice(0, 12)`，调用 `replaceOptions(sessSelect, ids)` 传完整 UUID。

**正面**:
- 修复了真实问题：UUID v4 前 12 位（`550e8400-e29`）区分度有限，多 session 场景下用户分不清。完整 UUID 是正确做法。
- 与 `charSelect` 行为一致（charSelect 一直显示完整 character_id）。
- CSS 已有约束：`select { width: 100%; }` + `#left-panel { width: 280px; min-width: 200px; overflow-y: auto; }`（[style.css:17, 98](../../webui/style.css#L17)），完整 UUID（36 字符）在 280px 面板内不会撑破布局，浏览器会自动截断显示。

**发现**:
- `low`（A4）— `replaceOptions` 渲染 `<option>` 时未设 `title` 属性。当 UUID 被截断显示时，用户无法 hover 看完整值。建议给 option 加 `title=完整UUID`，零成本改善可读性。**非阻塞**。
- `info` — 注释提到"select 元素宽度由 CSS 控制"，但 `<select size="6">` 是 listbox 渲染模式（非 dropdown），option 文本超出时各浏览器行为不一致（Chrome 截断、Firefox 可能横向滚动）。当前不构成问题，但若未来 left-panel 收窄需注意。

**结论**: 改动正确，建议合并。`title` 属性可作为后续微调。

### 1.2 W-06: appendMsg 加可选 ts 参数

**位置**: [webui/app.js:440-464](../../webui/app.js#L440-L464)，[webui/style.css:30-32](../../webui/style.css#L30-L32)

**改动**:
- `appendMsg(role, text, isStreaming, ts)` 加第 4 参数；`ts instanceof Date` 时渲染 `HH:MM:SS` 的 `.ts` span。
- `doSend`/`doSendText`/所有 error 路径传 `new Date()`；`loadHistory` 不传。
- CSS 加 `.msg .ts` 样式；`.role` 改 `display: inline-block`。

**正面**:
- 时间戳格式化逻辑正确（`padStart(2, '0')` 保证 HH:MM:SS 两位）。
- `ts instanceof Date` 类型守卫健壮：`undefined`/`null`/非 Date 值都不会误渲染。
- 调用点覆盖完整：已核对全部 11 处 `appendMsg` 调用（doSend 7 处、doSendText 2 处、loadHistory 1 处、loadHistory error 1 处），无遗漏。
- `loadHistory` 不传 `ts` 的理由（engine 的 `chat_log.jsonl` 不存消息时间戳）经源码验证属实：[chat_store.rs:36](../../engine/src/chat_store.rs#L36) 的 `messages: Vec<ChatMessage>`，`ChatMessage`（[adapter.rs:80](../../engine/src/adapter.rs#L80)）只有 `role` + `content`；`read_messages_jsonl` 直接反序列化为 `ChatMessage`，确实无时间戳字段。开发方没有把"加载时刻"冒充消息时间，这个判断是对的。

**发现**:

- `medium`（A1）— **UX 不一致隐患**。同一聊天视图中，新消息有 `HH:MM:SS` 时间戳，历史消息（loadHistory 加载的）没有。用户看到"为什么有些消息有时间、有些没有"时无法理解。开发方注释只解释了"为什么不传"，但没考虑用户感知。

  **我的替代方案**（按守则第 2 条提出）: 历史消息渲染一个占位标记（如 `—` 或灰色 `历史`），让"无时间戳"变成"显式标记为历史"，而非"看起来漏了"。或者，从 `ChatLogMeta`（[chat_store.rs:47](../../engine/src/chat_store.rs#L47)）读取会话级 `created_at`/`updated_at`，在视图顶部显示"会话时间范围"，给历史消息提供粗粒度时间上下文。这两个方案都比"什么都不显示"更透明。

- `low`（A3）— **`.role` 改 `inline-block` 缺乏注释说明**。[style.css:31](../../webui/style.css#L31) 把 `.role` 设为 `display: inline-block`，但代码注释和 commit message 都没解释为什么需要 `inline-block`。`.ts` 仍是默认 `inline`（[style.css:30](../../webui/style.css#L30) 未设 display），两者一个 `inline` 一个 `inline-block` 混搭，垂直对齐依赖浏览器默认 `vertical-align: baseline`。如果是为了对齐，应该两个都设 `inline-block` 并显式写 `vertical-align: baseline`；如果只是随手加的，应该删掉。**当前不会出 bug**（baseline 对齐在 10px/11px 字号下视觉无差异），但缺乏理由的样式声明不利于后续维护。

- `info`（A5）— 函数签名注释说"ts 参数（Date 或 null）"，但 `loadHistory` 实际不传参（`ts` 为 `undefined` 而非 `null`）。`instanceof Date` 对两者都返回 `false`，行为一致，但注释与实际调用方式有细微不符。建议注释改成"Date 或省略"。

- `info` — `appendInline(div, 'span', 'ts', ...)` 后紧跟 `div.append(' ')` 手动加空格（[app.js:452](../../webui/app.js#L452)）。`logEvent`（[app.js:57](../../webui/app.js#L57)）也用同样模式。这依赖 text node 分隔 inline 元素，可行但脆弱（若后续有人用 innerHTML 重写 div 会丢空格）。当前无问题，记录备查。

**结论**: 实现正确，建议合并。`medium` 级 UX 不一致问题建议后续迭代解决（可作为 GitHub issue 跟进）。

### 1.3 agent-input: placeholder 文案替换

**位置**: [webui/index.html:113](../../webui/index.html#L113)

**改动**: placeholder 从开发期占位 `'然后调 /v1/agent/run'` 改为 `'输入 agent run 指令…（如：用 echo 工具探测 loop-skeleton）'`。

**正面**:
- 清空开发期占位文本是正确的——面向用户的输入框不应暴露内部 API 路径。
- 新 placeholder 给出了具体示例（"用 echo 工具探测 loop-skeleton"），降低了用户理解 agent run 用法的门槛。
- placeholder 是纯 HTML 属性，不影响 `agentInput.value` 的读取逻辑，零回归风险。

**发现**:
- `info`（A7）— 示例引用 "loop-skeleton" 这个角色名。若该角色是测试专用、不在生产角色库中，用户照搬示例会得到 "character not found" 错误。考虑到 WebUI 本身是 dev console（AIRP Engine Console），引用测试角色可接受，但若未来 WebUI 面向更广泛用户群，示例应改为更通用的指令。

**结论**: 纯改善，建议合并。

### 1.4 C3: import 客户端 size gate

**位置**: [webui/app.js:822-829](../../webui/app.js#L822-L829)

**改动**: 上传前检查 `file.size > 7.5 * 1024 * 1024`，超限直接提示，不走 base64 编码 + 网络请求。

**正面**:
- 思路正确：客户端提前拦截，避免用户等完整 base64 编码 + 网络往返后才看到 413，体验明显改善。
- engine body limit 经源码验证：[daemon/mod.rs:200](../../engine/src/daemon/mod.rs#L200) 的 `DefaultBodyLimit::max(10 * 1024 * 1024)` 确实是 10MB。
- 错误提示信息有用：显示了实际文件大小和上限。

**发现**:

- `low`（A2）— **阈值计算有边界误差**。开发方注释说"base64 编码膨胀 4/3 倍 + JSON 外壳约 30 字节"，但实际计算如下：

  - engine limit = `10 * 1024 * 1024 = 10,485,760` 字节
  - PNG 路径 body = `{"card_png_base64":"<b64>"}`，外壳 = 22 字节（非 30）
  - base64 长度 = `ceil(N / 3) * 4`
  - 安全阈值 N 应满足：`ceil(N / 3) * 4 + 22 <= 10,485,760`
  - 解得 N <= 7,864,302 字节

  开发方设的 `IMPORT_RAW_LIMIT = 7.5 * 1024 * 1024 = 7,864,320` 字节，比安全值大 18 字节。这意味着 `[7,864,303, 7,864,320]` 区间内的文件会通过客户端检查但在 engine 端 413。

  **实际影响极小**（18 字节窗口，真实 PNG 文件几乎不可能恰好落在这个区间），且 engine 413 仍会被 `importResult.textContent` 显示，用户不会看到无反馈的失败。但注释中"约 30 字节"的估算偏松（实际 22 字节），且阈值未向下取整以容纳外壳开销。

  **我的替代方案**（按守则第 2 条）：与其用固定阈值近似，不如动态计算：

  ```js
  const b64Len = Math.ceil(file.size / 3) * 4;
  const bodyLen = b64Len + 22; // PNG 路径外壳
  if (bodyLen > 10 * 1024 * 1024) { ... }
  ```

  这样零误差，且 JSON 路径（`card_json`，外壳仅 16 字节，无 base64 膨胀）可以单独设更高阈值（接近 10MB 而非 7.5MB），避免对 JSON 文件过度限制。**非阻塞**，当前方案足够用。

- `info`（A6）— 阈值对 JSON 路径过度保守。JSON 文件不经 base64，body ≈ `file.size + 16`，10MB 限制下本可接受 ~10MB 的 JSON 文件，但 7.5MB 阈值统一拦截了。考虑到 character card 主流格式是 PNG（含嵌入图片），JSON 通常是小文件，这个过度保守在实践无影响。

- `info` — 注释中"JSON 外壳约 30 字节"应为 22 字节（PNG 路径）或 16 字节（JSON 路径）。不影响逻辑，但注释精度可改善。

**结论**: 实现正确，建议合并。阈值精度问题可作为后续微调。

---

## 2. 严重度评级汇总

| 编号 | 严重度 | 项 | 描述 | 建议时机 |
|---|---|---|---|---|
| A1 | `medium` | W-06 | 历史消息无时间戳，与新消息不一致，用户可能困惑 | 后续迭代 |
| A2 | `low` | C3 | 7.5MB 阈值比安全值大 18 字节，边界文件会 engine 413 | 后续微调 |
| A3 | `low` | W-06 | `.role` 改 `inline-block` 缺注释说明，且与 `.ts` 的 display 混搭 | 后续清理 |
| A4 | `low` | W-08 | `<option>` 未设 `title`，截断时无法 hover 看完整 UUID | 后续微调 |
| A5 | `info` | W-06 | 函数注释"Date 或 null"与实际"Date 或省略"不符 | 顺手改 |
| A6 | `info` | C3 | 注释"JSON 外壳约 30 字节"实际 22 字节 | 顺手改 |
| A7 | `info` | agent-input | 示例引用 "loop-skeleton" 测试角色 | 视受众决定 |

**无 `blocking` / `high` 级问题。**

---

## 3. 我的独立判断（不附和开发文档）

按守则第 1、2、3 条，以下是我作为独立审计者的看法，部分与开发方结论不同：

### 3.1 W-06 的"不传 ts"是正确判断，但止步太早

开发方选择"历史消息不显示时间戳"——这个**否**判断是对的（避免用加载时刻误导）。但**止步太早**。

`ChatLogMeta`（[chat_store.rs:47-52](../../engine/src/chat_store.rs#L47-L52)）已经存了会话级 `created_at` / `updated_at`。`/v1/chat/history` 端点返回的 `ChatLog` 也带这两个字段（[chat_store.rs:30-43](../../engine/src/chat_store.rs#L30-L43)）。WebUI 完全有能力显示"会话时间范围"或"最后更新时间"，给历史消息提供粗粒度时间上下文。

开发方注释只考虑了"消息级时间戳"这一维度，忽略了"会话级时间戳"这一已有数据源。建议后续迭代中，`loadHistory` 解析 `r.data.created_at` / `r.data.updated_at`，在聊天视图顶部或某处显示，让"历史消息无时间戳"不再显得突兀。

### 3.2 C3 的固定阈值是"够用但不够好"

开发方用 `7.5MB` 固定阈值，注释承认是近似（"约 30 字节"、"可能撞 413"）。近似本身可接受，但既然 `Math.ceil(file.size / 3) * 4` 可以零成本精确计算 base64 长度，没理由用近似。

更重要的：**7.5MB 阈值对 JSON 路径过度限制**。JSON 文件不经 base64，本可接受接近 10MB 的文件，但被统一卡在 7.5MB。开发方注释只考虑了 PNG（base64）路径，没区分两条路径的膨胀差异。

建议后续改为按路径动态计算阈值。当前方案不阻塞合并，因为 character card 主流是 PNG，JSON 文件通常很小，过度限制在实践无影响。

### 3.3 历史决策质疑：ChatMessage 为什么没有时间戳？

按守则第 3 条，我质疑 `ChatMessage`（[adapter.rs:80](../../engine/src/adapter.rs#L80)）只有 `role` + `content` 的历史设计。

OpenAI 兼容协议的 `ChatMessage` 确实只有这两个字段，这是协议层面的事实。但 `chat_store.rs` 的 `ChatLog` 是 AIRP 自己的持久化结构，完全可以在写入 jsonl 时额外存 `ts` 字段（serde `#[serde(flatten)]` 或包装结构），读取时再剥离。这样既不破坏 OpenAI 协议兼容性，又能支持消息级时间戳。

当前设计把"协议兼容"和"持久化格式"绑定了，导致 WebUI 无法显示历史消息时间戳。这是一个**可修正的历史决策**，建议作为后续 engine 侧改进项（需要 chat_store jsonl 格式迁移）。

### 3.4 总体评价

这 4 项改动是合格的 usability polish。开发方的判断在"是否做"层面基本正确，在"怎么做"层面有改进空间（W-06 止步太早、C3 用近似而非精确、.role 改动缺注释）。但所有问题都在 `low`/`info`/`medium` 级别，不阻塞合并。

**建议合并，A1-A4 作为 GitHub issue 跟进。**

---

## 4. 测试验证

| 测试 | 结果 |
|---|---|
| `node --check webui/app.js` | OK |
| `node --check webui/serve.js` | OK |
| `cargo test -p airp-core --lib` | 339 passed / 0 failed / 1 ignored |
| `cargo test -p airp-core --test '*'` | 19 passed / 0 failed（3 agent_run + 11 openai_compat + 5 sse_wiremock） |
| 神圣不变式 `subagent_context_has_no_orchestrator_noise` | ok |
| 神圣不变式 `subagent_prepared_pipeline_has_no_orchestrator_noise` | ok |
| `target/test-md-v2.js` | 24 pass / 0 fail |
| `target/test-serve-security.js` | 12 pass / 0 fail |

总计：358 cargo tests + 36 webui tests 全绿，2/2 神圣不变式通过。

---

*审计源: GLM-5.2 | 审计完成时间: 2026-07-06 | 依据: AGENTS.md 审计守则三条*
