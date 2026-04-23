//! PTY facade for the UI crate.
//!
//! `DaemonPty` exposes the same synchronous surface the UI used to get
//! from the in-process `PtyManager`, but every call round-trips through
//! `unshit-ptyd` over IPC. The old `PtyManager` lives in the daemon
//! crate (`unshit_ptyd::pty::PtyManager`) and is only driven by the
//! daemon process now.
//!
//! The worker thread owns a tokio runtime and the async `Client`. The
//! UI thread pushes `Command`s over an unbounded channel and waits on
//! `std::sync::mpsc` one-shot reply slots; output bytes are delivered
//! through per-session std channels wrapped in `ChannelReader`, which
//! fits the blocking reader loop `bridge.rs` runs under
//! `spawn_blocking`.
//!
//! The old `PtyManager` re-export is preserved so existing UI call
//! sites keep compiling until the follow-up slice flips the field type
//! in `AppState` over to `DaemonPty`.

pub use unshit_ptyd::pty::{
    build_powershell_cwd_args, default_shell, is_powershell_shell, PtyManager, PtyPair,
};

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex};
use std::thread;

use tokio::sync::mpsc as tokio_mpsc;

use unshit_ptyd::client::Client;
use unshit_ptyd::protocol::{ProtocolError, Response, ServerEvent};

/// Shim around the daemon client that keeps the old `PtyManager` API.
pub struct DaemonPty {
    inner: Option<Inner>,
}

struct Inner {
    cmd_tx: tokio_mpsc::UnboundedSender<Command>,
    sessions: HashMap<u32, u64>,
    worker: Option<thread::JoinHandle<()>>,
}

enum Command {
    Spawn {
        cols: u16,
        rows: u16,
        cwd: Option<PathBuf>,
        byte_tx: std_mpsc::Sender<Vec<u8>>,
        reply: std_mpsc::SyncSender<io::Result<u64>>,
    },
    Write {
        session_id: u64,
        bytes: Vec<u8>,
        reply: std_mpsc::SyncSender<io::Result<()>>,
    },
    Resize {
        session_id: u64,
        cols: u16,
        rows: u16,
    },
    Kill {
        session_id: u64,
    },
}

impl DaemonPty {
    pub fn new() -> Self {
        Self { inner: None }
    }

    pub fn connect_to(&mut self, socket_path: &Path) -> io::Result<()> {
        if self.inner.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "daemon pty already connected",
            ));
        }

        let (cmd_tx, cmd_rx) = tokio_mpsc::unbounded_channel::<Command>();
        let (ready_tx, ready_rx) = std_mpsc::sync_channel::<io::Result<()>>(1);
        let socket_path = socket_path.to_path_buf();

        let worker = thread::Builder::new()
            .name("daemon-pty-worker".into())
            .spawn(move || {
                worker_main(socket_path, cmd_rx, ready_tx);
            })
            .map_err(io::Error::other)?;

        match ready_rx.recv() {
            Ok(Ok(())) => {
                self.inner = Some(Inner {
                    cmd_tx,
                    sessions: HashMap::new(),
                    worker: Some(worker),
                });
                Ok(())
            }
            Ok(Err(e)) => Err(e),
            Err(_) => Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "daemon pty worker exited before signalling readiness",
            )),
        }
    }

    pub fn spawn(
        &mut self,
        pane_id: u32,
        cols: u16,
        rows: u16,
    ) -> io::Result<Box<dyn io::Read + Send>> {
        self.spawn_in(pane_id, cols, rows, None)
    }

    pub fn spawn_in(
        &mut self,
        pane_id: u32,
        cols: u16,
        rows: u16,
        cwd: Option<&Path>,
    ) -> io::Result<Box<dyn io::Read + Send>> {
        let inner = self.inner.as_mut().ok_or_else(not_connected)?;
        let (byte_tx, byte_rx) = std_mpsc::channel::<Vec<u8>>();
        let (reply_tx, reply_rx) = std_mpsc::sync_channel::<io::Result<u64>>(1);
        let cmd = Command::Spawn {
            cols,
            rows,
            cwd: cwd.map(Path::to_path_buf),
            byte_tx,
            reply: reply_tx,
        };
        inner.cmd_tx.send(cmd).map_err(|_| worker_gone())?;
        let session_id = reply_rx.recv().map_err(|_| worker_gone())??;
        inner.sessions.insert(pane_id, session_id);
        Ok(Box::new(ChannelReader::new(byte_rx)))
    }

    pub fn write(&mut self, pane_id: u32, data: &[u8]) -> io::Result<()> {
        let inner = self.inner.as_mut().ok_or_else(not_connected)?;
        let session_id = *inner.sessions.get(&pane_id).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("no PTY for pane {pane_id}"),
            )
        })?;
        let (reply_tx, reply_rx) = std_mpsc::sync_channel::<io::Result<()>>(1);
        let cmd = Command::Write {
            session_id,
            bytes: data.to_vec(),
            reply: reply_tx,
        };
        inner.cmd_tx.send(cmd).map_err(|_| worker_gone())?;
        reply_rx.recv().map_err(|_| worker_gone())?
    }

    pub fn resize(&mut self, pane_id: u32, cols: u16, rows: u16) {
        let Some(inner) = self.inner.as_mut() else {
            log::warn!("DaemonPty::resize called before connect");
            return;
        };
        if let Some(&session_id) = inner.sessions.get(&pane_id) {
            let _ = inner.cmd_tx.send(Command::Resize {
                session_id,
                cols,
                rows,
            });
        }
    }

    pub fn destroy(&mut self, pane_id: u32) {
        let Some(inner) = self.inner.as_mut() else {
            log::warn!("DaemonPty::destroy called before connect");
            return;
        };
        if let Some(session_id) = inner.sessions.remove(&pane_id) {
            let _ = inner.cmd_tx.send(Command::Kill { session_id });
        }
    }

    pub fn destroy_all(&mut self) {
        let Some(inner) = self.inner.as_mut() else {
            return;
        };
        let ids: Vec<u64> = inner.sessions.drain().map(|(_pane, sid)| sid).collect();
        for session_id in ids {
            let _ = inner.cmd_tx.send(Command::Kill { session_id });
        }
    }

    pub fn has(&self, pane_id: u32) -> bool {
        self.inner
            .as_ref()
            .map(|i| i.sessions.contains_key(&pane_id))
            .unwrap_or(false)
    }
}

impl Default for DaemonPty {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for DaemonPty {
    fn drop(&mut self) {
        self.destroy_all();
        if let Some(mut inner) = self.inner.take() {
            // Dropping the sender lets the worker break out of cmd_rx.recv().
            drop(inner.cmd_tx);
            if let Some(handle) = inner.worker.take() {
                let _ = handle.join();
            }
        }
    }
}

fn not_connected() -> io::Error {
    io::Error::new(io::ErrorKind::NotConnected, "daemon pty is not connected")
}

fn worker_gone() -> io::Error {
    io::Error::new(
        io::ErrorKind::BrokenPipe,
        "daemon pty worker is no longer running",
    )
}

/// `std::io::Read` adapter backed by an `std::sync::mpsc::Receiver`.
///
/// `recv` blocks until more bytes arrive. A chunk larger than `buf` is
/// stashed in `leftover` and drained on subsequent reads so no bytes
/// are lost. A channel-disconnected recv maps to `Ok(0)`, signalling
/// EOF the same way the VTE reader loop expects the old PTY reader to.
struct ChannelReader {
    rx: std_mpsc::Receiver<Vec<u8>>,
    leftover: Vec<u8>,
    offset: usize,
}

impl ChannelReader {
    fn new(rx: std_mpsc::Receiver<Vec<u8>>) -> Self {
        Self {
            rx,
            leftover: Vec::new(),
            offset: 0,
        }
    }
}

impl io::Read for ChannelReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.offset < self.leftover.len() {
            let remaining = &self.leftover[self.offset..];
            let n = remaining.len().min(buf.len());
            buf[..n].copy_from_slice(&remaining[..n]);
            self.offset += n;
            if self.offset >= self.leftover.len() {
                self.leftover.clear();
                self.offset = 0;
            }
            return Ok(n);
        }

        let chunk = match self.rx.recv() {
            Ok(c) => c,
            Err(_) => return Ok(0),
        };
        if chunk.is_empty() {
            return Ok(0);
        }
        let n = chunk.len().min(buf.len());
        buf[..n].copy_from_slice(&chunk[..n]);
        if n < chunk.len() {
            self.leftover = chunk;
            self.offset = n;
        }
        Ok(n)
    }
}

type SessionSinks = Arc<Mutex<HashMap<u64, std_mpsc::Sender<Vec<u8>>>>>;

fn worker_main(
    socket_path: PathBuf,
    mut cmd_rx: tokio_mpsc::UnboundedReceiver<Command>,
    ready: std_mpsc::SyncSender<io::Result<()>>,
) {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(r) => r,
        Err(e) => {
            let _ = ready.send(Err(e));
            return;
        }
    };

    runtime.block_on(async move {
        let (client, events) = match Client::connect_with_events(&socket_path).await {
            Ok(pair) => pair,
            Err(e) => {
                let _ = ready.send(Err(e));
                return;
            }
        };
        if ready.send(Ok(())).is_err() {
            return;
        }

        let sinks: SessionSinks = Arc::new(Mutex::new(HashMap::new()));
        let sinks_for_events = sinks.clone();
        let event_task = tokio::spawn(async move {
            event_loop(events, sinks_for_events).await;
        });

        let mut client = client;
        while let Some(cmd) = cmd_rx.recv().await {
            match cmd {
                Command::Spawn {
                    cols,
                    rows,
                    cwd,
                    byte_tx,
                    reply,
                } => {
                    let cwd_string = cwd.map(|p| p.display().to_string());
                    let result = match client.spawn_session(cols, rows, cwd_string, None).await {
                        Ok(Response::SessionSpawned { session_id, .. }) => {
                            if let Ok(mut guard) = sinks.lock() {
                                guard.insert(session_id, byte_tx);
                            }
                            Ok(session_id)
                        }
                        Ok(Response::Error { code, message, .. }) => {
                            Err(io::Error::other(format!("{code}: {message}")))
                        }
                        Ok(other) => Err(io::Error::other(format!("unexpected: {other:?}"))),
                        Err(ProtocolError::Io(e)) => Err(e),
                        Err(other) => Err(io::Error::other(other.to_string())),
                    };
                    let _ = reply.send(result);
                }
                Command::Write {
                    session_id,
                    bytes,
                    reply,
                } => {
                    let result = match client.write(session_id, bytes).await {
                        Ok(Response::Ack { .. }) => Ok(()),
                        Ok(Response::Error { code, message, .. }) => {
                            Err(io::Error::other(format!("{code}: {message}")))
                        }
                        Ok(other) => Err(io::Error::other(format!("unexpected: {other:?}"))),
                        Err(ProtocolError::Io(e)) => Err(e),
                        Err(other) => Err(io::Error::other(other.to_string())),
                    };
                    let _ = reply.send(result);
                }
                Command::Resize {
                    session_id,
                    cols,
                    rows,
                } => {
                    let _ = client.resize(session_id, cols, rows).await;
                }
                Command::Kill { session_id } => {
                    if let Ok(mut guard) = sinks.lock() {
                        guard.remove(&session_id);
                    }
                    let _ = client.kill_session(session_id).await;
                }
            }
        }

        drop(client);
        event_task.abort();
        let _ = event_task.await;
    });
}

async fn event_loop(mut events: tokio_mpsc::Receiver<ServerEvent>, sinks: SessionSinks) {
    while let Some(ev) = events.recv().await {
        match ev {
            ServerEvent::Output { session_id, bytes } => {
                let sink = match sinks.lock() {
                    Ok(guard) => guard.get(&session_id).cloned(),
                    Err(_) => return,
                };
                if let Some(tx) = sink {
                    if tx.send(bytes).is_err() {
                        if let Ok(mut guard) = sinks.lock() {
                            guard.remove(&session_id);
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    #[cfg(windows)]
    const TEST_SHELL: &str = "cmd.exe";
    #[cfg(windows)]
    const ECHO_CMD: &[u8] = b"echo shim-hi\r\n";
    #[cfg(unix)]
    const TEST_SHELL: &str = "/bin/sh";
    #[cfg(unix)]
    const ECHO_CMD: &[u8] = b"echo shim-hi\n";

    fn unique_socket_path() -> PathBuf {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let pid = std::process::id();
        #[cfg(windows)]
        {
            PathBuf::from(format!(r"\\.\pipe\unshit-ptyd-uishim-{pid}-{n}"))
        }
        #[cfg(unix)]
        {
            std::env::temp_dir().join(format!("unshit-ptyd-uishim-{pid}-{n}.sock"))
        }
    }

    async fn start_daemon(path: &Path) -> tokio::task::JoinHandle<()> {
        let p = path.to_path_buf();
        let handle = tokio::spawn(async move {
            unshit_ptyd::daemon::run(&p).await.unwrap();
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        handle
    }

    fn connect_with_retry(shim: &mut DaemonPty, path: &Path) {
        let deadline = std::time::Instant::now() + Duration::from_millis(2000);
        loop {
            match shim.connect_to(path) {
                Ok(()) => return,
                Err(_) if std::time::Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(25));
                }
                Err(e) => panic!("shim failed to connect: {e}"),
            }
        }
    }

    #[test]
    fn new_shim_is_unconnected_and_rejects_ops() {
        let mut shim = DaemonPty::new();
        assert!(!shim.has(1));

        let spawn_err = match shim.spawn(1, 80, 24) {
            Err(e) => e,
            Ok(_) => panic!("spawn on unconnected shim must fail"),
        };
        assert_eq!(spawn_err.kind(), io::ErrorKind::NotConnected);
        let spawn_in_err = match shim.spawn_in(2, 80, 24, None) {
            Err(e) => e,
            Ok(_) => panic!("spawn_in on unconnected shim must fail"),
        };
        assert_eq!(spawn_in_err.kind(), io::ErrorKind::NotConnected);
        let write_err = shim.write(1, b"hi").unwrap_err();
        assert_eq!(write_err.kind(), io::ErrorKind::NotConnected);

        // resize / destroy / destroy_all must not panic when unconnected.
        shim.resize(1, 80, 24);
        shim.destroy(1);
        shim.destroy_all();
        assert!(!shim.has(1));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn connect_to_succeeds_against_live_daemon() {
        let path = unique_socket_path();
        let daemon = start_daemon(&path).await;

        let shim_path = path.clone();
        let result = tokio::task::spawn_blocking(move || {
            let mut shim = DaemonPty::new();
            connect_with_retry(&mut shim, &shim_path);
            let second = shim.connect_to(&shim_path);
            assert!(matches!(
                second.as_ref().map_err(|e| e.kind()),
                Err(io::ErrorKind::AlreadyExists)
            ));
            shim
        })
        .await
        .unwrap();

        drop(result);
        daemon.abort();
        let _ = daemon.await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn spawn_in_registers_pane_and_returns_reader() {
        std::env::set_var("SHELL", TEST_SHELL);
        let path = unique_socket_path();
        let daemon = start_daemon(&path).await;

        let shim_path = path.clone();
        let handle = tokio::task::spawn_blocking(move || {
            let mut shim = DaemonPty::new();
            connect_with_retry(&mut shim, &shim_path);
            let pane_id = 7u32;
            let mut reader = shim.spawn_in(pane_id, 80, 24, None).expect("spawn_in");
            assert!(shim.has(pane_id));

            shim.write(pane_id, ECHO_CMD).expect("write");

            let deadline = std::time::Instant::now() + Duration::from_millis(1500);
            let mut collected: Vec<u8> = Vec::new();
            let mut buf = [0u8; 4096];
            while std::time::Instant::now() < deadline {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        collected.extend_from_slice(&buf[..n]);
                        if String::from_utf8_lossy(&collected).contains("shim-hi") {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            let text = String::from_utf8_lossy(&collected).to_string();
            assert!(
                text.contains("shim-hi"),
                "expected echo marker in output, got: {text:?}"
            );
            shim.destroy(pane_id);
        })
        .await;
        handle.unwrap();

        daemon.abort();
        let _ = daemon.await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn resize_and_destroy_unknown_pane_are_silent_noops() {
        let path = unique_socket_path();
        let daemon = start_daemon(&path).await;

        let shim_path = path.clone();
        tokio::task::spawn_blocking(move || {
            let mut shim = DaemonPty::new();
            connect_with_retry(&mut shim, &shim_path);
            shim.resize(999, 120, 40);
            shim.destroy(999);
            assert!(!shim.has(999));
        })
        .await
        .unwrap();

        daemon.abort();
        let _ = daemon.await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn destroy_clears_pane_mapping() {
        std::env::set_var("SHELL", TEST_SHELL);
        let path = unique_socket_path();
        let daemon = start_daemon(&path).await;

        let shim_path = path.clone();
        tokio::task::spawn_blocking(move || {
            let mut shim = DaemonPty::new();
            connect_with_retry(&mut shim, &shim_path);
            let pane_id = 3u32;
            let _reader = shim.spawn_in(pane_id, 80, 24, None).expect("spawn_in");
            assert!(shim.has(pane_id));
            shim.destroy(pane_id);
            assert!(!shim.has(pane_id));
        })
        .await
        .unwrap();

        daemon.abort();
        let _ = daemon.await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn destroy_all_clears_everything() {
        std::env::set_var("SHELL", TEST_SHELL);
        let path = unique_socket_path();
        let daemon = start_daemon(&path).await;

        let shim_path = path.clone();
        tokio::task::spawn_blocking(move || {
            let mut shim = DaemonPty::new();
            connect_with_retry(&mut shim, &shim_path);
            let _r1 = shim.spawn_in(1, 80, 24, None).expect("spawn 1");
            let _r2 = shim.spawn_in(2, 80, 24, None).expect("spawn 2");
            assert!(shim.has(1));
            assert!(shim.has(2));
            shim.destroy_all();
            assert!(!shim.has(1));
            assert!(!shim.has(2));
        })
        .await
        .unwrap();

        daemon.abort();
        let _ = daemon.await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn drop_shim_destroys_all_and_exits_worker() {
        std::env::set_var("SHELL", TEST_SHELL);
        let path = unique_socket_path();
        let daemon = start_daemon(&path).await;

        let shim_path_for_spawn = path.clone();
        tokio::task::spawn_blocking(move || {
            let mut shim = DaemonPty::new();
            connect_with_retry(&mut shim, &shim_path_for_spawn);
            let _reader = shim.spawn_in(11, 80, 24, None).expect("spawn");
            assert!(shim.has(11));
            // Drop at end of scope sends Kill, then closes cmd_tx which
            // exits the worker loop.
        })
        .await
        .unwrap();

        // Poll a fresh client: the prior client's session must have been
        // reaped (either by our Kill, or by the connection drop from
        // the worker going away).
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            let (mut client, _events) = Client::connect_with_events(&path).await.unwrap();
            let list = client.list_sessions().await.unwrap();
            drop(client);
            if list.is_empty() {
                break;
            }
            if std::time::Instant::now() >= deadline {
                panic!("session was not reaped after shim drop: {list:?}");
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }

        daemon.abort();
        let _ = daemon.await;
    }
}
