#!/bin/sh
set -eu

# bootstrap-topology.sh — 启动 AIRP 生产拓扑子集供 agent browser exploration 使用。
#
# 从 deploy/production/smoke-ci.sh 抽取 (Task 9, issue #273 stage 2):
#   - 生成 mock provider TLS 证书 + secrets
#   - 写 .env
#   - 启动 mock provider (host node 进程)
#   - docker compose up -d (engine + gateway + 临时数据卷)
#   - 等待 gateway root.crt 就绪, 复制 CA, 生成 chrome_spki / trust_bundle
#   - 等待 /health 返回 engine:"ok" + /v1/models 200 (stages 1-2)
#   - 成功后退出, 拓扑继续运行 (不跑任何 smoke 测试)
#
# 用法:
#   ./bootstrap-topology.sh            # 启动拓扑, 成功后退出 (拓扑保留)
#   ./bootstrap-topology.sh --teardown # 幂等清理 (compose down + kill mock provider)
#
# 状态文件 $deploy/.bootstrap-topology.state 记录 smoke_id / compose_project_name / mock_pid,
# 供 --teardown 复用同一 COMPOSE_PROJECT_NAME (bootstrap 和 teardown 是不同 shell 进程, $$ 不同)。

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
deploy="$repo/deploy/production"
smoke_id=${GITHUB_RUN_ID:-local}-$$
case "$smoke_id" in *[!A-Za-z0-9._-]*) echo "invalid smoke id" >&2; exit 2 ;; esac

export AIRP_SMOKE_ID="$smoke_id"
export COMPOSE_PROJECT_NAME="airp-smoke-$smoke_id"
origin=https://localhost:9443
admin_user=airp-smoke
admin_password=synthetic-smoke-password
root_ca="$deploy/smoke-root.crt"
mock_log="$deploy/mock-provider.log"
mock_root="$deploy/certs/mock-root.crt"
mock_key="$deploy/certs/mock-provider.key"
mock_cert="$deploy/certs/mock-provider.crt"
trust_bundle="$deploy/smoke-trust.pem"
gateway_leaf="$deploy/smoke-gateway-leaf.crt"
state_file="$deploy/.bootstrap-topology.state"
compose="docker compose --env-file $deploy/.env -f $deploy/compose.yaml -f $deploy/smoke-compose.yaml"

# --teardown: 幂等清理, 即使容器已不存在也不报错
if [ "${1:-}" = "--teardown" ]; then
  if [ -f "$state_file" ]; then
    # shellcheck disable=SC1090
    . "$state_file"  # 定义 compose_project_name, mock_pid
    COMPOSE_PROJECT_NAME="$compose_project_name"
    compose_down="docker compose --env-file $deploy/.env -f $deploy/compose.yaml -f $deploy/smoke-compose.yaml"
    $compose_down down --volumes --remove-orphans >/dev/null 2>&1 || true
    if [ -n "${mock_pid:-}" ]; then kill "$mock_pid" >/dev/null 2>&1 || true; fi
    rm -f "$state_file"
    echo "topology torn down (project=$compose_project_name)"
  else
    echo "no bootstrap state file; nothing to tear down"
  fi
  exit 0
fi

# Boot 失败时清理部分拓扑, 避免泄漏容器; 成功时保留拓扑供后续 step 使用.
cleanup_on_failure() {
  status=$?
  trap - EXIT INT TERM
  if [ "$status" -ne 0 ]; then
    $compose logs --no-color > "$deploy/topology-bootstrap.log" 2>&1 || true
    $compose down --volumes --remove-orphans >/dev/null 2>&1 || true
    if [ -n "${mock_pid:-}" ]; then kill "$mock_pid" >/dev/null 2>&1 || true; fi
    rm -f "$state_file"
  fi
  exit "$status"
}
trap cleanup_on_failure EXIT INT TERM

# Wait for engine + gateway to be truly ready after `compose up`.
#
# Three-stage probe in smoke-ci.sh (only stages 1-2 needed here):
#   1. `/health` returns `engine:"ok"` — axum is listening.
#   2. `GET /v1/models` returns 200 — gateway → engine → provider egress path
#      can serve a real request, not just the health route.
#   3. (optional) `POST /v1/chat/completions` SSE round-trip — only needed for
#      restart-continuity smoke; not used by agent exploration.
#
# Stages 1-2 close the listener race from PR #243 run 29671033343.
#
# B7 修复：失败时 dump engine + gateway + mock_provider 日志到 stderr,
# 让 CI annotations 直接看到根因而不用手挖 artifact。
wait_for_engine_ready() {
  health_ready=0
  for _ in $(seq 1 60); do
    if $curl_tls --user "$admin_user:$admin_password" --fail "$origin/health" 2>/dev/null | grep -q '"engine":"ok"'; then
      health_ready=1
      break
    fi
    sleep 1
  done
  if [ "$health_ready" -ne 1 ]; then
    echo "wait_for_engine_ready: /health did not reach engine:\"ok\" within 60s" >&2
    echo "----- engine logs (last 200) -----" >&2
    $compose logs --no-color --tail=200 engine >&2 2>&1 || true
    echo "----- gateway logs (last 200) -----" >&2
    $compose logs --no-color --tail=200 gateway >&2 2>&1 || true
    echo "----- mock_provider logs -----" >&2
    if [ -f "$mock_log" ]; then tail -n 200 "$mock_log" >&2 2>&1 || true; fi
    echo "----- end dump -----" >&2
    return 1
  fi
  models_ready=0
  for _ in $(seq 1 30); do
    if $curl_tls --user "$admin_user:$admin_password" --fail "$origin/v1/models" >/dev/null 2>&1; then
      models_ready=1
      break
    fi
    sleep 1
  done
  if [ "$models_ready" -ne 1 ]; then
    echo "wait_for_engine_ready: /v1/models did not return 200 within 30s" >&2
    echo "----- engine logs (last 200) -----" >&2
    $compose logs --no-color --tail=200 engine >&2 2>&1 || true
    echo "----- gateway logs (last 200) -----" >&2
    $compose logs --no-color --tail=200 gateway >&2 2>&1 || true
    echo "----- mock_provider logs -----" >&2
    if [ -f "$mock_log" ]; then tail -n 200 "$mock_log" >&2 2>&1 || true; fi
    echo "----- end dump -----" >&2
    return 1
  fi
}

mkdir -p "$deploy/secrets" "$deploy/certs"

# B8 修复（方案 C 临时 umask）：
# smoke-ci.sh 也有 umask 077，但 pr-gate.yml 在 smoke 前会先跑
# `Create synthetic deployment inputs` step 预创建 secrets（默认 umask 022 = 0644）。
# smoke-ci.sh 用 `>` 覆盖这些文件时 shell 保留原 0644 权限，engine uid 65532 能读。
# 但 bootstrap-topology.sh 是独立调用，没有这个隐性前置，secrets 会是 0600，
# engine entrypoint 报 "missing required secret" 并无限重启。
# 用临时 umask 切换：TLS 私钥段保留 077（与 smoke-ci.sh 对齐），secrets 段切回 022。
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

# secrets 段切回默认 umask：engine 容器 uid 65532 需要读取 bind-mount 的 secret。
umask 022
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
node "$repo/deploy/production/mock-provider.js" > "$mock_log" 2>&1 &
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
curl_tls="curl --silent --show-error --connect-timeout 5 --max-time 30 --cacert $root_ca"

# Health ready probe (stages 1-2 of smoke-ci.sh wait_for_engine_ready)
wait_for_engine_ready

# 写状态文件供 --teardown 复用 (bootstrap 和 teardown 是不同 shell 进程, $$ 不同)
cat > "$state_file" <<EOF
smoke_id=$smoke_id
compose_project_name=$COMPOSE_PROJECT_NAME
mock_pid=$mock_pid
EOF

echo "topology bootstrapped (project=$COMPOSE_PROJECT_NAME origin=$origin)"
