# AIRP production deployment

This directory is the first-party single-instance, self-hosted WebUI bundle. It runs two
application services: a Caddy HTTPS gateway and a private AIRP engine. Only Caddy publishes
host ports. The browser uses the same origin and never receives the engine bearer.

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
them with `latest`. P2 backup/restore and upgrade rollback are not yet release-supported, so this
bundle remains a P0 preview until the production topology smoke and later release gates land.
