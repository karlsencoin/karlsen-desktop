//! Solo mining on the local node.
//!
//! KarlsenMiner speaks stratum only, karlsend speaks gRPC, so mining needs the Karlsen
//! stratum bridge in between. All three ship inside the Desktop installer as sidecars:
//!
//! ```text
//! karlsenminer --stratum--> karlsen-stratum-bridge --gRPC--> karlsend (this Desktop's node)
//!   GPU hashing              127.0.0.1 only                    block templates / submit
//! ```
//!
//! No pool is involved: every block found is submitted by the user's own node and its
//! reward goes straight to the user's mining address.
//!
//! One supervisor task owns a mining session: it starts the bridge, waits until the bridge
//! reports the node as synced and its stratum port answers, then starts the miner. If either
//! process exits, the other one is stopped too and the session is restarted with backoff.
//! Both run as child processes exactly like karlsend (see `child.rs` / `job.rs`), so they
//! can never outlive Karlsen Desktop.

use crate::child::{ChildEvent, NodeChild};
use serde::Serialize;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};
use tauri_plugin_shell::ShellExt;
use tokio::sync::{mpsc, watch, Mutex};

/// Versions of the bundled sidecars. Keep in sync with the pins in `scripts/fetch-miner.sh`.
pub const MINER_VERSION: &str = "3.2.0";
pub const BRIDGE_VERSION: &str = "3.3.2";

const MINER_SIDECAR: &str = "karlsenminer";
const BRIDGE_SIDECAR: &str = "karlsen-stratum-bridge";

const LOG_CAPACITY: usize = 3_000;
/// Time a process gets to shut down by itself after the stop signal.
const GRACEFUL_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_BACKOFF: Duration = Duration::from_secs(60);
/// A session at least this long counts as stable and clears the crash streak.
const STABLE_RUN: Duration = Duration::from_secs(600);
/// Consecutive failed sessions after which Desktop stops restarting.
const MAX_RESTARTS: u32 = 5;
/// How often the bridge's stratum port is probed while waiting for the node.
const PROBE_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MinerState {
    Stopped,
    /// The bridge is being started.
    Starting,
    /// The bridge runs and waits for the node to be reachable and fully synced.
    Waiting,
    /// The miner is connected to the bridge and hashing.
    Running,
    Stopping,
    Crashed,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MinerStatus {
    pub miner_version: &'static str,
    pub bridge_version: &'static str,
    pub state: MinerState,
    pub miner_pid: Option<u32>,
    pub bridge_pid: Option<u32>,
    /// Seconds since the miner started hashing in the current session.
    pub uptime_secs: Option<u64>,
    /// Total hashrate in MH/s from the miner's latest status line.
    pub hashrate_mhs: Option<f64>,
    /// Shares the bridge accepted / rejected. They only measure the hashrate; rewards come
    /// from blocks.
    pub accepted: Option<u64>,
    pub rejected: Option<u64>,
    /// Blocks this node accepted from the miner since Desktop started.
    pub blocks_found: u64,
    /// Consecutive failed sessions; reset after a stable run.
    pub restarts: u32,
    /// An automatic restart is scheduled after an unexpected exit.
    pub restart_pending: bool,
    /// Why mining is not running, for the UI (e.g. no usable GPU).
    pub last_error: Option<String>,
}

/// Validated command lines for one mining session (built by `MinerSettings`).
#[derive(Debug, Clone)]
pub struct Launch {
    pub bridge_args: Vec<String>,
    pub miner_args: Vec<String>,
    /// Loopback port the bridge listens on and the miner connects to.
    pub stratum_port: u16,
}

#[derive(Default)]
struct Stats {
    hashrate_mhs: Option<f64>,
    accepted: Option<u64>,
    rejected: Option<u64>,
}

struct Inner {
    state: MinerState,
    miner_pid: Option<u32>,
    bridge_pid: Option<u32>,
    started_at: Option<Instant>,
    restarts: u32,
    restart_pending: bool,
    last_error: Option<String>,
    /// Last "ERROR:" line printed by the miner in the current session.
    miner_error_line: Option<String>,
    /// A condition a restart cannot fix was reported (no GPU, stratum port in use).
    fatal: Option<String>,
    /// The bridge announced that the node is synced and its listeners are starting.
    bridge_ready: bool,
    blocks_found: u64,
    stats: Stats,
    launch: Option<Launch>,
}

/// How one mining session ended.
enum SessionEnd {
    Stopped,
    Failed { reason: String, fatal: bool },
}

#[derive(Clone, Copy, PartialEq)]
enum Source {
    Bridge,
    Miner,
}

#[derive(Clone)]
pub struct MinerManager {
    inner: Arc<Mutex<Inner>>,
    logs: Arc<std::sync::Mutex<VecDeque<String>>>,
    /// `true` asks the supervisor to stop the session.
    stop: Arc<watch::Sender<bool>>,
    /// `true` while a supervisor task is alive.
    active: Arc<watch::Sender<bool>>,
    /// Working directory of both sidecars: `<app local data>/miner`.
    work_dir: PathBuf,
}

impl MinerManager {
    pub fn new(work_dir: PathBuf) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                state: MinerState::Stopped,
                miner_pid: None,
                bridge_pid: None,
                started_at: None,
                restarts: 0,
                restart_pending: false,
                last_error: None,
                miner_error_line: None,
                fatal: None,
                bridge_ready: false,
                blocks_found: 0,
                stats: Stats::default(),
                launch: None,
            })),
            logs: Arc::new(std::sync::Mutex::new(VecDeque::with_capacity(LOG_CAPACITY))),
            stop: Arc::new(watch::channel(false).0),
            active: Arc::new(watch::channel(false).0),
            work_dir,
        }
    }

    /// Folder for the miner's and the bridge's config file arguments.
    pub fn work_dir(&self) -> &std::path::Path {
        &self.work_dir
    }

    pub async fn status(&self) -> MinerStatus {
        let mut g = self.inner.lock().await;
        if g.state == MinerState::Running && g.started_at.is_some_and(|t| t.elapsed() >= STABLE_RUN) {
            g.restarts = 0;
        }
        MinerStatus {
            miner_version: MINER_VERSION,
            bridge_version: BRIDGE_VERSION,
            state: g.state,
            miner_pid: g.miner_pid,
            bridge_pid: g.bridge_pid,
            uptime_secs: g.started_at.map(|t| t.elapsed().as_secs()),
            hashrate_mhs: g.stats.hashrate_mhs,
            accepted: g.stats.accepted,
            rejected: g.stats.rejected,
            blocks_found: g.blocks_found,
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

    /// Stop the session and wait until both processes are gone.
    pub async fn stop(&self, app: &AppHandle) -> anyhow::Result<()> {
        if !*self.active.borrow() {
            return Ok(());
        }
        self.stop.send_replace(true);
        let mut active = self.active.subscribe();
        // Two graceful timeouts plus kill time is the worst case inside the supervisor.
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
                g.bridge_pid = None;
                g.started_at = None;
                g.stats = Stats::default();
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

    fn spawn_sidecar(
        &self,
        app: &AppHandle,
        name: &str,
        args: &[String],
    ) -> anyhow::Result<(mpsc::Receiver<ChildEvent>, NodeChild)> {
        // The shell plugin resolves the bundled sidecar path (and hides the console window on
        // Windows); crate::child spawns it so its lifetime is tied to Desktop's.
        let cmd = app.shell().sidecar(name)?.args(args).current_dir(&self.work_dir);
        let (rx, child) = crate::child::spawn(cmd.into())?;
        if let Err(e) = crate::job::bind_to_desktop_lifetime(child.pid()) {
            self.push_log(app, format!("DESKTOP: warning: {e}"));
        }
        Ok((rx, child))
    }

    async fn run_session(&self, app: &AppHandle, stop: &mut watch::Receiver<bool>) -> SessionEnd {
        let launch = {
            let mut g = self.inner.lock().await;
            g.state = MinerState::Starting;
            g.restart_pending = false;
            g.last_error = None;
            g.miner_error_line = None;
            g.fatal = None;
            g.bridge_ready = false;
            g.stats = Stats::default();
            g.launch.clone()
        };
        let Some(launch) = launch else {
            return SessionEnd::Failed { reason: "No mining settings.".into(), fatal: true };
        };
        self.emit_status(app).await;

        // --- Phase 1: bridge, then wait for the node -------------------------------------
        self.push_log(app, format!("DESKTOP: starting {BRIDGE_SIDECAR} {}", launch.bridge_args.join(" ")));
        let (mut bridge_rx, bridge) = match self.spawn_sidecar(app, BRIDGE_SIDECAR, &launch.bridge_args) {
            Ok(v) => v,
            Err(e) => return SessionEnd::Failed { reason: format!("Cannot start the stratum bridge: {e}"), fatal: true },
        };
        let mut bridge_lines = LineBuffer::default();
        {
            let mut g = self.inner.lock().await;
            g.bridge_pid = Some(bridge.pid());
            g.state = MinerState::Waiting;
        }
        self.emit_status(app).await;
        self.push_log(app, "DESKTOP: waiting for the node to be fully synced; mining starts by itself".into());

        let mut probe = tokio::time::interval(PROBE_INTERVAL);
        loop {
            tokio::select! {
                ev = bridge_rx.recv() => {
                    if let Some(code) = self.handle_event(app, ev, &mut bridge_lines, Source::Bridge).await {
                        return self.bridge_failure(code).await;
                    }
                }
                _ = probe.tick() => {
                    // The port must answer *and* our bridge must have announced it, so a
                    // foreign program already listening on that port is not mistaken for it.
                    let (ready, fatal) = {
                        let g = self.inner.lock().await;
                        (g.bridge_ready, g.fatal.clone())
                    };
                    if let Some(reason) = fatal {
                        // e.g. the port is taken: do not wait for a bridge that cannot listen.
                        self.shutdown(app, &bridge, &mut bridge_rx, &mut bridge_lines, Source::Bridge).await;
                        return SessionEnd::Failed { reason, fatal: true };
                    }
                    if ready && tokio::net::TcpStream::connect(("127.0.0.1", launch.stratum_port)).await.is_ok() {
                        break;
                    }
                }
                _ = stop_requested(stop) => {
                    self.set_state(app, MinerState::Stopping).await;
                    self.shutdown(app, &bridge, &mut bridge_rx, &mut bridge_lines, Source::Bridge).await;
                    return SessionEnd::Stopped;
                }
            }
        }

        // --- Phase 2: miner ---------------------------------------------------------------
        self.push_log(app, format!("DESKTOP: node is synced, starting {MINER_SIDECAR} {}", launch.miner_args.join(" ")));
        let (mut miner_rx, miner) = match self.spawn_sidecar(app, MINER_SIDECAR, &launch.miner_args) {
            Ok(v) => v,
            Err(e) => {
                self.shutdown(app, &bridge, &mut bridge_rx, &mut bridge_lines, Source::Bridge).await;
                return SessionEnd::Failed { reason: format!("Cannot start KarlsenMiner: {e}"), fatal: true };
            }
        };
        let mut miner_lines = LineBuffer::default();
        {
            let mut g = self.inner.lock().await;
            g.miner_pid = Some(miner.pid());
            g.state = MinerState::Running;
            g.started_at = Some(Instant::now());
        }
        self.emit_status(app).await;

        loop {
            tokio::select! {
                ev = miner_rx.recv() => {
                    if let Some(code) = self.handle_event(app, ev, &mut miner_lines, Source::Miner).await {
                        self.push_log(app, format!("DESKTOP: karlsenminer exited (code {code:?})"));
                        self.shutdown(app, &bridge, &mut bridge_rx, &mut bridge_lines, Source::Bridge).await;
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
                ev = bridge_rx.recv() => {
                    if let Some(code) = self.handle_event(app, ev, &mut bridge_lines, Source::Bridge).await {
                        // Without the bridge the miner has nothing to mine on.
                        self.shutdown(app, &miner, &mut miner_rx, &mut miner_lines, Source::Miner).await;
                        return self.bridge_failure(code).await;
                    }
                }
                _ = stop_requested(stop) => {
                    self.set_state(app, MinerState::Stopping).await;
                    self.push_log(app, "DESKTOP: stopping mining".into());
                    self.shutdown(app, &miner, &mut miner_rx, &mut miner_lines, Source::Miner).await;
                    self.shutdown(app, &bridge, &mut bridge_rx, &mut bridge_lines, Source::Bridge).await;
                    return SessionEnd::Stopped;
                }
            }
        }
    }

    async fn bridge_failure(&self, code: Option<i32>) -> SessionEnd {
        match self.inner.lock().await.fatal.clone() {
            Some(reason) => SessionEnd::Failed { reason, fatal: true },
            None => SessionEnd::Failed { reason: format!("The stratum bridge exited (code {code:?})."), fatal: false },
        }
    }

    /// Log one child event. Returns `Some(exit code)` when the process has terminated.
    async fn handle_event(
        &self,
        app: &AppHandle,
        event: Option<ChildEvent>,
        lines: &mut LineBuffer,
        source: Source,
    ) -> Option<Option<i32>> {
        match event {
            Some(ChildEvent::Stdout(bytes)) | Some(ChildEvent::Stderr(bytes)) => {
                for line in lines.feed(&bytes) {
                    self.absorb_line(&line, source).await;
                    self.push_log(app, if source == Source::Bridge { format!("BRIDGE: {line}") } else { line });
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

    /// Update statistics and error state from one line of output.
    async fn absorb_line(&self, line: &str, source: Source) {
        match source {
            Source::Miner => {
                let info = parse_miner_line(line);
                if info == MinerLine::default() {
                    return;
                }
                let mut g = self.inner.lock().await;
                if let Some(h) = info.hashrate_mhs {
                    g.stats.hashrate_mhs = Some(h);
                }
                if let Some((a, r)) = info.shares {
                    g.stats.accepted = Some(a);
                    g.stats.rejected = Some(r);
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
            }
            Source::Bridge => match parse_bridge_line(line) {
                BridgeLine::Ready => self.inner.lock().await.bridge_ready = true,
                BridgeLine::BlockAccepted => self.inner.lock().await.blocks_found += 1,
                BridgeLine::PortInUse => {
                    self.inner.lock().await.fatal = Some(
                        "The stratum port is already in use by another program (another stratum bridge?). \
                         Close it or choose a different port in the mining settings."
                            .into(),
                    )
                }
                BridgeLine::Other => {}
            },
        }
    }

    /// Stop a child (the miner gracefully where possible, the bridge by kill) and drain its
    /// output until it has terminated.
    async fn shutdown(
        &self,
        app: &AppHandle,
        child: &NodeChild,
        rx: &mut mpsc::Receiver<ChildEvent>,
        lines: &mut LineBuffer,
        source: Source,
    ) {
        // The bridge is killed outright: it keeps no state, by now it has no miner left to
        // serve, and while it is still waiting for the node it does not react to Ctrl+C at
        // all (bridge v3.3.2), which would make "Stop mining" hang for the grace period.
        #[cfg(unix)]
        if source == Source::Miner {
            // KarlsenMiner shuts down cleanly on SIGINT (its own Ctrl+C path).
            unsafe {
                libc::kill(child.pid() as libc::pid_t, libc::SIGINT);
            }
        } else {
            let _ = child.kill();
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
                    if self.handle_event(app, ev, lines, source).await.is_some() {
                        break;
                    }
                }
            })
            .await;
            if drained.is_ok() {
                return;
            }
            if attempt == 0 {
                self.push_log(app, "DESKTOP: process did not stop in time, killing".into());
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

/// What one line of KarlsenMiner output tells Desktop.
#[derive(Debug, Default, PartialEq)]
struct MinerLine {
    hashrate_mhs: Option<f64>,
    /// (accepted, rejected)
    shares: Option<(u64, u64)>,
    error: Option<String>,
    no_gpu: bool,
}

/// Parse a leading unsigned decimal number, returning it and the rest of the string.
fn leading_number(s: &str) -> Option<(f64, &str)> {
    let end = s.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(s.len());
    let value = s[..end].parse::<f64>().ok()?;
    Some((value, &s[end..]))
}

fn leading_integer(s: &str) -> Option<(u64, &str)> {
    let end = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    let value = s[..end].parse::<u64>().ok()?;
    Some((value, &s[end..]))
}

/// Parse the miner's summary line, e.g.
/// `Total: 58.37 MH/s (avg 58.12 MH/s) | A:106/R:2 | 0.375 | 156W | Best: 4.21`,
/// and its `ERROR:` lines. Input must already be free of ANSI sequences.
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
        if let Some(a_pos) = rest.find("A:") {
            if let Some((accepted, after)) = leading_integer(&rest[a_pos + 2..]) {
                if let Some((rejected, _)) = after.strip_prefix("/R:").and_then(leading_integer) {
                    info.shares = Some((accepted, rejected));
                }
            }
        }
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

#[derive(Debug, PartialEq)]
enum BridgeLine {
    /// "Node is synced, starting stratum listeners"
    Ready,
    /// The node accepted a block mined here.
    BlockAccepted,
    /// The stratum port could not be bound.
    PortInUse,
    Other,
}

/// Recognise the karlsen-stratum-bridge v3.3.2 log lines Desktop reacts to.
fn parse_bridge_line(line: &str) -> BridgeLine {
    // Not just "starting stratum listeners": the earlier "Waiting for node to fully sync
    // before starting stratum listeners" line contains that too.
    if line.contains("Node is synced, starting stratum listeners") {
        BridgeLine::Ready
    } else if line.contains("BLOCK ACCEPTED BY NODE!") {
        // Printed once per block by the share handler (the API layer logs the same event
        // as "BLOCK ACCEPTED BY KARLSEN NODE", which is deliberately not matched).
        BridgeLine::BlockAccepted
    } else if line.contains("failed listening to socket") {
        BridgeLine::PortInUse
    } else {
        BridgeLine::Other
    }
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
        let line = "   Total: 58.37 MH/s (avg 58.12 MH/s) | A:106/R:2 | 0.375 | 156W | Best: 4.21";
        let info = parse_miner_line(line);
        assert_eq!(info.hashrate_mhs, Some(58.37));
        assert_eq!(info.shares, Some((106, 2)));
        assert_eq!(info.error, None);
        assert_eq!(parse_miner_line("Total: 1.5 GH/s | A:0/R:0").hashrate_mhs, Some(1500.0));
        assert_eq!(parse_miner_line("Total: 950.0 kH/s").hashrate_mhs, Some(0.95));
    }

    #[test]
    fn ignores_ordinary_miner_lines_and_per_gpu_rows() {
        assert_eq!(parse_miner_line("[2026-11-16 20:55:57] Enumerating CUDA GPU devices..."), MinerLine::default());
        assert_eq!(
            parse_miner_line("     0  RX 6700 XT        29.27 MH/s    51             0   0.353     83W"),
            MinerLine::default()
        );
        assert_eq!(parse_miner_line("Total: starting"), MinerLine::default());
    }

    #[test]
    fn detects_missing_gpu() {
        // Real output of karlsenminer 3.2.0 on a machine without a GPU.
        let raw = "[2026-11-16 20:55:57] \u{1b}[31mERROR: no usable GPU found (OpenCL or CUDA)\u{1b}[0m";
        assert!(parse_miner_line(&strip_ansi(raw)).no_gpu);
        let other = parse_miner_line("[..] ERROR: pool connection lost");
        assert!(!other.no_gpu);
        assert_eq!(other.error.as_deref(), Some("pool connection lost"));
    }

    #[test]
    fn recognises_bridge_lines() {
        // Real lines of karlsen-stratum-bridge 3.3.2.
        let waiting = "2026-11-16 21:17:04.928+00:00 [INFO]  karlsen_stratum_bridge: Waiting for node to fully sync before starting stratum listeners";
        assert_eq!(parse_bridge_line(waiting), BridgeLine::Other);
        let ready = "2026-11-16 21:17:09.100+00:00 [INFO]  karlsen_stratum_bridge: Node is synced, starting stratum listeners";
        assert_eq!(parse_bridge_line(ready), BridgeLine::Ready);
        assert_eq!(parse_bridge_line("[127.0.0.1:5555] [BLOCK] BLOCK ACCEPTED BY NODE! Hash: abc"), BridgeLine::BlockAccepted);
        assert_eq!(parse_bridge_line("[API] ===== BLOCK ACCEPTED BY KARLSEN NODE ===== Hash: abc"), BridgeLine::Other);
        assert_eq!(
            parse_bridge_line("failed listening to socket 127.0.0.1:5555: Address already in use"),
            BridgeLine::PortInUse
        );
        assert_eq!(parse_bridge_line("Connecting to Karlsen node at 127.0.0.1:42110"), BridgeLine::Other);
    }
}
