# AIRP Engine Console — WebUI Backend Validation Harness (M1)

Temporary browser-based harness to validate engine backend reliability.
**Not a product UI.** See [docs/WEBUI-BACKEND-PLAN.md](../docs/WEBUI-BACKEND-PLAN.md).

## Quick Start

1. **Start the engine** (in one terminal):
   ```
   cargo run -p airp-core -- daemon --port 8000
   ```
   Set `AIRP_ACCESS_KEY=<key>` env var if you need bearer auth.

2. **Serve the WebUI** (in another terminal):
   ```
   npx serve webui/
   ```
   Or `python -m http.server 9001 -d webui/`. Open http://localhost:9001.

3. **Connect**: enter Engine URL (default http://127.0.0.1:8000) and optional Bearer token, click Connect.

## Scope (M1)

-  /version (health check)
-  /v1/settings read (API key masked)
-  /v1/characters list
-  /v1/sessions/:character_id list + create
-  /v1/chat/completions (SSE streaming, token-by-token render)
-  /v1/chat/history, regen, rollback
-  /v1/agent/run (simple input/output)
-  Event log: request path, method, status code, latency, SSE chunk count

## Not in scope (deferred)

- Card import / multipart upload (M3)
- Concurrent stream test (M2)
- Error path coverage (M2)

## Verification Evidence

Each validation session should record:
- engine start command + URL
- provider/model name (API key masked)
- request path and payload summary
- status code and latency per call
- SSE event sequence for streaming calls
- data directory touched (under `data/`)
