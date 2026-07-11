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
- **Surface**: `POST /v1/settings` accepts runtime-only `api_key` and `access_api_key`; only non-secret provider/model metadata is persisted to `data/settings.json`.
- **Risk**: Process memory, environment variables, logs, support bundles, or a compromised local account can still expose runtime credentials.
- **Current control**: API responses are redacted, serialization omits secrets, and legacy plaintext fields are ignored on load.
- **Required direction**: Use the OS/service secret facility for durable non-interactive credentials and keep diagnostics redacted.

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

## RR-005 · State schema enforcement at the write boundary

- **Status**: Mitigated in PR #111; StateService validates schema before atomic live/history updates.
- **Surface**: Model-emitted `<state>` JSON is routed through StateService before updating `state/live.json` and history.
- **Risk**: Future adapters that bypass StateService could reintroduce divergent validation or non-atomic writes.
- **Current control**: Required/type/range/additionalProperties validation runs before revisioned atomic live/history updates.
- **Required direction**: Keep every new state adapter on StateService and extend the schema subset only with tests.

## RR-006 · Tauri sidecar process lifecycle

- **Status**: Mitigated in PR #111; desktop owns and terminates the sidecar. Packaged Windows smoke remains the release-level evidence gate.
- **Surface**: Tauri owns the spawned child handle, polls readiness, and terminates the sidecar during application shutdown.
- **Risk**: Packaged-runtime crashes, port conflicts, restart/backoff, and installer-specific shutdown behavior still require artifact evidence.
- **Current control**: Managed child state, logged output, readiness polling, and explicit shutdown.
- **Required direction**: Preserve lifecycle tests and require the packaged Windows smoke before release.

## RR-007 · Protocol and capability authority drift

- **Status**: Partially mitigated in PR #111 through wire discriminant fixtures and engine-side capability/allowlist/confirm enforcement. Broader widget/MCP/hook authority remains future work.
- **Surface**: Rust and TypeScript protocol types are maintained manually; UI consent is not enforced by engine authorization.
- **Risk**: A client can pass UI checks yet invoke an operation the engine never authoritatively authorized; wire changes can fail only at runtime.
- **Current control**: Both sides have unit tests and runtime guards.
- **Required direction**: Single schema/codegen or shared golden fixtures, plus engine-issued and engine-enforced capabilities.

## RR-008 · Automatic PR quality gate

- **Status**: Mitigated in PR #111; `pr-gate.yml` runs Rust and UI quality gates without persisted checkout credentials.
- **Surface**: `pr-gate.yml` runs formatting, strict Clippy, workspace tests, sacred prompt-boundary invariants, UI tests/typecheck, and WebUI syntax checks.
- **Risk**: Packaged installer/runtime behavior and provider-backed remote smoke are intentionally outside routine PR CI.
- **Current control**: Required PR checks plus local/human review; checkout credentials are not persisted.
- **Required direction**: Keep release artifact smoke as a separate release gate and expand CI only when deterministic fixtures exist.
