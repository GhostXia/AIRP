# AIRP Risk Register

> Last reviewed: 2026-07-11. Current documentation authority is in [DOC-AUDIT.md](DOC-AUDIT.md); the dated project audit remains historical evidence.

## RR-001 · `card_path` local arbitrary file read

- **Status**: Accepted for Tauri desktop local sidecar only.
- **Surface**: `/v1/characters/import` with `card_path`.
- **Risk**: If exposed to untrusted callers, `card_path` lets the engine process read arbitrary absolute paths visible to that process.
- **Current control**: Tauri UI obtains paths through the official file dialog, sends only the selected path, and the engine then validates the file as PNG/JSON before import.
- **Rule**: `card_path` is allowed only for trusted local UI. Future web clients, third-party widgets, or remote engines must use multipart/streaming upload, with base64 only as a last fallback. Do not expose server-side arbitrary path read to untrusted callers.
- **Future hardening**: Add engine-side caller capability checks before any multi-client/web exposure.

## RR-002 · Plaintext provider and access keys

- **Status**: Mitigated in PR #111; keys are runtime-only and omitted from persisted settings.
- **Surface**: `POST /v1/settings` persists `api_key` and `access_api_key` to `data/settings.json`.
- **Risk**: Local backups, support bundles, malware, shared accounts, or accidental commits can expose long-lived credentials.
- **Current control**: API responses are redacted and runtime files are ignored by git. This does not protect data at rest.
- **Required direction**: Store secrets in the OS credential store; keep only non-secret provider/model metadata in settings. Provide explicit migration and redacted diagnostics.

## RR-003 · Permissive local HTTP origin and optional authentication

- **Status**: Mitigated for the supported local topology in PR #111; configurable origins extend bundled defaults and authenticated desktop mode uses a process-scoped bearer.
- **Surface**: Engine routes are reachable on loopback; browser origins use an exact list and bearer authentication is topology-dependent.
- **Risk**: A malicious browser origin or mistakenly exposed port may invoke local data and generation APIs.
- **Current control**: Normal deployment binds loopback and rate-limits requests. Loopback is risk reduction, not caller authentication.
- **Required direction**: Desktop mode gets an ephemeral launch token and precise origin policy. Remote mode must be explicit opt-in with durable authentication and safe CORS defaults.

## RR-004 · Divergent write paths and non-atomic persistence

- **Status**: Partially mitigated in PR #111 through shared Chat/State/Lorebook services, atomic replacement, revision/schema validation, and shared locks. Cross-resource transactions and lock-cache lifecycle remain tracked separately.
- **Surface**: HTTP chat/state handlers and Agent tools directly access the same JSON/JSONL files but do not share all locks or transaction semantics.
- **Risk**: Concurrent append/rollback/regen/state updates can lose ordering, overwrite a newer snapshot, or make live state disagree with history.
- **Current control**: Agent tools have per-character/per-session locks; append-only paths reduce but do not remove the risk.
- **Required direction**: One versioned Chat/State service with shared locks, atomic replace, revisions/idempotency, schema validation, and concurrency tests.

## RR-005 · State schema is advisory at the write boundary

- **Status**: Mitigated in PR #111; StateService validates schema before atomic live/history updates.
- **Surface**: Model-emitted `<state>` JSON is written to `state/live.json` without enforcing `schema.json`.
- **Risk**: Invalid types, unknown fields, or out-of-range values silently become future prompt state.
- **Current control**: Schema improves prompt rendering only; malformed JSON is not persisted.
- **Required direction**: Define reject/clamp/preserve-unknown policy and validate before atomically updating live + history.

## RR-006 · Tauri sidecar process lifecycle

- **Status**: Mitigated in PR #111; desktop owns and terminates the sidecar. Packaged Windows smoke remains the release-level evidence gate.
- **Surface**: Tauri drops the spawned child handle after startup.
- **Risk**: Orphaned processes, duplicate engines, unrecoverable crashes, port conflicts, and uncertain application shutdown.
- **Current control**: Event output is logged and health is polled.
- **Required direction**: Managed child state, explicit shutdown/restart/backoff, port-conflict handling, and real packaged-app smoke evidence.

## RR-007 · Protocol and capability authority drift

- **Status**: Partially mitigated in PR #111 through wire discriminant fixtures and engine-side capability/allowlist/confirm enforcement. Broader widget/MCP/hook authority remains future work.
- **Surface**: Rust and TypeScript protocol types are maintained manually; UI consent is not enforced by engine authorization.
- **Risk**: A client can pass UI checks yet invoke an operation the engine never authoritatively authorized; wire changes can fail only at runtime.
- **Current control**: Both sides have unit tests and runtime guards.
- **Required direction**: Single schema/codegen or shared golden fixtures, plus engine-issued and engine-enforced capabilities.

## RR-008 · No automatic PR quality gate

- **Status**: Mitigated in PR #111; `pr-gate.yml` runs Rust and UI quality gates without persisted checkout credentials.
- **Surface**: The only GitHub workflow is manual packaging and omits engine/protocol full tests, fmt, and Clippy.
- **Risk**: A green manual artifact can include regressions or weakened invariants; current main already fails fmt and strict Clippy.
- **Current control**: Local tests and human review.
- **Required direction**: Pull-request workflow for workspace tests, UI tests/typecheck, fmt, then strict Clippy after the baseline is repaired.
