#!/usr/bin/env bash
# Install dok on this machine as a systemd user service.
#
#   ./install.sh
#   DOMAIN=dok.example.com BASE_DOMAIN=apps.example.com ./install.sh
#   EDGE=ku-bot ./install.sh        # also sync per-app routers to an edge Traefik
#
# Idempotent: re-running upgrades the binary and restarts the service.
set -euo pipefail
cd "$(dirname "$0")"

DOMAIN=${DOMAIN:-dok.solta.tech}
BASE_DOMAIN=${BASE_DOMAIN:-apps.solta.tech}
PORT=${PORT:-8080}
EDGE=${EDGE:-}
DATA=${DATA:-$HOME/.local/share/dok}
TRAEFIK_DIR=${TRAEFIK_DIR:-$DATA/traefik}
BIN=${BIN:-$HOME/bin/dok}

say() { printf '\033[1;36m▌\033[0m %s\n' "$*"; }

# 1. binary
if [ -z "${SKIP_BUILD:-}" ]; then
  say "building release"
  cargo build --release
  install -m755 target/release/dok "$BIN"
fi
mkdir -p "$HOME/bin" "$HOME/.config/systemd/user" "$HOME/.config/dok" "$TRAEFIK_DIR"

# 2. service unit
say "installing dok.service"
sed \
  -e "s#--http-port 8080#--http-port $PORT#" \
  -e "s#--public-host dok.solta.tech#--public-host $DOMAIN#" \
  -e "s#--dashboard-origin https://dok.solta.tech#--dashboard-origin https://$DOMAIN#" \
  -e "s#--base-domain apps.solta.tech#--base-domain $BASE_DOMAIN#" \
  -e "s#%h/.local/share/dok/traefik#$TRAEFIK_DIR#" \
  deploy/dok-desktop.service > "$HOME/.config/systemd/user/dok.service"

# 3. hook secret
if [ ! -f "$HOME/.config/dok/env" ]; then
  say "generating DOK_HOOK_SECRET"
  printf 'DOK_HOOK_SECRET=%s\n' "$(openssl rand -hex 16)" > "$HOME/.config/dok/env"
  chmod 600 "$HOME/.config/dok/env"
fi

systemctl --user daemon-reload
systemctl --user enable dok >/dev/null
if systemctl --user is-active --quiet dok; then
  systemctl --user restart dok
else
  systemctl --user start dok
fi
sleep 2
systemctl --user is-active dok

# 4. optional edge router sync
if [ -n "$EDGE" ]; then
  say "configuring router sync to $EDGE"
  sed "s#ku-bot#$EDGE#g" deploy/dok-traefik-sync.sh > "$HOME/bin/dok-traefik-sync"
  chmod +x "$HOME/bin/dok-traefik-sync"
  install -m644 deploy/dok-traefik-sync.service deploy/dok-traefik-sync.path \
    "$HOME/.config/systemd/user/"
  systemctl --user daemon-reload
  systemctl --user enable dok-traefik-sync.path >/dev/null
  systemctl --user start dok-traefik-sync.path || true
  systemctl --user start dok-traefik-sync.service || true
fi

say "dok is running"
echo "  dashboard: https://$DOMAIN"
echo "  apps:      https://<app>.$BASE_DOMAIN"
if [ -f "$DATA/initial-token" ]; then
  echo "  token:     $(cat "$DATA/initial-token")"
  echo "  next:      dok login https://$DOMAIN \$(cat $DATA/initial-token)"
fi
