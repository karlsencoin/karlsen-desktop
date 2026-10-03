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
  # Embedded build, served under /wallet/. The local node is handed over at runtime as
  # ?node=ws://127.0.0.1:<port> (see ui/src/App.tsx); the wallet uses it once it is synced.
  # The app origin (tauri.localhost) has no same-origin /wrpc or /bridge-api, so the public
  # node and the bridge API are given as absolute URLs (allowed by the CSP in tauri.conf.json).
  VITE_PUBLIC_NODE_URLS="wss://wallet.karlsencoin.org/wrpc" \
  VITE_FEATURE_BRIDGE="true" \
  VITE_BRIDGE_API_URL="https://wallet.karlsencoin.org/bridge-api/v1" \
    npx vite build --base=/wallet/ --outDir ../ui/public/wallet --emptyOutDir
)

rm -rf ui/public/karlsen-wasm
cp -r "$WALLET/public/karlsen-wasm" ui/public/karlsen-wasm
echo "wallet + SDK copied into ui/public/"
