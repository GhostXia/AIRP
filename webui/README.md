# AIRP Engine Console — WebUI Backend Validation Harness

Temporary browser-based harness to validate engine backend reliability.
**Not a product UI.** See [docs/WEBUI-BACKEND-PLAN.md](../docs/WEBUI-BACKEND-PLAN.md).

## Quick Start

### 方式 A：一键 .bat（Windows 推荐）

双击 `webui/start.bat` 即可。脚本会：

1. 按 `AGENTS.md` 设好 Rust/Node/MSYS2 工具链环境（产物落 D: 盘）
2. 在新窗口起 engine：`cargo run -p airp-core -- daemon --port 8000`
3. 在新窗口起 WebUI 静态 server：`node webui/serve.js`（零依赖纯 node，无需 npx/python）
4. 自动打开浏览器到 http://127.0.0.1:9001

关闭两个弹出的窗口即停止对应服务。业务配置（`AIRP_ENDPOINT` / `AIRP_MODEL` / `AIRP_ACCESS_KEY` 等）在 .bat 顶部注释里取消注释即可。

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

## Scope

**Reachability & config (P0)**
-  `/version` (health check)
-  `/v1/settings` read (API key masked)
-  `/v1/models` provider smoke + typed error display
-  `/v1/characters` list + avatar preview (`/v1/characters/:id/avatar` fetched as blob with bearer, rendered via object URL)
-  `/v1/sessions/:character_id` list + create — switching character/session clears the current chat view **and aborts any in-flight chat/agent stream** (防上一 session 残留消息串扰 / 防止 SSE chunk 回写新视图); 新建 session 后自动选中该 session（省手动点）
-  `/v1/characters/import` via `card_json` / `card_png_base64` only; **never `card_path`** (RR-001)

**Character state (M1)**
-  `/v1/characters/:id/state` — live.json view; 404 (角色尚无 state) 显式区分于空对象
-  `/v1/characters/:id/state/history?limit=N` — 最近 N 条 state 变更（默认 20，上限 1000）；404 显式提示

**Chat & agent loop**
-  `/v1/chat/completions` (SSE streaming, token-by-token render); 流式期间用 raw textContent（保 cursor 动画），完成后切 markdown 渲染
-  `/v1/chat/history`, `regen`, `rollback` — destructive ops (regen/rollback) require explicit confirm dialog; 切换 character/session 或初次连接后自动 load history（无需手点 History）
-  `/v1/agent/run` (SSE agent event log) — events classified as `PLAN` / `TOOL_CALL` / `TOOL_RESULT` / `DELTA` / `DONE` with color-coded labels, one-line summary, and collapsible raw JSON per event; step counter shows `stop_reason · steps_taken · ms`
-  Concurrent chat stream test (M2) — two parallel `/v1/chat/completions` to verify id-keyed chat state doesn't cross-talk

**Markdown rendering**
-  极简手写 renderer（零构建约束，不引第三方库）：fenced code blocks / inline code / h1-h3 / **bold** / *italic* / 段落换行
-  安全：先 escapeHtml 全转义，再用 private-use Unicode 占位符抽 code fence，最后应用其它转换；用户内容不会注入 HTML

**Diagnostics (P1)**
-  One-click backend sweep producing a copyable summary: engine URL, bearer status, version, endpoint/model/api_key presence, model count, character count, per-call status + latency.
-  **v1 scope**: covers 4 endpoints (`/version` → `/v1/settings` → `/v1/models` → `/v1/characters`) — backend reachability only. **chat/agent smoke deferred to P2/M2** reliability suite, to avoid consuming provider quota during a routine diagnostic; the 4-endpoint sweep already surfaces backend reachability failures (missing API key, no models, wrong endpoint). See `docs/WEBUI-BACKEND-PLAN.md §9 P1`.
-  Event log (right panel): request path, method, status code, latency, SSE chunk count, agent event labels.

## Not in scope (deferred)

-  Error path coverage as automated regression suite (M2) — currently manual via diagnostics + event log.
-  Multipart upload endpoint (M3); this harness currently uses JSON/base64 fallback.
-  Product UI polish, auth management UI, deployment, i18n, or plugin/runtime decisions.
-  Tauri desktop UI changes — WebUI never edits `ui/`.

## Verification Evidence

Each validation session should record:
- engine start command + URL
- provider/model name (API key masked)
- request path and payload summary
- status code and latency per call
- SSE event sequence for streaming calls (use the agent event log labels)
- data directory touched (under `data/`)
- failure screenshots or saved logs

The one-click diagnostics summary is designed to be pasted directly into `docs/WEBUI-BACKEND-VALIDATION.md`.
