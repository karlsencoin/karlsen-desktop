//! Spawns the karlsend sidecar and streams its output.
//!
//! tauri-plugin-shell's own `spawn()` offers no pre-exec hook, so the sidecar command is
//! resolved by the plugin (path, CREATE_NO_WINDOW on Windows) and spawned here instead.
//! That lets Linux set PR_SET_PDEATHSIG: the kernel sends karlsend SIGTERM the moment
//! Desktop dies, however it dies (SIGKILL, crash, closed terminal), so no orphaned node
//! keeps the RocksDB LOCK. Windows gets the same guarantee from the Job Object in `job.rs`.

use shared_child::SharedChild;
use std::io::{self, Read};
use std::process::{Command as StdCommand, Stdio};
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, OnceLock};
use tokio::sync::mpsc;

#[derive(Debug)]
pub enum ChildEvent {
    Stdout(Vec<u8>),
    Stderr(Vec<u8>),
    Error(String),
    Terminated(Option<i32>),
}

#[derive(Clone)]
pub struct NodeChild(Arc<SharedChild>);

impl NodeChild {
    pub fn pid(&self) -> u32 {
        self.0.id()
    }

    pub fn kill(&self) -> io::Result<()> {
        self.0.kill()
    }
}

type SpawnReply = std_mpsc::Sender<io::Result<SharedChild>>;

/// PR_SET_PDEATHSIG fires when the *thread* that forked the child exits, not the process.
/// Async runtime and blocking-pool threads can come and go, so every spawn happens on one
/// dedicated thread that lives as long as Desktop does.
fn spawner() -> &'static std_mpsc::Sender<(StdCommand, SpawnReply)> {
    static SPAWNER: OnceLock<std_mpsc::Sender<(StdCommand, SpawnReply)>> = OnceLock::new();
    SPAWNER.get_or_init(|| {
        let (tx, rx) = std_mpsc::channel::<(StdCommand, SpawnReply)>();
        std::thread::Builder::new()
            .name("karlsend-spawner".into())
            .spawn(move || {
                for (mut cmd, reply) in rx {
                    let _ = reply.send(SharedChild::spawn(&mut cmd));
                }
            })
            .expect("cannot start karlsend spawner thread");
        tx
    })
}

#[cfg(target_os = "linux")]
fn tie_to_parent(cmd: &mut StdCommand) {
    use std::os::unix::process::CommandExt;
    let parent = std::process::id() as libc::pid_t;
    // SAFETY: only async-signal-safe calls (prctl, getppid, _exit) run between fork and exec.
    unsafe {
        cmd.pre_exec(move || {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM as libc::c_ulong, 0, 0, 0) != 0 {
                return Err(io::Error::last_os_error());
            }
            // Desktop may have died between fork() and prctl(): then the signal never comes.
            if libc::getppid() != parent {
                libc::_exit(1);
            }
            Ok(())
        });
    }
}

#[cfg(not(target_os = "linux"))]
fn tie_to_parent(_cmd: &mut StdCommand) {}

fn spawn_reader<R: Read + Send + 'static>(
    mut src: R,
    tx: mpsc::Sender<ChildEvent>,
    wrap: fn(Vec<u8>) -> ChildEvent,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match src.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if tx.blocking_send(wrap(buf[..n].to_vec())).is_err() {
                        break;
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => {
                    let _ = tx.blocking_send(ChildEvent::Error(e.to_string()));
                    break;
                }
            }
        }
    })
}

/// Spawn `cmd` (normally `app.shell().sidecar("karlsend")?.args(..).into()`), returning
/// an event stream (output chunks and non-fatal errors, ending with exactly one
/// `Terminated`) and a handle.
pub fn spawn(mut cmd: StdCommand) -> io::Result<(mpsc::Receiver<ChildEvent>, NodeChild)> {
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    tie_to_parent(&mut cmd);

    let (reply_tx, reply_rx) = std_mpsc::channel();
    spawner()
        .send((cmd, reply_tx))
        .map_err(|_| io::Error::other("karlsend spawner thread is gone"))?;
    let child = reply_rx
        .recv()
        .map_err(|_| io::Error::other("karlsend spawner thread is gone"))??;

    let stdout = child.take_stdout().ok_or_else(|| io::Error::other("no stdout pipe"))?;
    let stderr = child.take_stderr().ok_or_else(|| io::Error::other("no stderr pipe"))?;
    let child = Arc::new(child);
    let (tx, rx) = mpsc::channel(256);

    let out = spawn_reader(stdout, tx.clone(), ChildEvent::Stdout);
    let err = spawn_reader(stderr, tx.clone(), ChildEvent::Stderr);
    let waiter = child.clone();
    std::thread::spawn(move || {
        let status = waiter.wait();
        // Deliver the last output lines before the exit event.
        let _ = out.join();
        let _ = err.join();
        let code = match status {
            Ok(s) => s.code(),
            Err(e) => {
                let _ = tx.blocking_send(ChildEvent::Error(format!("wait failed: {e}")));
                None
            }
        };
        // Always the final event, so the exit handler runs exactly once.
        let _ = tx.blocking_send(ChildEvent::Terminated(code));
    });

    Ok((rx, NodeChild(child)))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn streams_output_then_exit_code() {
        let mut cmd = StdCommand::new("sh");
        cmd.args(["-c", "echo out; echo err >&2; exit 3"]);
        let (mut rx, child) = spawn(cmd).unwrap();
        assert!(child.pid() > 0);
        let (mut out, mut err, mut code) = (Vec::new(), Vec::new(), None);
        while let Some(ev) = rx.blocking_recv() {
            match ev {
                ChildEvent::Stdout(b) => out.extend(b),
                ChildEvent::Stderr(b) => err.extend(b),
                ChildEvent::Error(e) => panic!("unexpected error {e}"),
                ChildEvent::Terminated(c) => {
                    code = Some(c);
                    break;
                }
            }
        }
        assert_eq!(out, b"out\n");
        assert_eq!(err, b"err\n");
        assert_eq!(code, Some(Some(3)));
    }

    #[test]
    fn kill_ends_the_child() {
        let mut cmd = StdCommand::new("sleep");
        cmd.arg("30");
        let (mut rx, child) = spawn(cmd).unwrap();
        child.kill().unwrap();
        loop {
            match rx.blocking_recv() {
                Some(ChildEvent::Terminated(code)) => {
                    assert_eq!(code, None); // killed by a signal
                    break;
                }
                Some(_) => {}
                None => panic!("channel closed without Terminated"),
            }
        }
    }

    /// Desktop SIGKILLed -> karlsend must not survive as an orphan (PR_SET_PDEATHSIG).
    /// Re-runs this test binary as a stand-in "Desktop" that spawns `sleep 60`.
    #[cfg(target_os = "linux")]
    #[test]
    fn child_dies_with_parent() {
        if std::env::var_os("KD_PDEATHSIG_PARENT").is_some() {
            let mut cmd = StdCommand::new("sleep");
            cmd.arg("60");
            let (_rx, child) = spawn(cmd).unwrap();
            println!("CHILD_PID={}", child.pid());
            std::thread::sleep(std::time::Duration::from_secs(60));
            return;
        }
        let mut parent = StdCommand::new(std::env::current_exe().unwrap())
            .args(["child::tests::child_dies_with_parent", "--exact", "--nocapture", "--test-threads=1"])
            .env("KD_PDEATHSIG_PARENT", "1")
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut stdout = io::BufReader::new(parent.stdout.take().unwrap());
        let mut pid = None;
        let mut line = String::new();
        while io::BufRead::read_line(&mut stdout, &mut line).unwrap() > 0 {
            // The harness prints "test <name> ... " on the same line first.
            if let Some(p) = line.trim().split("CHILD_PID=").nth(1) {
                pid = Some(p.parse::<i32>().unwrap());
                break;
            }
            line.clear();
        }
        let pid = pid.expect("parent did not report the child pid");
        parent.kill().unwrap(); // SIGKILL: no shutdown hook can run
        parent.wait().unwrap();
        let alive = || {
            std::fs::read_to_string(format!("/proc/{pid}/stat"))
                .map(|s| !s.contains(") Z "))
                .unwrap_or(false)
        };
        for _ in 0..50 {
            if !alive() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        unsafe { libc::kill(pid, libc::SIGKILL) };
        panic!("child {pid} outlived its parent");
    }
}
