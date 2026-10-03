#!/usr/bin/env bash
# Build the karlsen-web-wallet submodule for embedding and copy it, plus the
# shared WASM SDK, into ui/public/ so Tauri serves them from the app origin.
set -euo pipefail

cd "$(dirname "$0")/.."
WALLET=wallet
[[ -f "$WALLET/package.json" ]] || { echo "wallet submodule missing: git submodule update --init" >&2; exit 1; }

(
  cd "$WALLET"
  npm ci
  # Embedded build: served under /wallet/, defaults to the bundled local node.
  VITE_DEFAULT_NODE_URL="ws://127.0.0.1:43110" npx vite build --base=/wallet/ --outDir ../ui/public/wallet --emptyOutDir
)

rm -rf ui/public/karlsen-wasm
cp -r "$WALLET/public/karlsen-wasm" ui/public/karlsen-wasm
echo "wallet + SDK copied into ui/public/"
