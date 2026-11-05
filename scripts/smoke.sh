#!/usr/bin/env bash
# End-to-end smoke test: start the server, push a static app, fetch it through the proxy.
set -euo pipefail
cd "$(dirname "$0")/.."

BIN=${BIN:-target/debug/dok}
PORT=${PORT:-18080}
DATA=$(mktemp -d)
export DOK_SERVER="http://127.0.0.1:$PORT"
export DOK_TOKEN="smoke-token"

"$BIN" serve --http-port "$PORT" --public-host 127.0.0.1 --base-domain 127.0.0.1 --token "$DOK_TOKEN" --data "$DATA" &
PID=$!
trap 'kill $PID 2>/dev/null; rm -rf "$DATA"' EXIT

for _ in $(seq 1 50); do
  curl -fsS "$DOK_SERVER/api/v1/health" >/dev/null 2>&1 && break
  sleep 0.1
done

curl -fsS "$DOK_SERVER/api/v1/health" | grep -q '"ok":true'

"$BIN" push examples/hello --app hello >/dev/null

body=$(curl -fsS -H 'Host: hello.127.0.0.1' "http://127.0.0.1:$PORT/")
echo "$body" | grep -q 'hello from dok'

"$BIN" domain hello hello.test >/dev/null
curl -fsS -H 'Host: hello.test' "http://127.0.0.1:$PORT/" | grep -q 'hello from dok'

echo "smoke: OK"
