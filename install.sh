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

: "${DOMAIN:?set DOMAIN=dok.example.com (or DOMAIN=localhost for a local instance)}"
: "${BASE_DOMAIN:?set BASE_DOMAIN=apps.example.com}"
PORT=${PORT:-8080}
SCHEME=${SCHEME:-https}
if [ "$DOMAIN" = "localhost" ] || [ "$DOMAIN" = "127.0.0.1" ]; then SCHEME=http; fi
ORIGIN=${ORIGIN:-$SCHEME://$DOMAIN}
SELF_EDGE=${SELF_EDGE:-}
TRUSTED_PROXY=${TRUSTED_PROXY:-}
ACME_EMAIL=${ACME_EMAIL:-}
EDGE=${EDGE:-}
DATA=${DATA:-$HOME/.local/share/dok}
TRAEFIK_DIR=${TRAEFIK_DIR:-$DATA/traefik}
BIN=${BIN:-$HOME/bin/dok}

say() { printf '\033[1;36m▌\033[0m %s\n' "$*"; }

# 1. binary: DOK_BIN_URL downloads a prebuilt release; otherwise build locally.
mkdir -p "$HOME/bin" "$HOME/.config/systemd/user" "$HOME/.config/dok" "$TRAEFIK_DIR"
if [ -n "${DOK_BIN_URL:-}" ]; then
  say "downloading prebuilt binary"
  curl -fsSL "$DOK_BIN_URL" -o "$BIN.tmp"
  chmod 755 "$BIN.tmp"
  mv "$BIN.tmp" "$BIN"
elif [ -z "${SKIP_BUILD:-}" ]; then
  command -v cargo >/dev/null || {
    echo "cargo not found: install Rust (https://rustup.rs) or set DOK_BIN_URL to a prebuilt dok binary" >&2
    exit 1
  }
  say "building release"
  cargo build --release
  install -m755 target/release/dok "$BIN"
fi
mkdir -p "$HOME/bin" "$HOME/.config/systemd/user" "$HOME/.config/dok" "$TRAEFIK_DIR"

# 2. service unit
say "installing dok.service for $ORIGIN"
sed \
  -e "s#--http-port 8080#--http-port $PORT#" \
  -e "s#--public-host dok.solta.tech#--public-host $DOMAIN#" \
  -e "s#--dashboard-origin https://dok.solta.tech#--dashboard-origin $ORIGIN#" \
  -e "s#--base-domain apps.solta.tech#--base-domain $BASE_DOMAIN#" \
  -e "s#%h/.local/share/dok/traefik#$TRAEFIK_DIR#" \
  deploy/dok-desktop.service > "$HOME/.config/systemd/user/dok.service"

# Optional self-contained TLS edge and trusted proxy.
if [ -n "$SELF_EDGE" ] || [ -n "$TRUSTED_PROXY" ]; then
  awk -v edge="$SELF_EDGE" -v dom="$DOMAIN" -v mail="$ACME_EMAIL" -v proxy="$TRUSTED_PROXY" '
    /^  --data / {
      if (edge != "") {
        print "  --edge \\"
        print "  --acme-domain " dom " \\"
        if (mail != "") print "  --acme-email " mail " \\"
      }
      if (proxy != "") print "  --trusted-proxy " proxy " \\"
    }
    { print }
  ' "$HOME/.config/systemd/user/dok.service" > "$HOME/.config/systemd/user/dok.service.tmp"
  mv "$HOME/.config/systemd/user/dok.service.tmp" "$HOME/.config/systemd/user/dok.service"
fi
if [ -n "$SELF_EDGE" ] && command -v setcap >/dev/null; then
  sudo setcap 'cap_net_bind_service=+ep' "$BIN" || echo "warning: could not setcap $BIN; run edge as root or set net.ipv4.ip_unprivileged_port_start=80" >&2
fi

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
echo "  dashboard: $ORIGIN"
echo "  apps:      $SCHEME://<app>.$BASE_DOMAIN"
if [ -f "$DATA/initial-token" ]; then
  echo "  token:     $(cat "$DATA/initial-token")"
  echo "  next:      DOK_TOKEN=\$(cat $DATA/initial-token) dok login $ORIGIN"
fi
