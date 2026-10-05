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
  # MSYS_NO_PATHCONV: Git Bash on Windows rewrites "/wallet/" to "C:/Program Files/Git/wallet/",
  # which breaks every asset URL in the wallet (blank page).
  MSYS_NO_PATHCONV=1 \
  VITE_PUBLIC_NODE_URLS="wss://wallet.karlsencoin.org/wrpc" \
  VITE_FEATURE_BRIDGE="true" \
  VITE_BRIDGE_API_URL="https://wallet.karlsencoin.org/bridge-api" \
  VITE_LEGACY_WALLET_URL="https://wallet.karlsencoin.org/legacy" \
    npx vite build --base=/wallet/ --outDir ../ui/public/wallet --emptyOutDir
)

# Guard against the Git Bash path rewrite above: a wrong base makes the wallet a blank page.
if grep -q "Program Files" ui/public/wallet/index.html; then
  echo "wallet built with a Windows path as base (Git Bash path conversion); aborting" >&2
  exit 1
fi

# The SDK is built from rusty-karlsen and not committed to the wallet repo (.gitignore).
# Point KARLSEN_WASM_DIR at a built karlsen-wasm folder (karlsen.js + karlsen_bg.wasm).
SDK_DIR="${KARLSEN_WASM_DIR:-$WALLET/public/karlsen-wasm}"
[[ -f "$SDK_DIR/karlsen_bg.wasm" ]] || { echo "Karlsen WASM SDK not found in $SDK_DIR (set KARLSEN_WASM_DIR)" >&2; exit 1; }
rm -rf ui/public/karlsen-wasm
cp -r "$SDK_DIR" ui/public/karlsen-wasm
echo "wallet + SDK copied into ui/public/"
