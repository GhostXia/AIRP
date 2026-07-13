# AIRP WebUI

Browser-based AIRP client for RP use. The current implementation is a basically usable development build; the active target is a production-ready, single-instance self-hosted WebUI. Current baseline: [docs/CURRENT-BASELINE.md](../docs/CURRENT-BASELINE.md); production gates: [docs/WEBUI-PRODUCTION-PLAN.md](../docs/WEBUI-PRODUCTION-PLAN.md); accepted P0 architecture and implementation status: [docs/WEBUI-PRODUCTION-ARCHITECTURE.md](../docs/WEBUI-PRODUCTION-ARCHITECTURE.md); completed MVP contract: [docs/WEBUI-MVP-PLAN.md](../docs/WEBUI-MVP-PLAN.md).

> `start.bat`, `serve.js`, `cargo run`, manually entered engine URLs and optional bearer tokens are development paths. The first-party deployment artifact lives in [deploy/production](../deploy/production/README.md) and has a production topology CI gate, but P1-P3 release gates remain open. Do not expose port 8000 or this static development server directly to the public Internet.

## Quick Start

### 方式 A：一键 .bat（Windows 推荐）

双击 `webui/start.bat` 即可。脚本会：

1. 按 `AGENTS.md` 设好 Rust/Node/MSYS2 工具链环境（产物落 D: 盘）
2. 清理可丢弃的 `target/webui-smoke-data`，在新窗口起零密钥 mock provider
3. 在新窗口起 engine：`cargo run -p airp-core -- daemon --port 8000`
4. 在新窗口起 WebUI 静态 server：`node webui/serve.js`（零依赖纯 node，无需 npx/python）
5. 自动打开浏览器到 http://127.0.0.1:9001

关闭三个弹出的窗口即停止对应服务。业务配置（`AIRP_ENDPOINT` / `AIRP_MODEL` / `AIRP_ACCESS_KEY` 等）在 .bat 顶部修改即可。

跨设备访问（手机浏览器连桌面 engine）：把 .bat 里 `WEBUI_HOST=127.0.0.1` 改成 `0.0.0.0`，再用桌面机 IP 访问。

### 方式 B：手动起（跨平台）

1. **Start the engine** (in one terminal):
   ```sh
   cargo run -p airp-core -- daemon --port 8000
   ```
   Set `AIRP_ACCESS_KEY=<key>` env var if you need bearer auth.

2. **Serve the WebUI** (in another terminal):
   ```sh
   node webui/serve.js
   ```
   或 `npx serve webui/` / `python -m http.server 9001 -d webui/`。打开 http://localhost:9001。

3. **Connect**: enter Engine URL (default http://127.0.0.1:8000) and optional Bearer token, click Connect.

4. **One-click diagnostics**: after connecting, click 「一键诊断」 to run a backend reachability sweep (`/version` → `/v1/settings` → `/v1/models` → `/v1/characters`); 「复制摘要」 copies the report for filing as verification evidence.

## V2 layout

`webui/` now hosts the runnable V2 console. It is a zero-build surface with two hash-routed views and one workbench overlay:

- **角色列表** — character selection and import.
- **对话空间** — sessions, streaming chat, Agent Run, and diagnostics.
- **工作台** — character-card, lorebook, and decompose tools in an overlay that preserves the current conversation context.

The corresponding files under `../airp-engine-console/` remain design sources only;
they are not the served WebUI implementation.

## Scope

The current implementation covers connection, provider settings, character import, persistent basic User Persona, Preset selection/JSON import, session create/select/delete, streaming chat/history, regen/rollback, Agent Run and diagnostics. Persona name/variables and the selected Preset are applied to chat requests. PR #123 closed the basic zero-secret acceptance gate; PR #124/#125 added durable IDs, cursor history, rollback-by-ID and a 50-message WebUI window. The engine-truth harness has 67 checks plus a real-browser 50/54 → 54/54 prepend/scroll pass. Production runtime config uses the authenticated same origin and hides development engine URL/bearer controls; the production topology gate adds real HTTPS/system-Chrome injection and stream-cancel checks. This is not yet a production release; RP management, data recovery and remaining release gates in [WEBUI-PRODUCTION-PLAN.md](../docs/WEBUI-PRODUCTION-PLAN.md) are still required.

New capabilities should normally be delivered vertically through engine shared services, HTTP/SSE contracts, this WebUI, and tests. WebUI-specific DOM or interaction choices must not become engine contracts. Client-neutral contracts remain reusable, while desktop UI implementation is paused.

Workspace choices (non-secret User ID, selected character/session and Preset) are restored from browser-local state. Engine URL and optional bearer remain tab-scoped in `sessionStorage`; provider secrets are never written to `localStorage`.

**Reachability & config (P0)**
-  `/version` (health check)
-  `/v1/settings` read (API key masked)
-  `/v1/settings` runtime provider update (secrets are never persisted) followed by a real `/v1/models` provider validation
-  `/v1/models` provider smoke + typed error display
-  `/v1/characters` list + avatar preview (`/v1/characters/:id/avatar` fetched as blob with bearer, rendered via object URL)
-  `/v1/sessions/:character_id` list + create — switching character/session clears the current chat view **and aborts any in-flight chat/agent stream** (防上一 session 残留消息串扰 / 防止 SSE chunk 回写新视图); 新建 session 后自动选中该 session（省手动点）
-  `/v1/characters/import` via `card_json` / `card_png_base64` only; **never `card_path`** (RR-001)

**Character state (M1)**
-  `/v1/characters/:id/state` — live.json view; 404 (角色尚无 state) 显式区分于空对象
-  `/v1/characters/:id/state/history?limit=N` — 最近 N 条 state 变更（默认 20，上限 1000）；404 显式提示

**Chat & agent loop**
-  `/v1/chat/completions` (SSE streaming, token-by-token render); 流式期间用 raw textContent（保 cursor 动画），完成后切 markdown 渲染
-  `/v1/chat/history`, `regen`, `rollback` — history 首屏按 50 条窗口加载，可按 durable cursor 加载更早；消息节点按 durable ID 增量复用，rollback 先点选消息再确认；切换 character/session 或初次连接后自动加载当前 session
-  `/v1/agent/run` (SSE agent event log) — events classified as `PLAN` / `TOOL_CALL` / `TOOL_RESULT` / `DELTA` / `DONE` with color-coded labels, one-line summary, and collapsible raw JSON per event; step counter shows `stop_reason · steps_taken · ms`
-  `/v1/agent/tools` — runtime tool catalog; allow and destructive-confirm selections are generated from engine metadata while manual comma-separated overrides remain available
-  Concurrent chat stream test (M2) — two parallel `/v1/chat/completions` to verify id-keyed chat state doesn't cross-talk

**Markdown rendering**
-  极简手写 renderer（零构建约束，不引第三方库）：fenced code blocks / inline code / h1-h3 / **bold** / *italic* / 段落换行
-  安全：先 escapeHtml 全转义，再用 private-use Unicode 占位符抽 code fence，最后应用其它转换；用户内容不会注入 HTML

**Diagnostics (P1)**
-  One-click backend sweep producing a copyable summary: engine URL, bearer status, version, endpoint/model/api_key presence, model count, character count, per-call status + latency.
-  **v1 scope**: covers 4 endpoints (`/version` → `/v1/settings` → `/v1/models` → `/v1/characters`) — backend reachability only. **chat/agent smoke deferred to P2/M2** reliability suite, to avoid consuming provider quota during a routine diagnostic; the 4-endpoint sweep already surfaces backend reachability failures (missing API key, no models, wrong endpoint). See `docs/WEBUI-BACKEND-PLAN.md §9 P1`.
-  Event log (right panel): request path, method, status code, latency, SSE chunk count, agent event labels.

## Not in scope (deferred)

-  Exhaustive error-path regression beyond the MVP browser smoke; the MVP still covers 401, provider error, SSE interruption and stale-response isolation.
-  Multipart upload endpoint (M3); this harness currently uses JSON/base64 fallback.
-  Product UI polish, multi-Persona management, Style Review, ChangeInbox, auth management UI, deployment, i18n, or plugin/runtime decisions.
-  Tauri desktop UI changes — WebUI never edits `ui/`.

## Verification Evidence

For the deterministic engine-truth layer, start the mock provider and engine, then run `node webui/smoke.mjs`. The script is deliberately not described as browser automation: it verifies persisted history, Persona/Preset/session IDs, three streaming turns, isolation, rollback/regen/delete and typed errors through HTTP/SSE. A separate real-browser pass verifies WebUI connection, recovery, interaction and rendering.

Each validation session should record:
- engine start command + URL
- provider/model name (API key masked)
- request path and payload summary
- status code and latency per call
- SSE event sequence for streaming calls (use the agent event log labels)
- data directory touched (under `data/`)
- failure screenshots or saved logs

The one-click diagnostics summary is designed to be pasted directly into `docs/WEBUI-BACKEND-VALIDATION.md`.
