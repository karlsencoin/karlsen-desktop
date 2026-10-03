//! karlsend sidecar process manager: start, stop, auto-restart and log capture.

use serde::Serialize;
use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};
use tauri_plugin_shell::process::{CommandChild, CommandEvent};
use tauri_plugin_shell::ShellExt;
use tokio::sync::{Mutex, Notify};

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

const LOG_CAPACITY: usize = 5_000;
const GRACEFUL_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_BACKOFF: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum NodeState {
    Stopped,
    Starting,
    Running,
    Stopping,
    Crashed,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeStatus {
    pub state: NodeState,
    pub pid: Option<u32>,
    pub uptime_secs: Option<u64>,
    pub restarts: u32,
    pub last_exit_code: Option<i32>,
    pub data_dir: String,
}

struct Inner {
    child: Option<CommandChild>,
    state: NodeState,
    started_at: Option<Instant>,
    restarts: u32,
    last_exit_code: Option<i32>,
    /// Set when the user asked for a stop, so the exit is not treated as a crash.
    stop_requested: bool,
    data_dir: PathBuf,
    args: Vec<String>,
    auto_restart: bool,
}

#[derive(Clone)]
pub struct NodeManager {
    inner: Arc<Mutex<Inner>>,
    logs: Arc<std::sync::Mutex<VecDeque<String>>>,
    exited: Arc<Notify>,
}

impl NodeManager {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                child: None,
                state: NodeState::Stopped,
                started_at: None,
                restarts: 0,
                last_exit_code: None,
                stop_requested: false,
                data_dir: PathBuf::new(),
                args: Vec::new(),
                auto_restart: true,
            })),
            logs: Arc::new(std::sync::Mutex::new(VecDeque::with_capacity(LOG_CAPACITY))),
            exited: Arc::new(Notify::new()),
        }
    }

    pub async fn status(&self) -> NodeStatus {
        let g = self.inner.lock().await;
        NodeStatus {
            state: g.state,
            pid: g.child.as_ref().map(|c| c.pid()),
            uptime_secs: g.started_at.map(|t| t.elapsed().as_secs()),
            restarts: g.restarts,
            last_exit_code: g.last_exit_code,
            data_dir: g.data_dir.display().to_string(),
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
        let _ = app.emit("node-log", line);
    }

    async fn emit_status(&self, app: &AppHandle) {
        let _ = app.emit("node-status", self.status().await);
    }

    /// Start karlsend with the given data dir and arguments.
    pub async fn start(
        &self,
        app: &AppHandle,
        data_dir: PathBuf,
        args: Vec<String>,
        auto_restart: bool,
    ) -> anyhow::Result<()> {
        {
            let mut g = self.inner.lock().await;
            if g.child.is_some() {
                return Ok(());
            }
            g.data_dir = data_dir.clone();
            g.args = args;
            g.auto_restart = auto_restart;
            g.stop_requested = false;
        }
        std::fs::create_dir_all(&data_dir)?;
        self.spawn(app).await
    }

    /// Boxed explicitly: spawn -> on_exit -> spawn is recursive, and an explicit
    /// `Send` bound lets the restart task be spawned on the runtime.
    fn spawn<'a>(&'a self, app: &'a AppHandle) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(self.spawn_inner(app))
    }

    async fn spawn_inner(&self, app: &AppHandle) -> anyhow::Result<()> {
        let args = {
            let mut g = self.inner.lock().await;
            g.state = NodeState::Starting;
            g.args.clone()
        };
        self.emit_status(app).await;
        self.push_log(app, format!("DESKTOP: starting karlsend {}", args.join(" ")));

        let spawned = app.shell().sidecar("karlsend").and_then(|c| c.args(&args).spawn());
        let (mut rx, child) = match spawned {
            Ok(v) => v,
            Err(e) => {
                self.inner.lock().await.state = NodeState::Crashed;
                self.emit_status(app).await;
                return Err(e.into());
            }
        };
        {
            let mut g = self.inner.lock().await;
            g.child = Some(child);
            g.state = NodeState::Running;
            g.started_at = Some(Instant::now());
        }
        self.emit_status(app).await;

        let this = self.clone();
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            let mut partial = String::new();
            while let Some(event) = rx.recv().await {
                match event {
                    CommandEvent::Stdout(bytes) | CommandEvent::Stderr(bytes) => {
                        partial.push_str(&String::from_utf8_lossy(&bytes));
                        while let Some(pos) = partial.find('\n') {
                            let line: String = partial.drain(..=pos).collect();
                            let line = line.trim_end().to_string();
                            if !line.is_empty() {
                                this.push_log(&app, line);
                            }
                        }
                    }
                    CommandEvent::Error(e) => this.push_log(&app, format!("DESKTOP: node error: {e}")),
                    CommandEvent::Terminated(payload) => {
                        this.on_exit(&app, payload.code).await;
                        break;
                    }
                    _ => {}
                }
            }
        });
        Ok(())
    }

    async fn on_exit(&self, app: &AppHandle, code: Option<i32>) {
        let (restart, backoff) = {
            let mut g = self.inner.lock().await;
            g.child = None;
            g.started_at = None;
            g.last_exit_code = code;
            if g.stop_requested {
                g.state = NodeState::Stopped;
                (false, Duration::ZERO)
            } else {
                g.state = NodeState::Crashed;
                g.restarts += 1;
                let backoff = Duration::from_secs(2u64.saturating_pow(g.restarts.min(6))).min(MAX_BACKOFF);
                (g.auto_restart, backoff)
            }
        };
        self.exited.notify_waiters();
        self.push_log(app, format!("DESKTOP: karlsend exited (code {code:?})"));
        self.emit_status(app).await;

        if restart {
            self.push_log(app, format!("DESKTOP: restarting in {}s", backoff.as_secs()));
            let this = self.clone();
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(backoff).await;
                // Skip if the user stopped or restarted the node in the meantime.
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

    /// Stop karlsend gracefully (SIGTERM on Unix), falling back to a hard kill.
    pub async fn stop(&self, app: &AppHandle) -> anyhow::Result<()> {
        let pid = {
            let mut g = self.inner.lock().await;
            g.stop_requested = true;
            let Some(pid) = g.child.as_ref().map(|c| c.pid()) else {
                g.state = NodeState::Stopped;
                return Ok(());
            };
            g.state = NodeState::Stopping;
            pid
        };
        self.emit_status(app).await;
        self.push_log(app, "DESKTOP: stopping karlsend".into());

        let exited = self.exited.notified();
        tokio::pin!(exited);
        exited.as_mut().enable();

        #[cfg(unix)]
        unsafe {
            libc::kill(pid as libc::pid_t, libc::SIGTERM);
        }
        #[cfg(not(unix))]
        {
            // Windows has no SIGTERM equivalent for console-less children;
            // karlsend flushes RocksDB WAL on startup, so a kill is recoverable.
            let _ = pid;
            if let Some(child) = self.inner.lock().await.child.take() {
                let _ = child.kill();
            }
        }

        if tokio::time::timeout(GRACEFUL_TIMEOUT, exited).await.is_err() {
            self.push_log(app, "DESKTOP: graceful stop timed out, killing".into());
            if let Some(child) = self.inner.lock().await.child.take() {
                let _ = child.kill();
            }
        }
        Ok(())
    }

    /// Stop the node, delete its chain data and start a fresh sync.
    pub async fn delete_and_resync(&self, app: &AppHandle) -> anyhow::Result<()> {
        let (data_dir, args, auto_restart) = {
            let g = self.inner.lock().await;
            (g.data_dir.clone(), g.args.clone(), g.auto_restart)
        };
        self.stop(app).await?;
        // Layout (verified on karlsend 3.3.0): <appdir>/karlsen-mainnet/{datadir,logs}.
        // Only remove the chain database; keep logs so pre-resync issues stay inspectable.
        // Never touch the user-chosen root folder.
        let chain_dir = data_dir.join("karlsen-mainnet").join("datadir");
        if chain_dir.exists() {
            self.push_log(app, format!("DESKTOP: deleting {}", chain_dir.display()));
            std::fs::remove_dir_all(&chain_dir)?;
        }
        self.start(app, data_dir, args, auto_restart).await
    }
}
