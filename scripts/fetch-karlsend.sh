#!/usr/bin/env bash
# Place a karlsend binary where Tauri expects the sidecar:
#   src-tauri/binaries/karlsend-<target-triple>[.exe]
#
# Usage:
#   scripts/fetch-karlsend.sh /path/to/karlsend            # use a local build
#   KARLSEND_URL=https://.../karlsend.zip scripts/fetch-karlsend.sh   # download a release archive
set -euo pipefail

cd "$(dirname "$0")/.."
TRIPLE="$(rustc -vV | sed -n 's/^host: //p')"
EXT=""
[[ "$TRIPLE" == *windows* ]] && EXT=".exe"
DEST="src-tauri/binaries/karlsend-${TRIPLE}${EXT}"
mkdir -p src-tauri/binaries

if [[ $# -ge 1 ]]; then
  cp "$1" "$DEST"
elif [[ -n "${KARLSEND_URL:-}" ]]; then
  TMP="$(mktemp -d)"
  trap 'rm -rf "$TMP"' EXIT
  curl -fsSL "$KARLSEND_URL" -o "$TMP/pkg"
  case "$KARLSEND_URL" in
    *.zip) unzip -q "$TMP/pkg" -d "$TMP/x" ;;
    *.tar.gz|*.tgz) mkdir -p "$TMP/x" && tar -xzf "$TMP/pkg" -C "$TMP/x" ;;
    *) mkdir -p "$TMP/x" && mv "$TMP/pkg" "$TMP/x/karlsend${EXT}" ;;
  esac
  BIN="$(find "$TMP/x" -type f -name "karlsend${EXT}" | head -n1)"
  [[ -n "$BIN" ]] || { echo "karlsend${EXT} not found in archive" >&2; exit 1; }
  cp "$BIN" "$DEST"
else
  echo "Give a local karlsend path or set KARLSEND_URL (rusty-karlsen v3.3.1 release asset; its binary reports \"karlsend 3.3.0\")." >&2
  exit 1
fi

chmod +x "$DEST"
echo "sidecar ready: $DEST"
"$DEST" --version || true
