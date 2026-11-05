#!/usr/bin/env bash
# Rollback + retention: 4 deploys stay within the 10-build floor, rollback works.
set -euo pipefail
cd "$(dirname "$0")/.."

BIN=${BIN:-$PWD/target/debug/dok}
PORT=${PORT:-18087}
APP=rbapp
DATA=$(mktemp -d)
SRC=$(mktemp -d)
export DOK_SERVER="http://127.0.0.1:$PORT"
export DOK_TOKEN="smoke-token"

"$BIN" serve --http-port "$PORT" --public-host 127.0.0.1 --base-domain 127.0.0.1 --token "$DOK_TOKEN" --data "$DATA" &
PID=$!
trap 'kill $PID 2>/dev/null; rm -rf "$DATA" "$SRC"' EXIT

for _ in $(seq 1 50); do
  curl -fsS "$DOK_SERVER/api/v1/health" >/dev/null 2>&1 && break
  sleep 0.1
done
python3 scripts/seed-user.py "$DATA" smoke@example.com Smoke "$DOK_TOKEN" >/dev/null

for v in 1 2 3 4; do
  printf '<h1>version %s</h1>\n' "$v" > "$SRC/index.html"
  "$BIN" push "$SRC" --app "$APP" >/dev/null
done

curl -fsS -H "Host: $APP.127.0.0.1" "http://127.0.0.1:$PORT/" | grep -q 'version 4'

count=$(find "$DATA/builds/$APP" -mindepth 1 -maxdepth 1 -type d | wc -l)
test "$count" -eq 4

ids=$("$BIN" deploys "$APP" | python3 -c 'import sys,json;print(" ".join(d["id"] for d in json.load(sys.stdin)["deploys"]))')
oldest=$(echo "$ids" | awk '{print $3}')
test -n "$oldest"

"$BIN" rollback "$APP" "$oldest" >/dev/null
curl -fsS -H "Host: $APP.127.0.0.1" "http://127.0.0.1:$PORT/" | grep -q 'version 2'

echo "rollback smoke: OK"
