# AIRP production deployment

> Baseline checked: 2026-07-17 at `main@15cb6c0`.

This directory is the first-party single-instance, self-hosted WebUI bundle. It runs two
application services: a Caddy HTTPS gateway and a private AIRP engine. Only Caddy publishes
host ports. The browser uses the same origin and never receives the engine bearer.

This is an implemented P0 preview topology, not a formal release. Current product status and
remaining P1-P3 gates are authoritative in
[`docs/CURRENT-BASELINE.md`](../../docs/CURRENT-BASELINE.md) and
[`docs/WEBUI-PRODUCTION-PLAN.md`](../../docs/WEBUI-PRODUCTION-PLAN.md).

## Prerequisites

- Docker Engine with Docker Compose v2;
- a DNS name resolving to this host for `public` TLS mode, or operator-provided certificates
  for `files` mode;
- an Argon2id or bcrypt administrator password hash. Generate it interactively without putting
  plaintext in shell history:

  ```console
  docker run --rm -it caddy:2.11.4@sha256:af5fdcd76f2db5e4e974ee92f96ee8c0fc3edb55bd4ba5032547cbf3f65e486d caddy hash-password --algorithm argon2id
  ```

## Bootstrap and start

From this directory on Windows PowerShell:

```powershell
.\bootstrap.ps1 -AdminPasswordHash '<paste hash>' -ProviderApiKey '<optional provider key>'
notepad .env
docker compose --env-file .env -f compose.yaml config --quiet
docker compose --env-file .env -f compose.yaml up -d --build
```

On a POSIX host, use `./bootstrap.sh '<paste hash>' '<optional provider key>'` instead. Both
bootstrap scripts create a CSPRNG 32-byte engine bearer and gitignored secret files. They never
accept or persist the administrator plaintext password. Re-running bootstrap rotates the engine
key; restart both services together after rotation.

Select one explicit TLS mode in `.env`:

- `public`: Caddy obtains a public certificate for `AIRP_PUBLIC_ORIGIN`;
- `internal`: Caddy uses its private CA. Install the exported Caddy root CA on every client;
- `files`: place `fullchain.pem` and `privkey.pem` in `certs/` and keep the container paths from
  `.env.example`.

`AIRP_HTTPS_PORT` must match the explicit port in `AIRP_PUBLIC_ORIGIN`; both default to 443.
Public ACME mode requires the normal externally reachable ports 80 and 443. Use `internal` or
`files` mode for a deliberately non-standard HTTPS port.

The provider key file may be empty for an unauthenticated provider. It is mounted only into the
engine. The access key and administrator hash are mounted as Compose secrets and are absent from
the WebUI image and AIRP data volume.

## Lifecycle

```console
docker compose --env-file .env -f compose.yaml ps
docker compose --env-file .env -f compose.yaml logs --no-log-prefix gateway engine
docker compose --env-file .env -f compose.yaml restart
docker compose --env-file .env -f compose.yaml down
```

`down` preserves the named `airp-data`, `airp-caddy-data`, and `airp-caddy-config` volumes. Do
not use `down --volumes` on a real deployment. Image tags come from `AIRP_VERSION`; never replace
them with `latest`.

## CI topology proof

`smoke-ci.sh` plus `smoke-compose.yaml` are CI-only acceptance assets, not operator startup
commands. They create unique disposable data/Caddy volumes and a synthetic HTTPS provider, then
prove HTTPS perimeter auth, host-inaccessible engine, headers/CSP/body limits, content-only card
import, three incremental SSE turns, restart persistence, exact ephemeral-certificate SPKI trust,
system-Chrome injection/stream-cancel/prompt-preview
behavior and absence of synthetic secrets/private runner paths in logs, image metadata and WebUI
assets. Cleanup deletes only the uniquely named smoke volumes.

Passing this P0 topology gate does not make the bundle a formal release. P1 RP management, P2
backup/restore and upgrade rollback, and P3 provenance/compatibility/release-candidate gates remain
required by `docs/WEBUI-PRODUCTION-PLAN.md`.
