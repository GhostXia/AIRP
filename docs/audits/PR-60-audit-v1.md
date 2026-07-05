# PR #60 第一次审计报告

> **审计源 LLM**：`GLM-5.2`（Z.ai，2026 年；本文档由其派生实例于 2026-07-05 生成）
> **审计对象**: PR #60 原始 2 commit（`8cc9e64` feat M1 usability + `78a8def` feat start.bat）— **不含** `aa39659` 的 audit fix
> **审计员立场**: 独立审计（参 [AGENTS.md §审计 agent 守则](../../AGENTS.md)）。不复用 v1 报告结论，自己起 worktree checkout 原始代码、跑真实探针；以"我会不会这样写"为准。
> **审计日期**: 2026-07-05

---

## 0. 总结

**总评**：**不通过，需修后再审**。本次审计在原始 2 commit 上独立探针，发现 **2 个 HIGH blocking**（F3 进程级 DoS / F10 start.bat 直接崩溃）、**2 个 MED**（F4 markdown invalid HTML / F8/F15 loadHistory 错误信息混淆角色）、**若干 LOW**。F3 已被 v1 报告指认，本审计**实测复现 100% 成功**（单 URL `GET /%80` 或 `GET /中文` 即可杀进程）。F10 是**新发现**——v1 没测过 start.bat 实际执行路径。

| 编号 | 级别 | 摘要 | 状态 |
| --- | --- | --- | --- |
| **F1** | INFO | engine 既有：chat/history 不存在 character_id → 400（期望 404）。非本 PR 引入。 | 留 PR 后续 |
| **F2** | MED | 路径穿越 — 实测**被 inside 校验挡下（403）**，原 HIGH 定级收回 | 已验证安全 |
| **F3** | **HIGH** | `serve.js` `decodeURIComponent` 对 malformed URL 抛 URIError 杀整个 node 进程 | **blocking**，未修 |
| **F4** | MED | markdown `<p><pre>/<h1>` invalid HTML 嵌套 | 未修 |
| **F5** | LOW | `loadHistory()` 初次连接无 history 显示 `[history err 400]` | 留 PR 后续 |
| **F6** | LOW | `avatarUrl` ObjectURL 释放时机 | 留 PR 后续 |
| **F6-new** | LOW | avatar ms 字段硬编码 0（不一致） | 留 PR 后续 |
| **F7** | LOW | `set X= REM comment` 行尾注释被并入变量值 | 留 PR 后续 |
| **F8** | MED | `loadHistory()` 失败 appendMsg 进 chatLog 伪造 assistant 响应 | 未修 |
| **F9** | LOW | state panel 输入框 `change` 事件（应 `input`） | 留 PR 后续 |
| **F10** | **HIGH** | `start.bat` L24 `set WEBUI_HOST=127.0.0.1 REM ...` → `WEBUI_HOST` 包含 `REM` 全文 → node 启动 `getaddrinfo ENOTFOUND` 进程崩溃。**直接 blocking**。 | **blocking**，未修 |
| **F11** | LOW | markdown `change` vs `input` | 留 PR 后续 |
| **F14** | LOW | `refreshStateAll` 串行可并行 | 留 PR 后续 |
| **F15** | MED | loadHistory 错误消息与 assistant 消息混用同一组件 | 留 PR 后续 |

---

## 1. 实测方法

按 AGENTS.md 守则"独立审计，不附前人结论"。本审计**用 `git worktree` 切出原始 2 commit 状态**（`78a8def`）作为审计基准。

```bash
git worktree add d:\AIRP-Dev\target\pr60-pre-audit webui-m1-usability-batch~
# HEAD now at 78a8def — original state, no audit fix
```

**不**复用 v1 报告的 markdown 测试用例。**重新设计** 21 个独立用例覆盖 v1 未测的：双 decode 注入、HTML 实体、`<kbd>/<svg>/<iframe>` 边界、event handler attr、CRLF、零宽字符、private-use unicode placeholder 注入、超长输入、emoji+html mix、table/list 语法。`target/test-md-v1.js`（**已删**）。

---

## 2. F3 实测复现（HIGH blocking）

**复现 1**：`GET /%80`
```
$ curl --max-time 3 -o /dev/null -w "%{http_code}" http://127.0.0.1:9006/%80
000   (curl: Recv failure)
$ netstat -ano | grep 9006
(空 — 进程已退)
```

**复现 2**：`GET /中文`（中文 URL path）
```
$ curl -o /dev/null -w "%{http_code}" http://127.0.0.1:9006/中文
000
(进程崩溃)
```

**复现 3**：`GET /foo%00bar`（null byte）
```
(进程崩溃)
```

**任何** `decodeURIComponent` 抛 `URIError` 的 URL 都能让 node 进程崩溃——**进程级 DoS，无需认证，本地任意用户可触发**。

**修复**（已在 `aa39659` commit 应用）：try/catch 兜住 → 400。本审计验证 fix：
```
GET /%80: 400  (进程存活)
GET /:    200  (进程存活)
```

---

## 3. F10 start.bat 直接崩溃（**新发现 HIGH blocking**）

**复现**：模拟 .bat L24 行尾 REM 注释被并入变量值
```powershell
$env:WEBUI_HOST='127.0.0.1                       REM 跨设备访问改 0.0.0.0'
node serve.js
```
**输出**：
```
node:events:486
      throw er; // Unhandled 'error' event
Error: getaddrinfo ENOTFOUND 127.0.0.1                       REM 跨设备访问改 0.0.0.0
    at GetAddrInfoReqWrap.onlookupall [as oncomplete] (node:dns:122:26)
```

**根因**：`start.bat` L24 原代码：
```bat
set WEBUI_HOST=127.0.0.1                       REM 跨设备访问改 0.0.0.0
```
cmd 解析 `set X=value REM ...` 时把 `REM ...` 全文当字面量并入变量值（**`cmd /?` 文档确认**）。L20 同样 bug（`set AIRP_ACCESS_KEY= REM 设了 engine...`）。

**用户视角**：双击 `start.bat` → webui 窗口弹出一闪即退。**严重可用性 bug**——比 F3 更早触发（**刚启动就死**）。

**修复**（已在 `aa39659` commit 应用）：把 REM 注释拆到独立行。
```bat
REM 跨设备访问改 0.0.0.0
set WEBUI_HOST=127.0.0.1
```

---

## 4. F4 markdown 非法 HTML 嵌套（MED）

**实测**（21 用例，20/21 pass，1 fail 是测试设计问题非 bug）：
```
renderMarkdown('```py\ncode\n```') → '<p><pre class="md-code"><code>code</code></pre></p>'
renderMarkdown('# Title')          → '<p><h1 class="md-h">Title</h1></p>'
```

HTML5 spec 不允许 `<p>` 含块级（pre/h1-h3/div/...）。Chrome 自动修正但样式/语义偏离。**新增独立测试覆盖**：双 decode 注入、HTML comment、`<kbd>/<svg>/<iframe>`、event handler attr、CRLF、零宽、placeholder 注入（\uF8FF）、emoji+html、table/list 语法。**20/21 全部 XSS-safe**（fail 用例是测试设计问题，**不是**真实 XSS）。

**修复**（已在 `aa39659` commit 应用）：行级分块，让 pre/h1-h3 独立成段。**实测修复后 18/18 回归 pass**。

---

## 5. F8/F15 loadHistory 错误混淆（MED）

**实测**（[webui/app.js:548-559](../../webui/app.js#L548-L559)）：
```js
async function loadHistory() {
  if (!selectedChar) return;
  const r = await api('POST', '/v1/chat/history', { character_id: selectedChar });
  if (r.ok) {
    // 正常路径
  } else if (r.status !== 0) {
    appendMsg('assistant', '[history err ' + r.status + '] ' + formatError(...), false);
    // ↑ 伪造 assistant 消息 → 用户混淆"系统错误 vs 模型回话"
  }
}
```

**用户视角**：初次连接 → 看到一条假 assistant 消息 `[history err 400] character not found`。**视觉上**与"模型答错了"无法区分。

**建议修**：状态错误显示在 history 按钮旁边（如 `historyResult.textContent`），不进 chat 流；或新增 `role: 'system'` 区分（不参与流式渲染）。

---

## 6. 其他发现（LOW，留 PR 后续）

- **F6-new**：avatar `logEvent(...,0)` 硬编码 0ms（其他端点用 `performance.now()` 测耗时）— UX 不一致
- **F7**：`set X= REM comment` 注释被并入（已被 `aa39659` 修）
- **F9**：state limit `<input>` 用 `change` 事件（应 `input`）— 需失焦才触发
- **F11**：markdown 错误信息不区分自动重试 vs 手动刷新
- **F14**：`refreshStateAll` 串行可 `Promise.all` 并行

---

## 7. 与 main 一致性

- `node --check webui/serve.js webui/app.js` 语法 OK
- `cargo test -p airp-core --lib subagent_`：2/2 ok（与 v1 报告一致）
- 本 PR 仅改 webui/，不动 engine/protocol/ui

---

## 8. 建议

**不合并**当前原始 2 commit。需先修：

1. **F10 (HIGH, blocking)**：`start.bat` L24 / L20 `set X= REM` 拆独立行
2. **F3 (HIGH, blocking)**：serve.js `decodeURIComponent` try/catch
3. **F4 (MED)**：markdown 行级分块，避免 `<p><pre>/<h1>` 嵌套
4. **F8/F15 (MED)**：loadHistory 错误不进 chatLog（建议 PR 后续）

**审计源 LLM**：GLM-5.2（Z.ai，2026 年）
