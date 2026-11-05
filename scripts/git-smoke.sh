#!/usr/bin/env bash
# git push deploy: create a bare repo, push a commit, verify the site changed.
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

"$BIN" push examples/hello --app "$APP" >/dev/null
curl -fsS -H "Host: $APP.127.0.0.1" "http://127.0.0.1:$PORT/" | grep -q 'hello from dok'

"$BIN" git-init "$APP" >/dev/null
BARE=$(ls -d "$DATA"/repos/"$APP".git)

git clone -q "$BARE" "$WORK/clone"
cd "$WORK/clone"
git -c user.name=smoke -c user.email=smoke@example.com commit -q --allow-empty -m init
echo '<h1>pushed via git</h1>' > index.html
git add index.html
git -c user.name=smoke -c user.email=smoke@example.com commit -q -m "change page"
git push -q origin HEAD:master 2>/dev/null || git push -q origin HEAD:main 2>/dev/null || git push -q origin HEAD
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
