# AIRP Risk Register

> Last reviewed: 2026-07-18 at `main@63f1c5b`. Current implementation authority is [CURRENT-BASELINE.md](CURRENT-BASELINE.md); [README.md](README.md) defines documentation authority, and compressed archives remain historical evidence.

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

- **Status**: Partially mitigated in PR #111 through shared Chat/State/Lorebook services, atomic replacement, revision/schema validation, and shared locks. PR #219 further hardened single-resource durability (chat_store `replace_file` + `sync_data`, replace_file parent-dir fsync, character_lock serialization, quota Mutex, extract_card_assets lorebook preservation, next_volume_number saturating, append_to_current warning instead of silent swallow). Cross-resource transactions, `AIRP-TREE-SHA256-v1` integrity, full migration registry, backup/restore, lock-cache lifecycle and character_lock poison recovery remain tracked separately (see #220 deferred items).
- **Surface**: Chat/State/Lorebook use shared services with crash-atomic writes, but cross-resource operations and future session revisions still lack one transaction boundary.
- **Risk**: Concurrent append/rollback/regen/state updates can lose ordering, overwrite a newer snapshot, or make live state disagree with history; a crash mid-write no longer leaves a 0-byte `chat.jsonl`, but cross-resource consistency is still best-effort.
- **Current control**: Shared per-character/per-session locks, atomic replacement with parent-dir fsync, revision/schema validation, append-only history with `sync_data`, `character_lock` for `update_character_card`, `quota` Mutex, and `extract_card_assets` data-safety-first lorebook preservation cover current single-resource writes; cross-resource consistency remains incomplete.
- **Required direction**: One versioned Chat/State service with shared locks, atomic replace, revisions/idempotency, schema validation, and concurrency tests; then `AIRP-TREE-SHA256-v1` integrity, versioned migration registry, backup/restore and `character_lock` poison recovery (Phase P2).

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

- **Status**: Mitigated in PR #111; `pr-gate.yml` runs Rust and UI quality gates without persisted checkout credentials. PR #218 upgraded GitHub Actions to `actions/checkout@v7` / `setup-node@v6` / `upload-artifact@v7` on Node 24 runtime, and delivered `tools/dep-governance/` (dependency discovery + audit routing + SPDX-2.3 / CycloneDX 1.5 SBOM generation) as an offline manual toolchain; SBOM snapshot lives in `docs/sbom/`.
- **Surface**: `pr-gate.yml` runs formatting, strict Clippy, workspace tests, sacred prompt-boundary invariants, UI tests/typecheck, WebUI syntax checks and production topology smoke.
- **Risk**: Packaged installer/runtime behavior, provider-backed remote smoke, automatic upstream version detection, and SBOM-as-release-gate are intentionally outside routine PR CI; dep-governance tooling is manually run, not enforced.
- **Current control**: Required PR checks plus local/human review; checkout credentials are not persisted; dep-governance routing policy documents auto-pass / audit-required / block classes for known licenses.
- **Required direction**: Keep release artifact smoke as a separate release gate; wire dep-governance SBOM into release pipeline as a mandatory gate; add automatic upstream-version detection with de-duplicated issues (#192 follow-up slice); expand CI only when deterministic fixtures exist.

## RR-009 · Production gateway/engine authority confusion

- **Status**: P0 gateway/engine controls and production topology smoke implemented under #130; P1-P3 product and release gates remain open.
- **Surface**: A same-origin WebUI gateway authenticates a browser and calls the private engine with `AIRP_ACCESS_KEY`.
- **Risk**: Forwarding the browser's `Authorization` header, exposing the engine bearer to JavaScript, allowing runtime bearer replacement, or publishing the engine port could bypass or desynchronize the intended two-layer boundary.
- **Current control**: The gateway authenticates the whole site, replaces (never appends) `Authorization` for explicit engine routes, and holds the engine bearer server-side. The engine has no published port; production mode requires a strong key and exact HTTPS origin, rejects local-path import mode, and makes bearer rotation an operator restart action.
- **Current evidence**: PR #136 production smoke proves anonymous/wrong credentials fail, direct host access to the engine fails, a caller-supplied bearer cannot pass through, short/missing keys prevent listen, `card_path` is rejected, and logs/assets contain no credentials. See [WEBUI-PRODUCTION-ARCHITECTURE.md](WEBUI-PRODUCTION-ARCHITECTURE.md).

## RR-010 · Vulnerable frontend development toolchain

- **Status**: Mitigated by PR #191; #137 closed. Continue routine dependency discovery under #192.
- **Surface**: Locked Vite/Vitest/esbuild development and test dependencies under `ui/`.
- **Risk**: Known audit findings affect development-server or test-UI exposure to untrusted networks. These packages are not copied into the production gateway image, so this is not evidence of a production runtime compromise.
- **Current control**: Vite 8.1.4, Vitest 4.1.10 and `@vitejs/plugin-vue` 6.0.8 use bounded manifest ranges and an exact npm lockfile; Node support starts at 20.19.0, `npm audit` reports zero findings, and development services remain loopback-only/trusted. Production serves independent static assets without `ui/node_modules`.
- **Current evidence**: PR #191 passed UI typecheck, 98 Vitest cases, WebUI tests, Tauri config discovery and the real production HTTPS/Chrome topology gate. Future version discovery and audit routing are tracked by #192.

## RR-011 · Session snapshot and revision completeness

- **Status**: Open; PR #169 delivered identity/layout cleanup and accepted the phased contract, not the full runtime.
- **Surface**: Named sessions currently isolate history and memory, while state, character-card/worldbook working copies, unified revisions, integrity loading and complete export remain incomplete.
- **Risk**: A user may assume a session is a self-contained reproducible save, but later edits or external material changes can make an old turn impossible to reconstruct.
- **Current control**: Canonical UUID identity, durable history, metadata repair, stopped legacy directory creation, and an explicit target contract in [SESSION-DATA-DESIGN.md](SESSION-DATA-DESIGN.md).
- **Required direction**: Implement the contract in phases with atomic publication, approved file sets, cross-platform tree hashes, per-message `content_revision`, crash recovery and restore/export tests. Until then, UI and docs must not call current sessions fully self-contained or reproducible.

## RR-012 · Prompt diagnostics leak private configuration or mutate chat state

- **Status**: Mitigated for the PR #194 single-user preview boundary; broader multi-user authorization remains outside the current product topology.
- **Surface**: `POST /v1/chat/preview` and the WebUI prompt-assembly summary.
- **Risk**: A diagnostic endpoint could accidentally return prompt text, provider secrets/endpoints, or advance history/session state; even a redacted response can disclose selected asset IDs, provider/model names and prompt composition metadata.
- **Current control**: The endpoint uses the real assembly path but returns bounded metadata only, omits prompt content/API keys/endpoints, uses the common `/v1/*` bearer middleware (mandatory in production), and has regression tests proving it does not create sessions, append history, advance memory/state, or repair metadata. Production browser smoke covers the same-origin UI path.
- **Required direction**: Keep response fields allowlisted and regression-tested; redact this metadata from logs/support bundles, and add per-user authorization before any multi-user topology.
