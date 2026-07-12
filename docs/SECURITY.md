# Security and deployment boundary

> Baseline reviewed: 2026-07-12. Current implementation status and release gates are in [CURRENT-BASELINE.md](CURRENT-BASELINE.md).

AIRP defaults to a single-user local topology. The daemon binds to loopback; the bundled desktop UI owns its sidecar process and stops it when the UI exits.

## Credentials

- `AIRP_API_KEY` supplies the upstream provider credential.
- `AIRP_ACCESS_KEY` enables bearer authentication for `/v1/*`.
- Provider and access keys are runtime-only. `config.json` and `data/settings.json` no longer serialize them, and legacy plaintext fields are ignored when loading.
- `POST /v1/settings` may replace a key for the current process, but its persisted settings omit secrets.

Use the operating system/service secret facility for non-interactive deployment. Do not put keys in repository files, installer arguments, logs, or copied diagnostics.

## Browser origins and network exposure

Default CORS origins are the bundled WebUI (`127.0.0.1:9001` and `localhost:9001`) plus Tauri origins. Set `AIRP_CORS_ORIGINS` to a comma-separated exact allowlist when using another trusted frontend. Wildcard origins are not supported.

Loopback plus CORS is not authentication. Before exposing the daemon through a reverse proxy or non-loopback bind, set `AIRP_ACCESS_KEY`, terminate TLS at the proxy, restrict trusted origins, and apply network-level access control.

## Widgets and Agent tools

UI consent is a user-experience gate, not the authority. Agent tools are disabled unless daemon bearer authentication is enabled. The bundled sidecar generates a process-scoped random bearer and shares it only with the trusted BusRelay. After authentication, a tool must still be registered, the trusted host must grant `call:tool`, and an optional per-run allowlist must contain it. Destructive tools remain dry-run unless their exact name appears in `confirm_tools`.

Third-party widgets must never receive the daemon bearer key directly. The trusted host should translate a user grant into the smallest capability/allowlist request needed for one operation.

`GET /v1/agent/tools` exposes names, descriptions, and side-effect classes only; it grants no capability. `export_context_bundle` writes beneath the engine data root, validates identifiers, and applies the same model-facing size limit as lorebook reads. `update_lorebook` and `seal_volume` are destructive and therefore require exact-name confirmation.
