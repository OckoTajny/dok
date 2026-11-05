#!/usr/bin/env bash
# Preview deploys: prod stays live, previews get their own URL, rollback still works.
set -euo pipefail
cd "$(dirname "$0")/.."

BIN=${BIN:-$PWD/target/debug/dok}
PORT=${PORT:-18088}
APP=previewapp
DATA=$(mktemp -d)
SRC=$(mktemp -d)
export DOK_SERVER="http://127.0.0.1:$PORT"
export DOK_TOKEN="smoke-token"

"$BIN" serve --http-port "$PORT" --public-host 127.0.0.1 --base-domain 127.0.0.1 \
  --token "$DOK_TOKEN" --data "$DATA" >/dev/null 2>&1 &
PID=$!
trap 'kill $PID 2>/dev/null; rm -rf "$DATA" "$SRC"' EXIT
for _ in $(seq 1 50); do
  curl -fsS "$DOK_SERVER/api/v1/health" >/dev/null 2>&1 && break
  sleep 0.1
done
python3 scripts/seed-user.py "$DATA" smoke@example.com Smoke "$DOK_TOKEN" >/dev/null

printf '<h1>production v1</h1>\n' > "$SRC/index.html"
"$BIN" push "$SRC" --app "$APP" >/dev/null

last_url=""
for i in 1 2 3 4 5; do
  printf '<h1>preview %s</h1>\n' "$i" > "$SRC/index.html"
  out=$("$BIN" push "$SRC" --app "$APP" --env preview)
  last_url=$(echo "$out" | python3 -c 'import sys,json;print(json.load(sys.stdin)["url"])')
  echo "$last_url" | grep -q "$APP-" || { echo "bad preview url: $last_url" >&2; exit 1; }
done

# Preview deployments must not touch production.
curl -fsS -H "Host: $APP.127.0.0.1" "http://127.0.0.1:$PORT/" | grep -q 'production v1'

# The newest preview URL serves the preview artifact on its own host.
host=$(echo "$last_url" | python3 -c 'import sys,urllib.parse;print(urllib.parse.urlparse(sys.stdin.read().strip()).hostname)')
curl -fsS -H "Host: $host" "http://127.0.0.1:$PORT/" | grep -q 'preview 5'

# Production rollback still works after five previews.
printf '<h1>production v2</h1>\n' > "$SRC/index.html"
"$BIN" push "$SRC" --app "$APP" >/dev/null
for _ in $(seq 1 30); do
  curl -fsS -H "Host: $APP.127.0.0.1" "http://127.0.0.1:$PORT/" 2>/dev/null | grep -q 'production v2' && break
  sleep 0.2
done
old=$("$BIN" deploys "$APP" | python3 -c 'import sys,json
rows=[d for d in json.load(sys.stdin)["deploys"] if d["env"]=="production"]
print(rows[1]["id"])')
"$BIN" rollback "$APP" "$old" >/dev/null
for _ in $(seq 1 30); do
  curl -fsS -H "Host: $APP.127.0.0.1" "http://127.0.0.1:$PORT/" 2>/dev/null | grep -q 'production v1' && break
  sleep 0.2
done
curl -fsS -H "Host: $APP.127.0.0.1" "http://127.0.0.1:$PORT/" | grep -q 'production v1'

echo "preview smoke: OK"
