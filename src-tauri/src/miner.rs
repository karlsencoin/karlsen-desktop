//! KarlsenMiner process manager: install, start, stop, restart, log capture and
//! hashrate / share statistics parsed from the miner's output.
//!
//! The miner runs as a child process exactly like karlsend (see `child.rs` / `job.rs`), so
//! it can never outlive Karlsen Desktop and a miner crash never takes the UI down.

use crate::child::{ChildEvent, NodeChild};
use crate::miner_install::{self, InstallProgress, MINER_VERSION};
use serde::Serialize;
use std::collections::VecDeque;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::process::Command as StdCommand;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};
use tokio::sync::{Mutex, Notify};

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

const LOG_CAPACITY: usize = 2_000;
/// Time the miner gets to shut down by itself after the stop signal.
const GRACEFUL_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_BACKOFF: Duration = Duration::from_secs(60);
/// A run at least this long counts as stable and clears the crash streak.
const STABLE_RUN: Duration = Duration::from_secs(600);
/// Consecutive unexpected exits after which Desktop stops restarting the miner.
const MAX_RESTARTS: u32 = 5;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MinerState {
    Stopped,
    Starting,
    Running,
    Stopping,
    Crashed,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MinerStatus {
    /// False on platforms KarlsenMiner has no build for.
    pub supported: bool,
    /// KarlsenMiner version this Desktop build installs and runs.
    pub version: &'static str,
    /// The verified executable is present.
    pub installed: bool,
    /// Set while a download / install is in progress.
    pub install: Option<InstallProgress>,
    pub state: MinerState,
    pub pid: Option<u32>,
    pub uptime_secs: Option<u64>,
    /// Total hashrate in MH/s from the miner's latest status line.
    pub hashrate_mhs: Option<f64>,
    pub accepted: Option<u64>,
    pub rejected: Option<u64>,
    /// Consecutive unexpected exits; reset after a stable run.
    pub restarts: u32,
    /// An automatic restart is scheduled after an unexpected exit.
    pub restart_pending: bool,
    pub last_exit_code: Option<i32>,
    /// Why the miner is not running, for the UI (e.g. no usable GPU).
    pub last_error: Option<String>,
}

#[derive(Default)]
struct Stats {
    hashrate_mhs: Option<f64>,
    accepted: Option<u64>,
    rejected: Option<u64>,
}

struct Inner {
    child: Option<NodeChild>,
    state: MinerState,
    started_at: Option<Instant>,
    restarts: u32,
    last_exit_code: Option<i32>,
    last_error: Option<String>,
    /// Last "ERROR:" line printed by the miner in the current run.
    last_error_line: Option<String>,
    /// The miner reported a condition a restart cannot fix (no GPU).
    fatal: bool,
    stop_requested: bool,
    restart_pending: bool,
    bin: PathBuf,
    args: Vec<String>,
    stats: Stats,
}

#[derive(Clone)]
pub struct MinerManager {
    inner: Arc<Mutex<Inner>>,
    logs: Arc<std::sync::Mutex<VecDeque<String>>>,
    install: Arc<std::sync::Mutex<Option<InstallProgress>>>,
    /// Cached result of the executable hash check, so status polls do not re-hash the file.
    installed: Arc<AtomicBool>,
    exited: Arc<Notify>,
    /// `<app local data>/miner`: holds `karlsenminer-v<version>/`.
    miner_root: PathBuf,
}

impl MinerManager {
    pub fn new(miner_root: PathBuf) -> Self {
        let installed = miner_install::package().is_some_and(|p| miner_install::is_installed(&miner_root, p));
        Self {
            installed: Arc::new(AtomicBool::new(installed)),
            inner: Arc::new(Mutex::new(Inner {
                child: None,
                state: MinerState::Stopped,
                started_at: None,
                restarts: 0,
                last_exit_code: None,
                last_error: None,
                last_error_line: None,
                fatal: false,
                stop_requested: false,
                restart_pending: false,
                bin: PathBuf::new(),
                args: Vec::new(),
                stats: Stats::default(),
            })),
            logs: Arc::new(std::sync::Mutex::new(VecDeque::with_capacity(LOG_CAPACITY))),
            install: Arc::new(std::sync::Mutex::new(None)),
            exited: Arc::new(Notify::new()),
            miner_root,
        }
    }

    pub async fn status(&self) -> MinerStatus {
        let pkg = miner_install::package();
        let install = self.install.lock().unwrap().clone();
        let installed = install.is_none() && self.installed.load(Ordering::Relaxed);
        let mut g = self.inner.lock().await;
        if g.state == MinerState::Running && g.started_at.is_some_and(|t| t.elapsed() >= STABLE_RUN) {
            g.restarts = 0;
        }
        MinerStatus {
            supported: pkg.is_some(),
            version: MINER_VERSION,
            installed,
            install,
            state: g.state,
            pid: g.child.as_ref().map(|c| c.pid()),
            uptime_secs: g.started_at.map(|t| t.elapsed().as_secs()),
            hashrate_mhs: g.stats.hashrate_mhs,
            accepted: g.stats.accepted,
            rejected: g.stats.rejected,
            restarts: g.restarts,
            restart_pending: g.restart_pending,
            last_exit_code: g.last_exit_code,
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

    /// Download, verify and install the pinned KarlsenMiner release.
    pub async fn install(&self, app: &AppHandle) -> anyhow::Result<()> {
        let pkg = miner_install::package()
            .ok_or_else(|| anyhow::anyhow!("KarlsenMiner has no build for this platform (Windows x64 and Linux x64 only)."))?;
        if self.inner.lock().await.child.is_some() {
            anyhow::bail!("Stop the miner before reinstalling it.");
        }
        {
            let mut slot = self.install.lock().unwrap();
            if slot.is_some() {
                anyhow::bail!("An installation is already running.");
            }
            *slot = Some(InstallProgress { stage: "downloading", received_bytes: 0, total_bytes: None });
        }
        self.push_log(app, format!("DESKTOP: downloading KarlsenMiner v{MINER_VERSION} from {}", pkg.url));
        self.emit_status(app).await;

        let (root, slot, events) = (self.miner_root.clone(), self.install.clone(), app.clone());
        let result = tokio::task::spawn_blocking(move || {
            let mut last_emit = Instant::now();
            miner_install::download_and_install(&root, pkg, |p| {
                let downloading = p.stage == "downloading";
                *slot.lock().unwrap() = Some(p.clone());
                // Progress events are throttled; stage changes always go out.
                if !downloading || last_emit.elapsed() >= Duration::from_millis(150) {
                    last_emit = Instant::now();
                    let _ = events.emit("miner-install", p);
                }
            })
        })
        .await;
        *self.install.lock().unwrap() = None;

        let outcome = match result {
            Ok(Ok(path)) => {
                self.installed.store(true, Ordering::Relaxed);
                self.inner.lock().await.last_error = None;
                self.push_log(app, format!("DESKTOP: KarlsenMiner v{MINER_VERSION} verified and installed at {}", path.display()));
                Ok(())
            }
            Ok(Err(e)) => {
                self.push_log(app, format!("DESKTOP: install failed: {e}"));
                Err(anyhow::anyhow!(e))
            }
            Err(e) => Err(anyhow::anyhow!("install task failed: {e}")),
        };
        self.emit_status(app).await;
        outcome
    }

    /// Remove the installed miner. Refused while it is running.
    pub async fn uninstall(&self, app: &AppHandle) -> anyhow::Result<()> {
        if self.inner.lock().await.child.is_some() {
            anyhow::bail!("Stop the miner before removing it.");
        }
        if self.install.lock().unwrap().is_some() {
            anyhow::bail!("An installation is running.");
        }
        miner_install::uninstall(&self.miner_root).map_err(|e| anyhow::anyhow!(e))?;
        self.installed.store(false, Ordering::Relaxed);
        self.push_log(app, "DESKTOP: KarlsenMiner removed".into());
        self.emit_status(app).await;
        Ok(())
    }

    /// Start mining with an already validated command line (`MinerSettings::miner_args`).
    pub async fn start(&self, app: &AppHandle, args_for: impl FnOnce(&std::path::Path) -> Result<Vec<String>, String>) -> anyhow::Result<()> {
        let pkg = miner_install::package()
            .ok_or_else(|| anyhow::anyhow!("KarlsenMiner has no build for this platform (Windows x64 and Linux x64 only)."))?;
        if self.inner.lock().await.child.is_some() {
            return Ok(());
        }
        if self.install.lock().unwrap().is_some() {
            anyhow::bail!("KarlsenMiner is still being installed.");
        }
        let config_file = miner_install::install_dir(&self.miner_root).join("karlsenminer_config.json");
        let args = args_for(&config_file).map_err(|e| anyhow::anyhow!(e))?;
        let mut g = self.inner.lock().await;
        g.bin = miner_install::binary_path(&self.miner_root, pkg);
        g.args = args;
        g.stop_requested = false;
        g.restarts = 0;
        drop(g);
        self.spawn(app).await
    }

    /// Boxed explicitly: spawn -> on_exit -> spawn is recursive.
    fn spawn<'a>(&'a self, app: &'a AppHandle) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(self.spawn_inner(app))
    }

    async fn fail_start(&self, app: &AppHandle, error: String) -> anyhow::Error {
        {
            let mut g = self.inner.lock().await;
            g.state = MinerState::Crashed;
            g.last_error = Some(error.clone());
        }
        self.push_log(app, format!("DESKTOP: {error}"));
        self.emit_status(app).await;
        anyhow::anyhow!(error)
    }

    async fn spawn_inner(&self, app: &AppHandle) -> anyhow::Result<()> {
        let (bin, args) = {
            let mut g = self.inner.lock().await;
            g.state = MinerState::Starting;
            g.restart_pending = false;
            g.last_error = None;
            g.last_error_line = None;
            g.fatal = false;
            g.stats = Stats::default();
            (g.bin.clone(), g.args.clone())
        };
        self.emit_status(app).await;

        // Never execute a file that is not the pinned release binary.
        let root = self.miner_root.clone();
        let verified = tokio::task::spawn_blocking(move || {
            miner_install::package()
                .ok_or_else(|| "KarlsenMiner has no build for this platform.".to_string())
                .and_then(|pkg| miner_install::verify_installed(&root, pkg))
        })
        .await
        .unwrap_or_else(|e| Err(format!("verification task failed: {e}")));
        self.installed.store(verified.is_ok(), Ordering::Relaxed);
        if let Err(e) = verified {
            return Err(self.fail_start(app, e).await);
        }

        self.push_log(app, format!("DESKTOP: starting karlsenminer {}", args.join(" ")));
        let mut cmd = StdCommand::new(&bin);
        cmd.args(&args);
        if let Some(dir) = bin.parent() {
            cmd.current_dir(dir);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let (mut rx, child) = match crate::child::spawn(cmd) {
            Ok(v) => v,
            Err(e) => return Err(self.fail_start(app, format!("cannot start karlsenminer: {e}")).await),
        };
        if let Err(e) = crate::job::bind_to_desktop_lifetime(child.pid()) {
            self.push_log(app, format!("DESKTOP: warning: {e}"));
        }
        {
            let mut g = self.inner.lock().await;
            g.child = Some(child);
            g.state = MinerState::Running;
            g.started_at = Some(Instant::now());
        }
        self.emit_status(app).await;

        let this = self.clone();
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            let mut partial = String::new();
            while let Some(event) = rx.recv().await {
                match event {
                    ChildEvent::Stdout(bytes) | ChildEvent::Stderr(bytes) => {
                        partial.push_str(&String::from_utf8_lossy(&bytes));
                        // '\r' also ends a line: a status display may redraw in place.
                        while let Some(pos) = partial.find(['\n', '\r']) {
                            let raw: String = partial.drain(..=pos).collect();
                            let line = strip_ansi(&raw).trim_end().to_string();
                            if line.is_empty() {
                                continue;
                            }
                            this.absorb_line(&line).await;
                            this.push_log(&app, line);
                        }
                    }
                    ChildEvent::Error(e) => this.push_log(&app, format!("DESKTOP: miner error: {e}")),
                    ChildEvent::Terminated(code) => {
                        this.on_exit(&app, code).await;
                        break;
                    }
                }
            }
        });
        Ok(())
    }

    /// Update statistics and error state from one line of miner output.
    async fn absorb_line(&self, line: &str) {
        let info = parse_line(line);
        if info == LineInfo::default() {
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
        if let Some(e) = info.error {
            g.last_error_line = Some(e);
        }
        g.fatal |= info.fatal;
    }

    async fn on_exit(&self, app: &AppHandle, code: Option<i32>) {
        let (restart, backoff) = {
            let mut g = self.inner.lock().await;
            if g.started_at.is_some_and(|t| t.elapsed() >= STABLE_RUN) {
                g.restarts = 0;
            }
            g.child = None;
            g.started_at = None;
            g.last_exit_code = code;
            g.stats = Stats::default();
            if g.stop_requested {
                g.state = MinerState::Stopped;
                (false, Duration::ZERO)
            } else {
                g.state = MinerState::Crashed;
                g.restarts += 1;
                let reason = g.last_error_line.clone().unwrap_or_else(|| format!("KarlsenMiner exited (code {code:?})."));
                if g.fatal {
                    // e.g. no GPU: restarting cannot help.
                    g.last_error = Some(reason);
                    (false, Duration::ZERO)
                } else if g.restarts > MAX_RESTARTS {
                    g.last_error = Some(format!("{reason} Stopped after {MAX_RESTARTS} failed restarts."));
                    (false, Duration::ZERO)
                } else {
                    g.last_error = Some(reason);
                    g.restart_pending = true;
                    (true, Duration::from_secs(2u64.saturating_pow(g.restarts.min(6))).min(MAX_BACKOFF))
                }
            }
        };
        self.exited.notify_waiters();
        self.push_log(app, format!("DESKTOP: karlsenminer exited (code {code:?})"));
        self.emit_status(app).await;

        if restart {
            self.push_log(app, format!("DESKTOP: restarting the miner in {}s", backoff.as_secs()));
            let this = self.clone();
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(backoff).await;
                let skip = {
                    let g = this.inner.lock().await;
                    g.stop_requested || g.child.is_some()
                };
                if !skip {
                    if let Err(e) = this.spawn(&app).await {
                        this.push_log(&app, format!("DESKTOP: restart failed: {e}"));
                    }
                }
            });
        }
    }

    /// Stop the miner: SIGINT on Unix (the miner's own Ctrl+C path), then a hard kill.
    pub async fn stop(&self, app: &AppHandle) -> anyhow::Result<()> {
        let pid = {
            let mut g = self.inner.lock().await;
            g.stop_requested = true;
            g.restart_pending = false;
            let Some(pid) = g.child.as_ref().map(|c| c.pid()) else {
                // Also cancels a pending automatic restart.
                if g.state != MinerState::Stopped {
                    g.state = MinerState::Stopped;
                    drop(g);
                    self.emit_status(app).await;
                }
                return Ok(());
            };
            g.state = MinerState::Stopping;
            pid
        };
        self.emit_status(app).await;
        self.push_log(app, "DESKTOP: stopping karlsenminer".into());

        let exited = self.exited.notified();
        tokio::pin!(exited);
        exited.as_mut().enable();

        #[cfg(unix)]
        unsafe {
            libc::kill(pid as libc::pid_t, libc::SIGINT);
        }
        #[cfg(not(unix))]
        {
            // No Ctrl+C for a console-less child on Windows. Desktop never passes NVIDIA
            // tuning options, so there is no GPU state to restore and a kill is clean.
            let _ = pid;
            if let Some(child) = self.inner.lock().await.child.clone() {
                let _ = child.kill();
            }
        }

        if tokio::time::timeout(GRACEFUL_TIMEOUT, exited).await.is_err() {
            self.push_log(app, "DESKTOP: miner did not stop in time, killing".into());
            if let Some(child) = self.inner.lock().await.child.clone() {
                let _ = child.kill();
            }
        }
        Ok(())
    }
}

/// What one line of miner output tells Desktop.
#[derive(Debug, Default, PartialEq)]
struct LineInfo {
    hashrate_mhs: Option<f64>,
    /// (accepted, rejected)
    shares: Option<(u64, u64)>,
    error: Option<String>,
    fatal: bool,
}

/// Remove ANSI escape sequences (colours, cursor movement) from miner output.
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
fn parse_line(line: &str) -> LineInfo {
    let mut info = LineInfo::default();
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
            info.fatal = text.contains("no usable GPU");
            info.error = Some(if info.fatal {
                "No usable GPU found. KarlsenMiner needs an AMD, NVIDIA or Intel Arc GPU with 6 GB+ VRAM and a working OpenCL or CUDA driver."
                    .to_string()
            } else {
                text.to_string()
            });
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
    fn parses_summary_line() {
        let line = "   Total: 58.37 MH/s (avg 58.12 MH/s) | A:106/R:2 | 0.375 | 156W | Best: 4.21";
        let info = parse_line(line);
        assert_eq!(info.hashrate_mhs, Some(58.37));
        assert_eq!(info.shares, Some((106, 2)));
        assert_eq!(info.error, None);
        assert_eq!(parse_line("Total: 1.5 GH/s | A:0/R:0").hashrate_mhs, Some(1500.0));
        assert_eq!(parse_line("Total: 950.0 kH/s").hashrate_mhs, Some(0.95));
    }

    #[test]
    fn ignores_ordinary_lines_and_per_gpu_rows() {
        assert_eq!(parse_line("[2026-11-16 20:55:57] Enumerating CUDA GPU devices..."), LineInfo::default());
        assert_eq!(parse_line("     0  RX 6700 XT        29.27 MH/s    51             0   0.353     83W"), LineInfo::default());
        assert_eq!(parse_line("Total: starting"), LineInfo::default());
    }

    #[test]
    fn detects_missing_gpu_as_fatal() {
        // Real output of karlsenminer 3.2.0 on a machine without a GPU.
        let raw = "[2026-11-16 20:55:57] \u{1b}[31mERROR: no usable GPU found (OpenCL or CUDA)\u{1b}[0m";
        let info = parse_line(&strip_ansi(raw));
        assert!(info.fatal);
        assert!(info.error.unwrap().contains("6 GB+ VRAM"));
        let other = parse_line("[..] ERROR: pool connection lost");
        assert!(!other.fatal);
        assert_eq!(other.error.as_deref(), Some("pool connection lost"));
    }
}
