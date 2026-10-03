# Karlsen Desktop v3

Karlsen Desktop runs a **rusty-karlsen full node automatically** and bundles the
**Karlsen Web Wallet**, in one native app for Windows and Linux.
It replaces the old NW.js Karlsen Desktop v2 (Go `karlsend` v2.2.0), which can
no longer sync with the current network.

Written from scratch: a Rust backend on Tauri 2 plus a React UI. No code from
Kaspa NG is used.

## Architecture

```
Karlsen Desktop (Tauri 2)
├─ src-tauri/  Rust backend
│   ├─ node.rs      karlsend sidecar: auto-start, auto-restart with backoff,
│   │               graceful stop (SIGTERM → kill), log capture, delete & resync
│   ├─ settings.rs  JSON settings → karlsend command line
│   └─ lib.rs       Tauri commands, tray icon, single instance, shutdown hook
├─ ui/         React frontend
│   ├─ Dashboard    node metrics over wRPC Borsh (127.0.0.1:43110) via the Karlsen WASM SDK
│   ├─ Wallet tab   embedded karlsen-web-wallet (git submodule, served at /wallet/)
│   ├─ Settings     data folder, ports, UPnP, UTXO index, tray, resync
│   └─ KARLSEND     live node log console
└─ wallet/     git submodule → karlsencoin/karlsen-web-wallet
               (the same code also runs on wallet.karlsencoin.org)
```

The node runs as a **child process**, not in-process, so a node crash never
takes down the UI and the node can be upgraded independently.

## Build

Requirements: Rust stable, Node 20+, Tauri 2 system deps
(Linux: `libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev`).

```bash
git clone --recursive https://github.com/karlsencoin/karlsen-desktop
cd karlsen-desktop
scripts/fetch-karlsend.sh /path/to/karlsend    # or KARLSEND_URL=<v3.3.1 release asset>
scripts/build-wallet.sh
npm --prefix ui ci
cargo install tauri-cli --version "^2" --locked
cd src-tauri && cargo tauri dev                # or: cargo tauri build
```

## Default ports

| Service | Address |
|---|---|
| gRPC | 127.0.0.1:42110 |
| wRPC Borsh (dashboard + wallet) | 127.0.0.1:43110 |
| P2P | 0.0.0.0:42111 |

## License

ISC

## Wallet submodule (one-time setup)

The wallet repo must be pushed first, then:

```bash
git submodule add https://github.com/karlsencoin/karlsen-web-wallet wallet
git commit -m "Add karlsen-web-wallet submodule"
```
