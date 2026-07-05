# AIRP Engine Console — WebUI Backend Validation Harness

Temporary browser-based harness to validate engine backend reliability.
**Not a product UI.** See [docs/WEBUI-BACKEND-PLAN.md](../docs/WEBUI-BACKEND-PLAN.md).

## Quick Start

1. **Start the engine** (in one terminal):
   ```sh
   cargo run -p airp-core -- daemon --port 8000
   ```
   Set `AIRP_ACCESS_KEY=<key>` env var if you need bearer auth.

2. **Serve the WebUI** (in another terminal):
   ```sh
   npx serve webui/
   ```
   Or `python -m http.server 9001 -d webui/`. Open http://localhost:9001.

3. **Connect**: enter Engine URL (default http://127.0.0.1:8000) and optional Bearer token, click Connect.

4. **One-click diagnostics**: after connecting, click 「一键诊断」 to run a backend reachability sweep (`/version` → `/v1/settings` → `/v1/models` → `/v1/characters`); 「复制摘要」 copies the report for filing as verification evidence.

## Scope

**Reachability & config (P0)**
-  `/version` (health check)
-  `/v1/settings` read (API key masked)
-  `/v1/models` provider smoke + typed error display
-  `/v1/characters` list
-  `/v1/sessions/:character_id` list + create
-  `/v1/characters/import` via `card_json` / `card_png_base64` only; **never `card_path`** (RR-001)

**Chat & agent loop**
-  `/v1/chat/completions` (SSE streaming, token-by-token render)
-  `/v1/chat/history`, `regen`, `rollback` — destructive ops (regen/rollback) require explicit confirm dialog
-  `/v1/agent/run` (SSE agent event log) — events classified as `PLAN` / `TOOL_CALL` / `TOOL_RESULT` / `DELTA` / `DONE` with color-coded labels, one-line summary, and collapsible raw JSON per event; step counter shows `stop_reason · steps_taken · ms`
-  Concurrent chat stream test (M2) — two parallel `/v1/chat/completions` to verify id-keyed chat state doesn't cross-talk

**Diagnostics (P1)**
-  One-click backend sweep producing a copyable summary: engine URL, bearer status, version, endpoint/model/api_key presence, model count, character count, per-call status + latency.
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
