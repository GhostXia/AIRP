#!/usr/bin/env bash
# AIRP WebUI launcher for Linux (portable musl build).
# Mirrors deploy/windows-webui/Start-AIRP.cmd.
set -euo pipefail

AIRP_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
AIRP_DATA_DIR="$AIRP_ROOT/data"
export AIRP_PERSIST_PROVIDER_KEY=true
export AIRP_ALLOW_LOCAL_PATH=false

# Defensive: clear inherited production env so the launcher never accidentally
# starts as a production node (mirrors Start-AIRP.cmd's unset behavior).
unset AIRP_ACCESS_KEY
unset AIRP_DEPLOYMENT_MODE
unset AIRP_PUBLIC_ORIGIN
unset AIRP_CORS_ORIGINS

# NOTE: --open-browser is rejected by the engine on non-Windows platforms
# (engine/src/main.rs). The engine prints the WebUI URL on startup; the user
# opens it manually.

if [[ ! -x "$AIRP_ROOT/airp-core" ]]; then
    echo "Missing airp-core in $AIRP_ROOT" >&2
    exit 1
fi
if [[ ! -f "$AIRP_ROOT/webui/index.html" ]]; then
    echo "Missing webui/index.html in $AIRP_ROOT" >&2
    exit 1
fi

mkdir -p "$AIRP_DATA_DIR"

echo "Starting AIRP WebUI at http://127.0.0.1:8765"
echo "User data stays in $AIRP_DATA_DIR"
echo "Close this terminal or press Ctrl+C to stop AIRP."
echo ""

exec "$AIRP_ROOT/airp-core" \
    --config "$AIRP_ROOT/config.json" \
    daemon \
    --host 127.0.0.1 \
    --port 8765 \
    --webui-dir "$AIRP_ROOT/webui"
