#!/bin/sh
set -eu

read_required_secret() {
  secret_path="$1"
  variable_name="$2"
  if [ ! -r "$secret_path" ]; then
    echo "missing required secret: $secret_path" >&2
    exit 1
  fi
  secret_value=$(cat "$secret_path")
  if [ -z "$secret_value" ]; then
    echo "required secret is empty: $secret_path" >&2
    exit 1
  fi
  export "$variable_name=$secret_value"
}

read_required_secret /run/secrets/engine_access_key AIRP_ACCESS_KEY
read_required_secret /run/secrets/admin_password_hash AIRP_ADMIN_PASSWORD_HASH

if [ "${#AIRP_ACCESS_KEY}" -ne 43 ]; then
  echo "engine access key must be 43-character unpadded base64url" >&2
  exit 1
fi
case "$AIRP_ACCESS_KEY" in
  *[!A-Za-z0-9_-]*) echo "engine access key must be unpadded base64url" >&2; exit 1 ;;
esac
case "${AIRP_ADMIN_USER:-}" in
  ''|*[!A-Za-z0-9._-]*) echo "AIRP_ADMIN_USER must use only A-Z, a-z, 0-9, dot, underscore, or hyphen" >&2; exit 1 ;;
esac
case "$AIRP_ADMIN_PASSWORD_HASH" in
  '$argon2id$'*|'$2a$'*|'$2b$'*|'$2y$'*) ;;
  *) echo "administrator password secret must contain an Argon2id or bcrypt hash" >&2; exit 1 ;;
esac

case "${AIRP_TLS_MODE:-}" in
  public|internal|files) config="/etc/caddy/Caddyfile.${AIRP_TLS_MODE}" ;;
  *) echo "AIRP_TLS_MODE must be public, internal, or files" >&2; exit 1 ;;
esac

if [ "$AIRP_TLS_MODE" = "files" ]; then
  : "${AIRP_TLS_CERT_FILE:?AIRP_TLS_CERT_FILE is required for files TLS mode}"
  : "${AIRP_TLS_KEY_FILE:?AIRP_TLS_KEY_FILE is required for files TLS mode}"
fi

caddy validate --config "$config" --adapter caddyfile
exec caddy run --config "$config" --adapter caddyfile
