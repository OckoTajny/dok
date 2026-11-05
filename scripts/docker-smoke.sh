#!/usr/bin/env bash
# Docker end-to-end test: push an app with a Dockerfile, build, run, proxy to it.
set -euo pipefail
cd "$(dirname "$0")/.."

BIN=${BIN:-target/debug/dok}
PORT=${PORT:-18089}
APPNAME=${APPNAME:-hello-docker}
DATA=$(mktemp -d)
APP=$(mktemp -d)
export DOK_SERVER="http://127.0.0.1:$PORT"
export DOK_TOKEN="smoke-token"

cat > "$APP/Dockerfile" <<'EOF'
FROM python:3-alpine
WORKDIR /app
COPY . .
EXPOSE 8000
CMD ["python", "-m", "http.server", "8000"]
EOF
echo '<h1>hello from docker</h1>' > "$APP/index.html"

"$BIN" serve --http-port "$PORT" --public-host 127.0.0.1 --base-domain 127.0.0.1 --token "$DOK_TOKEN" --data "$DATA" &
PID=$!
cleanup() {
  kill "$PID" 2>/dev/null || true
  docker rm -f "dok-$APPNAME-production" "dok-$APPNAME-preview" >/dev/null 2>&1 || true
  rm -rf "$DATA" "$APP"
}
trap cleanup EXIT

for _ in $(seq 1 50); do
  curl -fsS "$DOK_SERVER/api/v1/health" >/dev/null 2>&1 && break
  sleep 0.1
done
python3 scripts/seed-user.py "$DATA" smoke@example.com Smoke "$DOK_TOKEN" >/dev/null

"$BIN" push "$APP" --app "$APPNAME" --port 8000 >/dev/null

body=""
for _ in $(seq 1 40); do
  if body=$(curl -fsS -H "Host: $APPNAME.127.0.0.1" "http://127.0.0.1:$PORT/" 2>/dev/null); then
    break
  fi
  sleep 0.25
done
echo "$body" | grep -q 'hello from docker'

docker ps --filter "name=dok-$APPNAME-production" --format '{{.Names}}' | grep -q "dok-$APPNAME-production"

echo "docker smoke: OK"
