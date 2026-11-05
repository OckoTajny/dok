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

echo "git smoke: OK"
