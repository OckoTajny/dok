#!/bin/sh
# Push the generated router file to the edge Traefik on ku-bot.
set -e
SRC="$HOME/.local/share/dok/traefik/dok-apps.yaml"
[ -f "$SRC" ] || exit 0
scp -q -o BatchMode=yes "$SRC" ku-bot:/tmp/dok-apps.yaml
ssh -o BatchMode=yes ku-bot 'sudo install -m644 /tmp/dok-apps.yaml /data/coolify/proxy/dynamic/dok-apps.yaml'
