# Karlsen Desktop v3

Karlsen Desktop runs a **rusty-karlsen full node automatically** and bundles the
**Karlsen Web Wallet**, in one native app for Windows and Linux. From v3.1.0 it also ships
**KarlsenMiner** (GPU), so anyone running Karlsen Desktop can solo mine directly on their
own node from the MINER tab.
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
│   ├─ miner_settings.rs  miner.json → validated KarlsenMiner command line
│   ├─ miner.rs     solo mining session: KarlsenMiner sidecar on the local node,
│   │               start/stop/restart, log capture,
│   │               hashrate and share counters parsed from its output
│   └─ lib.rs       Tauri commands, tray icon, single instance, shutdown hook
├─ ui/         React frontend
│   ├─ Dashboard    node metrics over wRPC Borsh (127.0.0.1:43110) via the Karlsen WASM SDK
│   ├─ Wallet tab   embedded karlsen-web-wallet (git submodule, served at /wallet/)
│   ├─ Miner tab    solo mining: address / GPU settings, hashrate, blocks found, live output
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
scripts/fetch-miner.sh                         # KarlsenMiner sidecar (pinned release, SHA-256 checked)
scripts/build-wallet.sh
npm --prefix ui ci
cargo install tauri-cli --version "^2" --locked
cd src-tauri && cargo tauri dev                # or: cargo tauri build
```

### Release builds (GitHub Actions)

`.github/workflows/release.yml` builds the installers for Windows (NSIS + MSI) and Linux
(AppImage + deb) without any secrets:

- push the commit to release to the branch `release/desktop` (or use "Run workflow"): the
  workflow builds the WASM SDK from the pinned rusty-karlsen commit, fetches karlsend and
  KarlsenMiner from their releases (SHA-256 checked), runs the backend tests, builds the
  installers and creates a **draft** release `v<version>` with the files and
  `SHA256SUMS.txt`. Publish the draft after the installers were tested.
- push to a `ci/...` branch: same build, no release (to test a change).

The installers are not code-signed.

## Solo mining (MINER tab)

```
karlsenminer --node grpc://127.0.0.1:42110  ──gRPC──▶  karlsend (the node Desktop runs)
 GPU hashing                                            block templates / block submit
```

[KarlsenMiner](https://github.com/karlsencoin/karlsen-miner) 3.3.0 and newer talks to
karlsend directly: no pool and no stratum bridge. It ships inside the installer as a
sidecar, next to karlsend. A block found is submitted by the user's own node and its whole
reward goes to the mining address the user entered.

- **Start mining** starts the miner. It waits until the node is reachable and fully synced
  and only then opens the GPUs; it stops hashing by itself whenever the node falls out of
  sync. If the miner exits unexpectedly it is restarted with backoff (not when no usable
  GPU is found).
- Nothing listens on the network for mining: the miner is a client of the node's gRPC
  server on `127.0.0.1`.
- There are no shares in solo mining. The tab shows the hashrate the miner measures, the
  network hashrate, the average time per block at that hashrate, and the blocks found.
- Mining address (checksum verified), rig name and GPU selection are validated in the Rust
  backend before they reach the command line. Settings are in `miner.json` next to
  `settings.json`.
- NVIDIA tuning options (`--pl`, `--cclk`, …) are not offered: they need administrator
  rights and make the miner restart itself outside Desktop's control.
- The miner stops when Desktop exits, also when Desktop is killed.
- Requirements are KarlsenMiner's: AMD, NVIDIA or Intel Arc GPU with 6 GB+ VRAM.

**Antivirus:** GPU miners are commonly reported as "potentially unwanted". Because the
miner is inside the installer, such a detection hits the Karlsen Desktop installer itself.
Sign the installer and submit each release to Microsoft as a false positive before
publishing.

**Updating the bundled miner:** edit the pins at the top of `scripts/fetch-miner.sh`
(version, archive SHA-256 and executable SHA-256, for Windows and Linux) and
`MINER_VERSION` in `src-tauri/src/miner.rs`. `miner.rs` follows the miner through a few of
its output lines (the `Total:` line of the table, the node-state messages, `Block accepted
by the node`); its tests list them. Check them when the miner version changes.

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
