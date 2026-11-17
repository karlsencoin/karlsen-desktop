#!/usr/bin/env bash
# Place the mining sidecars where Tauri expects them:
#   src-tauri/binaries/karlsenminer-<target-triple>[.exe]            KarlsenMiner (GPU miner)
#   src-tauri/binaries/karlsen-stratum-bridge-<target-triple>[.exe]  stratum <-> karlsend bridge
#
# Both are downloaded from their official GitHub releases. The archive and the executable
# inside it must match the SHA-256 values pinned below, otherwise nothing is installed.
#
# Usage:
#   scripts/fetch-miner.sh                 # download the pinned releases
#   scripts/fetch-miner.sh /dir            # use archives already in /dir (same file names)
#
# Updating a pin: change the version, the archive hash (from the release's checksum file
# or `sha256sum <archive>`) and the executable hash (`sha256sum` of the extracted file) for
# BOTH platforms, and the matching constant in src-tauri/src/miner.rs
# (MINER_VERSION / BRIDGE_VERSION).
set -euo pipefail

MINER_VERSION="3.2.0"
BRIDGE_VERSION="3.3.2"

cd "$(dirname "$0")/.."
TRIPLE="$(rustc -vV | sed -n 's/^host: //p')"
LOCAL_DIR="${1:-}"

case "$TRIPLE" in
  x86_64-*windows*)
    EXT=".exe"
    MINER_ARCHIVE="karlsenminer-v${MINER_VERSION}-windows.zip"
    MINER_ARCHIVE_SHA256="94b07c354eaab3e2da9a9893bcaa0799f4f3a3eaa11f4ba0683e32d39134b8cd"
    MINER_BIN_SHA256="de7e561dab2c6627333c13e1fff04da61dff536ad1f6911142fd62c4b81c3e73"
    BRIDGE_ARCHIVE="karlsen-stratum-bridge-v${BRIDGE_VERSION}-windows-x64.zip"
    BRIDGE_ARCHIVE_SHA256="c14af298d097646f4119567dd17b6c8f16b0f15c5fb9a8e7382479957f747cf9"
    BRIDGE_BIN_SHA256="46bc6d4282e079b32e7cf2121bae42c9b895f52cf27d6e2db597dc4f3a587f1a"
    ;;
  x86_64-*linux*)
    EXT=""
    MINER_ARCHIVE="karlsenminer-v${MINER_VERSION}-linux.tar.gz"
    MINER_ARCHIVE_SHA256="cb6dfc8378f5b67598deba54818d6817b0d73cfeef82ca560147eb5295e42ddb"
    MINER_BIN_SHA256="d5fa519acba89811363e2bd4bc56c164b1d278a7066054447c016855279ee2e8"
    BRIDGE_ARCHIVE="karlsen-stratum-bridge-v${BRIDGE_VERSION}-linux-x86_64.tar.gz"
    BRIDGE_ARCHIVE_SHA256="f00d41080d20f0bddc4d67162ccf563733ab811b7a7122c653c505c8ab85936f"
    BRIDGE_BIN_SHA256="387cf8d8e6bac287c08aba991ecc88b2a805759176d0bc46178dd8e0af9b6ebd"
    ;;
  *)
    echo "No KarlsenMiner / stratum bridge release for target $TRIPLE (Windows x64 and Linux x64 only)." >&2
    exit 1
    ;;
esac

MINER_URL="https://github.com/karlsencoin/karlsen-miner/releases/download/v${MINER_VERSION}/${MINER_ARCHIVE}"
BRIDGE_URL="https://github.com/karlsencoin/karlsen-stratum-bridge/releases/download/v${BRIDGE_VERSION}/${BRIDGE_ARCHIVE}"

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
mkdir -p src-tauri/binaries

sha256_of() {
  sha256sum "$1" | cut -d' ' -f1
}

# fetch_sidecar <name> <archive file name> <url> <archive sha256> <executable sha256>
fetch_sidecar() {
  local name="$1" archive="$2" url="$3" archive_sha="$4" bin_sha="$5"
  local pkg="$TMP/$archive" out="$TMP/x-$name" bin actual

  if [[ -n "$LOCAL_DIR" ]]; then
    cp "$LOCAL_DIR/$archive" "$pkg"
  else
    echo "downloading $url"
    curl -fsSL "$url" -o "$pkg"
  fi
  actual="$(sha256_of "$pkg")"
  if [[ "$actual" != "$archive_sha" ]]; then
    echo "REFUSED: $archive has SHA-256 $actual, expected $archive_sha" >&2
    exit 1
  fi

  mkdir -p "$out"
  case "$archive" in
    *.zip) unzip -q -o "$pkg" -d "$out" ;;
    *) tar -xzf "$pkg" -C "$out" ;;
  esac
  bin="$(find "$out" -type f -name "${name}${EXT}" | head -n1)"
  [[ -n "$bin" ]] || { echo "${name}${EXT} not found in $archive" >&2; exit 1; }
  actual="$(sha256_of "$bin")"
  if [[ "$actual" != "$bin_sha" ]]; then
    echo "REFUSED: ${name}${EXT} has SHA-256 $actual, expected $bin_sha" >&2
    exit 1
  fi

  local dest="src-tauri/binaries/${name}-${TRIPLE}${EXT}"
  cp "$bin" "$dest"
  chmod +x "$dest"
  echo "sidecar ready: $dest"
  "$dest" --version || true
}

fetch_sidecar karlsenminer "$MINER_ARCHIVE" "$MINER_URL" "$MINER_ARCHIVE_SHA256" "$MINER_BIN_SHA256"
fetch_sidecar karlsen-stratum-bridge "$BRIDGE_ARCHIVE" "$BRIDGE_URL" "$BRIDGE_ARCHIVE_SHA256" "$BRIDGE_BIN_SHA256"
