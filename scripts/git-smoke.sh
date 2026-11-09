#!/usr/bin/env bash
# git push deploy: git-init a brand-new app, push to main, verify the site.
set -euo pipefail
cd "$(dirname "$0")/.."

BIN=${BIN:-$PWD/target/debug/dok}
PORT=${PORT:-18082}
APP=gitapp
DATA=$(mktemp -d)
WORK=$(mktemp -d)
export DOK_SERVER="http://127.0.0.1:$PORT"
export DOK_TOKEN="smoke-token"

"$BIN" serve --http-port "$PORT" --public-host 127.0.0.1 --base-domain 127.0.0.1 --token "$DOK_TOKEN" --data "$DATA" &
PID=$!
cleanup() {
  kill "$PID" 2>/dev/null || true
  rm -rf "$DATA" "$WORK"
}
trap cleanup EXIT

for _ in $(seq 1 50); do
  curl -fsS "$DOK_SERVER/api/v1/health" >/dev/null 2>&1 && break
  sleep 0.1
done
python3 scripts/seed-user.py "$DATA" smoke@example.com Smoke "$DOK_TOKEN" >/dev/null

# No prior `dok push`: git-init must create the app.
"$BIN" git-init "$APP" >/dev/null
BARE="$DATA/repos/$APP.git"

git clone -q "$BARE" "$WORK/clone" 2>/dev/null || git clone -q "$BARE" "$WORK/clone"
cd "$WORK/clone"
echo '<h1>pushed via git</h1>' > index.html
git add index.html
git -c user.name=smoke -c user.email=smoke@example.com commit -q -m "first commit"
git push -q origin HEAD:main
cd - >/dev/null

body=""
for _ in $(seq 1 40); do
  if body=$(curl -fsS -H "Host: $APP.127.0.0.1" "http://127.0.0.1:$PORT/" 2>/dev/null); then
    echo "$body" | grep -q 'pushed via git' && break
  fi
  sleep 0.25
done
echo "$body" | grep -q 'pushed via git'

# A push to a non-production branch becomes a preview; production stays live.
python3 - "$DATA" <<'PY'
import sqlite3, sys
db = sqlite3.connect(sys.argv[1] + "/dok.db")
db.execute("INSERT OR REPLACE INTO settings (key, value) VALUES (?, ?)", ("webhook_branch:gitapp", "main"))
db.commit()
PY
cd "$WORK/clone"
git checkout -q -b dev
echo '<h1>branch build</h1>' > index.html
git add index.html
git -c user.name=smoke -c user.email=smoke@example.com commit -q -m "dev branch"
git push -q origin dev
cd - >/dev/null
curl -fsS -H "Host: $APP.127.0.0.1" "http://127.0.0.1:$PORT/" | grep -q 'pushed via git'
preview_url=$(DOK_SERVER="$DOK_SERVER" DOK_TOKEN="$DOK_TOKEN" "$BIN" deploys "$APP" | python3 -c 'import sys,json
rows=[d for d in json.load(sys.stdin)["deploys"] if d["env"]=="preview"]
print(rows[0]["url"] if rows else "")')
test -n "$preview_url"
preview_host=$(python3 -c 'import sys,urllib.parse;print(urllib.parse.urlparse(sys.argv[1]).hostname)' "$preview_url")
curl -fsS -H "Host: $preview_host" "http://127.0.0.1:$PORT/" | grep -q 'branch build'

# Switching the production branch makes `dok redeploy` build dev for production.
python3 - "$DATA" <<'PY'
import sqlite3, sys
db = sqlite3.connect(sys.argv[1] + "/dok.db")
db.execute("INSERT OR REPLACE INTO settings (key, value) VALUES (?, ?)", ("webhook_branch:gitapp", "dev"))
db.commit()
PY
DOK_SERVER="$DOK_SERVER" DOK_TOKEN="$DOK_TOKEN" "$BIN" redeploy "$APP" >/dev/null
body=""
for _ in $(seq 1 40); do
  if body=$(curl -fsS -H "Host: $APP.127.0.0.1" "http://127.0.0.1:$PORT/" 2>/dev/null); then
    echo "$body" | grep -q 'branch build' && break
  fi
  sleep 0.25
done
echo "$body" | grep -q 'branch build'

echo "git smoke: OK"
