#!/bin/sh
set -eu

if [ "$#" -lt 1 ]; then
  echo "usage: ./bootstrap.sh '<argon2id-or-bcrypt-hash>' [provider-api-key]" >&2
  echo "plaintext administrator passwords are not accepted" >&2
  exit 2
fi

admin_hash=$1
provider_key=${2:-}
case "$admin_hash" in
  '$argon2id$'*|'$2a$'*|'$2b$'*|'$2y$'*) ;;
  *) echo "administrator password must be a Caddy-supported Argon2id or bcrypt hash" >&2; exit 2 ;;
esac

root=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
mkdir -p "$root/secrets"
[ -f "$root/.env" ] || cp "$root/.env.example" "$root/.env"

umask 077
openssl rand -base64 32 | tr '+/' '-_' | tr -d '=\n' > "$root/secrets/engine_access_key"
printf '%s' "$admin_hash" > "$root/secrets/admin_password_hash"
printf '%s' "$provider_key" > "$root/secrets/provider_api_key"

echo "Created production secret files and .env (if absent)."
echo "Review .env, then run: docker compose --env-file .env -f compose.yaml up -d --build"
