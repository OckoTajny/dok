#!/usr/bin/env bash
# Dashboard/auth endpoints: login page, WebAuthn options, status.
set -euo pipefail
cd "$(dirname "$0")/.."

BIN=${BIN:-target/debug/dok}
PORT=${PORT:-18083}
DATA=$(mktemp -d)

"$BIN" serve --http-port "$PORT" --public-host 127.0.0.1 --base-domain 127.0.0.1 \
  --dashboard-origin "http://localhost:$PORT" --token smoke-token --data "$DATA" &
PID=$!
trap 'kill $PID 2>/dev/null; rm -rf "$DATA"' EXIT

for _ in $(seq 1 50); do
  curl -fsS "http://127.0.0.1:$PORT/api/v1/health" >/dev/null 2>&1 && break
  sleep 0.1
done

page=$(curl -fsS "http://localhost:$PORT/")
echo "$page" | grep -q 'Create a passkey'

opts=$(curl -fsS -X POST -H 'content-type: application/json' \
  --data '{"email":"smoke@example.com","name":"Smoke","team":"Smoke"}' \
  "http://localhost:$PORT/auth/setup/start")
echo "$opts" | grep -q '"publicKey"'
echo "$opts" | grep -q '"challenge"'
echo "$opts" | grep -q '"id"'

status=$(curl -fsS "http://localhost:$PORT/auth/status")
echo "$status" | grep -q '"authenticated":false'
echo "$status" | grep -q '"first_run":true'

code=$(curl -s -o /dev/null -w '%{http_code}' "http://localhost:$PORT/dashboard")
test "$code" = "303"

echo "auth smoke: OK"
