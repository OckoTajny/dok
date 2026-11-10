#!/usr/bin/env bash
# Rebuild the visual-review screenshots in docs/design/visual/.
# Needs Playwright in NODE_PATH (see README: npm --prefix /tmp/dok-e2e install playwright).
set -euo pipefail
cd "$(dirname "$0")/.."

BIN=${BIN:-$PWD/target/debug/dok}
PORT=${PORT:-18092}
DATA=$(mktemp -d)
STATE=$(mktemp -u).json
SAMPLES=$(mktemp -d)
URL="http://localhost:$PORT"

cleanup() {
  [ -n "${PID:-}" ] && kill "$PID" 2>/dev/null || true
  rm -rf "$DATA" "$STATE" "$SAMPLES"
}
trap cleanup EXIT

"$BIN" serve --http-port "$PORT" --public-host 127.0.0.1 --base-domain 127.0.0.1 \
  --dashboard-origin "$URL" --token smoke-token --data "$DATA" >/dev/null 2>&1 &
PID=$!
for _ in $(seq 1 50); do
  curl -fsS "$URL/api/v1/health" >/dev/null 2>&1 && break
  sleep 0.1
done

node scripts/e2e-register.js "$URL" "$STATE"

mkdir -p "$SAMPLES/landing" "$SAMPLES/console" "$SAMPLES/docs"
for app in landing console docs; do
  printf '<!doctype html><title>%s</title><h1>%s</h1>\n' "$app" "$app" > "$SAMPLES/$app/index.html"
done

export DOK_SERVER="$URL" DOK_TOKEN=smoke-token
"$BIN" push "$SAMPLES/landing" --app landing >/dev/null
"$BIN" push "$SAMPLES/console" --app console >/dev/null
"$BIN" push "$SAMPLES/docs" --app docs >/dev/null
"$BIN" push "$SAMPLES/landing" --app landing --env preview >/dev/null
"$BIN" env landing PUBLIC_URL=https://landing.example.com >/dev/null
"$BIN" domain landing staging.landing.example.com >/dev/null

# Empty-state screenshot: a second team with no projects.
curl -fsS -X POST "$URL/api/v1/teams" \
  -H "Authorization: Bearer smoke-token" -H 'content-type: application/json' \
  -d '{"name":"Studio"}' >/dev/null

node scripts/design-shots.js "$URL" "$STATE"
