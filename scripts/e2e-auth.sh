#!/usr/bin/env bash
# Browser e2e: register a passkey, restart the server, stay logged in,
# check CSRF on dashboard actions and the logs page.
set -euo pipefail
cd "$(dirname "$0")/.."

BIN=${BIN:-$PWD/target/debug/dok}
PORT=${PORT:-18084}
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
DOK_SERVER="$URL" DOK_TOKEN=smoke-token "$BIN" push examples/hello --app hello >/dev/null

node scripts/e2e-register.js "$URL" "$STATE"

kill "$PID"
wait "$PID" 2>/dev/null || true
start

node scripts/e2e-resume.js "$URL" "$STATE"

# backup was written at startup
ls "$DATA"/backups/*.db >/dev/null
echo "backup: OK"
