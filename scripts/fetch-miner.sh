#!/usr/bin/env bash
# Place the KarlsenMiner sidecar where Tauri expects it:
#   src-tauri/binaries/karlsenminer-<target-triple>[.exe]
#
# The archive is downloaded from the official KarlsenMiner release on GitHub. The archive
# and the executable inside it must match the SHA-256 values pinned below, otherwise
# nothing is installed.
#
# Usage:
#   scripts/fetch-miner.sh                 # download the pinned release
#   scripts/fetch-miner.sh /dir            # use the archive already in /dir (same file name)
#
# Updating the pin: change MINER_VERSION, the archive hashes (SHA256SUMS.txt of the release)
# and the executable hashes (`sha256sum` of the extracted file) for BOTH platforms, and
# MINER_VERSION in src-tauri/src/miner.rs. Desktop needs KarlsenMiner 3.3.0 or newer
# (solo mining with --node); src-tauri/src/miner.rs reads some of its output lines.
set -euo pipefail

MINER_VERSION="3.3.0"

cd "$(dirname "$0")/.."
TRIPLE="$(rustc -vV | sed -n 's/^host: //p')"
LOCAL_DIR="${1:-}"

case "$TRIPLE" in
  x86_64-*windows*)
    EXT=".exe"
    ARCHIVE="karlsenminer-v${MINER_VERSION}-windows.zip"
    ARCHIVE_SHA256="79e036878ceaff3cdabe62a05da06d0fd11789fd0a9a4d379e668d0855af0773"
    BIN_SHA256="11393f29ce01858d5ae2a8ce4e07f416fcd33849f7e95d1024678fb5cee560ac"
    ;;
  x86_64-*linux*)
    EXT=""
    ARCHIVE="karlsenminer-v${MINER_VERSION}-linux.tar.gz"
    ARCHIVE_SHA256="2019834c6f40d2643176c43741ba5cfb7cc340e66de5bbd9c26dd76a9e75cbe5"
    BIN_SHA256="ff028297c66392976cf41942865360cf25f73c272e48f999c13ce49529ef06ed"
    ;;
  *)
    echo "No KarlsenMiner release for target $TRIPLE (Windows x64 and Linux x64 only)." >&2
    exit 1
    ;;
esac

URL="https://github.com/karlsencoin/karlsen-miner/releases/download/v${MINER_VERSION}/${ARCHIVE}"

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
mkdir -p src-tauri/binaries

sha256_of() {
  sha256sum "$1" | cut -d' ' -f1
}

PKG="$TMP/$ARCHIVE"
if [[ -n "$LOCAL_DIR" ]]; then
  cp "$LOCAL_DIR/$ARCHIVE" "$PKG"
else
  echo "downloading $URL"
  curl -fsSL "$URL" -o "$PKG"
fi
actual="$(sha256_of "$PKG")"
if [[ "$actual" != "$ARCHIVE_SHA256" ]]; then
  echo "REFUSED: $ARCHIVE has SHA-256 $actual, expected $ARCHIVE_SHA256" >&2
  exit 1
fi

mkdir -p "$TMP/x"
case "$ARCHIVE" in
  *.zip) unzip -q -o "$PKG" -d "$TMP/x" ;;
  *) tar -xzf "$PKG" -C "$TMP/x" ;;
esac
BIN="$(find "$TMP/x" -type f -name "karlsenminer${EXT}" | head -n1)"
[[ -n "$BIN" ]] || { echo "karlsenminer${EXT} not found in $ARCHIVE" >&2; exit 1; }
actual="$(sha256_of "$BIN")"
if [[ "$actual" != "$BIN_SHA256" ]]; then
  echo "REFUSED: karlsenminer${EXT} has SHA-256 $actual, expected $BIN_SHA256" >&2
  exit 1
fi

DEST="src-tauri/binaries/karlsenminer-${TRIPLE}${EXT}"
cp "$BIN" "$DEST"
chmod +x "$DEST"
echo "sidecar ready: $DEST"
"$DEST" --version || true
