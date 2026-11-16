# Karlsen Desktop v3

Karlsen Desktop runs a **rusty-karlsen full node automatically** and bundles the
**Karlsen Web Wallet**, in one native app for Windows and Linux. From v3.1.0 it can also
install and run **KarlsenMiner** (GPU) from the MINER tab.
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
│   ├─ miner_install.rs   KarlsenMiner download, SHA-256 pins, extraction
│   ├─ miner_settings.rs  miner.json → validated KarlsenMiner command line
│   ├─ miner.rs     KarlsenMiner child process: start/stop/restart, log capture,
│   │               hashrate and share counters parsed from its output
│   └─ lib.rs       Tauri commands, tray icon, single instance, shutdown hook
├─ ui/         React frontend
│   ├─ Dashboard    node metrics over wRPC Borsh (127.0.0.1:43110) via the Karlsen WASM SDK
│   ├─ Wallet tab   embedded karlsen-web-wallet (git submodule, served at /wallet/)
│   ├─ Miner tab    install KarlsenMiner, wallet / pool / GPU settings, live output
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
scripts/fetch-karlsend.sh /path/to/karlsend    # or KARLSEND_URL=<rusty-karlsen v3.3.1 release asset; binary reports "karlsend 3.3.0">
scripts/build-wallet.sh
npm --prefix ui ci
cargo install tauri-cli --version "^2" --locked
cd src-tauri && cargo tauri dev                # or: cargo tauri build
```

## KarlsenMiner (MINER tab)

[KarlsenMiner](https://github.com/karlsencoin/karlsen-miner) is **not** inside the Desktop
installer: antivirus products flag GPU miners, and a bundled miner would get the node and
wallet installer quarantined with it. The user installs it from the MINER tab instead.

- Desktop downloads the release archive pinned in `src-tauri/src/miner_install.rs`
  (currently v3.2.0) from GitHub and installs it only if the archive **and** the executable
  inside match the SHA-256 values compiled into Desktop. The executable is checked again
  before every start.
- It is stored in the app's local data folder
  (`%LOCALAPPDATA%\org.karlsencoin.desktop\miner` on Windows,
  `~/.local/share/org.karlsencoin.desktop/miner` on Linux); settings are in `miner.json`
  next to `settings.json`.
- Mining address (checksum verified), rig name, pool and GPU selection are validated in the
  Rust backend before they reach the miner's command line.
- NVIDIA tuning options (`--pl`, `--cclk`, …) are not offered: they need administrator
  rights and make the miner restart itself outside Desktop's control. Run KarlsenMiner on
  its own for tuned rigs.
- The miner stops when Desktop exits, also when Desktop is killed.

**Updating the pinned miner release:** change `MINER_VERSION`, the two URLs and the four
SHA-256 values in `miner_install.rs` (archive hashes are in the release's `SHA256SUMS.txt`;
hash the extracted executables with `sha256sum`), then run

```bash
KD_MINER_ASSETS=/folder/with/both/release/archives cargo test   # pins match the real files
KD_MINER_NETWORK_TEST=1 cargo test downloads_and_installs       # full download from GitHub
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
