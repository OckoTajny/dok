#!/usr/bin/env bash
# Browser e2e: register a virtual passkey, log out, log back in.
set -euo pipefail
cd "$(dirname "$0")/.."

BIN=${BIN:-$PWD/target/debug/dok}
PORT=${PORT:-18084}
DATA=$(mktemp -d)

"$BIN" serve --http-port "$PORT" --public-host 127.0.0.1 --base-domain 127.0.0.1 \
  --dashboard-origin "http://localhost:$PORT" --token smoke-token --data "$DATA" &
PID=$!
trap 'kill $PID 2>/dev/null; rm -rf "$DATA"' EXIT

for _ in $(seq 1 50); do
  curl -fsS "http://localhost:$PORT/api/v1/health" >/dev/null 2>&1 && break
  sleep 0.1
done

node scripts/e2e-auth.js "http://localhost:$PORT"
