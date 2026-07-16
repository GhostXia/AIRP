# Security and deployment boundary

> Baseline reviewed: 2026-07-16 at `main@c47585b`. Current implementation status and release gates are in [CURRENT-BASELINE.md](CURRENT-BASELINE.md).

AIRP defaults to a single-user local topology. The daemon binds to loopback; the bundled desktop UI owns its sidecar process and stops it when the UI exits.

## Credentials

- `AIRP_API_KEY` supplies the upstream provider credential.
- `AIRP_ACCESS_KEY` enables bearer authentication for `/v1/*`.
- Provider and access keys are runtime-only. `config.json` and `data/settings.json` no longer serialize them, and legacy plaintext fields are ignored when loading.
- In development, `POST /v1/settings` may replace a key for the current process, but its persisted settings omit secrets. In production, the engine bearer is immutable through this endpoint and must be rotated with the gateway secret followed by restart.

Use the operating system/service secret facility for non-interactive deployment. Do not put keys in repository files, installer arguments, logs, or copied diagnostics.

## Browser origins and network exposure

Development CORS origins are the bundled WebUI (`127.0.0.1:9001` and `localhost:9001`) plus Tauri origins. `AIRP_CORS_ORIGINS` extends this development allowlist. Production ignores those conveniences and allows only the canonical HTTPS `AIRP_PUBLIC_ORIGIN`. Wildcard origins are not supported.

Loopback plus CORS is not authentication. Before exposing the daemon through a reverse proxy or non-loopback bind, set `AIRP_ACCESS_KEY`, terminate TLS at the proxy, restrict trusted origins, and apply network-level access control.

## WebUI production profile (deployment artifact and topology smoke implemented)

The first supported WebUI deployment is specified by [WEBUI-PRODUCTION-ARCHITECTURE.md](WEBUI-PRODUCTION-ARCHITECTURE.md): a versioned OCI/Compose bundle with Caddy as the only public HTTPS entry point and `airp-core` on a private network.

- Caddy authenticates the user at the perimeter and replaces the incoming `Authorization` header with the server-held engine bearer for `/v1/*`, `/health` and `/version`.
- The browser never receives `AIRP_ACCESS_KEY`, provider credentials or the engine address. Static files are behind the same perimeter authentication.
- `AIRP_DEPLOYMENT_MODE=production` validates its environment-only policy before loading or creating persisted config, and fails before listen unless `AIRP_ACCESS_KEY` is exactly 32 bytes encoded as canonical unpadded base64url, `AIRP_PUBLIC_ORIGIN` is one canonical HTTPS origin, and `AIRP_DATA_DIR` is absolute, existing, writable and not a filesystem root. It rejects `AIRP_ALLOW_LOCAL_PATH` and runtime engine-bearer replacement.
- Production WebUI imports upload JSON/PNG content only. `card_path`, host/UNC paths, file URLs and arbitrary remote fetches are outside this trust boundary even for authenticated callers.
- The private engine keeps its own bearer, validation, body limits, path guards and outbound redirect policy. Gateway controls do not replace engine controls.

The engine fail-closed slice and `deploy/production/` OCI/Compose + Caddy artifact are implemented. The bundle pins base images by digest, mounts runtime secrets from gitignored files, publishes only Caddy, uses a private engine network, and makes the production WebUI same-origin without browser-visible engine credentials. The `Production topology` CI gate exercises real internal TLS, negative perimeter authentication, private-engine reachability, CSP/headers/body limits, content-only import, incremental SSE, restart persistence, system-Chrome injection/cancellation and runtime-secret scans. P1-P3 release gates remain open; never expose `webui/serve.js` or port 8000 as a remote deployment.

PR #191 upgraded the `ui/` toolchain to Vite 8.1.4, Vitest 4.1.10 and Vue plugin 6.0.8; its locked dependency tree reports zero `npm audit` findings and passed UI/WebUI plus production-browser gates. These remain development dependencies and are not copied into production runtime images. Development servers and test UIs must still remain loopback-only or otherwise restricted to trusted networks.

## Widgets and Agent tools

UI consent is a user-experience gate, not the authority. Agent tools are disabled unless daemon bearer authentication is enabled. The bundled sidecar generates a process-scoped random bearer and shares it only with the trusted BusRelay. After authentication, a tool must still be registered, the trusted host must grant `call:tool`, and an optional per-run allowlist must contain it. Destructive tools remain dry-run unless their exact name appears in `confirm_tools`.

Third-party widgets must never receive the daemon bearer key directly. The trusted host should translate a user grant into the smallest capability/allowlist request needed for one operation.

`GET /v1/agent/tools` exposes names, descriptions, and side-effect classes only; it grants no capability. `export_context_bundle` writes beneath the engine data root, validates identifiers, and applies the same model-facing size limit as lorebook reads. `update_lorebook` and `seal_volume` are destructive and therefore require exact-name confirmation.
