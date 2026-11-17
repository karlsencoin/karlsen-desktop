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

MINER_VERSION="3.2.1"

cd "$(dirname "$0")/.."
TRIPLE="$(rustc -vV | sed -n 's/^host: //p')"
LOCAL_DIR="${1:-}"

case "$TRIPLE" in
  x86_64-*windows*)
    EXT=".exe"
    ARCHIVE="karlsenminer-v${MINER_VERSION}-windows.zip"
    ARCHIVE_SHA256="9af4fd4976407832020788b159d962ea873e64fcff82af109c1d47d2728f9c9d"
    BIN_SHA256="12554e481b6bc7406184eea9c4ec40839b657d2b0a41cdf337a6810b6cac273c"
    ;;
  x86_64-*linux*)
    EXT=""
    ARCHIVE="karlsenminer-v${MINER_VERSION}-linux.tar.gz"
    ARCHIVE_SHA256="e730cc99d0466e24126f700896d831da9360c738c937bfc1eae475c0be908dd2"
    BIN_SHA256="0898fe364cefb188a5683a5a3f68c8aba48dce2ae531f258ac2d3e15b51973c5"
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
  # unzip exits 1 for warnings only (e.g. a zip made by PowerShell with backslash paths).
  *.zip) unzip -q -o "$PKG" -d "$TMP/x" || [[ $? -eq 1 ]] ;;
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
