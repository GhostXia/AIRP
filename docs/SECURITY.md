# Security and deployment boundary

> Baseline reviewed: 2026-08-24 (#564 PR 7 candidate). Plugin HTTPS webhook DNS is fail-closed with request-time re-resolve/pin (#381 E-P0-3 / #329 N3); plugins remain trusted-user extensions, not a code sandbox (RR-014). The widget extension line adds digest-pinned install, engine-authoritative capability grants and opaque-origin iframe isolation to both WebUI and Vue hosts. Tauri still defaults to WebUI `/`; only the explicit Blueprint switch selects the same-origin, read-only `/desktop/` Surface entry. Formal `v0.0.5` remains blocked by #130 real provider/browser/Compose acceptance and the missing required-reviewer policy on the `release` environment.

AIRP defaults to a single-user local topology. The current priority artifact is a portable Windows WebUI package; the Tauri desktop line now hosts the same-origin WebUI through the engine (C-P0) rather than its former Vue surface.

## Credentials

- `AIRP_API_KEY` supplies the upstream provider credential.
- `AIRP_ACCESS_KEY` enables bearer authentication for `/v1/*`.
- Access keys remain runtime-only. `config.json` and `data/settings.json` never serialize provider/access keys, and legacy plaintext fields are ignored when loading. The portable Windows launcher explicitly enables one local-user exception: the provider key is saved in `data/secrets.json`, while API responses, UI, logs and diagnostics remain redacted.
- Multi-provider credentials are separated from routing metadata: `data/providers.json` contains no keys, while `data/provider_keys.json` stores the provider-name/key map. Plugin webhook headers follow the same split between `data/plugin_tools.json` and `data/plugin_tool_headers.json`. GET responses expose only `api_key_set` / `headers_set`, never the values. These files are plaintext local secrets, not encryption; keep the entire data root private and exclude them from sharing and support bundles.
- In development, `POST /v1/settings` may replace a key for the current process, but its persisted settings omit secrets. In production, the engine bearer is immutable through this endpoint and must be rotated with the gateway secret followed by restart.
- WebUI diagnostics recursively redact credential fields, quoted JSON credentials, URL userinfo and secret query parameters. HTTP/SSE clients receive stable public error messages; upstream bodies, internal persistence details and server paths such as `PathEscape` values remain server-side only.

Use the operating system/service secret facility for non-interactive deployment. Do not put keys in repository files, installer arguments, logs, or copied diagnostics.

## Browser origins and network exposure

Development CORS origins are the bundled WebUI (`127.0.0.1:9001` and `localhost:9001`) plus Tauri origins. `AIRP_CORS_ORIGINS` extends this development allowlist. Production ignores those conveniences and allows only the canonical HTTPS `AIRP_PUBLIC_ORIGIN`. Wildcard origins are not supported.

Loopback plus CORS is not authentication. Before exposing the daemon through a reverse proxy or non-loopback bind, set `AIRP_ACCESS_KEY`, terminate TLS at the proxy, restrict trusted origins, and apply network-level access control.

## Portable Windows WebUI boundary

The portable launcher binds `airp-core.exe` to `127.0.0.1:8765` and serves WebUI/API from one origin. It fixes mutable state to the extracted package (`data/` plus root `config.json`), explicitly disables `AIRP_ALLOW_LOCAL_PATH`, and clears inherited deployment/access/public-origin/CORS variables. The browser therefore imports card content rather than asking the engine to read arbitrary host paths. Static responses use a same-origin CSP, `nosniff`, frame denial, no referrer, and `no-store`; SSE uses `no-cache`.

The launcher enables one versioned plaintext `data/secrets.json`, following the transparent local-user product tradeoff publicly documented by SillyTavern. AIRP does not expose the stored value back through settings APIs or UI. This is convenience, not encryption: any process or user that can read the package can recover the key. The folder must remain private, and `secrets.json` must be excluded from source control, shared archives, logs, diagnostics and support bundles.

This is a local single-user boundary, not authentication against other processes running as the same Windows user. Do not expose port 8765, run the package from a shared/synchronized directory, or grant untrusted users write access to the package. Back up and carry forward `data/` before replacing the extracted directory.

## WebUI production profile (deployment artifact and topology smoke implemented)

The first supported WebUI deployment is specified by [WEBUI-PRODUCTION-ARCHITECTURE.md](WEBUI-PRODUCTION-ARCHITECTURE.md): a versioned OCI/Compose bundle with Caddy as the only public HTTPS entry point and `airp-core` on a private network.

- Caddy authenticates the user at the perimeter and replaces the incoming `Authorization` header with the server-held engine bearer for `/v1/*`, `/health` and `/version`.
- The browser never receives `AIRP_ACCESS_KEY`, provider credentials or the engine address. Static files are behind the same perimeter authentication.
- `AIRP_DEPLOYMENT_MODE=production` validates its environment-only policy before loading or creating persisted config, and fails before listen unless `AIRP_ACCESS_KEY` is exactly 32 bytes encoded as canonical unpadded base64url, `AIRP_PUBLIC_ORIGIN` is one canonical HTTPS origin, and `AIRP_DATA_DIR` is absolute, existing, writable and not a filesystem root. It rejects `AIRP_ALLOW_LOCAL_PATH` and runtime engine-bearer replacement.
- Production WebUI imports upload JSON/PNG content only. `card_path`, host/UNC paths, file URLs and arbitrary remote fetches are outside this trust boundary even for authenticated callers.
- The private engine keeps its own bearer, validation, body limits, path guards and outbound redirect policy. Gateway controls do not replace engine controls.
- `AIRP_BIND_ADDRESS` controls only the gateway host bind. The P1 rollback procedure binds it to loopback while operators perform read-only health, asset, session and history checks, then restores the public bind only after verification succeeds. This is a manual traffic barrier, not a multi-user authorization system.

The engine fail-closed slice and `deploy/production/` OCI/Compose + Caddy artifact are implemented. The bundle pins base images by digest, mounts runtime secrets from gitignored files, publishes only Caddy, uses a private engine network, and makes the production WebUI same-origin without browser-visible engine credentials. The `Production topology` CI gate exercises real internal TLS, negative perimeter authentication, private-engine reachability, CSP/headers/body limits, content-only import, incremental SSE, restart persistence, system-Chrome injection/cancellation and runtime-secret scans. P1-P3 release gates remain open; never expose `webui/serve.js` or port 8000 as a remote deployment.

`POST /v1/chat/preview` uses the same `/v1/*` bearer middleware (mandatory in production) and returns a bounded assembly summary rather than prompt text. It omits API keys and provider endpoints and performs no timeline/session write or metadata repair. The response still exposes non-secret configuration metadata such as selected IDs, provider/model names and ordered material kinds; treat it as user-private diagnostic data and do not place it in public logs or support bundles without redaction.

PR #191 upgraded the `ui/` toolchain to Vite 8.1.4, Vitest 4.1.10 and `@vitejs/plugin-vue` 6.0.8; its locked dependency tree reports zero `npm audit` findings and passed UI/WebUI plus production-browser gates. These remain development dependencies and are not copied into production runtime images. Development servers and test UIs must still remain loopback-only or otherwise restricted to trusted networks.

PR #218 delivered `tools/dep-governance/` as an offline supply-chain governance toolchain: dependency discovery across Cargo workspace and npm package-lock.json v3, audit routing (auto-pass / audit-required / block + five upgrade routes), and SPDX-2.3 / CycloneDX 1.5 SBOM plus human-readable third-party notices generated into `docs/sbom/`. #527/#554 wired an explicit `workflow_dispatch` exact-tag validation/publish code gate: `publish_release=true` checks the draft/prerelease tag, exact checkout/ref/`HEAD`, package/browser/desktop smoke, and the release-only `contents: write` context remains isolated while ordinary packaging stays read-only. The workflow does not generate or upload dependency inventory, SBOM, notices, or sign-off files; the public release path uploads only `airp-webui-windows-x64.zip`. Development users can inspect the committed dependency/audit snapshot directly from the tagged git tree under `docs/sbom/`, and the existing `v0.0.5-rc.2` assets are not changed by this update. There is no `release: published` pre-publish gate. Hosted environment configuration and a successful publish proof are not present as required-reviewer evidence: the hosted `release` environment API currently returns `protection_rules=[]` and `can_admins_bypass=true`, so no required reviewer is configured. The toolchain still does not replace per-dependency license/provenance verification at introduction time (see [DEV-GUIDE.md §7.1](DEV-GUIDE.md)).

PR #219 hardened single-resource persistence boundaries: `chat_store::append_message` and `replace_file` now use tmp + `sync_all` + rename + parent-dir `sync_dir` for crash-atomic writes; `quota::check_and_increment` / `record_tokens` are serialized by a process-wide `Mutex` to prevent TOCTOU under concurrent requests; `update_character_card` acquires `character_lock(cid).write()` before the existence check; `extract_card_assets` preserves the existing lorebook when the new card's `character_book` is missing/empty or normalization fails, deleting only on explicit absence. These mitigate data-loss and race conditions on the local single-user boundary; cross-resource transactions, full migration registry, backup/restore and `AIRP-TREE-SHA256-v1` integrity verification remain open (Phase P2).

PR #232 hardened the remaining P1 failure boundary: user messages are persisted before timeline advancement; assistant live state, ChatLog and `current.md` writes fail the turn instead of returning a false success; SSE errors expose retryability and commit state so ambiguous/partial commits are not blindly resent; Persona deletion validates identifiers before destructive path construction and preserves the working copy on cleanup errors. The documented cold backup and rollback escape hatch verifies the archive SHA-256 and target volume before startup, but automated backup/restore, migration and cross-resource atomicity remain Phase P2 work.

## Widgets and Agent tools

UI consent is a user-experience gate, not the authority. Agent tools are disabled unless daemon bearer authentication is enabled. The bundled sidecar generates a process-scoped random bearer and shares it only with the trusted BusRelay. After authentication, a tool must still be registered, the trusted host must grant `call:tool`, and an optional per-run allowlist must contain it. Destructive tools remain dry-run unless their exact name appears in `confirm_tools`.

Third-party widgets must never receive the daemon bearer key directly. The trusted host should translate a user grant into the smallest capability/allowlist request needed for one operation.

`GET /v1/agent/tools` exposes names, descriptions, and side-effect classes only; it grants no capability. `export_context_bundle` writes beneath the engine data root, validates identifiers, and applies the same model-facing size limit as lorebook reads. `update_lorebook` and `seal_volume` are destructive and therefore require exact-name confirmation.

## Widget extension boundary (install, sandbox, capability grants, token rotation)

The widget extension line (C-P0~C-P4, originally delivered on `main@e28ea02` and present at `main@affa315`) adds a first install/authorization surface for third-party browser code. Its security boundaries, as implemented in code:

**Desktop shell bearer channel.** The Tauri shell holds the access key, exchanges it via `POST /v1/desktop-session` for a short-lived UI token, and navigates the first screen with the token in the URL fragment (`#airp-token=`), which does not reach server logs or Referer; the WebUI moves it into `sessionStorage.airp_bearer` and clears the fragment. The shell renewal loop deliberately uses exchange (additive) rather than rotation so the WebUI's in-flight token is not revoked mid-flight; the tradeoff is that superseded tokens stay valid for their own TTL. The renewal loop has no GUI real-device verification yet (see RISK-REGISTER).

**Widget sandbox.** Third-party esm widgets run inside an opaque-origin iframe with exactly `sandbox="allow-scripts"` and no `allow-same-origin`: they cannot read host DOM, storage or cookies. Production Vue catalog entries are not imported into the host process. Bridge messages are accepted only for the exact iframe window, random per-mount bridge session and stable Widget instance id; the frame additionally checks its parent window and origin, and teardown removes the listener/frame. The iframe receives only its instance, projected state and Engine-granted capabilities; the bearer is never passed. First-party builtin widgets do not use the iframe. This is browser isolation, not a full code sandbox: there is no CPU, network or resource isolation, and public same-origin digest assets remain fetchable without bearer authentication.

**Digest-pinned install.** `POST /v1/extensions/install` verifies each file's SHA-256 against the declared payload and rejects the whole package on mismatch; the package-level digest becomes the content-addressed directory (`data_root/extensions/<digest>/`). Install forces `entry.source` to the same-origin `/extensions/<digest>/index.js` and `entry.sandbox=true`, so no cross-origin module load path exists at the registry surface; slot must be within the built-in closed set. Static package serving (`GET /extensions/:digest/*file`) intentionally sits outside bearer auth because content is immutable and content-addressed, and opaque-origin sandbox frames need `Access-Control-Allow-Origin: *` for module imports; every serve re-checks the file digest and refuses with 500 on mismatch, and unregistered digests always 404. Residual: any local process that can write the data root can replace package files between install and serve, which the serve-time digest recheck turns into a denial rather than silent substitution.

**Capability grants.** Capability authority lives in the engine, not the UI. Grants are a closed set of six capabilities (`read/write:memory`, `read:worldbook`, `read/write:state`, `call:tool`), subset grants must be declared in the installed manifest, and reinstalling a type clears all grants (consent never carries across package identity). `POST /v1/widget-intents` enforces per call: the widget type must map to an enabled extension and the requested capability must be granted, otherwise 403 `intent_denied`; grant/revoke/deny decisions are audit-logged. The WebUI consent UI is a user-experience gate only. Residual: the intent face has no executor yet (authorization accepted is not execution), and the closed set predates any MCP/plugin grant subjects.

**Token rotation.** `POST /v1/desktop-session/renew` rotates: it revokes the presented token and issues a new one immediately; stale tokens get 401 and the full access key cannot be renewed (full-authority credentials never participate in rotation).

## Surface Protocol v2 declarative boundary

Surface v2 is a data contract, not an executable UI payload. [`protocol/surface-protocol-v2.json`](../protocol/surface-protocol-v2.json) is the payload authority and [`protocol/surface-sse-events.json`](../protocol/surface-sse-events.json) is the SSE transport authority. Rust and TypeScript guards scan the complete snapshot or patch, reject executable-looking fields at any depth, enforce byte/count/depth/revision limits, and reject unknown majors. Patch application is clone-then-validate: immutable metadata and root mutation are blocked, and any failed operation preserves the last-known-good snapshot and requests resynchronization. Unknown additive fields are opaque and must never reach DOM injection, module loading, template compilation, or evaluation.

The Engine session Surface endpoints are read-only and remain behind bearer or desktop-session authentication; they additionally fail closed when daemon access-key authentication is disabled. Internal scope keys include the effective data root, character, and session, preventing replay/snapshot aliasing across users with equal domain IDs. Missing Surface reads do not create user/session assets. The bearer still has daemon-wide authority: `user_id` selects an existing effective root and is not a tenant-bound identity claim, so this is isolation against accidental data mixing rather than multi-tenant authorization. Cursors are opaque, boot-scoped, scope-bound, and held in a bounded in-memory ring; foreign, expired, future, or previous-boot cursors resynchronize from a fresh domain projection. Projection disk reads run on the blocking pool and never occur while holding the Surface registry mutex.

Activity persistence is a separate closed-schema control-plane receipt store. It retains at most 32 stable failure records and excludes arbitrary messages, prompts, RP text, tool parameters/output, provider endpoints, and credentials. Malformed, oversized, or unknown-schema receipts are not overwritten and degrade only the Activity projection. Neither Activity nor Surface state is imported by prompt assembly. The `/desktop/` entry consumes the read-only Surface endpoints through `HttpEngineBus` with bearer authentication, bounded parsing, cursor replay, and snapshot resync. The Bus has a separate intent transport seam, but the PR 6 production host suppresses Widget intents and no real executor is delivered. A future write path remains independently gated by Engine capability enforcement and a concrete executor; Surface-read controls never authorize actions.

## Plugin/custom tool boundary

Plugin tools are trusted-user extensions, not a security sandbox for untrusted code:

- Webhooks allow literal loopback HTTP or public HTTPS. HTTPS hostnames are checked at registration and every request: DNS failures/empty answers fail closed; loopback/private/link-local/special-use addresses are rejected; domain targets pin the pre-connect resolution via a one-shot client. Redirects are disabled. Residual: not a code sandbox; hostile OS resolver TOCTOU remains (RR-014).
- Local scripts must resolve beneath `data_root/plugins/` and are canonicalized both at registration and execution. The process clears inherited environment variables and passes bounded JSON through stdin/environment, but the script still executes with the AIRP process user's operating-system authority. Only install code the user trusts.
- Input/output or response bodies are capped at 1 MiB and execution is clamped to 1–30 seconds. These are resource limits, not CPU, filesystem, network, child-process, or syscall isolation.
- A plugin's declared side-effect class and handling of the `confirm` flag are plugin-supplied behavior. AIRP enforces registry capability/allowlist/confirmation before dispatch, but cannot prove that a plugin labeled read-only is actually read-only or that a destructive plugin implements a reversible dry-run.
- Webhook headers and provider keys are separately persisted and redacted from list APIs. Error/log paths must not print header values, provider keys, full private responses, or script environment data.

Production and portable packages must not enable preinstalled custom tools silently. Adding or enabling a plugin is an explicit trusted-user action; broader plugin distribution, signing, permission manifests, isolation, and revocation remain release work.

### Plugin DNS / SSRF controls (updated 2026-07-30)

Webhook HTTPS hosts are validated at registration **and** immediately before each request:

- DNS resolution errors and empty answers are **fail-closed** (request/registration rejected).
- Any resolved loopback, private, link-local, or special-use address is rejected.
- Domain targets pin the pre-connect resolution result into a one-shot client (`resolve_to_addrs`) so connect uses those addresses rather than a second unbound lookup.

This closes #381 E-P0-3 / #329 N3 for the near-term SSRF residual. Plugins remain trusted-user extensions, not a sandbox for untrusted code (see RR-014).

