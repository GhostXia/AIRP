# PR #60 第二次审计报告

> **审计源 LLM**：`Kimi-K2.7-Code`（Moonshot AI，2025 年）
> **审计对象**：PR #60 在 `aa39659` 之后的当前工作区（分支 `webui-m1-usability-batch`）
> **审计员立场**：独立审计（参 [AGENTS.md §审计 agent 守则](../../AGENTS.md)）。不附和 v1 结论，以"我会不会这样写"为准；对前次审计未发现的问题继续追查。
> **审计日期**：2026-07-05

---

## 0. 总结

**总体评价**：v1 审计修复了 F3/F4/F7/F8/F10 后，本次审计在真实探针中进一步发现 **F3 修复不完整**（null byte / 无效 UTF-8 仍会崩溃进程），以及 markdown 渲染中的 **F16 占位符注入**、**F17 inline code 占位符误替换**、**F18 CRLF 未分段** 三个 MED/LOW 问题。本次审计已**自交修复**上述问题，并补充 24/24 markdown 回归用例 + 12/12 serve.js 安全回归用例。建议合并。

| 编号 | 级别 | 摘要 | 状态 |
| --- | --- | --- | --- |
| **F3-bis** | **HIGH** | `serve.js` v1 fix 只兜住 `decodeURIComponent` 抛错；`/foo%00bar`、`/%C0%AF..` 等通过 decode 后让 `fs.readFile` 抛同步 `TypeError`，**进程仍崩溃**。 | **已修** |
| **F16** | MED | markdown 占位符使用固定序列 `\uF8FFCB<i>\uF8FF`，用户输入同形序列会被错误替换为 code block 内容。 | **已修**（随机 nonce 占位符） |
| **F17** | MED | inline code 中出现旧版占位符序列时，恢复阶段把它当 fence 占位符替换，渲染成 `undefined`。 | **已修**（同 F16 nonce） |
| **F18** | LOW | CRLF（`\r\n`）未被统一为 `\n`，导致 `a\r\n\r\nb` 只生成一个 `<p>`。 | **已修**（预处理统一换行符） |
| **F2-bis** | MED | v1 的 `path.normalize + startsWith` 在 Windows 下对 `/app.js` 会解析到 `d:\app.js`；同时加固为 `path.resolve + path.relative`。 | **已修** |

---

## 1. 实测方法

本次审计继续采用"起真实 serve.js 进程 + Node 探针"，不复用 v1 的结论。

### 1.1 serve.js 安全探针

启动临时实例（`WEBUI_PORT=19012`），用 `target/test-serve-security.js` 自动探针：

```
PASS malformed %80 400
PASS malformed % 400
PASS malformed %zz 400
PASS null byte 400
PASS invalid utf8 %C0%AF 400
PASS invalid utf8 traversal 400
PASS double dot encoded 403
PASS double dot lowercase 403
PASS backslash traversal 403
PASS root index 200
PASS index.html 200
PASS app.js 200
---
12 pass, 0 fail
```

**关键发现**：v1 fix 后的代码对 `/foo%00bar` 直接崩溃（`TypeError [ERR_INVALID_ARG_VALUE]: path must be a string without null bytes`），因为 `decodeURIComponent` 不抛错，但 `fs.readFile` 会抛同步异常。`/%C0%AF..` 同样因 decode 出非法 UTF-8 序列触发底层异常。

### 1.2 markdown 回归探针

用 `target/test-md-v2.js` 从 `app.js` 中抽出 `renderMarkdown` 并跑 24 个用例：

```
PASS placeholder injection 0
PASS placeholder injection 1
PASS inline code placeholder
PASS CRLF paragraphs
PASS CRLF single line break
PASS legacy placeholder literal
PASS CRLF inside fence
...
---
24 pass, 0 fail
```

### 1.3 神圣不变式

```
node --check webui/serve.js webui/app.js     # OK
cargo test -p airp-core --lib subagent_      # 2 passed
```

---

## 2. 问题与修复

### F3-bis — serve.js 进程仍会在 null byte / 无效 UTF-8 下崩溃

**复现**：
```
GET /foo%00bar        → ECONNRESET，node 崩溃
GET /%C0%AF..         → ECONNRESET，node 崩溃
```

**根因**：v1 的 try/catch 只包住了 `decodeURIComponent`。`decodeURIComponent('/foo%00bar')` 成功返回 `'/foo\x00bar'`，随后 `fs.readFile('d:\\AIRP-Dev\\webui\\foo\x00bar')` 抛同步 `TypeError`，未捕获则整个进程退出。

**修复**：[webui/serve.js:38-59](../../webui/serve.js#L38-L59)
1. decode 后显式检查 `urlPath.includes('\0')`，遇 null byte 直接 400。
2. `fs.readFile` 外层加 try/catch，任何同步异常返回 500 但不崩溃进程。
3. 路径穿越检查升级为 `path.resolve(ROOT, filePath) + path.relative`，更严格。
4. 处理 Windows 下 `path.resolve(ROOT, '/foo')` 解析为当前盘根目录的陷阱：统一去掉 urlPath 前导 `/` 再拼接。

### F16/F17 — markdown 占位符冲突

**复现**：
```js
renderMarkdown('\uF8FFCB0\uF8FF')
// 修复前：输出第一个 code block 的内容（若存在）

renderMarkdown('`\uF8FFCB0\uF8FF`')
// 修复前：<code class="md-code-inline">undefined</code>
```

**根因**：占位符使用固定模式 `\uF8FFCB<i>\uF8FF`，用户输入或 inline code 中可能出现同形序列；恢复阶段正则全局替换，把非 fence 占位符也替换了。

**修复**：[webui/app.js:380-385](../../webui/app.js#L380-L385)
- 生成 8 位随机 nonce：`phPrefix = '\uF8FFCB' + nonce + '_'`
- 恢复正则只匹配带 nonce 的占位符，用户无法预知或构造。

### F18 — CRLF 未分段

**复现**：
```js
renderMarkdown('a\r\n\r\nb')
// 修复前：<p>a<br><br>b</p>
// 修复后：<p>a</p>\n<p>b</p>
```

**修复**：[webui/app.js:387-388](../../webui/app.js#L387-L388) 预处理阶段统一换行符：`s.replace(/\r\n/g, '\n').replace(/\r/g, '\n')`。

---

## 3. 建议

**合并**。本次审计发现的 F3-bis（HIGH）已修且 12/12 安全探针通过；F16/F17（MED）和 F18（LOW）已修且 24/24 markdown 回归用例通过；F2-bis 路径穿越加固完成。未引入 engine/protocol/ui 改动，神圣不变式通过。

**审计源 LLM**：Kimi-K2.7-Code（Moonshot AI，2025 年）
