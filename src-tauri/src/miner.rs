//! Solo mining on the local node.
//!
//! KarlsenMiner (v3.3.0+) talks to karlsend directly over gRPC; there is no pool and no
//! stratum bridge in between. It ships inside the Desktop installer as a sidecar, next to
//! karlsend:
//!
//! ```text
//! karlsenminer --node grpc://127.0.0.1:42110  --gRPC-->  karlsend (this Desktop's node)
//!   GPU hashing                                           block templates / block submit
//! ```
//!
//! Every block found is submitted by the user's own node and its reward goes straight to
//! the user's mining address.
//!
//! The miner itself waits for the node to be reachable and fully synced before it opens the
//! GPUs, and stops hashing whenever the node falls out of sync. One supervisor task owns
//! the process and restarts it with backoff when it exits unexpectedly. It runs as a child
//! process exactly like karlsend (see `child.rs` / `job.rs`), so it can never outlive
//! Karlsen Desktop.

use crate::child::{ChildEvent, NodeChild};
use serde::Serialize;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};
use tauri_plugin_shell::ShellExt;
use tokio::sync::{mpsc, watch, Mutex};

/// Version of the bundled KarlsenMiner. Keep in sync with the pin in `scripts/fetch-miner.sh`.
pub const MINER_VERSION: &str = "3.3.1";

const MINER_SIDECAR: &str = "karlsenminer";

const LOG_CAPACITY: usize = 3_000;
/// Time the miner gets to shut down by itself after the stop signal.
const GRACEFUL_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_BACKOFF: Duration = Duration::from_secs(60);
/// A session at least this long counts as stable and clears the crash streak.
const STABLE_RUN: Duration = Duration::from_secs(600);
/// Consecutive failed sessions after which Desktop stops restarting.
const MAX_RESTARTS: u32 = 5;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MinerState {
    Stopped,
    /// The miner process is up: checking the node, then opening the GPUs and building the DAG.
    Starting,
    /// The miner waits for the node to be reachable and fully synced; the GPUs are idle.
    Waiting,
    /// The GPUs are hashing on the node's block templates.
    Running,
    Stopping,
    Crashed,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MinerStatus {
    pub miner_version: &'static str,
    pub state: MinerState,
    pub miner_pid: Option<u32>,
    /// Seconds since the miner process of the current session started.
    pub uptime_secs: Option<u64>,
    /// Total hashrate in MH/s from the miner's latest status table.
    pub hashrate_mhs: Option<f64>,
    /// Blocks this node accepted from the miner since Desktop started.
    pub blocks_found: u64,
    /// Blocks the node refused (stale or invalid) since Desktop started.
    pub blocks_rejected: u64,
    /// Consecutive failed sessions; reset after a stable run.
    pub restarts: u32,
    /// An automatic restart is scheduled after an unexpected exit.
    pub restart_pending: bool,
    /// Why mining is not running, for the UI (e.g. no usable GPU).
    pub last_error: Option<String>,
}

/// Validated command line for one mining session (built by `MinerSettings`).
#[derive(Debug, Clone)]
pub struct Launch {
    pub miner_args: Vec<String>,
}

struct Inner {
    state: MinerState,
    miner_pid: Option<u32>,
    started_at: Option<Instant>,
    restarts: u32,
    restart_pending: bool,
    last_error: Option<String>,
    /// Last "ERROR:" line printed by the miner in the current session.
    miner_error_line: Option<String>,
    /// A condition a restart cannot fix was reported (no usable GPU).
    fatal: Option<String>,
    hashrate_mhs: Option<f64>,
    blocks_found: u64,
    blocks_rejected: u64,
    launch: Option<Launch>,
}

/// How one mining session ended.
enum SessionEnd {
    Stopped,
    Failed { reason: String, fatal: bool },
}

#[derive(Clone)]
pub struct MinerManager {
    inner: Arc<Mutex<Inner>>,
    logs: Arc<std::sync::Mutex<VecDeque<String>>>,
    /// `true` asks the supervisor to stop the session.
    stop: Arc<watch::Sender<bool>>,
    /// `true` while a supervisor task is alive.
    active: Arc<watch::Sender<bool>>,
    /// Working directory of the miner: `<app local data>/miner`.
    work_dir: PathBuf,
}

impl MinerManager {
    pub fn new(work_dir: PathBuf) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                state: MinerState::Stopped,
                miner_pid: None,
                started_at: None,
                restarts: 0,
                restart_pending: false,
                last_error: None,
                miner_error_line: None,
                fatal: None,
                hashrate_mhs: None,
                blocks_found: 0,
                blocks_rejected: 0,
                launch: None,
            })),
            logs: Arc::new(std::sync::Mutex::new(VecDeque::with_capacity(LOG_CAPACITY))),
            stop: Arc::new(watch::channel(false).0),
            active: Arc::new(watch::channel(false).0),
            work_dir,
        }
    }

    /// Folder for the miner's `--config` argument.
    pub fn work_dir(&self) -> &std::path::Path {
        &self.work_dir
    }

    pub async fn status(&self) -> MinerStatus {
        let mut g = self.inner.lock().await;
        if g.state != MinerState::Crashed && g.started_at.is_some_and(|t| t.elapsed() >= STABLE_RUN) {
            g.restarts = 0;
        }
        MinerStatus {
            miner_version: MINER_VERSION,
            state: g.state,
            miner_pid: g.miner_pid,
            uptime_secs: g.started_at.map(|t| t.elapsed().as_secs()),
            hashrate_mhs: g.hashrate_mhs,
            blocks_found: g.blocks_found,
            blocks_rejected: g.blocks_rejected,
            restarts: g.restarts,
            restart_pending: g.restart_pending,
            last_error: g.last_error.clone(),
        }
    }

    pub fn logs(&self) -> Vec<String> {
        self.logs.lock().unwrap().iter().cloned().collect()
    }

    fn push_log(&self, app: &AppHandle, line: String) {
        {
            let mut logs = self.logs.lock().unwrap();
            if logs.len() == LOG_CAPACITY {
                logs.pop_front();
            }
            logs.push_back(line.clone());
        }
        let _ = app.emit("miner-log", line);
    }

    async fn emit_status(&self, app: &AppHandle) {
        let _ = app.emit("miner-status", self.status().await);
    }

    async fn set_state(&self, app: &AppHandle, state: MinerState) {
        self.inner.lock().await.state = state;
        self.emit_status(app).await;
    }

    /// Start a mining session. No-op when one is already running.
    pub async fn start(&self, app: &AppHandle, launch: Launch) -> anyhow::Result<()> {
        if *self.active.borrow() {
            if *self.stop.borrow() {
                anyhow::bail!("Mining is still stopping. Try again in a moment.");
            }
            return Ok(());
        }
        std::fs::create_dir_all(&self.work_dir)?;
        {
            let mut g = self.inner.lock().await;
            g.launch = Some(launch);
            g.restarts = 0;
            g.last_error = None;
            g.state = MinerState::Starting;
        }
        self.stop.send_replace(false);
        self.active.send_replace(true);
        self.emit_status(app).await;

        let this = self.clone();
        let app = app.clone();
        tauri::async_runtime::spawn(async move { this.supervise(app).await });
        Ok(())
    }

    /// Stop the session and wait until the miner is gone.
    pub async fn stop(&self, app: &AppHandle) -> anyhow::Result<()> {
        if !*self.active.borrow() {
            return Ok(());
        }
        self.stop.send_replace(true);
        let mut active = self.active.subscribe();
        // One graceful timeout plus the kill is the worst case inside the supervisor.
        if tokio::time::timeout(GRACEFUL_TIMEOUT * 3, active.wait_for(|a| !*a)).await.is_err() {
            self.push_log(app, "DESKTOP: mining did not stop in time".into());
        }
        Ok(())
    }

    /// Runs sessions until the user stops mining or restarting cannot help.
    async fn supervise(&self, app: AppHandle) {
        let mut stop = self.stop.subscribe();
        loop {
            let end = self.run_session(&app, &mut stop).await;
            let backoff = {
                let mut g = self.inner.lock().await;
                let stable = g.started_at.is_some_and(|t| t.elapsed() >= STABLE_RUN);
                g.miner_pid = None;
                g.started_at = None;
                g.hashrate_mhs = None;
                match end {
                    SessionEnd::Stopped => {
                        g.state = MinerState::Stopped;
                        None
                    }
                    SessionEnd::Failed { reason, fatal } => {
                        if stable {
                            g.restarts = 0;
                        }
                        g.restarts += 1;
                        g.state = MinerState::Crashed;
                        if fatal {
                            g.last_error = Some(reason);
                            None
                        } else if g.restarts > MAX_RESTARTS {
                            g.last_error = Some(format!("{reason} Stopped after {MAX_RESTARTS} failed restarts."));
                            None
                        } else {
                            g.last_error = Some(reason);
                            g.restart_pending = true;
                            Some(Duration::from_secs(2u64.saturating_pow(g.restarts.min(6))).min(MAX_BACKOFF))
                        }
                    }
                }
            };
            if let Some(e) = self.inner.lock().await.last_error.clone() {
                self.push_log(&app, format!("DESKTOP: {e}"));
            }
            self.emit_status(&app).await;
            let Some(backoff) = backoff else { break };

            self.push_log(&app, format!("DESKTOP: restarting mining in {}s", backoff.as_secs()));
            let stopped = tokio::select! {
                _ = tokio::time::sleep(backoff) => false,
                _ = stop_requested(&mut stop) => true,
            };
            let mut g = self.inner.lock().await;
            g.restart_pending = false;
            if stopped {
                g.state = MinerState::Stopped;
                break;
            }
        }
        self.inner.lock().await.restart_pending = false;
        self.active.send_replace(false);
        self.emit_status(&app).await;
    }

    async fn run_session(&self, app: &AppHandle, stop: &mut watch::Receiver<bool>) -> SessionEnd {
        let launch = {
            let mut g = self.inner.lock().await;
            g.state = MinerState::Starting;
            g.restart_pending = false;
            g.last_error = None;
            g.miner_error_line = None;
            g.fatal = None;
            g.hashrate_mhs = None;
            g.launch.clone()
        };
        let Some(launch) = launch else {
            return SessionEnd::Failed { reason: "No mining settings.".into(), fatal: true };
        };
        self.emit_status(app).await;

        self.push_log(app, format!("DESKTOP: starting {MINER_SIDECAR} {}", launch.miner_args.join(" ")));
        // The shell plugin resolves the bundled sidecar path (and hides the console window on
        // Windows); crate::child spawns it so its lifetime is tied to Desktop's.
        let spawned = app
            .shell()
            .sidecar(MINER_SIDECAR)
            .map_err(anyhow::Error::from)
            .and_then(|cmd| {
                crate::child::spawn(cmd.args(&launch.miner_args).current_dir(&self.work_dir).into())
                    .map_err(anyhow::Error::from)
            });
        let (mut rx, miner) = match spawned {
            Ok(v) => v,
            Err(e) => return SessionEnd::Failed { reason: format!("Cannot start KarlsenMiner: {e}"), fatal: true },
        };
        if let Err(e) = crate::job::bind_to_desktop_lifetime(miner.pid()) {
            self.push_log(app, format!("DESKTOP: warning: {e}"));
        }
        let mut lines = LineBuffer::default();
        {
            let mut g = self.inner.lock().await;
            g.miner_pid = Some(miner.pid());
            g.started_at = Some(Instant::now());
        }
        self.emit_status(app).await;

        loop {
            tokio::select! {
                ev = rx.recv() => {
                    if let Some(code) = self.handle_event(app, ev, &mut lines).await {
                        self.push_log(app, format!("DESKTOP: karlsenminer exited (code {code:?})"));
                        let g = self.inner.lock().await;
                        return match &g.fatal {
                            Some(reason) => SessionEnd::Failed { reason: reason.clone(), fatal: true },
                            None => SessionEnd::Failed {
                                reason: g
                                    .miner_error_line
                                    .clone()
                                    .unwrap_or_else(|| format!("KarlsenMiner exited (code {code:?}).")),
                                fatal: false,
                            },
                        };
                    }
                }
                _ = stop_requested(stop) => {
                    self.set_state(app, MinerState::Stopping).await;
                    self.push_log(app, "DESKTOP: stopping mining".into());
                    self.shutdown(app, &miner, &mut rx, &mut lines).await;
                    return SessionEnd::Stopped;
                }
            }
        }
    }

    /// Log one child event. Returns `Some(exit code)` when the process has terminated.
    async fn handle_event(
        &self,
        app: &AppHandle,
        event: Option<ChildEvent>,
        lines: &mut LineBuffer,
    ) -> Option<Option<i32>> {
        match event {
            Some(ChildEvent::Stdout(bytes)) | Some(ChildEvent::Stderr(bytes)) => {
                for line in lines.feed(&bytes) {
                    if self.absorb_line(&line).await {
                        self.emit_status(app).await;
                    }
                    self.push_log(app, line);
                }
                None
            }
            Some(ChildEvent::Error(e)) => {
                self.push_log(app, format!("DESKTOP: process error: {e}"));
                None
            }
            Some(ChildEvent::Terminated(code)) => Some(code),
            // The event channel closes only after `Terminated`; treat a bare close the same.
            None => Some(None),
        }
    }

    /// Update state and statistics from one line of miner output. Returns true when the UI
    /// should be told at once instead of at its next poll (state change or a block).
    async fn absorb_line(&self, line: &str) -> bool {
        let info = parse_miner_line(line);
        if info == MinerLine::default() {
            return false;
        }
        let mut g = self.inner.lock().await;
        if let Some(h) = info.hashrate_mhs {
            g.hashrate_mhs = Some(h);
        }
        match info.block {
            Some(BlockOutcome::Accepted) => g.blocks_found += 1,
            Some(BlockOutcome::Rejected) => g.blocks_rejected += 1,
            None => {}
        }
        if info.no_gpu {
            g.fatal = Some(
                "No usable GPU found. KarlsenMiner needs an AMD, NVIDIA or Intel Arc GPU with 6 GB+ VRAM \
                 and a working OpenCL or CUDA driver."
                    .into(),
            );
        }
        if let Some(e) = info.error {
            g.miner_error_line = Some(e);
        }
        let before = g.state;
        // Late output must never overwrite Stopping / Crashed.
        if matches!(g.state, MinerState::Starting | MinerState::Waiting | MinerState::Running) {
            match info.node {
                Some(NodeState::Waiting) => {
                    g.state = MinerState::Waiting;
                    g.hashrate_mhs = None;
                }
                // The node is fine; the GPUs are being opened and the DAG is built.
                Some(NodeState::Synced) => g.state = MinerState::Starting,
                Some(NodeState::Mining) => g.state = MinerState::Running,
                None => {}
            }
        }
        g.state != before || info.block.is_some()
    }

    /// Stop the miner (its own Ctrl+C path where possible) and drain its output until it
    /// has terminated.
    async fn shutdown(
        &self,
        app: &AppHandle,
        child: &NodeChild,
        rx: &mut mpsc::Receiver<ChildEvent>,
        lines: &mut LineBuffer,
    ) {
        #[cfg(unix)]
        unsafe {
            // KarlsenMiner shuts down cleanly on SIGINT.
            libc::kill(child.pid() as libc::pid_t, libc::SIGINT);
        }
        #[cfg(not(unix))]
        {
            // No Ctrl+C for a console-less child on Windows. Desktop never passes NVIDIA
            // tuning options, so there is no GPU state to restore and a kill is clean.
            let _ = child.kill();
        }
        for attempt in 0..2 {
            let drained = tokio::time::timeout(GRACEFUL_TIMEOUT, async {
                loop {
                    let ev = rx.recv().await;
                    if self.handle_event(app, ev, lines).await.is_some() {
                        break;
                    }
                }
            })
            .await;
            if drained.is_ok() {
                return;
            }
            if attempt == 0 {
                self.push_log(app, "DESKTOP: miner did not stop in time, killing".into());
                let _ = child.kill();
            }
        }
    }
}

/// Resolves once the stop flag is set. Returns `()` instead of the watch `Ref`, which is
/// not `Send` and must not live across the awaits of a `select!` branch.
async fn stop_requested(stop: &mut watch::Receiver<bool>) {
    let _ = stop.wait_for(|s| *s).await;
}

/// Splits a byte stream into ANSI-free, non-empty lines.
#[derive(Default)]
struct LineBuffer {
    partial: String,
}

impl LineBuffer {
    fn feed(&mut self, bytes: &[u8]) -> Vec<String> {
        self.partial.push_str(&String::from_utf8_lossy(bytes));
        let mut out = Vec::new();
        // '\r' also ends a line: a status display may redraw in place.
        while let Some(pos) = self.partial.find(['\n', '\r']) {
            let raw: String = self.partial.drain(..=pos).collect();
            let line = strip_ansi(&raw).trim_end().to_string();
            if !line.is_empty() {
                out.push(line);
            }
        }
        out
    }
}

/// Remove ANSI escape sequences (colours, cursor movement).
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        if chars.peek() == Some(&'[') {
            chars.next();
            // CSI: parameter and intermediate bytes, then one final byte in '@'..='~'.
            for f in chars.by_ref() {
                if ('@'..='~').contains(&f) {
                    break;
                }
            }
        } else {
            // Two-character escape.
            chars.next();
        }
    }
    out
}

/// What the miner said about the node.
#[derive(Debug, PartialEq, Clone, Copy)]
enum NodeState {
    /// Node unreachable or still syncing; the miner waits.
    Waiting,
    /// Node reachable and synced; the GPUs are not hashing yet.
    Synced,
    /// A block template reached the GPUs.
    Mining,
}

#[derive(Debug, PartialEq, Clone, Copy)]
enum BlockOutcome {
    Accepted,
    Rejected,
}

/// What one line of KarlsenMiner (3.3.1) output tells Desktop.
#[derive(Debug, Default, PartialEq)]
struct MinerLine {
    hashrate_mhs: Option<f64>,
    node: Option<NodeState>,
    block: Option<BlockOutcome>,
    error: Option<String>,
    no_gpu: bool,
}

/// Parse a leading unsigned decimal number, returning it and the rest of the string.
fn leading_number(s: &str) -> Option<(f64, &str)> {
    let end = s.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(s.len());
    let value = s[..end].parse::<f64>().ok()?;
    Some((value, &s[end..]))
}

/// Parse the lines Desktop reacts to (all printed by `karlsenminer --node`, 3.3.1):
/// the table's summary line, e.g.
/// `Total: 58.37 MH/s (avg 58.12 MH/s) | A:1/R:0 | 0.375 | 156W | Best: 6,538,822,998`,
/// the node-state messages, block results and `ERROR:` lines.
/// Input must already be free of ANSI sequences.
fn parse_miner_line(line: &str) -> MinerLine {
    let mut info = MinerLine::default();
    if let Some(pos) = line.find("Total:") {
        let rest = line[pos + "Total:".len()..].trim_start();
        if let Some((value, unit)) = leading_number(rest) {
            let unit = unit.trim_start();
            // (unit, MH/s per unit as multiplier, or as divisor to keep decimal values exact)
            let units = [("GH/s", 1e3, 1.0), ("MH/s", 1.0, 1.0), ("kH/s", 1.0, 1e3), ("KH/s", 1.0, 1e3), ("H/s", 1.0, 1e6)];
            if let Some((_, mul, div)) = units.into_iter().find(|(u, _, _)| unit.starts_with(u)) {
                info.hashrate_mhs = Some(value * mul / div);
            }
        }
    }

    const WAITING: [&str; 4] = [
        "is still syncing - waiting",    // before the GPUs are opened
        "waiting for the node to start", // node not reachable yet
        "The node is not synced yet",    // fell out of sync while mining
        "Connection to the node lost",
    ];
    if WAITING.iter().any(|m| line.contains(m)) {
        info.node = Some(NodeState::Waiting);
    } else if line.contains("Ready to mine") || line.contains("The node is synced - mining") {
        info.node = Some(NodeState::Mining);
    } else if line.contains("Node ") && line.ends_with(" is synced") {
        // "Node grpc://127.0.0.1:42110: karlsend 3.3.0 is synced"
        info.node = Some(NodeState::Synced);
    }

    if line.contains("Block accepted by the node") {
        info.block = Some(BlockOutcome::Accepted);
    } else if line.contains("Block rejected by the node") || line.contains("SubmitBlock failed") {
        info.block = Some(BlockOutcome::Rejected);
    }

    if let Some(pos) = line.find("ERROR:") {
        let text = line[pos + "ERROR:".len()..].trim();
        if !text.is_empty() {
            info.no_gpu = text.contains("no usable GPU");
            info.error = Some(text.to_string());
        }
    }
    info
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_colour_and_cursor_sequences() {
        assert_eq!(strip_ansi("\u{1b}[31mERROR: no usable GPU found\u{1b}[0m"), "ERROR: no usable GPU found");
        assert_eq!(strip_ansi("\u{1b}[2K\u{1b}[1;1Hplain"), "plain");
        assert_eq!(strip_ansi("no escapes"), "no escapes");
    }

    #[test]
    fn line_buffer_joins_chunks_and_splits_on_cr_and_lf() {
        let mut buf = LineBuffer::default();
        assert!(buf.feed(b"first li").is_empty());
        assert_eq!(buf.feed(b"ne\nsecond\r\n\n\x1b[32mthird\x1b[0m\rpart"), ["first line", "second", "third"]);
        assert_eq!(buf.feed(b"ial\n"), ["partial"]);
    }

    #[test]
    fn parses_miner_summary_line() {
        let line = "   Total: 173.29 MH/s (avg 173.31 MH/s) | A:7/R:0 | 0.421 | 412W | Best: 6,538,822,998";
        let info = parse_miner_line(line);
        assert_eq!(info.hashrate_mhs, Some(173.29));
        assert_eq!((info.node, info.block, info.error.clone()), (None, None, None));
        assert_eq!(parse_miner_line("Total: 1.5 GH/s | A:0/R:0").hashrate_mhs, Some(1500.0));
        assert_eq!(parse_miner_line("Total: 950.00 KH/s (avg 0.00 H/s)").hashrate_mhs, Some(0.95));
    }

    #[test]
    fn ignores_ordinary_miner_lines_and_per_gpu_rows() {
        assert_eq!(parse_miner_line("[2026-11-16 20:55:57] Enumerating CUDA GPU devices..."), MinerLine::default());
        assert_eq!(
            parse_miner_line("     0  RX 6700 XT        29.27 MH/s    51             0   0.353     83W"),
            MinerLine::default()
        );
        assert_eq!(parse_miner_line("Total: starting"), MinerLine::default());
        assert_eq!(parse_miner_line("[..] Solo mining on node: grpc://127.0.0.1:42110 (no pool)"), MinerLine::default());
        assert_eq!(parse_miner_line("[..] Connected to karlsend 3.3.0 (synced: true)"), MinerLine::default());
    }

    #[test]
    fn detects_missing_gpu() {
        // Real output of karlsenminer on a machine without a GPU.
        let raw = "[2026-11-16 20:55:57] \u{1b}[31mERROR: no usable GPU found (OpenCL or CUDA)\u{1b}[0m";
        assert!(parse_miner_line(&strip_ansi(raw)).no_gpu);
        let other = parse_miner_line("[..] ERROR: cannot start the node client runtime: x");
        assert!(!other.no_gpu);
        assert_eq!(other.error.as_deref(), Some("cannot start the node client runtime: x"));
    }

    #[test]
    fn follows_the_node_state() {
        // Real lines of karlsenminer 3.3.x --node (ANSI already stripped).
        let node = |l: &str| parse_miner_line(l).node;
        assert_eq!(
            node("[2026-11-17 03:38:43] Node grpc://127.0.0.1:42520: karlsend 3.3.0 is still syncing - waiting; the GPUs start when it is synced"),
            Some(NodeState::Waiting)
        );
        assert_eq!(
            node("[2026-11-17 03:38:07] cannot connect to karlsend at grpc://127.0.0.1:42599: Endpoint connection error: transport error - waiting for the node to start"),
            Some(NodeState::Waiting)
        );
        assert_eq!(node("[..] The node is not synced yet - mining starts by itself when it is"), Some(NodeState::Waiting));
        assert_eq!(node("[..] Connection to the node lost"), Some(NodeState::Waiting));
        assert_eq!(node("[..] Node grpc://127.0.0.1:42110: karlsend 3.3.0 is synced"), Some(NodeState::Synced));
        assert_eq!(node("[2026-11-17 16:11:34] Ready to mine - network difficulty 6,538,822,998"), Some(NodeState::Mining));
        assert_eq!(node("[..] The node is synced - mining"), Some(NodeState::Mining));
    }

    #[test]
    fn counts_blocks() {
        let block = |l: &str| parse_miner_line(l).block;
        // "BLOCK FOUND" is only the candidate; the node's answer is what counts.
        assert_eq!(block("[..] GPU 2: BLOCK FOUND 21a4ffbc (network difficulty 6,538,822,998)"), None);
        // Real line from the Mine4 mainnet test (2026-11-17).
        assert_eq!(
            block("[2026-11-17 16:17:05] Block accepted by the node: 21a4ffbc3f37372568530684b949436b53d10e7c763ccc6bb91368ee8dfbf185 (blocks found: 8)"),
            Some(BlockOutcome::Accepted)
        );
        assert_eq!(block("[..] Block rejected by the node: b284d48d (BlockInvalid)"), Some(BlockOutcome::Rejected));
        assert_eq!(block("[..] SubmitBlock failed for b284d48d: transport error"), Some(BlockOutcome::Rejected));
    }
}
