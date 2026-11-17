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

MINER_VERSION="3.3.1"

cd "$(dirname "$0")/.."
TRIPLE="$(rustc -vV | sed -n 's/^host: //p')"
LOCAL_DIR="${1:-}"

case "$TRIPLE" in
  x86_64-*windows*)
    EXT=".exe"
    ARCHIVE="karlsenminer-v${MINER_VERSION}-windows.zip"
    ARCHIVE_SHA256="7c0e6423e20484f163384be455a2aa610bc5da0f4b7a7cd182cf23e6ab4a4dd9"
    BIN_SHA256="bde42858e7eb25453476f77bc2aaf307cb3b01c8ea9ea37b749b657ab50275c8"
    ;;
  x86_64-*linux*)
    EXT=""
    ARCHIVE="karlsenminer-v${MINER_VERSION}-linux.tar.gz"
    ARCHIVE_SHA256="143767267604bb8adf3b0fa06407f3ec6f5225770c4e1b5ae223af7308e07d94"
    BIN_SHA256="74e78293a6f683e95ccaa40ff841058d82d5e94b41df855f1968b84d1a4d8940"
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
