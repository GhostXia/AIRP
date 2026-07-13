#!/bin/sh
set -eu

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
deploy="$repo/deploy/production"
smoke_id=${GITHUB_RUN_ID:-local}-$$
case "$smoke_id" in *[!A-Za-z0-9._-]*) echo "invalid smoke id" >&2; exit 2 ;; esac

export AIRP_SMOKE_ID="$smoke_id"
export COMPOSE_PROJECT_NAME="airp-smoke-$smoke_id"
origin=https://localhost:9443
admin_user=airp-smoke
admin_password=synthetic-smoke-password
result_file="$deploy/smoke-result.json"
root_ca="$deploy/smoke-root.crt"
mock_log="$deploy/mock-provider.log"
mock_root="$deploy/certs/mock-root.crt"
mock_key="$deploy/certs/mock-provider.key"
mock_cert="$deploy/certs/mock-provider.crt"
trust_bundle="$deploy/smoke-trust.pem"
gateway_leaf="$deploy/smoke-gateway-leaf.crt"
webui_asset="$deploy/smoke-app.js"
compose="docker compose --env-file $deploy/.env -f $deploy/compose.yaml -f $deploy/smoke-compose.yaml"

cleanup() {
  status=$?
  trap - EXIT INT TERM
  $compose logs --no-color > "$deploy/topology-smoke.log" 2>&1 || true
  $compose down --volumes --remove-orphans >/dev/null 2>&1 || true
  if [ -n "${mock_pid:-}" ]; then kill "$mock_pid" >/dev/null 2>&1 || true; fi
  exit "$status"
}
trap cleanup EXIT INT TERM

mkdir -p "$deploy/secrets" "$deploy/certs"
umask 077
openssl req -x509 -newkey rsa:2048 -nodes -days 1 \
  -subj '/CN=AIRP smoke mock root' \
  -keyout "$deploy/certs/mock-root.key" -out "$mock_root" >/dev/null 2>&1
openssl req -newkey rsa:2048 -nodes -subj '/CN=host.docker.internal' \
  -keyout "$mock_key" -out "$deploy/certs/mock-provider.csr" >/dev/null 2>&1
printf '%s\n' 'subjectAltName=DNS:host.docker.internal,DNS:localhost,IP:127.0.0.1' > "$deploy/certs/mock-provider.ext"
openssl x509 -req -days 1 -sha256 \
  -in "$deploy/certs/mock-provider.csr" \
  -CA "$mock_root" -CAkey "$deploy/certs/mock-root.key" -CAcreateserial \
  -extfile "$deploy/certs/mock-provider.ext" -out "$mock_cert" >/dev/null 2>&1
# The engine runs as uid 65532 and must be able to read only the public CA certificate.
# Private CA/provider keys retain the restrictive umask.
chmod 0644 "$mock_root"
openssl rand -base64 32 | tr '+/' '-_' | tr -d '=\n' > "$deploy/secrets/engine_access_key"
openssl rand -base64 32 | tr '+/' '-_' | tr -d '=\n' > "$deploy/secrets/provider_api_key"
docker run --rm --entrypoint caddy airp-gateway:0.1.0 \
  hash-password --algorithm argon2id --plaintext "$admin_password" \
  > "$deploy/secrets/admin_password_hash"

cat > "$deploy/.env" <<EOF
AIRP_VERSION=0.1.0
AIRP_PUBLIC_ORIGIN=$origin
AIRP_TLS_MODE=internal
AIRP_ADMIN_USER=$admin_user
AIRP_ENDPOINT=https://host.docker.internal:8889/v1/chat/completions
AIRP_MODEL=airp-mock-1
AIRP_LOG=info
AIRP_HTTP_PORT=9080
AIRP_HTTPS_PORT=9443
AIRP_TLS_CERT_DIR=./certs
AIRP_TLS_CERT_FILE=/run/airp-tls/fullchain.pem
AIRP_TLS_KEY_FILE=/run/airp-tls/privkey.pem
EOF

MOCK_PROVIDER_HOST=0.0.0.0 \
MOCK_PROVIDER_TLS_CERT_FILE="$mock_cert" \
MOCK_PROVIDER_TLS_KEY_FILE="$mock_key" \
node "$repo/webui/mock-provider.js" > "$mock_log" 2>&1 &
mock_pid=$!

$compose up -d --no-build
for _ in $(seq 1 60); do
  if $compose exec -T gateway test -s /data/caddy/pki/authorities/local/root.crt >/dev/null 2>&1; then break; fi
  sleep 1
done
$compose exec -T gateway test -s /data/caddy/pki/authorities/local/root.crt
$compose cp gateway:/data/caddy/pki/authorities/local/root.crt "$root_ca" >/dev/null
gateway_leaf_path=$($compose exec -T gateway sh -c "find /data/caddy/certificates/local -type f -name '*.crt' | head -n 1" | tr -d '\r')
[ -n "$gateway_leaf_path" ]
$compose cp "gateway:$gateway_leaf_path" "$gateway_leaf" >/dev/null
chrome_spki=$(openssl x509 -in "$gateway_leaf" -pubkey -noout \
  | openssl pkey -pubin -outform der \
  | openssl dgst -sha256 -binary \
  | openssl base64 -A)
[ -n "$chrome_spki" ]
cat "$root_ca" "$mock_root" > "$trust_bundle"

auth_header="Basic $(printf '%s' "$admin_user:$admin_password" | openssl base64 -A)"
curl_tls="curl --silent --show-error --cacert $root_ca"

anonymous_status=$($curl_tls --output /dev/null --write-out '%{http_code}' "$origin/")
[ "$anonymous_status" = 401 ]
wrong_status=$($curl_tls --user "$admin_user:wrong" --output /dev/null --write-out '%{http_code}' "$origin/")
[ "$wrong_status" = 401 ]
if curl --silent --max-time 1 http://127.0.0.1:8000/health >/dev/null 2>&1; then
  echo "engine port is reachable from the host" >&2
  exit 1
fi

for _ in $(seq 1 60); do
  if $curl_tls --user "$admin_user:$admin_password" --fail "$origin/health" >/dev/null 2>&1; then break; fi
  sleep 1
done
$curl_tls --user "$admin_user:$admin_password" --fail "$origin/health" | grep -q '"engine":"ok"'
$curl_tls --user "$admin_user:$admin_password" --fail "$origin/version" | grep -q '"name":"airp-core"'

headers=$($curl_tls --user "$admin_user:$admin_password" --dump-header - --output /dev/null "$origin/")
printf '%s' "$headers" | grep -qi "content-security-policy:.*script-src 'self'"
if printf '%s' "$headers" | grep -Eqi 'unsafe-inline|unsafe-eval'; then exit 1; fi
printf '%s' "$headers" | grep -qi 'x-content-type-options: nosniff'
printf '%s' "$headers" | grep -qi 'x-frame-options: DENY'
printf '%s' "$headers" | grep -qi 'strict-transport-security: max-age=31536000'
printf '%s' "$headers" | grep -qi 'cache-control: no-store'

cors_headers=$($curl_tls --user "$admin_user:$admin_password" --request OPTIONS \
  --header 'Origin: https://attacker.example' \
  --header 'Access-Control-Request-Method: POST' \
  --dump-header - --output /dev/null "$origin/v1/settings")
if printf '%s' "$cors_headers" | grep -qi '^access-control-allow-origin:'; then exit 1; fi

path_status=$($curl_tls --user "$admin_user:$admin_password" \
  --header 'Content-Type: application/json' \
  --data '{"character_id":"forbidden-path","card_path":"/etc/passwd"}' \
  --output "$deploy/card-path-response.json" --write-out '%{http_code}' \
  "$origin/v1/characters/import")
[ "$path_status" = 400 ]
grep -q 'card_path' "$deploy/card-path-response.json"

node -e "process.stdout.write(JSON.stringify({payload:'x'.repeat(11*1024*1024)}))" > "$deploy/oversized.json"
oversized_status=$($curl_tls --user "$admin_user:$admin_password" \
  --header 'Content-Type: application/json' --data-binary "@$deploy/oversized.json" \
  --output /dev/null --write-out '%{http_code}' "$origin/v1/characters/import")
[ "$oversized_status" = 413 ]

AIRP_ENGINE_URL="$origin" \
AIRP_MOCK_URL=https://localhost:8889 \
AIRP_AUTH_HEADER="$auth_header" \
AIRP_SMOKE_KEEP_SESSION=1 \
AIRP_SMOKE_RESULT_FILE="$result_file" \
NODE_EXTRA_CA_CERTS="$trust_bundle" \
node "$repo/webui/smoke.mjs"

$compose restart engine gateway >/dev/null
for _ in $(seq 1 60); do
  if $curl_tls --user "$admin_user:$admin_password" --fail "$origin/health" >/dev/null 2>&1; then break; fi
  sleep 1
done
character_id=$(node -p "JSON.parse(require('fs').readFileSync(process.argv[1])).character_id" "$result_file")
session_id=$(node -p "JSON.parse(require('fs').readFileSync(process.argv[1])).session_id" "$result_file")
history=$($curl_tls --user "$admin_user:$admin_password" \
  --header 'Content-Type: application/json' \
  --data "{\"character_id\":\"$character_id\",\"session_id\":\"$session_id\"}" \
  "$origin/v1/chat/history")
HISTORY_JSON="$history" node -e "const h=JSON.parse(process.env.HISTORY_JSON); if(h.messages?.length!==3) process.exit(1)"

AIRP_SMOKE_ORIGIN="$origin" \
AIRP_SMOKE_ADMIN_USER="$admin_user" \
AIRP_SMOKE_ADMIN_PASSWORD="$admin_password" \
AIRP_SMOKE_RESULT_FILE="$result_file" \
AIRP_CHROME_SPKI="$chrome_spki" \
NODE_EXTRA_CA_CERTS="$trust_bundle" \
node "$repo/ui/production-browser-smoke.mjs"

$curl_tls --user "$admin_user:$admin_password" "$origin/version?smoke_secret_query=marker" >/dev/null
$compose logs --no-color > "$deploy/topology-smoke.log"
engine_key=$(cat "$deploy/secrets/engine_access_key")
provider_key=$(cat "$deploy/secrets/provider_api_key")
admin_hash=$(cat "$deploy/secrets/admin_password_hash")
basic_value=$(printf '%s' "$admin_user:$admin_password" | openssl base64 -A)
forbidden_labels=('engine access key' 'provider API key' 'admin password hash' 'admin username' 'admin password' 'Basic authorization value' 'query marker' 'runner path')
forbidden_values=("$engine_key" "$provider_key" "$admin_hash" "$admin_user" "$admin_password" "$basic_value" 'smoke_secret_query=marker' '/home/runner/work')
for index in "${!forbidden_values[@]}"; do
  forbidden=${forbidden_values[$index]}
  if grep -F -- "$forbidden" "$deploy/topology-smoke.log" >/dev/null; then
    echo "secret or private path leaked to runtime logs (sentinel: ${forbidden_labels[$index]})" >&2
    exit 1
  fi
done
for image in airp-engine:0.1.0 airp-gateway:0.1.0; do
  for index in "${!forbidden_values[@]}"; do
    forbidden=${forbidden_values[$index]}
    if docker image inspect "$image" | grep -F -- "$forbidden" >/dev/null; then
      echo "runtime secret leaked to image metadata (sentinel: ${forbidden_labels[$index]})" >&2
      exit 1
    fi
  done
done
$curl_tls --user "$admin_user:$admin_password" --fail "$origin/app.js" > "$webui_asset"
for index in "${!forbidden_values[@]}"; do
  forbidden=${forbidden_values[$index]}
  if grep -F -- "$forbidden" "$webui_asset" >/dev/null; then
    echo "runtime secret leaked to WebUI asset (sentinel: ${forbidden_labels[$index]})" >&2
    exit 1
  fi
done

echo "production topology smoke passed"
