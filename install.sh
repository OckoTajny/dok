#!/bin/sh
# dok installer: curl -fsSL https://dok.solta.tech/install.sh | sh
set -eu

REPO="${DOK_REPO:-OckoTajny/dok}"
BIN_DIR="${DOK_BIN_DIR:-$HOME/.local/bin}"
VERSION="${DOK_VERSION:-latest}"

os="$(uname -s | tr '[:upper:]' '[:lower:]')"
arch="$(uname -m)"
case "$arch" in
  x86_64|amd64) arch="x86_64" ;;
  aarch64|arm64) arch="aarch64" ;;
  *) echo "dok: unsupported architecture: $arch" >&2; exit 1 ;;
esac
case "$os" in
  linux|darwin) ;;
  *) echo "dok: unsupported OS: $os" >&2; exit 1 ;;
esac

if [ "$VERSION" = "latest" ]; then
  url="https://github.com/$REPO/releases/latest/download/dok-$os-$arch"
else
  url="https://github.com/$REPO/releases/download/$VERSION/dok-$os-$arch"
fi

command -v curl >/dev/null 2>&1 || { echo "dok: curl is required" >&2; exit 1; }
mkdir -p "$BIN_DIR"
tmp="$(mktemp)"
trap 'rm -f "$tmp"' EXIT

echo "downloading $url"
if ! curl -fsSL "$url" -o "$tmp"; then
  echo "dok: no prebuilt binary for $os/$arch; building from source" >&2
  command -v cargo >/dev/null 2>&1 || { echo "dok: install Rust (https://rustup.rs) or use the AUR package" >&2; exit 1; }
  cargo install --git "https://github.com/$REPO.git" --locked --root "${DOK_INSTALL_ROOT:-$HOME/.local}" dok
  echo "installed $BIN_DIR/dok"
  exit 0
fi

chmod +x "$tmp"
mv "$tmp" "$BIN_DIR/dok"
echo "installed $BIN_DIR/dok ($("$BIN_DIR/dok" --version 2>/dev/null || echo dok))"
case ":$PATH:" in
  *":$BIN_DIR:"*) ;;
  *) echo "add it to PATH: export PATH=\"$BIN_DIR:\$PATH\"" ;;
esac
echo
echo "next:"
echo "  1. create an API token in the dashboard (Account > API tokens)"
echo "  2. dok login https://your-dok-server <token>"
echo "  3. dok whoami   # verifies the account"
echo "  4. dok push ./my-app"
