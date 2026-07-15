# WebUI production architecture and threat boundary

> Status: accepted P0 implementation contract; engine fail-closed, deployment artifact and production topology smoke implemented
>
> Decision date: 2026-07-13
>
> Last baseline check: 2026-07-15 (`main@c54428e`; no topology change after PR #136)
>
> Scope: the first supported single-instance, self-hosted, single-user WebUI topology in
> [WEBUI-PRODUCTION-PLAN.md](WEBUI-PRODUCTION-PLAN.md)

This document fixes the boundary that the first executable production slice must implement.
It deliberately does not define a multi-user identity system, hosted SaaS control plane, or
desktop packaging path.

## 1. Decision

The first-party production artifact will be a versioned OCI/Compose bundle with two runtime
services:

```text
Internet or private user network
  -> Caddy 2.11.4 :443 (the only published application port)
       -> HTTP Basic perimeter authentication
       -> static AIRP WebUI files
       -> /v1/*, /health, /version
            Authorization is replaced with Bearer <AIRP_ACCESS_KEY>
            -> airp-core:8000 on a private container network
                 -> persistent /var/lib/airp data volume
                 -> configured model provider over outbound HTTPS
```

- The repository owns the engine image, WebUI image layer/configuration, Compose manifest,
  smoke test, upgrade notes and checksums. A loose collection of example commands is not the
  production artifact.
- The gateway uses the official `caddy:2.11.4` image, pinned by version and ultimately by
  image digest in the executable slice. Caddy supplies automatic HTTPS, static file serving,
  perimeter Basic authentication, security headers and reverse proxying. AIRP does not copy
  or adapt Caddy source.
- Only the gateway publishes ports. The engine has no host `ports` mapping and binds only
  inside the private deployment boundary. Direct engine exposure is unsupported.
- The public WebUI and API share one origin. The browser never receives the engine bearer,
  provider key, engine hostname or private port.
- HTTP Basic is a narrow first-release perimeter, not AIRP's future identity model. It is
  acceptable only over HTTPS and may later be replaced behind the same gateway boundary.

Native binaries and Kubernetes manifests may be added later, but they cannot silently define
different authentication or data contracts.

## 2. Trust zones and authority

| Zone | Trusted with | Must not receive |
|---|---|---|
| Browser | RP content, public origin, perimeter username/password entered by the user | `AIRP_ACCESS_KEY`, provider key, engine address, host paths |
| Caddy gateway | Basic password hash, engine bearer, public hostname, TLS state | Provider key, writable AIRP data root |
| AIRP engine | Engine bearer, provider key, AIRP data root, outbound provider access | Perimeter plaintext password, public TLS private key |
| Persistent data volume | User RP data and versioned application state | Provider/access keys, gateway password or password hash |
| Image/build pipeline | Public application assets and dependency metadata | Runtime secrets or real user data |

The gateway authenticates the human-facing request. It then **replaces**, rather than appends
or forwards, the incoming `Authorization` header with `Bearer {$AIRP_ACCESS_KEY}` for engine
routes. The engine therefore never interprets browser-supplied Basic credentials, and a caller
cannot smuggle an alternate bearer through the proxy.

Static files are also behind perimeter authentication. This keeps the first release honest:
the login boundary protects the whole product, not only mutating API calls.

## 3. Production configuration contract

The executable slice must expose this minimal contract. Names are fixed here so deployment,
engine validation, documentation and smoke tests do not invent parallel schemas.

| Setting | Required | Owner | Rule |
|---|---:|---|---|
| `AIRP_DEPLOYMENT_MODE=production` | yes | engine | Enables fail-closed production validation; an unknown value is an error |
| `AIRP_PUBLIC_ORIGIN` | yes | gateway + engine | One canonical `https://host[:port]` origin; no path, query, fragment or wildcard |
| `AIRP_TLS_MODE` | yes | gateway | `public`, `internal` or `files`; defaults are not guessed from reachability |
| `AIRP_TLS_CERT_FILE` / `AIRP_TLS_KEY_FILE` | `files` only | gateway | Read-only PEM secret mounts whose SAN covers the public origin |
| `AIRP_ACCESS_KEY` | yes | gateway + engine | Exactly 32 CSPRNG bytes encoded as 43-character unpadded base64url (`A-Z a-z 0-9 _ -`); runtime secret, never persisted or returned |
| `AIRP_ADMIN_USER` | yes | gateway | Non-empty perimeter username |
| `AIRP_ADMIN_PASSWORD_HASH` | yes | gateway | Caddy-supported Argon2id or bcrypt hash; plaintext is rejected/not accepted by the bundle |
| `AIRP_DATA_DIR` | yes | engine | Fixed container path backed by the persistent data volume |
| `AIRP_ENDPOINT` | yes for first provider use | engine | Valid provider HTTPS endpoint under existing outbound redirect policy |
| `AIRP_MODEL` | yes for first provider use | engine | Provider model identifier |
| `AIRP_API_KEY` | provider-dependent | engine only | Runtime secret; may be absent for providers that do not require one |

Production validation happens before the listener is opened. In production mode the engine
must fail startup when:

- `AIRP_ACCESS_KEY` is not exactly 43 ASCII characters from the unpadded base64url alphabet;
- `AIRP_PUBLIC_ORIGIN` is not one exact HTTPS origin;
- `AIRP_ALLOW_LOCAL_PATH` is enabled;
- `AIRP_DATA_DIR` is not an existing writable absolute directory, or resolves to the
  filesystem root;
- a production-only setting is malformed or an unsupported deployment mode is supplied.

`AIRP_PUBLIC_ORIGIN` is also the sole production CORS origin. Built-in development/Tauri
origins must not be silently added in production mode.

The engine can validate the key's representation, not its entropy. The first-party bootstrap
must generate exactly 32 bytes with an operating-system CSPRNG and encode them as unpadded
base64url. Operator-supplied keys must follow the same process; memorable text padded to 43
characters does not satisfy the contract even though representation validation cannot detect it.

Runtime `POST /v1/settings` must not replace `AIRP_ACCESS_KEY` in production: changing it in
only the engine would desynchronize the gateway and lock out all proxied calls. Engine bearer
rotation is an operator action that updates both services and restarts them. Provider key
updates may remain process-local, but the UI must state that they do not survive restart unless
the operator updates the deployment secret.

## 4. Gateway policy

The first executable configuration must enforce all of the following and validate the rendered
Caddy configuration before startup:

- TLS is mandatory; port 80 may exist only for redirect/ACME challenge handling. `public` mode
  uses Caddy-managed public certificates, `internal` uses Caddy's internal CA, and `files` loads
  an operator-provided PEM certificate/key pair. Internal-CA deployments must document how to
  export and install the root CA on every client. Smoke tests must use a trusted chain and must
  not bypass certificate verification.
- Basic authentication covers static files and every proxied route. Only password hashes enter
  configuration; plaintext credentials do not.
- Engine route matching is an explicit allowlist: `/v1/*`, `/health` and `/version`. Unknown
  reserved paths fail closed rather than falling through to the SPA.
- Incoming `Authorization` is overwritten for proxied requests. Browser cookies and proxy
  authentication state are not treated as engine authority.
- Request bodies are capped at 10 MiB at the gateway, with tighter engine endpoint limits kept
  as the authoritative second layer. Oversized requests return `413`.
- Security headers target this CSP without `unsafe-inline` or `unsafe-eval`:
  `default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data: blob:;
  connect-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'none';
  form-action 'self'`, plus `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`,
  `X-Frame-Options: DENY` and a restrictive `Permissions-Policy`.
- Dynamic inline-style writes are outside the production CSP contract. Commit `7866dbf` replaced
  the two `document.body.style.userSelect` assignments with a CSS class and replaced
  `workbenchPanel.style.width` with bounded `data-width` states. The system-Chrome smoke listens
  for `securitypolicyviolation`, asserts an empty violation list and has passed through the
  production gateway. Future UI changes must preserve that test; weakening `style-src` with
  `unsafe-inline` is not an accepted fix.
- HSTS is emitted only by the HTTPS production site. The first slice uses `Cache-Control:
  no-store` for API responses and unversioned WebUI assets to prevent mixed-version UI state;
  immutable caching waits for content-hashed assets.
- Proxy connect/response-header timeouts are bounded, while SSE bodies are not given a short
  total-response timeout. Engine SSE responses must retain `Content-Type: text/event-stream`
  and `Cache-Control: no-cache`; the engine route must not enable `response_buffers` or forced
  compression. This deliberately uses Caddy's immediate flush for `text/event-stream` rather
  than `flush_interval -1`, because the latter also keeps the upstream request alive after an
  early client disconnect. The smoke must assert incremental delivery and cancellation.
- Access logs omit request/response bodies and the complete query string. The Caddy `log`
  formatter must filter `request>uri` with the regexp `\?.*$` → empty string, and explicitly
  delete `user_id`, `request>headers>Authorization`, `request>headers>Proxy-Authorization`,
  `request>headers>Cookie` and `response>headers>Set-Cookie` even though Caddy redacts common
  credential headers by default. Runtime/access logging must receive an explicit retention/size
  contract in P2; `log_credentials` must never be enabled.

The gateway is a security boundary, but not the only one. Engine bearer validation, endpoint
body limits, typed validation, outbound redirect policy and path guards remain mandatory.

### 4.1 Independent complexity audit (2026-07-13)

This audit does not treat the merged P0 design as an unquestionable premise. Its current
disposition is:

- **Keep the edge-process boundary.** TLS termination, static WebUI serving, same-origin API
  routing, browser-facing authentication, security headers and replacement of the browser's
  credentials with the server-held engine bearer are real production responsibilities. Removing
  Caddy would move those responsibilities into the RP engine or replace Caddy with another edge
  proxy; it would not remove them. Caddy remains replaceable infrastructure, not a third AIRP
  domain service and not a revival of the historical AIRP-Gateway product boundary.
- **Reconsider site access logging.** Caddy's site-level `log` directive is optional request
  logging and is separate from Caddy runtime logs. It is not required for TLS, authentication,
  static files or reverse proxying. The initial deployment artifact enabled it in commit
  `7866dbf`, including the filter for queries and credential-bearing headers. Commit `c968580`
  only added `user_id delete` after the topology leak gate proved that successful Basic auth
  placed the administrator username in the access record.
- **Preliminary finding: localized but premature complexity.** The filter is appropriate only
  while access logging is enabled; it is not independently useful. Enabling access logs in P0
  pulled part of the P2 observability problem forward before AIRP had specified the operator use
  case, stable field set, output destination and retention contract. Compose currently bounds
  container logs, but that does not complete the P2 observability design.
- **Smallest follow-up decision.** At P2, choose explicitly between (a) removing the whole
  site-level access-log block and retaining only runtime/startup logs, or (b) keeping access logs
  and completing a bounded, documented operator contract. Do not retain the filter merely
  because the smoke test scans it. Until that decision is implemented, every field filter in the
  current block remains required because the access log is still enabled.

This calibration is documentation-only; it deliberately does not change the merged gateway
configuration. The unresolved choice belongs to the P2 redacted-observability gate.

## 5. Remote import policy

Production WebUI imports are content uploads only:

- accepted character inputs: JSON content or PNG bytes (the current JSON/base64 envelope is
  temporarily allowed within the 10 MiB request limit; multipart/streaming is the follow-up);
- accepted preset/worldbook inputs: validated uploaded JSON content;
- rejected in production: `card_path`, file URLs, UNC paths, server absolute/relative paths,
  symlink targets and any request that asks the engine to fetch an arbitrary remote URL;
- `AIRP_ALLOW_LOCAL_PATH=1` is a startup error in production mode, independent of bearer auth.

The local Tauri sidecar may continue to use its separately documented trusted file-dialog path
flow. That exception does not cross into the WebUI production profile.

## 6. Data and lifecycle boundary

- The data root is the only writable application volume. Images and static assets are
  read-only; the engine runs as a non-root user and receives no Docker socket or host filesystem
  mount.
- Caddy certificate/config state uses separate gateway-owned volumes. It cannot traverse the
  AIRP data volume.
- Stop/restart must preserve the data root. Upgrade must be explicit about the image version and
  must never use a floating `latest` tag.
- P0 smoke may create synthetic data in a disposable volume. It must never package or upload a
  developer's real `data/`, `.env`, logs, absolute paths or credentials.
- Backup/restore and migration rollback are P2 gates, but the P0 layout must not make them
  impossible: the data volume is named, externally discoverable and not mixed with binaries.

## 7. Threat model and required evidence

| Threat | Control | Executable evidence required |
|---|---|---|
| Direct engine exposure | No published engine port; private network only | Host connection to engine port fails; gateway path succeeds |
| Anonymous access | TLS + Basic auth over the whole site | Missing/wrong credentials return `401`; valid credentials load UI/API |
| Browser obtains engine bearer | Gateway-owned secret and header replacement | Static assets/config/logs contain no bearer; supplied bearer cannot pass through |
| Weak/missing production secret | Canonical generated 32-byte key plus pre-listener representation validation | Missing, non-43-character or non-base64url key causes non-zero startup and no listening socket |
| Server-side arbitrary file read | Production rejects local-path mode and `card_path` | Authenticated `card_path` request returns typed `400`; upload still succeeds |
| Cross-origin drive-by request | Exact HTTPS origin + perimeter auth + production CORS | Foreign Origin preflight/request is rejected |
| Script injection/clickjacking | Strict CSP, no unsafe inline/eval, removal of dynamic inline-style writes, frame denial, output escaping | Header assertions, zero CSP violations on main flows, plus browser injection fixtures |
| Oversized import/body exhaustion | Gateway 10 MiB cap + tighter engine caps | Over-limit request returns `413`; process remains ready |
| SSE buffering or premature timeout | SSE-aware reverse proxy, no short body timeout | Three streamed turns arrive incrementally through HTTPS |
| Secret/PII leakage | Runtime-only secrets, redacted bounded logs, clean images | Secret scan plus log/image inspection |
| Upgrade mixes UI/API versions | Versioned images and no-store unversioned assets | Refresh after controlled upgrade loads a coherent version |

The production smoke exit path is:

1. start from a clean disposable deployment with generated synthetic secrets;
2. prove anonymous and wrong-password requests fail;
3. prove the engine port is not reachable from the host;
4. authenticate through HTTPS and verify `/health` and `/version`;
5. configure a mock or explicitly opted-in real provider without logging its secret;
6. import a synthetic card by content, and prove `card_path` is rejected;
7. complete three streamed turns, refresh, and recover the persisted history;
8. restart both services and repeat the health/history checks;
9. inspect logs and artifact contents for secrets, local paths and real user data.

This is a P0 topology smoke, not yet the P2 backup/restore or P3 release-candidate drill.

## 8. Rejected alternatives

- **Expose engine TLS directly and enable CORS:** rejected because it gives the browser the
  engine address/bearer and duplicates static hosting, TLS and browser security policy inside
  the RP engine.
- **Publish engine port beside a static server:** rejected because CORS plus loopback/private
  intent is not authentication and the topology is easy to misdeploy.
- **Put the engine bearer in WebUI local storage or generated JavaScript:** rejected because any
  browser script, extension, support bundle or static asset leak would gain engine authority.
- **Use only `AIRP_ACCESS_KEY` as the human login:** rejected because the gateway needs a
  browser-facing authentication mechanism while keeping the engine bearer server-side.
- **Ship `start.bat`, `cargo run` or `serve.js` as production:** rejected because they do not
  define TLS, secret handling, persistence, upgrades, header policy or reproducible artifacts.
- **Use floating image tags:** rejected because a production deployment cannot be reproduced or
  rolled back from an unpinned dependency graph.

## 9. Follow-up implementation slices

1. Engine production-mode validation and immutable production bearer tests. **Implemented in the first P0 execution slice.**
2. First-party engine/WebUI images, pinned Compose/Caddy configuration and operator secret
   bootstrap. **Implemented in `deploy/production/`; CI image/config validation is required on every PR.**
3. Production topology smoke covering the evidence table above. **Implemented by the `Production topology` PR gate with disposable data/Caddy volumes, a synthetic HTTPS provider and a real system-Chrome pass.**
4. P1 RP management surface, followed by P2 recovery/operations and P3 release gates.

P0 topology proof does not make the product a formal release. P1 RP management, P2
backup/restore and migration, and P3 compatibility/provenance/release-candidate gates remain
required by [WEBUI-PRODUCTION-PLAN.md](WEBUI-PRODUCTION-PLAN.md).

## 10. Upstream verification basis

The dependency and gateway decisions were checked on 2026-07-13 against the upstream
[Caddy v2.11.4 release](https://github.com/caddyserver/caddy/releases/tag/v2.11.4),
[official container image guidance](https://hub.docker.com/_/caddy),
[Basic authentication](https://caddyserver.com/docs/caddyfile/directives/basic_auth),
[request-body limit](https://caddyserver.com/docs/caddyfile/directives/request_body),
[response header](https://caddyserver.com/docs/caddyfile/directives/header) and
[reverse proxy/SSE](https://caddyserver.com/docs/caddyfile/directives/reverse_proxy),
[TLS](https://caddyserver.com/docs/caddyfile/directives/tls) and
[access log filtering](https://caddyserver.com/docs/caddyfile/directives/log)
documentation. Caddy v2.11.4 is Apache-2.0. The P0 artifact pins the Caddy, Rust builder and
Debian runtime multi-platform image digests in its Dockerfiles and records them in
[ACKNOWLEDGEMENTS.md](ACKNOWLEDGEMENTS.md). Complete package-level notices/SBOM remain a P3
formal-release gate; their absence is one reason the current artifact is only a preview.
