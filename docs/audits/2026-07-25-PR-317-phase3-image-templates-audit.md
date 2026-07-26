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

## 3.1 二次修复：CodeRabbit inline review（commit `bdf20f9`）

> 本审计首轮（commit `338f911`）只覆盖 B1/B2/B3。CodeRabbit 在 head `6456c03` 上发了 12 条 actionable inline review 线程 + 1 条 markdownlint 线程，本轮全部就地修复。
>
> **ID 规约**：本表用 `#N` 指 CodeRabbit inline 线程序号；与 §4 `Nn` 非阻塞项的对应关系在"映射"列显式标注，避免 ID 冲突。

| #线程 | 位置 | 修复 | §4 映射 |
|---|---|---|---|
| #1 download 失败 abort 整个 handler | `engine/src/daemon/handlers/image_gen.rs:127-153` | `match` download 结果，失败时 `tracing::warn!` 并保留 `resp.image_url` 返回 URL-only 响应（不丢已计费的上游生成结果） | — |
| #2 webui `<img src>` 404（`ServeDir` 指向 webui 不指 data_root） | `engine/src/daemon/mod.rs` + `handlers/image_gen.rs:237-272` | 新增 `GET /v1/characters/:cid/images/:filename` 与 `GET /v1/characters/:cid/sessions/:sid/images/:filename`，handler 校验 `CharacterId` / `SessionId` / `validate_image_filename` 后从 `data_root` 服务图片字节 | — |
| #3 秒级时间戳文件名碰撞 | `engine/src/image_gen.rs:194-204` | 改用毫秒时间戳 + 碰撞自增后缀 `{millis}_{n}.png` | — |
| #4 `index.json` 读-改-写竞态 + handler re-read `index.last()` | `engine/src/image_gen.rs:171-173,261-272` + `handlers/image_gen.rs:137-141` | 全局 `tokio::sync::Mutex` 序列化读-改-写；`download_image_to_session` 返回 `ImageMeta`，handler 直接用不再 re-read | **resolves §4 N5** |
| #5 `default_size`/`default_style` 重复定义 | `engine/src/image_gen.rs:34-41` + `handlers/image_gen.rs:12-15` | 提为 `pub(crate)` 单一来源，handler `use` 复用 | — |
| #6 图片下载无大小上限（DoS） | `engine/src/image_gen.rs:167-169,225-250` | `MAX_IMAGE_BYTES = 20 MiB`，Content-Length 预检 + 读后复核 | **resolves §4 N3** |
| #7 跨源 engine 仍带 session bearer（凭据外泄） | `webui/assets/character-templates.js:11-12` | `(base === location.origin) ? storedBearer : ''`，跨源 engine 不带 bearer | — |
| #8 模板卡 `<div>` 不可键盘操作 | `webui/assets/character-templates.js:55-57` + `.css:7-12` | 改 `<button type="button">`，加 `:focus-visible` 样式 | — |
| #9 `BGM_RULES` 第三标题乱码 | `webui/assets/chat-space.js:701` | 改为 `进击的巨人 OST - ətˈæk 0N tάɪtn`（原文 IPA 标题） | — |
| #10 `suggestBgm` 子串误命中（`warmth` 命中 `war`） | `webui/assets/chat-space.js:717-723` | ASCII 关键词用 `\b` 词边界正则，中文仍用 `includes` | 部分缓解 §4 N13 |
| #12 `18-group-chat.html` 重复 `<!DOCTYPE>` 文档 | `webui/screens/18-group-chat.html` | 删除合并遗留的第二个文档块，保留单一 `<html>` | — |
| #13 审计文档 markdownlint MD038（inline code span 转义反引号） | `docs/audits/2026-07-25-PR-317-phase3-image-templates-audit.md:181` | `` `[*_#\`]` `` → `` `` `[*_#`]` `` ``（双反引号分隔） | — |

## 3.2 三次修复：CodeRabbit 二次 inline review（commit `d09d618`）

> CodeRabbit 在 head `bdf20f9` 上又发了 2 条 actionable 线程，本轮就地修复。

| #线程 | 位置 | 修复 |
|---|---|---|
| #14 审计文档 §3.1 与 §4 的 N1/N2 ID 冲突 | `docs/audits/2026-07-25-PR-317-phase3-image-templates-audit.md:122-123` | §3.1 表删去 `/N1` `/N2` 后缀，改用"§4 映射"列显式标注对应关系（#4→N5，#6→N3）；§4 N3/N5 标注"已由 §3.1 修复" |
| #15 `validate_image_filename` 漏拒 `:`（Windows 驱动器前缀逃逸） | `engine/src/daemon/handlers/image_gen.rs:191-207` | 黑名单改白名单：仅允许 `[A-Za-z0-9_.-]`，显式拒 `..`。`C:foo.png` / `D:evil.png` / `:hidden.png` 全部拒。新增 6 个单元测试锁定行为 |

## 3.3 四次修复：CodeRabbit outside-diff + nitpick（commit `7f6e42d`）

> CodeRabbit 在 head `bdf20f9` 的 review body（折叠区）还有 2 条 "Outside diff range" actionable 线程 + 1 条 nitpick，前轮漏读。本轮就地修复。

| #线程 | 类型 | 位置 | 修复 |
|---|---|---|---|
| #16 `showDetail` 旧响应覆盖新选择（stale response race） | outside-diff | `webui/assets/character-templates.js:84-90` | 引入 `detailRequestId` 递增 token；每次 `showDetail` 进入时 `++` 取本请求 id，`await` 后比对，不匹配则丢弃响应（成功与失败分支都查）。stale 响应不再能覆盖用户最新选择的 `selectedTemplate` |
| #17 `INDEX_LOCK` TOCTOU：锁在文件写入后才获取 | outside-diff | `engine/src/image_gen.rs:194-273` | 重构为两阶段：Phase 1 锁外下载字节（保留网络并行吞吐）；Phase 2 持锁贯穿"选文件名 + 写文件 + 更新 index.json" 整段 critical section。同毫秒并发请求不再能在 `exists()` 检查处重叠选同一文件名 |
| #18 缺 `MAX_IMAGE_BYTES` / 文件名碰撞测试 | nitpick | `engine/src/image_gen.rs:287-314` | 提取 `pick_unique_image_filename(dir, millis)` 纯函数（调用方仍须持锁），新增 3 个测试：`max_image_bytes_is_20_mib`（锁定常量值防回归）、`pick_unique_image_filename_skips_existing`（用 tempdir 真实建文件验证跳后缀）、`download_image_to_session_writes_unique_files_under_collision`（标 ignore，需 mockito） |

## 3.4 五次修复：CI `Portable Windows WebUI` smoke 失败（本 commit）

> §4 N15 早期判断"`Portable Windows WebUI` failure 非本 PR 引入"**有误**——经查 actions 日志（run `30165183029`, job `89696855096`），失败发生在 `Smoke packaged engine and real Chrome` 步骤，`node ui/local-webui-browser-smoke.mjs` 在第 49 行 `waitForFunction(() => document.querySelector('#view')?.children.length > 0)` 抛 `TimeoutError`。根因是本 PR commit `9e98594`（Phase 3.1 多角色群聊 UI）把 `webui/screens/18-group-chat.html` 从控制台骨架（`#view` / `#heading-title` / `#runtime-status` + `console-runtime.js`）**整体重写**为专用群聊布局（`#group-flow` / `#scene-list` + `group-chat.js`），但 `ui/local-webui-browser-smoke.mjs` 的 15 屏导航 loop 仍按控制台契约检查 `#view`，对 18 屏必然超时。

| #线程 | 类型 | 位置 | 修复 |
|---|---|---|---|
| #19 `local-webui-browser-smoke.mjs` 对 18 屏检查不存在的 `#view` | CI blocker | `ui/local-webui-browser-smoke.mjs:38-53` | 在 15 屏 loop 内为 `18-group-chat.html` 单独走专用布局校验：`waitForFunction` 同时检查 `#engine-status` 已 finalize（`ok` 或 `danger`）且 `#scene-list.textContent` 不再是初始 "加载中…"，断言 `#engine-status` 不含 `danger`，然后 `continue` 跳过 `#view`/`#heading-title`/`#runtime-status` 的控制台断言。其他 14 屏走原契约不变 |

**为什么 `webui/tests/runtime-pages.test.mjs` 没抓到**：该测试 line 88-99 早已在 CodeRabbit #12 修复时把 `18-group-chat.html` 从"必须加载 `console-runtime.js`"的契约列表里剔除（见该处注释），但只覆盖了**静态 HTML 契约**，不覆盖**运行时 DOM 契约**。`local-webui-browser-smoke.mjs` 才是真正用 Chrome 跑 18 屏的运行时检查，而它没同步更新。两个测试文件各自正确、但契约不同步——这是测试覆盖的盲区。

**未修复（仍待合并后入 issue）**：§4 N1（URL 构造）/ N2（POST body limit）/ N4（Content-Type 校验）/ N6（SSRF）/ N7（b64_json）/ N8（instantiate 顺序）/ N9（`images_dir` pub 但不校验）/ N10（list 无分页）/ N11（无独立限流）/ N12（group-chat session 创建失败静默）/ N13（`JSON.stringify` 关键词匹配，部分已由 #10 缓解，仍建议改扫语义字段）/ N14（TTS 正则过简）。§4 N3 与 N5 已由 §3.1 修复，§4 N15 由本节 §3.4 修复，从待办中移除。

---

## 3.5 六次修复：CodeRabbit 第五轮 review（本 commit）

> CodeRabbit 在 `bce1260` 推送后又发了一轮 review（2026-07-26T02:04:24Z），提出 2 条 actionable 线程。本节为对这两条的修复。

| #线程 | 类型 | 位置 | 修复 |
|---|---|---|---|
| #20 `image_gen.rs` 图片下载 `response.bytes().await` 一次性缓冲整个 body，超大 chunked response 在 post-read 检查前已耗尽内存 | 🟠 Major，outside-diff | `engine/src/image_gen.rs:232-257` | 改用 `response.chunk().await` 流式接口，每个 chunk 累加检查 `MAX_IMAGE_BYTES`；超上限立即返回 `AirpError::Upstream`，不再继续读取。`Vec::with_capacity(1024 * 1024)` 用 1 MiB 起步而非预分配 20 MiB，避免对大图过度预留。`checked_add` 防 usize 溢出。Content-Length 预检保留（快速 reject），原 post-read 复核已删除（被流式检查取代） |
| #21 `local-webui-browser-smoke.mjs` 18 屏 predicate 在 engine `danger` 时仍要求 `#scene-list` populated，导致健康失败时 timeout 而非进 connectivity 错误报告 | Inline，actionable | `ui/local-webui-browser-smoke.mjs:55-62` | predicate 改为 `finalized && (danger OR populated)`（对应 JS `||`；用 `OR` 文本以避免 markdown 表格 cell 内 `\|\|` 被解析为列分隔符）——engine 进入 `danger` 时立即返回 true，下一行 `assert.equal(... classList.contains('danger'), false, 'must stay connected')` 会立即抛错并打印实际状态，不再 timeout。完整 predicate 代码见下方段落 |

**为何两条之前没修**：#20 是 CodeRabbit 在 §3.3 修复（commit `7f6e42d`）之后的复核中才提的新 outside-diff，此前 `response.bytes().await` 旁边还没有 `MAX_IMAGE_BYTES` 检查，§3.1 #6 只加了 Content-Length 预检 + 读后复核，没改读取方式本身；#21 是 §3.4（commit `bce1260`）引入的 18 屏专用 predicate 的副作用——只考虑了"engine 启动成功但 sceneList 未填充"的等待场景，没考虑"engine 启动失败、sceneList 永不填充"的失败场景，CodeRabbit 在复核 `bce1260` 时指出。

**视觉审查声明**：本次修复涉及 WebUI 改动（`ui/local-webui-browser-smoke.mjs` 是测试脚本而非 `webui/` 视觉资产，但 18 屏渲染行为间接相关），按 issue #319 补充要求（2026-07-26 用户立）应执行多模态视觉审查。**本审计 agent 为 GLM-5.2 纯文本模型，未执行视觉审查**——本次修改仅触及测试 predicate 逻辑，不改变 18 屏的视觉渲染；如需补审，应由 KIMI K3+ 多模态 agent 在 PR 合并前对 18 屏渲染截图独立审查。

---

## 4. 非阻塞项（合并后入 issue）

> 按 AGENTS.md "审计遗留项处理" 规约，以下非阻塞项将在 PR 合并后整理为 GitHub issue。

### N1 — `image_gen.rs:90-97` 图片端点 URL 构造脆弱

`generate_image` 用 `base_url.contains("/images")` 判定是否原样使用 endpoint，过于宽泛：若 endpoint 为 `https://provider.com/images-api/chat/completions`，`contains("/images")` 命中 → 原样发请求到 `…/images-api/chat/completions`（错误）。建议改为更精确的后缀匹配或显式配置 `image_endpoint`。

### N2 — `image_gen.rs` / `character_templates.rs` POST 端点无显式 body limit

`POST /v1/image/generate` 与 `POST /v1/character-templates/:id/instantiate` 路由未挂 `DefaultBodyLimit`，依赖 axum 默认 2MB。虽未违反"PUT endpoints must have body limit"硬约束（这两条是 POST），但与仓库其他 POST/PUT 端点显式设限的惯例不一致。建议显式 `.layer(DefaultBodyLimit::max(2 * 1024 * 1024))`。

### N3 — `image_gen.rs:166-236` 图片下载无大小限制（DoS）—— ✅ 已由 §3.1 #6 修复

`download_image_to_session` 用 `response.bytes().await` 一次性读全部响应体，无上限。恶意/被攻陷的上游可返回超大响应填满磁盘。建议流式读取并设上限（如 20MB）。

**状态**：已在 commit `bdf20f9` 由 §3.1 #6 修复（`MAX_IMAGE_BYTES = 20 MiB`，Content-Length 预检 + 读后复核）。无需入 issue。

### N4 — `image_gen.rs:166-236` 图片下载无 Content-Type 校验

下载的 bytes 直接存为 `{timestamp}.png`，不校验是否真为 PNG/图片。恶意上游可返回 HTML/JS 存为 `.png`。若后续静态文件服务按扩展名设 `Content-Type: image/png`，风险可控；但若按内容嗅探，可能被当 HTML 渲染。建议读 magic bytes 校验。

### N5 — `image_gen.rs:209-223` `index.json` 读写竞态（lost update）—— ✅ 已由 §3.1 #4 修复

`download_image_to_session` 读 `index.json` → push → 写回，非原子。两个并发请求可能 last-write-wins，丢失前一条记录。建议文件锁或 append-only 日志 + 周期 compact。

**状态**：已在 commit `bdf20f9` 由 §3.1 #4 修复（全局 `tokio::sync::Mutex` 序列化读-改-写；handler 直接用返回的 `ImageMeta`，不再 re-read `index.last()`）。无需入 issue。

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

`speakText` 用 `.replace(/\[.*?\]/g, '')` 清理 `[动作]` 标记，但 `.*?` 不跨行，多行动作描述会残留。且 `` `[*_#`]` `` 只清单字符，`**bold**` 会变成 `bold`（OK）但 `~~strike~~` 不处理。非阻塞，仅影响朗读体验。

### N15 — CI `explore` 与 `Portable Windows WebUI` failure —— ✅ `Portable Windows WebUI` 部分已由 §3.4 修复

`git diff --stat 30a33ae..HEAD -- .github/workflows/ tools/agent-exploration/` 为空，PR #317 不触碰这两个 CI 的输入。`explore` 失败通常是 `AGENT_EXPLORATION_OPENAI_KEY` secret 缺失（fallback 模式仍可能因 DOM 契约变化失败），仍建议合并后单独排查。

**`Portable Windows WebUI` 部分修订**：本节最初判断"非本 PR 引入"**有误**——该 CI 失败确由本 PR 重写 `18-group-chat.html` 引入（详见 §3.4）。已在 commit（§3.4）通过为 `local-webui-browser-smoke.mjs` 增加 18 屏专用布局校验修复，无需入 issue。`explore` 失败部分仍按原判断入 issue。

---

## 5. 结论

PR #317 的 3 个阻塞项（B1 XSS / B2 功能不可用 / B3 CI lint）**已由本审计就地修复**（commit `338f911`）。随后对 CodeRabbit 四轮 review + 一轮 CI 修复 + 一轮 CodeRabbit 复核共做了 21 条 actionable 线程的修复：
- §3.1（commit `bdf20f9`）：12 条 inline + 1 条 markdownlint
- §3.2（commit `d09d618`）：2 条 inline（N1/N2 ID 冲突 + `:` filename 安全漏）
- §3.3（commit `7f6e42d`）：2 条 outside-diff（showDetail stale race + INDEX_LOCK TOCTOU）+ 1 条 nitpick（测试覆盖）
- §3.4（commit `bce1260`）：1 条 CI blocker（`local-webui-browser-smoke.mjs` 对 18 屏检查不存在的 `#view`）
- §3.5（本 commit）：2 条（image_gen 流式读取 + smoke predicate 让 danger 立即返回）

六轮修复后本地验证通过：`cargo fmt --all -- --check` 干净；`cargo clippy --lib --tests --workspace --all-targets -- -D warnings` 干净；`cargo test --lib -p airp-core` = **900 passed / 2 ignored**（ignored 为 `image_gen::tests::download_image_to_session_writes_unique_files_under_collision` 需 mockito feature，与 `orchestrator::lorebook::tests::bench_aho_corasick_vs_naive` 基准测试）；`node --test tests/runtime-pages.test.mjs tests/api-client.test.mjs tests/operations.test.mjs tests/agent-harness.test.mjs`（webui）= **50 passed**；`node --test script-lint.test.mjs`（agent-exploration）= **19 passed**；`node --check ui/local-webui-browser-smoke.mjs` 语法通过。§4 中 N3、N5 已由 §3.1 修复，N15 的 `Portable Windows WebUI` 部分由 §3.4 修复，剩余 12 个非阻塞项（N1/N2/N4/N6–N14，及 N15 的 `explore` 部分）建议合并后入 issue。

**独立审计立场修订**：本审计否决了 PR 描述中"15 项 webui 测试全过 = 已验证"的暗示——B2 在所有测试通过的情况下仍然存在，证明 runtime-pages 测试**不覆盖运行时数据契约**。本审计亦承认前两轮漏读了 CodeRabbit review body 折叠区内的 outside-diff 线程——以后 review CodeRabbit 结论时必须读完整 review body 而非仅看 inline 评论。**§4 N15 早期"CI 失败非本 PR 引入"判断有误**——`Portable Windows WebUI` 失败正是本 PR 重写 18 屏后未同步 smoke 测试导致，已由 §3.4 修复。今后涉及"屏幕重写"的 PR 必须同时检查 `ui/local-webui-browser-smoke.mjs` 的导航 loop 是否还兼容新布局，不能只过 `runtime-pages.test.mjs`。**§3.5 #20 的修复进一步揭示**：§3.1 #6 当时只补了 post-read 复核，未触及读取方式本身——`response.bytes().await` 在 chunked transfer 下仍会先缓冲后检查。审计应回到 `download_image_to_session` 的读取语义本身，而非满足于"加了上限检查"。

**视觉审查声明**：按 issue #319 补充要求（2026-07-26 用户立），WebUI 改动 PR 必须由 KIMI K3+ 多模态 agent 执行视觉审查。**本审计 agent 为 GLM-5.2 纯文本模型，整轮审计未执行视觉审查**——B1/B2/B3 与 §3.1-§3.5 全部修复均基于 HTML 字符串/DOM 契约/源码语义判断，未对 18 屏重写后的实际渲染做截图审查。建议在合并前由多模态 agent 对 PR #317 涉及的视觉改动（特别是 18 屏重写、`chat-space.js` `shareAsCard` XSS 修复、character-templates.js 详情面板）独立补审。

**建议**：六次修复 commit 推送后，待人工 review、CodeRabbit 复审与 `Portable Windows WebUI` CI 复跑通过后可合并；合并后由审计 agent 将 §4 中剩余非阻塞项（N1/N2/N4/N6–N14，及 N15 的 `explore` 部分）整理为 GitHub issue。
