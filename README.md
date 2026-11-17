# Karlsen Desktop v3

Karlsen Desktop runs a **rusty-karlsen full node automatically** and bundles the
**Karlsen Web Wallet**, in one native app for Windows and Linux. From v3.1.0 it also ships
**KarlsenMiner** (GPU) and the **Karlsen stratum bridge**, so anyone running Karlsen Desktop
can solo mine on their own node from the MINER tab.
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
│   ├─ miner_settings.rs  miner.json → validated miner and bridge command lines
│   ├─ miner.rs     solo mining session: stratum bridge + KarlsenMiner sidecars,
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
scripts/fetch-miner.sh                         # KarlsenMiner + stratum bridge sidecars (pinned, SHA-256 checked)
scripts/build-wallet.sh
npm --prefix ui ci
cargo install tauri-cli --version "^2" --locked
cd src-tauri && cargo tauri dev                # or: cargo tauri build
```

## Solo mining (MINER tab)

```
karlsenminer ──stratum──▶ karlsen-stratum-bridge ──gRPC──▶ karlsend (the node Desktop runs)
 GPU hashing               127.0.0.1:5555 only               block templates / block submit
```

[KarlsenMiner](https://github.com/karlsencoin/karlsen-miner) speaks stratum only, so the
[Karlsen stratum bridge](https://github.com/karlsencoin/karlsen-stratum-bridge) sits between
it and the node. Both ship inside the installer as sidecars, next to karlsend. No pool is
involved: a block found is submitted by the user's own node and its whole reward goes to
the mining address the user entered.

- **Start mining** starts the bridge, which waits until the node is reachable and fully
  synced; Desktop then starts the miner. If either process exits, the other is stopped and
  the session is restarted with backoff (not when no usable GPU is found or the stratum
  port is taken).
- The bridge listens on `127.0.0.1` only and its Prometheus, dashboard and health-check
  servers are off, so nothing is exposed to the network.
- The bridge runs with variable share difficulty (`--solo-mining false`): the frequent
  low-difficulty shares only measure the hashrate. Rewards come from blocks alone.
- Mining address (checksum verified), rig name, GPU selection and port are validated in the
  Rust backend before they reach either command line. Settings are in `miner.json` next to
  `settings.json`.
- NVIDIA tuning options (`--pl`, `--cclk`, …) are not offered: they need administrator
  rights and make the miner restart itself outside Desktop's control.
- Miner and bridge stop when Desktop exits, also when Desktop is killed.
- Requirements are KarlsenMiner's: AMD, NVIDIA or Intel Arc GPU with 6 GB+ VRAM.

**Antivirus:** GPU miners are commonly reported as "potentially unwanted". Because the
miner is inside the installer, such a detection hits the Karlsen Desktop installer itself.
Sign the installer and submit each release to Microsoft as a false positive before
publishing.

**Updating the bundled miner or bridge:** edit the pins at the top of
`scripts/fetch-miner.sh` (version, archive SHA-256 and executable SHA-256, for Windows and
Linux) and the matching `MINER_VERSION` / `BRIDGE_VERSION` in `src-tauri/src/miner.rs`.
`miner.rs` recognises three bridge log lines (`Node is synced, starting stratum listeners`,
`BLOCK ACCEPTED BY NODE!`, `failed listening to socket`); check them when the bridge
version changes.

## Default ports

| Service | Address |
|---|---|
| gRPC | 127.0.0.1:42110 |
| wRPC Borsh (dashboard + wallet) | 127.0.0.1:43110 |
| P2P | 0.0.0.0:42111 |
| Stratum bridge (only while mining) | 127.0.0.1:5555 |

## License

ISC

## Wallet submodule (one-time setup)

The wallet repo must be pushed first, then:

```bash
git submodule add https://github.com/karlsencoin/karlsen-web-wallet wallet
git commit -m "Add karlsen-web-wallet submodule"
```
