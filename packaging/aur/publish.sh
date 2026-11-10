#!/usr/bin/env bash
# Publish both AUR packages. Requires an AUR account and this machine's
# ~/.ssh/aur key added at https://aur.archlinux.org (My Account → SSH keys).
set -euo pipefail
cd "$(dirname "$0")"
WORK="${WORK:-$HOME/.cache/aur-publish}"
mkdir -p "$WORK"

publish() {
  local name="$1" src="$PWD/$1"
  local dir="$WORK/$name"
  if [ ! -d "$dir/.git" ]; then
    rm -rf "$dir"
    git clone "ssh://aur@aur.archlinux.org/$name.git" "$dir" 2>/dev/null || {
      mkdir -p "$dir"; git -C "$dir" init -q; git -C "$dir" remote add origin "ssh://aur@aur.archlinux.org/$name.git";
    }
  fi
  cp "$src/PKGBUILD" "$src/.SRCINFO" "$dir/"
  git -C "$dir" add PKGBUILD .SRCINFO
  git -C "$dir" -c user.name="OckoTajny" -c user.email="o2t2j2s2@gmail.com" \
    commit -q -m "Update to $(grep -m1 '^pkgver=' PKGBUILD | cut -d= -f2)" || true
  git -C "$dir" push origin master 2>/dev/null || git -C "$dir" push origin main
  echo "published $name"
}

publish dok
publish dok-git
