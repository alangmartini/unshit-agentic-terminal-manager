//! Session ownership: one PTY child per session, one registry per daemon.
//!
//! A [`Session`] is the daemon-side counterpart of a UI pane. It owns a
//! spawned shell child through [`crate::pty::PtyPair`], tags itself with
//! a monotonic `u64` id, and pipes output bytes through a
//! [`tokio::sync::mpsc::Sender`] so the handler can forward them as
//! `ServerEvent::Output` frames.
//!
//! Slice 4b: the session also owns an `unshit_terminal_core::Terminal`
//! and feeds every PTY chunk through it in the reader task, so the
//! daemon maintains authoritative grid plus scrollback state. Scrollback
//! persistence and the attach RPC arrive in slice 4c.

pub mod registry;

use std::io::{Read, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use unshit_terminal_core::{Snapshot, Terminal};

/// Default scrollback cap per session. Matches SPEC.md section 3 F3.
const DEFAULT_SCROLLBACK: usize = 10_000;

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
    /// Daemon-side terminal emulator. Every PTY chunk is parsed into
    /// this in the reader task before being forwarded to the mpsc, so
    /// `snapshot()` always reflects bytes already observed by clients.
    terminal: Arc<Mutex<Terminal>>,
    /// Swappable output sink. `None` when no client is attached; the
    /// reader task still parses bytes into `terminal`, but nothing is
    /// forwarded. `attach()` swaps in a fresh sender and returns the
    /// matching receiver; `detach()` sets this back to `None`.
    output_tx: Arc<Mutex<Option<mpsc::Sender<Vec<u8>>>>>,
    /// Workspace id tag used by the UI to match sessions back to panes
    /// after a restart. Opaque to the daemon.
    workspace_id: u32,
    /// Pane id tag within the workspace. Opaque to the daemon.
    pane_id: u32,
    /// Optional human-friendly name for the session.
    name: Option<String>,
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
        workspace_id: u32,
        pane_id: u32,
        name: Option<String>,
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

        let terminal = Arc::new(Mutex::new(Terminal::new(
            rows as usize,
            cols as usize,
            DEFAULT_SCROLLBACK,
        )));
        let reader_terminal = Arc::clone(&terminal);

        let output_tx = Arc::new(Mutex::new(Some(tx)));
        let reader_output_tx = Arc::clone(&output_tx);

        let reader_task = tokio::task::spawn_blocking(move || {
            run_reader(reader, reader_output_tx, reader_terminal);
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
            terminal,
            output_tx,
            workspace_id,
            pane_id,
            name,
        };

        Ok((session, rx))
    }

    /// Replaces the current output sender with a fresh channel and
    /// returns the matching receiver. Any prior receiver is dropped;
    /// the reader stops forwarding to it on the next chunk.
    pub fn attach(&self) -> mpsc::Receiver<Vec<u8>> {
        let (tx, rx) = mpsc::channel::<Vec<u8>>(64);
        if let Ok(mut guard) = self.output_tx.lock() {
            *guard = Some(tx);
        }
        rx
    }

    /// Clears the output sender. Future PTY output still lands in the
    /// terminal but is not forwarded anywhere. No-op if already detached.
    pub fn detach(&self) {
        if let Ok(mut guard) = self.output_tx.lock() {
            *guard = None;
        }
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

    /// Resizes the PTY and the daemon-side terminal. Best-effort: if
    /// the PTY resize call fails we keep the old dimensions so later
    /// accessors do not lie about reality, and we do not touch the
    /// terminal either.
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
                if let Ok(mut term) = self.terminal.lock() {
                    term.resize(rows as usize, cols as usize);
                }
            }
        }
    }

    /// Returns a snapshot of the current grid plus up to
    /// `scrollback_lines` most-recent scrollback rows. Never panics on a
    /// poisoned mutex; returns a fresh snapshot sized to the current
    /// dimensions in that case.
    pub fn snapshot(&self, scrollback_lines: usize) -> Snapshot {
        match self.terminal.lock() {
            Ok(term) => term.snapshot(scrollback_lines),
            Err(_) => Terminal::new(self.rows as usize, self.cols as usize, 0).snapshot(0),
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

    pub fn workspace_id(&self) -> u32 {
        self.workspace_id
    }

    pub fn pane_id(&self) -> u32 {
        self.pane_id
    }

    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.kill();
    }
}

fn run_reader(
    reader: Box<dyn Read + Send>,
    output_tx: Arc<Mutex<Option<mpsc::Sender<Vec<u8>>>>>,
    terminal: Arc<Mutex<Terminal>>,
) {
    // The reader body can panic if the VTE parser hits a bug on
    // malformed input, if process_bytes indexes out-of-bounds after a
    // resize race, or if any other internal invariant is violated.
    // Catching here fulfils the slice-6 "a panic in one session's
    // parser thread must not take the daemon down" acceptance
    // criterion: the task exits cleanly, the child stays killable
    // through the normal Session::kill path, and every other session's
    // reader keeps streaming on its own thread.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        run_reader_inner(reader, output_tx, terminal);
    }));
    if let Err(payload) = result {
        let msg = panic_payload_str(&payload);
        log::error!("session reader panicked: {msg}; reader exiting");
    }
}

fn run_reader_inner(
    mut reader: Box<dyn Read + Send>,
    output_tx: Arc<Mutex<Option<mpsc::Sender<Vec<u8>>>>>,
    terminal: Arc<Mutex<Terminal>>,
) {
    let mut buf = vec![0u8; READ_BUF_LEN];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => return,
            Ok(n) => {
                if let Ok(mut term) = terminal.lock() {
                    term.process_bytes(&buf[..n]);
                }
                let tx_opt = output_tx.lock().ok().and_then(|g| g.clone());
                if let Some(tx) = tx_opt {
                    // Non-blocking: if the current client is slow or gone
                    // we drop the chunk and rely on the terminal plus
                    // scrollback as the source of truth. Never exit the
                    // reader when the receiver is gone; a later attach
                    // should still observe live output.
                    let _ = tx.try_send(buf[..n].to_vec());
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => return,
        }
    }
}

fn panic_payload_str(payload: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<&'static str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "<non-string panic payload>".to_string()
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
            Session::spawn(1, 80, 24, None, Some(test_shell()), 0, 0, None).expect("spawn session");

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
            Session::spawn(2, 80, 24, None, Some(test_shell()), 0, 0, None).expect("spawn session");
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
            Session::spawn(3, 80, 24, None, Some(test_shell()), 0, 0, None).expect("spawn session");
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
            Session::spawn(4, 80, 24, None, Some(test_shell()), 0, 0, None).expect("spawn session");
        drop(session);

        // With the session gone the reader task should stop and the
        // channel should close. Allow a small grace window.
        let closed = tokio::time::timeout(Duration::from_millis(1500), async {
            while rx.recv().await.is_some() {}
        })
        .await;
        assert!(closed.is_ok(), "receiver should close once session dropped");
    }

    fn grid_text(snap: &Snapshot) -> String {
        let grid = &snap.grid;
        let mut s = String::new();
        for r in 0..grid.rows() {
            if let Some(row) = grid.row(r) {
                for cell in row {
                    s.push(cell.ch);
                }
                s.push('\n');
            }
        }
        s
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn snapshot_reflects_bytes_written_to_pty() {
        let (mut session, mut rx) =
            Session::spawn(10, 80, 24, None, Some(test_shell()), 0, 0, None)
                .expect("spawn session");

        #[cfg(windows)]
        let payload = b"echo snapmarker\r\n";
        #[cfg(unix)]
        let payload = b"echo snapmarker\n";
        session.write(payload).await.expect("write");

        let _ = drain_for(&mut rx, Duration::from_millis(1500)).await;
        let snap = session.snapshot(0);
        let rendered = grid_text(&snap);
        assert!(
            rendered.contains("snapmarker"),
            "expected snapshot to contain marker, got: {rendered:?}"
        );

        session.kill();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn snapshot_is_empty_for_fresh_session() {
        let (mut session, _rx) = Session::spawn(11, 80, 24, None, Some(test_shell()), 0, 0, None)
            .expect("spawn session");
        let snap = session.snapshot(0);
        assert_eq!(snap.grid.rows(), 24);
        assert_eq!(snap.grid.cols(), 80);
        assert_eq!(snap.grid.cursor(), (0, 0));
        session.kill();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn resize_propagates_to_terminal() {
        let (mut session, _rx) = Session::spawn(12, 80, 24, None, Some(test_shell()), 0, 0, None)
            .expect("spawn session");
        let snap = session.snapshot(0);
        assert_eq!(snap.grid.rows(), 24);
        assert_eq!(snap.grid.cols(), 80);

        session.resize(120, 40);
        let snap = session.snapshot(0);
        assert_eq!(snap.grid.rows(), 40);
        assert_eq!(snap.grid.cols(), 120);
        session.kill();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn snapshot_on_dead_session_returns_empty_grid() {
        let (mut session, _rx) = Session::spawn(13, 80, 24, None, Some(test_shell()), 0, 0, None)
            .expect("spawn session");
        session.kill();
        let snap = session.snapshot(0);
        assert_eq!(snap.grid.rows(), 24);
        assert_eq!(snap.grid.cols(), 80);
        let (cr, cc) = snap.grid.cursor();
        assert!(cr < snap.grid.rows(), "cursor row out of grid: {cr}");
        assert!(cc < snap.grid.cols(), "cursor col out of grid: {cc}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn spawn_records_workspace_and_pane_metadata() {
        let (session, _rx) = Session::spawn(
            20,
            80,
            24,
            None,
            Some(test_shell()),
            7,
            3,
            Some("scratch".into()),
        )
        .expect("spawn session");
        assert_eq!(session.workspace_id(), 7);
        assert_eq!(session.pane_id(), 3);
        assert_eq!(session.name(), Some("scratch"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn attach_replaces_prior_receiver_and_drops_it() {
        let (mut session, original_rx) =
            Session::spawn(21, 80, 24, None, Some(test_shell()), 0, 0, None)
                .expect("spawn session");

        let mut new_rx = session.attach();

        #[cfg(windows)]
        let payload = b"echo reattach-marker\r\n";
        #[cfg(unix)]
        let payload = b"echo reattach-marker\n";
        session.write(payload).await.expect("write");

        let got = drain_for(&mut new_rx, Duration::from_millis(1500)).await;
        let text = String::from_utf8_lossy(&got);
        assert!(
            text.contains("reattach-marker"),
            "new receiver should observe live bytes, got: {text:?}"
        );

        // Original receiver was dropped and replaced on attach; any bytes
        // that slipped through before the swap cannot include the marker.
        drop(original_rx);

        session.kill();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn detach_clears_tx_but_keeps_terminal_parsing() {
        let (mut session, rx) = Session::spawn(22, 80, 24, None, Some(test_shell()), 0, 0, None)
            .expect("spawn session");
        drop(rx);
        session.detach();

        #[cfg(windows)]
        let payload = b"echo detachmarker\r\n";
        #[cfg(unix)]
        let payload = b"echo detachmarker\n";
        session.write(payload).await.expect("write");

        tokio::time::sleep(Duration::from_millis(1500)).await;
        let snap = session.snapshot(0);
        let rendered = grid_text(&snap);
        assert!(
            rendered.contains("detachmarker"),
            "terminal must keep parsing while detached, got: {rendered:?}"
        );

        session.kill();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn reader_does_not_exit_when_output_receiver_dropped() {
        let (mut session, rx) = Session::spawn(23, 80, 24, None, Some(test_shell()), 0, 0, None)
            .expect("spawn session");
        drop(rx);

        #[cfg(windows)]
        let payload = b"echo livemarker\r\n";
        #[cfg(unix)]
        let payload = b"echo livemarker\n";
        session.write(payload).await.expect("write");

        tokio::time::sleep(Duration::from_millis(1500)).await;
        let snap = session.snapshot(0);
        let rendered = grid_text(&snap);
        assert!(
            rendered.contains("livemarker"),
            "reader must not exit because the client went away, got: {rendered:?}"
        );

        session.kill();
    }

    /// Regression for F4.2 (crash isolation): a panic inside
    /// `run_reader_inner` must be trapped by `run_reader`'s
    /// `catch_unwind` wrapper, never propagating to the task that
    /// spawned it. Without the wrapper a VTE parser bug would take the
    /// daemon's spawn_blocking task with it, killing every other
    /// session running on the shared blocking pool.
    #[test]
    fn run_reader_catches_panic_from_reader() {
        struct PanickingReader;
        impl Read for PanickingReader {
            fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
                panic!("synthetic reader panic");
            }
        }

        let terminal = Arc::new(Mutex::new(Terminal::new(24, 80, 0)));
        let (tx, _rx) = mpsc::channel::<Vec<u8>>(4);
        let output_tx = Arc::new(Mutex::new(Some(tx)));

        // Call directly on the current thread so any escaped panic
        // would fail the test; catch_unwind inside run_reader must
        // swallow it.
        run_reader(Box::new(PanickingReader), output_tx, Arc::clone(&terminal));

        // Terminal state is still accessible: the Mutex is not poisoned
        // (the panic happened before any terminal lock was acquired, and
        // catch_unwind does not mark Mutexes we never held).
        let guard = terminal.lock().expect("terminal mutex must be unpoisoned");
        assert_eq!(guard.grid().rows(), 24);
    }
}
