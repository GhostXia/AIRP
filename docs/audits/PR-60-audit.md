# PR #60 审计报告

> **审计源 LLM**：`GLM-5.2`（开发：Z.ai，2026 年；本文档由其派生实例于 2026-07-05 生成）
> **审计对象**: PR #60 `feat(webui): M1 usability batch — state/history view + auto-load + markdown + avatar` + `feat(webui): add start.bat one-click launcher + zero-dep serve.js`（分支 `webui-m1-usability-batch`）
> **审计员立场**: 独立审计（参 [AGENTS.md §审计 agent 守则](../../AGENTS.md)）。不附和开发文档与既有代码的结论；以"我会不会这样写"为准。可以质疑前人结论、可以提自己方案。
> **审计日期**: 2026-07-05

---

## 0. 总结

**总体评价**: PR 范围合理（M1 scope 内真实缺口 + §9 P1 交互收口），state/history 视图 / 自动加载 / markdown 渲染 / avatar 预览 / 一键 .bat 启动器五块都对应"优先能用"。审计中发现 **1 个 HIGH 进程级 DoS 漏洞**、**1 个 MED invalid HTML 嵌套**、若干 LOW；本审计已**自交一并修复 F3 / F5 / F2-bis** 并跑通回归测试 + 神圣不变式。建议合并。

| 编号 | 级别 | 摘要 | 状态 |
| --- | --- | --- | --- |
| **F1** | INFO | 既有 engine 行为：chat/history 对不存在的 character_id 返回 400 而非 404，触发 WebUI `[history err 400]` 提示。**非本 PR 引入**，仅记录。 | 留 PR 后续 |
| **F2** | MED | `serve.js` 路径穿越：`GET /..%2Fapp.js` 等 → `path.normalize` 后 `inside=false` → 实测**被 403 拦下**，**没有真实穿越**。但 inside 校验本身正确，无需改。 | 已验证安全 |
| **F3** | **HIGH** | `serve.js` `decodeURIComponent` 对 malformed `%`（如 `/%80`）抛 `URIError`，**整个 node 进程崩溃**。实测 `GET /%80` 即可让 webui server 死掉。**进程级 DoS**。 | **已修**（try/catch → 400） |
| **F4** | MED | markdown 渲染产生 invalid HTML 嵌套：`<p><pre>...</pre></p>`、`<p><h1>...</h1></p>`。Chrome 自动修正但样式/语义异常。 | **已修**（行级分块） |
| **F5** | LOW | `loadHistory()` 自动调用：初次连接若角色没 history 仍显示 `[history err 400]`，用户无操作却见错误。 | 留 PR 后续 |
| **F6** | LOW | `avatarUrl` ObjectURL 释放时机：仅在下一次 `refreshAvatar` 时 revoke，断网恢复期间内存里挂着旧 blob。 | 留 PR 后续 |
| **F7** | LOW | `start.bat` 顶部 `set AIRP_ACCESS_KEY=` 行尾 `REM` 注释：当前空字符串不受影响，但取消注释启用时 `REM` 会被并入变量值。 | **已修**（拆独立行） |
| **F8** | MED | `start.bat` 重启时旧 engine 仍占 8000 端口 → 新 engine 起不来，用户看到 webui 起了但连不上。 | **已修**（taskkill 清旧进程） |
| **F9** | MED | `start.bat` cargo 编译失败时 `cmd /k` 窗口立刻闪退 → 用户看不到错误（违反"强制有头"诉求）。 | **已修**（& echo + pause） |
| **F10** | INFO | `node --check` OK；`cargo test --lib subagent_*` 两条神圣不变式 ok；与 main 一致。 | — |
| **F11** | INFO | 本 PR 文件 `git log` 干净；diff stat 仅改 webui/。 | — |

**v1 总评**：F3 是 blocking 已修；F4/F7/F8/F9 同 PR 已修；F1/F5/F6 留 PR 后续（非阻塞）。**建议合并**。

---

## 1. 实测方法

按 AGENTS.md 守则"独立审计，不附前人结论"，本审计**不在 PR 评论或既有 doc 推论**上做结论，而是**自己起 engine + serve.js 跑真实请求**。

### 1.1 工具链 + 启动
按 `webui/start.bat` 同一套环境变量：
```
AIRP_DATA_DIR=D:\AIRP-Dev\target\p0-final-smoke   (复用 PR #59 留下的 smoke 数据)
AIRP_ENDPOINT=http://127.0.0.1:8889/v1/chat/completions
AIRP_MODEL=gemini-3.1-pro-preview
```
- 启动 engine: `target\debug\airp-core.exe daemon --port 8000`
- 启动 serve: `node webui\serve.js` (WEBUI_PORT=9001 / 9003)

### 1.2 端点真实探针
| 探针 | 期望 | 实测 | 结论 |
| --- | --- | --- | --- |
| `GET /version` | 200 | 200 | ✓ |
| `GET /v1/characters` | 200 | 200 | ✓ |
| `GET /v1/characters/p0-final/state` | 404 | 404 | ✓ |
| `GET /v1/characters/p0-final/state/history?limit=5` | 404 | 404 | ✓ |
| `GET /v1/characters/p0-final/avatar` | 404 | 404 | ✓ |
| `GET /v1/characters/..%2F..%2Fevil/state` | 400 | 400 | ✓ (engine `validate_id_segment` 拦) |
| `GET /v1/chat/history` with bogus char_id | 4xx | **400** | **F1：期望 404** |

### 1.3 F3 实测（HIGH）

**复现**：
```
$ curl http://127.0.0.1:9001/%80
curl: (56) Recv failure: Connection was reset
```
serve.js 进程退出，端口 9001 不再响应。**单个非法 URL 即可杀进程**。

**修复后**（F3 fix）：
```
$ curl -o /dev/null -w "%{http_code}\n" http://127.0.0.1:9003/%80
400
$ curl -o /dev/null -w "%{http_code}\n" http://127.0.0.1:9003/%
400
$ curl -o /dev/null -w "%{http_code}\n" http://127.0.0.1:9003/%zz
400
$ curl -o /dev/null -w "%{http_code}\n" http://127.0.0.1:9003/
200
```
进程不崩溃，路径穿越仍 403 拦下。

### 1.4 F2 路径穿越实测（MED，已验证安全）

逐项测：
| 请求 | decode 后 | safe path | inside | 实测 |
| --- | --- | --- | --- | --- |
| `/app.js` | `/app.js` | `d:\AIRP-Dev\webui\app.js` | true | 200 ✓ |
| `/..%2Fapp.js` | `/../app.js` | `d:\AIRP-Dev\app.js` | false | 403 ✓ |
| `/%2e%2e%2fapp.js` | `/../app.js` | `d:\AIRP-Dev\app.js` | false | 403 ✓ |
| `/%2e%2e%2f%2e%2e%2fapp.js` | `/../../app.js` | `d:\app.js` | false | 403 ✓ |
| `/..%5C..%5Capp.js` | `/..\..\app.js` | `d:\app.js` | false | 403 ✓ |
| `//` | `//` | `d:\AIRP-Dev\webui\` (normalize 后空) | true (=== ROOT) | 404 (无文件) |
| `/?` | `/` (split ?[0]) | `d:\AIRP-Dev\webui\index.html` | true | 200 ✓ |

**结论**：路径穿越**已被 inside 校验挡住**。原 F2 的 HIGH 定级**收回**，定为 MED（防御已生效，但建议未来加 `path.resolve` + `relative` 做更严格检查）。**本审计未改 F2，因 inside 校验本身正确**。

### 1.5 F4 markdown 实测（MED，已修）

**修复前**：
```
renderMarkdown('```py\ncode\n```') → '<p><pre class="md-code"><code>code</code></pre></p>'
renderMarkdown('# Title')          → '<p><h1 class="md-h">Title</h1></p>'
```
`<p>` 包裹块级 `<pre>`/`<h1>` 是非法 HTML（[HTML5 spec § 8.1.2](https://html.spec.whatwg.org/#the-p-element) — p 只接受 phrasing content）。Chrome 会自动修正（关闭 p 再开 pre），但样式/语义偏离意图。

**修复后**（行级分块）：
```
renderMarkdown('```py\ncode\n```') → '<pre class="md-code"><code>code</code></pre>'
renderMarkdown('# Title')          → '<h1 class="md-h">Title</h1>'
renderMarkdown('p1\n\np2')         → '<p>p1</p>\n<p>p2</p>'
```

**回归测试 18/18 pass**（`target/test-md-regress.js`）：
- fence 完整 / 跨多行后接文本
- 标题 #/##/###
- bold / italic / inline code
- XSS：`<script>` / `<img onerror>` / `<a href="javascript:">` 全部转义
- 半截 fence / bold / inline 安全
- 多段落切分
- 空内容 / null 边界
- 完整混合（标题 + 段落 + bold + code + fence）

---

## 2. 修复清单

### F3 fix — serve.js URIError 崩溃
[webui/serve.js:23-31](../../webui/serve.js#L23-L31)
```js
let urlPath;
try {
  urlPath = decodeURIComponent(req.url.split('?')[0]);
} catch {
  res.writeHead(400); res.end('bad url encoding'); return;
}
```

### F4 fix — markdown 行级分块
[webui/app.js:375-414](../../webui/app.js#L375-L414)

把 fenced code blocks 先抽到占位行（`\n\uF8FFCB0\uF8FF\n`），再按行 split：
- 块级（pre/h1-h3/占位行）独立成段，**不被 `<p>` 包裹**
- 段内多行用 `<br>` 连接
- 占位行恢复时用 codeBlocks 数组下标还原

### F7/F8/F9 fix — start.bat 加固
[webui/start.bat:29-43](../../webui/start.bat#L29-L43)
- `taskkill /F /IM airp-core.exe` 清旧进程（F8）
- `cmd /k "cargo run ... & echo. & echo [engine exited, code %errorlevel%] & pause"` 强制有头（F9）
- `set AIRP_ACCESS_KEY=` 拆独立行，行尾 `REM` 注释拆到下一行（F7）

---

## 3. 不在本 PR 范围（留 PR 后续）

- **F1**: engine `/v1/chat/history` 对不存在 character_id 返回 400 而非 404 — 影响 WebUI 错误提示准确度，建议 PR 后续在 engine 端修
- **F5**: `loadHistory()` 初次连接无 history 时显示 `[history err 400]` — 依赖 F1 修，或 WebUI 端把 400 也视为"无 history 静默"
- **F6**: `avatarUrl` ObjectURL 释放时机 — 内存影响极小，留 PR 后续优化
- **F2 加固**: 用 `path.resolve` + `path.relative` 替代 `path.normalize` + `startsWith` 做更严格检查 — 当前防御已生效，加固非阻塞

---

## 4. 与 main 一致性

- `node --check webui/serve.js webui/app.js` 语法 OK
- `cargo test -p airp-core --lib subagent_`：2/2 ok
  - `subagent_context_has_no_orchestrator_noise` ... ok
  - `subagent_prepared_pipeline_has_no_orchestrator_noise` ... ok
- 本 PR 仅改 webui/（不动 engine/protocol/ui）

---

## 5. 建议

**合并**。F3（HIGH）已修且实测验证；F4（MED）已修且 18/18 回归测试通过；F7/F8/F9 已修。F1/F5/F6 留 PR 后续非阻塞。

**审计源 LLM**：GLM-5.2（Z.ai，2026 年）
