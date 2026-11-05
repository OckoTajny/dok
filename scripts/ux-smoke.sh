#!/usr/bin/env bash
# Browser e2e for the dashboard UX (env secrets, logs, teams, errors, mobile).
set -euo pipefail
cd "$(dirname "$0")/.."

BIN=${BIN:-$PWD/target/debug/dok}
PORT=${PORT:-18085}
DATA=$(mktemp -d)
STATE=$(mktemp -u).json
URL="http://localhost:$PORT"

start() {
  "$BIN" serve --http-port "$PORT" --public-host 127.0.0.1 --base-domain 127.0.0.1 \
    --dashboard-origin "$URL" --token smoke-token --data "$DATA" &
  PID=$!
  for _ in $(seq 1 50); do
    curl -fsS "$URL/api/v1/health" >/dev/null 2>&1 && return
    sleep 0.1
  done
}

cleanup() {
  kill "$PID" 2>/dev/null || true
  rm -rf "$DATA" "$STATE"
}
trap cleanup EXIT

start
node scripts/e2e-register.js "$URL" "$STATE"
DOK_SERVER="$URL" DOK_TOKEN=smoke-token "$BIN" push examples/hello --app uxapp >/dev/null
node scripts/e2e-ux.js "$URL" "$STATE"

# A foreign host never exposes the dashboard or the project list.
code=$(curl -s -o /tmp/dok-ux-foreign.html -w '%{http_code}' -H 'Host: foreign.test' "$URL/dashboard")
[ "$code" = "404" ] || { echo "foreign dashboard host: $code" >&2; exit 1; }
grep -q 'http-equiv\|Not found' /tmp/dok-ux-foreign.html
grep -q 'E2E Team' /tmp/dok-ux-foreign.html && { echo "foreign host leaked team names" >&2; exit 1; }
grep -q 'uxapp' /tmp/dok-ux-foreign.html && { echo "foreign host leaked project names" >&2; exit 1; }

# Missing static files get a branded 404 with the app name and request id.
code=$(curl -s -o /tmp/dok-ux-404.html -w '%{http_code}' -H 'Host: uxapp.127.0.0.1' "$URL/missing.js")
[ "$code" = "404" ] || { echo "branded 404: $code" >&2; exit 1; }
grep -q 'Project <strong>uxapp' /tmp/dok-ux-404.html
grep -q 'Request id' /tmp/dok-ux-404.html
rid=$(curl -s -D - -o /dev/null -H 'Host: uxapp.127.0.0.1' "$URL/missing.js" | tr -d '\r' | awk -F': ' 'tolower($1)=="x-request-id"{print $2}')
[ -n "$rid" ] || { echo "missing x-request-id header" >&2; exit 1; }

echo "ux: OK"
