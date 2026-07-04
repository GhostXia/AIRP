# AIRP Engine Console — WebUI Backend Validation Harness (M1)

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

## Scope (M1)

-  /version (health check)
-  /v1/settings read (API key masked)
-  /v1/models provider smoke + typed error display
-  /v1/characters list
-  /v1/sessions/:character_id list + create
-  /v1/chat/completions (SSE streaming, token-by-token render)
-  /v1/chat/history, regen, rollback
-  /v1/agent/run (SSE agent event log)
-  /v1/characters/import via card_json/card_png_base64 only; never card_path
-  Concurrent chat stream test (M2)
-  Event log: request path, method, status code, latency, SSE chunk count

## Not in scope (deferred)

- Error path coverage (M2)
- Multipart upload endpoint (M3); this harness currently uses JSON/base64 fallback.
- Product UI polish, auth management, deployment, or plugin/runtime decisions.

## Verification Evidence

Each validation session should record:
- engine start command + URL
- provider/model name (API key masked)
- request path and payload summary
- status code and latency per call
- SSE event sequence for streaming calls
- data directory touched (under `data/`)
