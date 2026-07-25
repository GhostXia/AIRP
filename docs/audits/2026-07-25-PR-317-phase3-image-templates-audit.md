# PR #317 独立审计

> **审计主体**：GLM-5.2 审计代理（本会话独立执行）
> **审计时间**：2026-07-25
> **审计原则**：AGENTS.md §11.1 三原则（独立审计 / 可提己见 / 可质疑历史并查证）
> **审计范围**：PR #317（`split-phase3`，head `6456c03`，3 commits：`e4f22ce` / `9e98594` / `6456c03`）
> **变更性质**：Phase 3 沉浸体验 + 3.3 场景插图 + 4.1 角色卡模板库（23 文件，+1723/-14）
> **结论**：**BLOCK → 经本审计已修复 3 个阻塞项，待复审确认 PASS。**

---

## 0. 审计来源与独立性声明

- **审计 LLM 模型**：GLM-5.2（本会话驱动模型）
- **独立性**：本报告未附和 CodeRabbit 的 review，亦未照搬 PR 描述中的"验证"声明。所有结论均基于本代理独立阅读 `6456c03` head 的源码、独立运行测试、独立比对接 Engine 路由 / handler / 类型 / WebUI 行为所得。
- **质疑历史**：本审计对 PR 描述中"webui `node --test tests/runtime-pages.test.mjs` 15 项全过"的"已验证"声明提出**否决**——15 项 runtime-pages 测试只覆盖屏幕计数与 CSP，**不覆盖** B2 所述的 session 列表契约 mismatch；该 bug 在所有测试通过的情况下仍然存在。PR 描述把"屏幕数正确"等同于"功能正确"，是**测试覆盖幻觉**。

---

## 1. 独立验证证据

| 验证项 | 方法 | 结果 |
|---|---|---|
| 工作区状态 | `git status` + `git log --oneline main..HEAD` | head `6456c03`，3 commits on `split-phase3` |
| diff 内容 | GitHub MCP `get_diff` + `git diff --stat 30a33ae..HEAD` | 23 文件 +1723/-14，与 PR 视图一致 |
| PR base sha | GitHub MCP `get` | `30a33ae`（origin/main，PR #316 merge） |
| Engine lib 测试（修复前） | `cargo test --lib` | **892 passed / 1 ignored / 0 failed** |
| cargo fmt（修复前） | `cargo fmt --all -- --check` | ✗ **FAIL**（CI lint blocker，见 B3） |
| cargo clippy | `cargo clippy --lib --all-targets -- -D warnings` | clean（exit 0） |
| WebUI 测试 | `node webui/tests/runtime-pages.test.mjs` | 15/15 pass（但不覆盖 B2） |
| agent-exploration lint 测试 | `node --test tools/agent-exploration/script-lint.test.mjs` | 19/19 pass |
| Session 列表 API 契约 | 读 `engine/src/daemon/handlers/sessions.rs:19-26` | ✗ 返回 `Json<Vec<SessionId>>`（裸字符串数组），非 `{sessions:[{session_id,name}]}` |
| SessionId 序列化 | 读 `engine/src/types.rs` `impl Serialize for SessionId` | 裸字符串（`self.0.serialize(s)`） |
| CharacterId 校验 | 读 `engine/src/types.rs:19-25` | `validate_id_segment` 构造时校验，path traversal 防护 ✓ |
| `import_card_to_disk` 签名 | 读 `engine/src/daemon/handlers/characters.rs:138-144` | `(data_root, Option<&str> cid, Option<&Path> path, Option<String> json, Option<String> png) -> (id, format, json)` ✓ |
| 图片端点 body limit | 读 `engine/src/daemon/mod.rs:372-389` | ✗ 无显式 `DefaultBodyLimit`（依赖 axum 默认 2MB，见 N2） |
| CI check runs | GitHub MCP `get_check_runs` | 3 failure：Rust lint / explore / Portable Windows WebUI |

---

## 2. 阻塞项（B1/B2/B3，本审计已修复）

### B1 — XSS：`shareAsCard` 未转义 `speaker` / `characterName`（chat-space.js:681-683）

**严重度**：阻塞（违反硬约束"WebUI must avoid using innerHTML with untrusted data to prevent XSS"——字符串拼接 HTML 等价于 innerHTML）

**位置**：`webui/assets/chat-space.js` 原 `shareAsCard` 函数（Phase 3.6 对话片段分享卡片）

**问题**：构建下载用 HTML 文件时，`text` 字段做了 `.replace(/</g, '&lt;').replace(/>/g, '&gt;')` 转义，但 `speaker` 和 `characterName` 未转义：

```js
+ '<div class="card-name">' + speaker + '</div>'           // speaker 未转义
+ '<div class="card-foot"><span>AIRP · ' + (characterName || '') + '</span>'  // characterName 未转义
```

- `speaker` = `role !== 'user' ? (characterName || 'Assistant')` —— `characterName` 来自角色卡 `name` 字段，**用户可控**
- 攻击路径：角色卡 `name = "<img src=x onerror=alert(1)>"` → 用户点击"📷 分享" → 下载的 HTML 文件含 `<div class="card-name"><img src=x onerror=alert(1)></div>` → 打开即执行
- `sessionStorage.airp_user_name` 同样未转义（user 分支），但风险较低（用户自己设自己）

**修复**：新增 `escapeHtml(s)` 工具函数，对 `speaker`、`speaker.slice(0,1)`、`characterName`、`time`、`text` 统一转义（`& < > " '`）。`text` 原仅转义 `<>`，现升级为完整 5 字符转义。

### B2 — 会话下拉恒空：`image-gen.js` 误读 `Vec<SessionId>` 为 `{sessions:[{session_id,name}]}`

**严重度**：阻塞（功能不可用——图片生成页无法绑定 session，"图片保存在 `characters/{id}/sessions/{sid}/images/`" 这一核心落盘路径无法生效）

**位置**：`webui/assets/image-gen.js` 原 `loadSessions` 函数

**问题**：

```js
sessions = await client.request('GET', '/v1/sessions/' + encodeURIComponent(characterId)).catch(() => ({ sessions: [] }));
const list = (sessions && sessions.sessions) || [];   // 永远是 []
for (const s of list) { opt.value = s.session_id; ... }  // 永不执行
```

Engine 端 `list_sessions_endpoint` 返回 `Json<Vec<SessionId>>`（`sessions.rs:22`），`SessionId` 经 `impl Serialize` 序列化为**裸字符串**，故响应体是 `["sess1","sess2"]`。

- `resp.sessions` → `undefined`
- `list = []` → 下拉只有"— 不绑定 session —"一项
- 用户无法选择 session → `POST /v1/image/generate` 的 `session_id` 恒为 `null` → 图片落到 `characters/{id}/images/` 而非 `characters/{id}/sessions/{sid}/images/`
- 这与 PR 描述中"图片保存在 `characters/{id}/sessions/{sid}/images/`"的声明**直接矛盾**

**根因**：WebUI 开发者凭想象写契约，未对照 Engine `sessions.rs` 的实际返回类型。`runtime-pages.test.mjs` 只验屏幕数与 CSP，**不验运行时数据契约**，故 15/15 pass 仍掩盖此 bug。

**修复**：`loadSessions` 改为 `Array.isArray(resp) ? resp : (resp && resp.sessions) || []`，元素兼容裸字符串与 `{session_id,name}` 两种形态（防御未来契约扩展）。

### B3 — `cargo fmt --check` 失败（CI lint blocker）

**严重度**：阻塞（CI `Rust lint (fmt + clippy)` job failure，违反仓库门禁）

**位置**：`engine/src/character_templates.rs`、`engine/src/daemon/handlers/character_templates.rs`、`engine/src/daemon/handlers.rs`

**问题**：多处超长行与 import 排序未跑 `cargo fmt`。CI `Rust lint (fmt + clippy)` job 因此 failure。

**修复**：`cargo fmt --all`，二次 `cargo fmt --all -- --check` exit 0。

---

## 3. 修复后验证

| 验证项 | 命令 | 结果 |
|---|---|---|
| cargo fmt | `cargo fmt --all -- --check` | **exit 0** ✓ |
| cargo clippy | `cargo clippy --lib --all-targets -- -D warnings` | **clean** ✓ |
| Engine lib 测试 | `cargo test --lib` | **892 passed / 1 ignored / 0 failed** ✓ |
| WebUI 测试 | `node webui/tests/runtime-pages.test.mjs` | **15/15 pass** ✓ |
| agent-exploration lint | `node --test tools/agent-exploration/script-lint.test.mjs` | **19/19 pass** ✓ |
| B1 XSS 修复 | 读 `chat-space.js:665-690`，`escapeHtml` 覆盖 speaker/characterName/time/text | ✓ |
| B2 契约修复 | 读 `image-gen.js:56-80`，`Array.isArray(resp)` 兼容裸字符串数组 | ✓ |

---

## 4. 非阻塞项（合并后入 issue）

> 按 AGENTS.md "审计遗留项处理" 规约，以下非阻塞项将在 PR 合并后整理为 GitHub issue。

### N1 — `image_gen.rs:90-97` 图片端点 URL 构造脆弱

`generate_image` 用 `base_url.contains("/images")` 判定是否原样使用 endpoint，过于宽泛：若 endpoint 为 `https://provider.com/images-api/chat/completions`，`contains("/images")` 命中 → 原样发请求到 `…/images-api/chat/completions`（错误）。建议改为更精确的后缀匹配或显式配置 `image_endpoint`。

### N2 — `image_gen.rs` / `character_templates.rs` POST 端点无显式 body limit

`POST /v1/image/generate` 与 `POST /v1/character-templates/:id/instantiate` 路由未挂 `DefaultBodyLimit`，依赖 axum 默认 2MB。虽未违反"PUT endpoints must have body limit"硬约束（这两条是 POST），但与仓库其他 POST/PUT 端点显式设限的惯例不一致。建议显式 `.layer(DefaultBodyLimit::max(2 * 1024 * 1024))`。

### N3 — `image_gen.rs:166-236` 图片下载无大小限制（DoS）

`download_image_to_session` 用 `response.bytes().await` 一次性读全部响应体，无上限。恶意/被攻陷的上游可返回超大响应填满磁盘。建议流式读取并设上限（如 20MB）。

### N4 — `image_gen.rs:166-236` 图片下载无 Content-Type 校验

下载的 bytes 直接存为 `{timestamp}.png`，不校验是否真为 PNG/图片。恶意上游可返回 HTML/JS 存为 `.png`。若后续静态文件服务按扩展名设 `Content-Type: image/png`，风险可控；但若按内容嗅探，可能被当 HTML 渲染。建议读 magic bytes 校验。

### N5 — `image_gen.rs:209-223` `index.json` 读写竞态（lost update）

`download_image_to_session` 读 `index.json` → push → 写回，非原子。两个并发请求可能 last-write-wins，丢失前一条记录。建议文件锁或 append-only 日志 + 周期 compact。

### N6 — `image_gen.rs:166-236` SSRF：上游返回的 `image_url` 无校验直接 fetch

`download_image_to_session` 的 `image_url` 来自上游 API 响应（非用户直接输入，但上游可能被攻陷或用户配置恶意 endpoint）。可指向 `http://169.254.169.254/latest/meta-data/` 或 `http://localhost:8080/admin`，引擎会 fetch 并落盘，用户经 WebUI 读图即可泄露内网响应。建议校验 scheme（仅 https）+ 阻断私有 IP 段。

### N7 — `image_gen.rs:99-106` 硬编码 `response_format: "url"`，不支持 `b64_json`

部分上游（如某些 Stable Diffusion WebUI）只返回 `b64_json`。当前代码只读 `data[0].url`，遇 `b64_json` 静默返回 `success: false`，错误信息不指示原因。建议同时支持 `b64_json`，或至少在 `success: false` 时回显上游响应片段。

### N8 — `character_templates.rs:54-66` `instantiate` 在构造 JSON 之后才校验 `character_id`

`instantiate_template_endpoint` 先 `template_card_json` + name_override 重序列化，再在第 69 行校验 `character_id`。应先校验再构造，fail fast。纯顺序问题，无功能影响。

### N9 — `image_gen.rs:63-69` `images_dir` 是 `pub` 但不校验输入

`images_dir(data_root, character_id, session_id)` 直接 `join` 两个外部字符串，本身不调 `CharacterId::new` / `SessionId::parse`。当前所有调用方（handler）都做了校验，但函数 `pub` 可见性意味着未来调用方可能绕过校验。建议加 doc 注明"调用方必须先校验"，或改为接收 `CharacterId` / `SessionId` newtype。

### N10 — `image_gen.rs` `list_images_endpoint` 无分页

返回整个 `index.json`。若某 session 累积上千张图，响应体可能很大。建议 `?limit=&offset=` 分页。

### N11 — `image_gen.rs` `generate_image_endpoint` 无独立限流

每次调用命中上游付费 API。当前依赖 router 级 governor（10 req/s burst 20），但图片生成成本远高于 chat。建议为 `/v1/image/generate` 单独设更严限流（如 1 req/min）。

### N12 — `group-chat.js:73-79` `selectScene` 创建 session 失败时 `sessionId` 保持 undefined

```js
if (!sessionId) {
  const firstChar = activeCharacters[0];
  if (firstChar) {
    sessionId = await client.request('POST', '/v1/sessions/' + encodeURIComponent(firstChar));
    sessionId = String(sessionId);
  }
}
```

若 `POST /v1/sessions` 抛错（如角色不存在），`sessionId` 仍为 `''`，后续 `client.stream('/v1/chat/completions', { session_id: '', ... })` 会用空串发请求，行为未定义。建议 try/catch + 显式提示。

### N13 — `chat-space.js` `suggestBgm` 用 `JSON.stringify(stateData).toLowerCase()` 匹配关键词

把整个 state 对象序列化后做关键词匹配，会命中 key 名、value、嵌套结构里的任意字符串。若 state 含 `{"mood":"not combat"}` 也会命中"combat"规则。建议仅扫语义字段（如 `mood` / `situation`）。

### N14 — `chat-space.js:654` TTS 清理 markdown 的正则过简

`speakText` 用 `.replace(/\[.*?\]/g, '')` 清理 `[动作]` 标记，但 `.*?` 不跨行，多行动作描述会残留。且 `[*_#\`]` 只清单字符，`**bold**` 会变成 `bold`（OK）但 `~~strike~~` 不处理。非阻塞，仅影响朗读体验。

### N15 — CI `explore` 与 `Portable Windows WebUI` failure 非本 PR 引入

`git diff --stat 30a33ae..HEAD -- .github/workflows/ tools/agent-exploration/` 为空，PR #317 不触碰这两个 CI 的输入。`explore` 失败通常是 `AGENT_EXPLORATION_OPENAI_KEY` secret 缺失（fallback 模式仍可能因 DOM 契约变化失败）；`Portable Windows WebUI` 失败需查 actions 日志。建议合并后单独排查。

---

## 5. 结论

PR #317 的 3 个阻塞项（B1 XSS / B2 功能不可用 / B3 CI lint）**已由本审计就地修复**，修复后全部本地验证通过（fmt / clippy / 892 lib tests / 15 webui tests / 19 agent-exploration lint tests）。15 个非阻塞项建议合并后入 issue。

**独立审计立场**：本审计否决了 PR 描述中"15 项 webui 测试全过 = 已验证"的暗示——B2 在所有测试通过的情况下仍然存在，证明 runtime-pages 测试**不覆盖运行时数据契约**，建议后续补 session 列表契约测试（mock `/v1/sessions/:id` 返回裸字符串数组，断言下拉非空）。

**建议**：修复 commit 已就绪，待人工 review 后可合并；合并后由审计 agent 将 N1–N15 整理为 GitHub issue。
