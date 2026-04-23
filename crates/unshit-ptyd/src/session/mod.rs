//! Session ownership: one PTY child per session, one registry per daemon.
//!
//! A [`Session`] is the daemon-side counterpart of a UI pane. It owns a
//! spawned shell child through [`crate::pty::PtyPair`], tags itself with
//! a monotonic `u64` id, and pipes output bytes through a
//! [`tokio::sync::mpsc::Sender`] so the handler can forward them as
//! `ServerEvent::Output` frames.
//!
//! Scope note: this module does NOT parse the byte stream with VTE yet,
//! does NOT keep scrollback, and does NOT persist session identity. All
//! three arrive in slices 4 and 5 (see SPEC.md section 11).

pub mod registry;

use std::io::{Read, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

/// Size of the read buffer fed into the mpsc. Matches the value used
/// elsewhere in the UI bridge so throughput characteristics do not drift
/// between slice 3a (daemon owns PTYs, UI still in-process) and later
/// slices.
const READ_BUF_LEN: usize = 4096;

/// Owns one PTY child and the reader task that fans its bytes into the
/// outbound mpsc.
pub struct Session {
    /// Monotonic id assigned by the registry.
    pub id: u64,
    /// Underlying PTY state (master, child, writer).
    pty: Option<PtyPair>,
    /// Last known geometry. Kept for `ListSessions`.
    cols: u16,
    rows: u16,
    /// PID of the child shell at spawn time.
    pid: Option<u32>,
    /// Reader task handle; aborted on drop so the blocking read does not
    /// keep the child's master alive.
    reader_task: Option<JoinHandle<()>>,
}

/// Internal representation of one PTY child, mirrored after the
/// `PtyPair` used by the old UI-side manager but without per-pane
/// bookkeeping.
struct PtyPair {
    child: Arc<Mutex<Box<dyn Child + Send>>>,
    writer: Box<dyn Write + Send>,
    master: Box<dyn MasterPty + Send>,
}

impl Session {
    /// Spawns a shell with the requested geometry and starts the reader
    /// task. On success the caller receives both the `Session` and a
    /// `Receiver` it can poll for outbound bytes.
    ///
    /// `shell` overrides the platform default when `Some`; falling back
    /// to `SHELL` + platform default when `None`.
    pub fn spawn(
        id: u64,
        cols: u16,
        rows: u16,
        cwd: Option<&Path>,
        shell: Option<&str>,
    ) -> std::io::Result<(Self, mpsc::Receiver<Vec<u8>>)> {
        let pty_system = native_pty_system();
        let size = PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        };
        let pty = pty_system.openpty(size).map_err(std::io::Error::other)?;

        let shell = shell
            .map(|s| s.to_string())
            .unwrap_or_else(crate::pty::default_shell);

        let mut cmd = CommandBuilder::new(&shell);
        if let Some(dir) = cwd {
            cmd.cwd(dir);
            if crate::pty::is_powershell_shell(&shell) {
                for arg in crate::pty::build_powershell_cwd_args(dir) {
                    cmd.arg(arg);
                }
            }
        } else if let Some(home) = dirs::home_dir() {
            cmd.cwd(home);
        }

        let child = pty
            .slave
            .spawn_command(cmd)
            .map_err(std::io::Error::other)?;
        let pid = child.process_id();

        let reader = pty
            .master
            .try_clone_reader()
            .map_err(std::io::Error::other)?;
        let writer = pty.master.take_writer().map_err(std::io::Error::other)?;

        let (tx, rx) = mpsc::channel::<Vec<u8>>(64);

        let reader_task = tokio::task::spawn_blocking(move || {
            run_reader(reader, tx);
        });

        let session = Self {
            id,
            pty: Some(PtyPair {
                child: Arc::new(Mutex::new(child)),
                writer,
                master: pty.master,
            }),
            cols,
            rows,
            pid,
            reader_task: Some(reader_task),
        };

        Ok((session, rx))
    }

    /// Writes `bytes` to the PTY stdin. Uses `spawn_blocking` because
    /// the `Write` impl from portable-pty is blocking.
    pub async fn write(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        let pty = self
            .pty
            .as_mut()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotConnected, "session dead"))?;
        pty.writer.write_all(bytes)?;
        pty.writer.flush()
    }

    /// Resizes the PTY. Best-effort: if the resize call fails we keep
    /// the old dimensions so later accessors do not lie about reality.
    pub fn resize(&mut self, cols: u16, rows: u16) {
        if let Some(pty) = self.pty.as_mut() {
            let new_size = PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            };
            if pty.master.resize(new_size).is_ok() {
                self.cols = cols;
                self.rows = rows;
            }
        }
    }

    /// Kills the child and aborts the reader task.
    ///
    /// Safe to call multiple times; subsequent calls are no-ops.
    pub fn kill(&mut self) {
        if let Some(pty) = self.pty.take() {
            if let Ok(mut child) = pty.child.lock() {
                let _ = child.kill();
                let _ = child.wait();
            }
            // Explicitly drop the writer/master so the reader sees EOF.
            drop(pty.writer);
            drop(pty.master);
        }
        if let Some(handle) = self.reader_task.take() {
            handle.abort();
        }
    }

    /// Reports whether the child is still running.
    pub fn alive(&self) -> bool {
        let Some(pty) = self.pty.as_ref() else {
            return false;
        };
        let Ok(mut child) = pty.child.lock() else {
            return false;
        };
        matches!(child.try_wait(), Ok(None))
    }

    pub fn cols(&self) -> u16 {
        self.cols
    }

    pub fn rows(&self) -> u16 {
        self.rows
    }

    pub fn pid(&self) -> Option<u32> {
        self.pid
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.kill();
    }
}

fn run_reader(mut reader: Box<dyn Read + Send>, tx: mpsc::Sender<Vec<u8>>) {
    let mut buf = vec![0u8; READ_BUF_LEN];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => return,
            Ok(n) => {
                let chunk = buf[..n].to_vec();
                if tx.blocking_send(chunk).is_err() {
                    return;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn test_shell() -> &'static str {
        #[cfg(windows)]
        {
            "cmd.exe"
        }
        #[cfg(unix)]
        {
            "/bin/sh"
        }
    }

    /// Drains the receiver for up to `timeout` ms, returning the
    /// accumulated bytes. Stops early when the channel closes.
    async fn drain_for(rx: &mut mpsc::Receiver<Vec<u8>>, timeout: Duration) -> Vec<u8> {
        let deadline = tokio::time::Instant::now() + timeout;
        let mut collected = Vec::new();
        while let Ok(chunk) = tokio::time::timeout_at(deadline, rx.recv()).await {
            match chunk {
                Some(bytes) => collected.extend(bytes),
                None => break,
            }
        }
        collected
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn spawn_emits_output_from_echo() {
        let (mut session, mut rx) =
            Session::spawn(1, 80, 24, None, Some(test_shell())).expect("spawn session");

        #[cfg(windows)]
        let payload = b"echo session-hi\r\n";
        #[cfg(unix)]
        let payload = b"echo session-hi\n";
        session.write(payload).await.expect("write");

        let got = drain_for(&mut rx, Duration::from_millis(1500)).await;
        let text = String::from_utf8_lossy(&got);
        assert!(
            text.contains("session-hi"),
            "expected echo output to contain marker, got: {text:?}"
        );

        session.kill();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn resize_updates_recorded_dimensions() {
        let (mut session, _rx) =
            Session::spawn(2, 80, 24, None, Some(test_shell())).expect("spawn session");
        assert_eq!(session.cols(), 80);
        assert_eq!(session.rows(), 24);

        session.resize(120, 40);
        assert_eq!(session.cols(), 120);
        assert_eq!(session.rows(), 40);

        session.kill();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn kill_is_idempotent_and_marks_session_dead() {
        let (mut session, _rx) =
            Session::spawn(3, 80, 24, None, Some(test_shell())).expect("spawn session");
        assert!(
            session.alive(),
            "session must be alive immediately after spawn"
        );
        session.kill();
        // Second kill is a no-op and must not panic.
        session.kill();
        assert!(!session.alive(), "dead session must not report alive");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn drop_kills_child_and_closes_receiver() {
        let (session, mut rx) =
            Session::spawn(4, 80, 24, None, Some(test_shell())).expect("spawn session");
        drop(session);

        // With the session gone the reader task should stop and the
        // channel should close. Allow a small grace window.
        let closed = tokio::time::timeout(Duration::from_millis(1500), async {
            while rx.recv().await.is_some() {}
        })
        .await;
        assert!(closed.is_ok(), "receiver should close once session dropped");
    }
}
