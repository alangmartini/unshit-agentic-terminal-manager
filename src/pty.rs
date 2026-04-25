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
//! Slice 5 adds cross-UI-run session survival: sessions live on the
//! daemon beyond a single shim's lifetime. On `connect_to`, the shim
//! snapshots the daemon's session list and caches the mapping
//! `(workspace_id, pane_id) -> session_id` so a subsequent
//! `attach_or_spawn` for a surviving pane reattaches instead of
//! spawning a second shell. The `Drop` impl no longer tears down
//! sessions; only an explicit `destroy` / `destroy_all` does.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use tokio::sync::mpsc as tokio_mpsc;

use unshit_ptyd::client::Client;
use unshit_ptyd::protocol::message::{SessionInfo, SNAPSHOT_MAX_SCROLLBACK_LINES};
use unshit_ptyd::protocol::{ProtocolError, Response, ServerEvent};
use unshit_terminal_core::Snapshot;

/// Shim around the daemon client that keeps the old `PtyManager` API.
pub struct DaemonPty {
    inner: Option<Inner>,
    /// Local record of the cwd each pane was asked to spawn in. Lives
    /// outside `Inner` so the record survives before `connect_to` (unit
    /// tests that never connect) and across a disconnect. Populated by
    /// `spawn_in` and read by `spawn_cwd`.
    spawn_cwds: HashMap<u32, PathBuf>,
}

struct Inner {
    cmd_tx: tokio_mpsc::UnboundedSender<Command>,
    sessions: HashMap<u32, u64>,
    /// Reconciliation cache populated on `connect_to` from the
    /// daemon's current session list. Entries are consumed by
    /// `attach_or_spawn` on cache hits so a second pane with the same
    /// `(workspace_id, pane_id)` tuple still gets a fresh spawn.
    reattach_cache: HashMap<(u32, u32), u64>,
    worker: Option<thread::JoinHandle<()>>,
}

enum Command {
    Spawn {
        cols: u16,
        rows: u16,
        cwd: Option<PathBuf>,
        workspace_id: u32,
        pane_id: u32,
        name: Option<String>,
        byte_tx: std_mpsc::Sender<Vec<u8>>,
        reply: std_mpsc::SyncSender<io::Result<u64>>,
    },
    Attach {
        session_id: u64,
        scrollback_lines: u32,
        byte_tx: std_mpsc::Sender<Vec<u8>>,
        reply: std_mpsc::SyncSender<io::Result<Snapshot>>,
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
    List {
        reply: std_mpsc::SyncSender<io::Result<Vec<SessionInfo>>>,
    },
    Rename {
        session_id: u64,
        name: Option<String>,
        reply: std_mpsc::SyncSender<io::Result<()>>,
    },
}

impl DaemonPty {
    pub fn new() -> Self {
        Self {
            inner: None,
            spawn_cwds: HashMap::new(),
        }
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
                    reattach_cache: HashMap::new(),
                    worker: Some(worker),
                });
                // Populate the reattach cache from the daemon. A failure
                // here (fresh-daemon case, slow daemon, transient IO)
                // only costs a cold start on the first pane; it must not
                // make `connect_to` itself fail.
                match self.list_sessions() {
                    Ok(sessions) => {
                        if let Some(inner) = self.inner.as_mut() {
                            for info in sessions {
                                if info.alive {
                                    inner
                                        .reattach_cache
                                        .insert((info.workspace_id, info.pane_id), info.id);
                                }
                            }
                        }
                    }
                    Err(e) => {
                        log::warn!(
                            "DaemonPty::connect_to: initial list_sessions failed, \
                             starting with empty reattach cache: {e}"
                        );
                    }
                }
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
        workspace_id: u32,
        cols: u16,
        rows: u16,
    ) -> io::Result<Box<dyn io::Read + Send>> {
        self.spawn_in(pane_id, workspace_id, cols, rows, None)
    }

    pub fn spawn_in(
        &mut self,
        pane_id: u32,
        workspace_id: u32,
        cols: u16,
        rows: u16,
        cwd: Option<&Path>,
    ) -> io::Result<Box<dyn io::Read + Send>> {
        let cwd_owned = cwd.map(Path::to_path_buf);
        // Record the cwd before the IPC attempt so tests that never
        // connect, and production failures mid-handshake, still leave
        // a trace of what we asked for.
        if let Some(path) = cwd_owned.as_ref() {
            self.spawn_cwds.insert(pane_id, path.clone());
        }
        let inner = self.inner.as_mut().ok_or_else(not_connected)?;
        let (byte_tx, byte_rx) = std_mpsc::channel::<Vec<u8>>();
        let (reply_tx, reply_rx) = std_mpsc::sync_channel::<io::Result<u64>>(1);
        let cmd = Command::Spawn {
            cols,
            rows,
            cwd: cwd_owned,
            workspace_id,
            pane_id,
            name: None,
            byte_tx,
            reply: reply_tx,
        };
        inner.cmd_tx.send(cmd).map_err(|_| worker_gone())?;
        let session_id = reply_rx.recv().map_err(|_| worker_gone())??;
        inner.sessions.insert(pane_id, session_id);
        Ok(Box::new(ChannelReader::new(byte_rx)))
    }

    /// Return the cwd this pane's session was asked to spawn in, if any.
    /// The record is populated locally by `spawn_in`; panes that
    /// reattached to a daemon session this process did not create
    /// (cross-run restart) will return `None`.
    pub fn spawn_cwd(&self, pane_id: u32) -> Option<&Path> {
        self.spawn_cwds.get(&pane_id).map(PathBuf::as_path)
    }

    pub fn attach_to(
        &mut self,
        pane_id: u32,
        session_id: u64,
        scrollback_lines: u32,
    ) -> io::Result<(Snapshot, Box<dyn io::Read + Send>)> {
        let inner = self.inner.as_mut().ok_or_else(not_connected)?;
        let (byte_tx, byte_rx) = std_mpsc::channel::<Vec<u8>>();
        let (reply_tx, reply_rx) = std_mpsc::sync_channel::<io::Result<Snapshot>>(1);
        let cmd = Command::Attach {
            session_id,
            scrollback_lines,
            byte_tx,
            reply: reply_tx,
        };
        inner.cmd_tx.send(cmd).map_err(|_| worker_gone())?;
        let snapshot = reply_rx.recv().map_err(|_| worker_gone())??;
        inner.sessions.insert(pane_id, session_id);
        Ok((snapshot, Box::new(ChannelReader::new(byte_rx))))
    }

    /// Reconcile a pane against the daemon's surviving sessions: attach
    /// to an existing session if one matches `(workspace_id, pane_id)`,
    /// otherwise spawn a fresh one. Returns the snapshot (on attach) or
    /// `None` (on fresh spawn) together with the live byte reader.
    ///
    /// Cache entries are consumed by hits so a second call with the
    /// same `(workspace_id, pane_id)` tuple will spawn a fresh session
    /// rather than double-attach.
    pub fn attach_or_spawn(
        &mut self,
        pane_id: u32,
        workspace_id: u32,
        cols: u16,
        rows: u16,
        cwd: Option<&Path>,
    ) -> io::Result<(Option<Snapshot>, Box<dyn io::Read + Send>)> {
        let cache_hit = self
            .inner
            .as_mut()
            .ok_or_else(not_connected)?
            .reattach_cache
            .remove(&(workspace_id, pane_id));
        if let Some(session_id) = cache_hit {
            match self.attach_to(pane_id, session_id, SNAPSHOT_MAX_SCROLLBACK_LINES as u32) {
                Ok((snapshot, reader)) => return Ok((Some(snapshot), reader)),
                Err(e) => {
                    log::warn!(
                        "attach_or_spawn: cached session {session_id} for pane {pane_id} \
                         in workspace {workspace_id} failed to reattach ({e}); \
                         falling back to fresh spawn"
                    );
                }
            }
        }
        let reader = self.spawn_in(pane_id, workspace_id, cols, rows, cwd)?;
        Ok((None, reader))
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
        self.spawn_cwds.remove(&pane_id);
        let Some(inner) = self.inner.as_mut() else {
            log::warn!("DaemonPty::destroy called before connect");
            return;
        };
        if let Some(session_id) = inner.sessions.remove(&pane_id) {
            let _ = inner.cmd_tx.send(Command::Kill { session_id });
        }
    }

    /// Kill a session by its daemon id without touching the local
    /// pane map. Used by the sessions panel where orphan sessions
    /// (not mirrored into any pane) still need a kill button.
    pub fn kill_session_id(&mut self, session_id: u64) {
        let Some(inner) = self.inner.as_mut() else {
            log::warn!("DaemonPty::kill_session_id called before connect");
            return;
        };
        inner.sessions.retain(|_pane, sid| *sid != session_id);
        let _ = inner.cmd_tx.send(Command::Kill { session_id });
    }

    /// Iterate over every `(pane_id, session_id)` mapping the shim
    /// currently tracks. Returns an empty iterator when disconnected.
    pub fn sessions_iter(&self) -> Box<dyn Iterator<Item = (u32, u64)> + '_> {
        match self.inner.as_ref() {
            Some(i) => Box::new(i.sessions.iter().map(|(&p, &s)| (p, s))),
            None => Box::new(std::iter::empty()),
        }
    }

    pub fn destroy_all(&mut self) {
        self.spawn_cwds.clear();
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

    pub fn list_sessions(&mut self) -> io::Result<Vec<SessionInfo>> {
        let inner = self.inner.as_mut().ok_or_else(not_connected)?;
        let (reply_tx, reply_rx) = std_mpsc::sync_channel::<io::Result<Vec<SessionInfo>>>(1);
        inner
            .cmd_tx
            .send(Command::List { reply: reply_tx })
            .map_err(|_| worker_gone())?;
        // A short timeout keeps connect_to from hanging indefinitely if
        // the daemon is unresponsive. Two seconds is long enough for
        // any normal local IPC and short enough that a stuck daemon
        // does not stall UI startup forever.
        reply_rx
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| worker_gone())?
    }

    /// Set or clear the display name of a session. An empty `name`
    /// or `None` clears it.
    pub fn rename_session(&mut self, session_id: u64, name: Option<String>) -> io::Result<()> {
        let inner = self.inner.as_mut().ok_or_else(not_connected)?;
        let (reply_tx, reply_rx) = std_mpsc::sync_channel::<io::Result<()>>(1);
        inner
            .cmd_tx
            .send(Command::Rename {
                session_id,
                name: name.filter(|s| !s.is_empty()),
                reply: reply_tx,
            })
            .map_err(|_| worker_gone())?;
        reply_rx
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| worker_gone())?
    }

    /// Resolve the session id for a pane, if this shim spawned or
    /// reattached one for it. Used by the UI so rename / kill actions
    /// keyed on a pane_id can reach the daemon.
    pub fn session_id(&self, pane_id: u32) -> Option<u64> {
        self.inner
            .as_ref()
            .and_then(|i| i.sessions.get(&pane_id).copied())
    }

    #[cfg(test)]
    fn session_id_for_pane(&self, pane_id: u32) -> Option<u64> {
        self.inner
            .as_ref()
            .and_then(|i| i.sessions.get(&pane_id).copied())
    }

    #[cfg(test)]
    fn reattach_cache_len(&self) -> usize {
        self.inner
            .as_ref()
            .map(|i| i.reattach_cache.len())
            .unwrap_or(0)
    }
}

impl Default for DaemonPty {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for DaemonPty {
    fn drop(&mut self) {
        // Slice 5: sessions survive the shim. We no longer call
        // `destroy_all` here; only explicit user-driven close paths
        // (pane close, "kill all terminals") tear sessions down. The
        // worker is still joined so the IPC thread exits cleanly.
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
                    workspace_id,
                    pane_id,
                    name,
                    byte_tx,
                    reply,
                } => {
                    let cwd_string = cwd.map(|p| p.display().to_string());
                    let result = match client
                        .spawn_session(cols, rows, cwd_string, None, workspace_id, pane_id, name)
                        .await
                    {
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
                Command::Attach {
                    session_id,
                    scrollback_lines,
                    byte_tx,
                    reply,
                } => {
                    // Register the sink before the RPC so live Output
                    // events that arrive between the daemon accepting
                    // the attach and us wiring up the route are not
                    // dropped.
                    if let Ok(mut guard) = sinks.lock() {
                        guard.insert(session_id, byte_tx);
                    }
                    let result = match client.attach_session(session_id, scrollback_lines).await {
                        Ok(snapshot) => Ok(snapshot),
                        Err(ProtocolError::Io(e)) => {
                            if let Ok(mut guard) = sinks.lock() {
                                guard.remove(&session_id);
                            }
                            Err(e)
                        }
                        Err(other) => {
                            if let Ok(mut guard) = sinks.lock() {
                                guard.remove(&session_id);
                            }
                            Err(io::Error::other(other.to_string()))
                        }
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
                Command::List { reply } => {
                    let result = match client.list_sessions().await {
                        Ok(list) => Ok(list),
                        Err(ProtocolError::Io(e)) => Err(e),
                        Err(other) => Err(io::Error::other(other.to_string())),
                    };
                    let _ = reply.send(result);
                }
                Command::Rename {
                    session_id,
                    name,
                    reply,
                } => {
                    let result = match client.rename_session(session_id, name).await {
                        Ok(()) => Ok(()),
                        Err(ProtocolError::Io(e)) => Err(e),
                        Err(other) => Err(io::Error::other(other.to_string())),
                    };
                    let _ = reply.send(result);
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

        let spawn_err = match shim.spawn(1, 1, 80, 24) {
            Err(e) => e,
            Ok(_) => panic!("spawn on unconnected shim must fail"),
        };
        assert_eq!(spawn_err.kind(), io::ErrorKind::NotConnected);
        let spawn_in_err = match shim.spawn_in(2, 1, 80, 24, None) {
            Err(e) => e,
            Ok(_) => panic!("spawn_in on unconnected shim must fail"),
        };
        assert_eq!(spawn_in_err.kind(), io::ErrorKind::NotConnected);
        let write_err = shim.write(1, b"hi").unwrap_err();
        assert_eq!(write_err.kind(), io::ErrorKind::NotConnected);
        let list_err = shim.list_sessions().unwrap_err();
        assert_eq!(list_err.kind(), io::ErrorKind::NotConnected);
        let attach_or_spawn_err = match shim.attach_or_spawn(1, 1, 80, 24, None) {
            Err(e) => e,
            Ok(_) => panic!("attach_or_spawn on unconnected shim must fail"),
        };
        assert_eq!(attach_or_spawn_err.kind(), io::ErrorKind::NotConnected);

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
            let mut reader = shim.spawn_in(pane_id, 1, 80, 24, None).expect("spawn_in");
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
            let _reader = shim.spawn_in(pane_id, 1, 80, 24, None).expect("spawn_in");
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
            let _r1 = shim.spawn_in(1, 1, 80, 24, None).expect("spawn 1");
            let _r2 = shim.spawn_in(2, 1, 80, 24, None).expect("spawn 2");
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

    #[test]
    fn attach_to_on_unconnected_shim_returns_not_connected() {
        let mut shim = DaemonPty::new();
        let err = match shim.attach_to(1, 42, 0) {
            Err(e) => e,
            Ok(_) => panic!("attach_to on unconnected shim must fail"),
        };
        assert_eq!(err.kind(), io::ErrorKind::NotConnected);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn attach_to_returns_snapshot_and_reader_for_live_session() {
        std::env::set_var("SHELL", TEST_SHELL);
        let path = unique_socket_path();
        let daemon = start_daemon(&path).await;

        let shim_path = path.clone();
        tokio::task::spawn_blocking(move || {
            let mut shim = DaemonPty::new();
            connect_with_retry(&mut shim, &shim_path);
            // Spawn a session owned by this shim's connection. Per-
            // connection registries in the daemon mean only this shim
            // can reach the session; attach_to via a fresh pane id
            // still works because both panes live on the same shim.
            let _reader = shim.spawn_in(100, 1, 80, 24, None).expect("spawn_in");
            let session_id = shim
                .session_id_for_pane(100)
                .expect("spawn_in must register pane");

            let (snapshot, mut reader) = shim
                .attach_to(200, session_id, 0)
                .expect("attach_to live session");
            assert_eq!(snapshot.grid.rows(), 24);
            assert_eq!(snapshot.grid.cols(), 80);

            // Writing through the attached pane must reach the same
            // session and the live reader must see SOMETHING back.
            shim.write(200, ECHO_CMD).expect("write via attached pane");
            let deadline = std::time::Instant::now() + Duration::from_millis(1500);
            let mut collected: Vec<u8> = Vec::new();
            let mut buf = [0u8; 4096];
            while std::time::Instant::now() < deadline {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        collected.extend_from_slice(&buf[..n]);
                        if !collected.is_empty() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            assert!(
                !collected.is_empty(),
                "attached reader should receive live output"
            );
            shim.destroy(200);
        })
        .await
        .unwrap();

        daemon.abort();
        let _ = daemon.await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn attach_to_registers_pane_mapping_so_write_targets_matching_session() {
        std::env::set_var("SHELL", TEST_SHELL);
        let path = unique_socket_path();
        let daemon = start_daemon(&path).await;

        let shim_path = path.clone();
        tokio::task::spawn_blocking(move || {
            let mut shim = DaemonPty::new();
            connect_with_retry(&mut shim, &shim_path);
            let _reader_a = shim.spawn_in(10, 1, 80, 24, None).expect("spawn_in");
            let session_id = shim
                .session_id_for_pane(10)
                .expect("spawn_in must register pane");

            let (_snap, mut reader_b) = shim
                .attach_to(300, session_id, 0)
                .expect("attach_to live session");
            assert!(shim.has(300));

            // write + resize on the attached pane must not error.
            shim.write(300, ECHO_CMD).expect("write via attached pane");
            shim.resize(300, 120, 40);

            // And the route is live: drain briefly, assert bytes flow.
            let deadline = std::time::Instant::now() + Duration::from_millis(1500);
            let mut collected: Vec<u8> = Vec::new();
            let mut buf = [0u8; 4096];
            while std::time::Instant::now() < deadline {
                match reader_b.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        collected.extend_from_slice(&buf[..n]);
                        if !collected.is_empty() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            assert!(
                !collected.is_empty(),
                "mapped pane should feed the attached reader"
            );
            shim.destroy(300);
        })
        .await
        .unwrap();

        daemon.abort();
        let _ = daemon.await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn attach_to_unknown_session_returns_error() {
        let path = unique_socket_path();
        let daemon = start_daemon(&path).await;

        let shim_path = path.clone();
        tokio::task::spawn_blocking(move || {
            let mut shim = DaemonPty::new();
            connect_with_retry(&mut shim, &shim_path);
            let err = match shim.attach_to(5, 9999, 0) {
                Err(e) => e,
                Ok(_) => panic!("attach_to on unknown session must fail"),
            };
            let msg = err.to_string();
            assert!(
                msg.contains("session_not_found"),
                "expected session_not_found in error, got: {msg}"
            );
            assert!(!shim.has(5));
        })
        .await
        .unwrap();

        daemon.abort();
        let _ = daemon.await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn list_sessions_returns_empty_on_fresh_daemon() {
        let path = unique_socket_path();
        let daemon = start_daemon(&path).await;

        let shim_path = path.clone();
        tokio::task::spawn_blocking(move || {
            let mut shim = DaemonPty::new();
            connect_with_retry(&mut shim, &shim_path);
            let list = shim.list_sessions().expect("list_sessions");
            assert!(
                list.is_empty(),
                "fresh daemon must have no sessions, got {list:?}"
            );
            assert_eq!(shim.reattach_cache_len(), 0);
        })
        .await
        .unwrap();

        daemon.abort();
        let _ = daemon.await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn attach_or_spawn_cache_miss_spawns_fresh_session() {
        std::env::set_var("SHELL", TEST_SHELL);
        let path = unique_socket_path();
        let daemon = start_daemon(&path).await;

        let shim_path = path.clone();
        tokio::task::spawn_blocking(move || {
            let mut shim = DaemonPty::new();
            connect_with_retry(&mut shim, &shim_path);
            let (snapshot, _reader) = shim
                .attach_or_spawn(1, 1, 80, 24, None)
                .expect("attach_or_spawn");
            assert!(snapshot.is_none(), "cache miss must spawn fresh");
            assert!(shim.has(1));
        })
        .await
        .unwrap();

        daemon.abort();
        let _ = daemon.await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn attach_or_spawn_cache_hit_attaches_to_existing_session() {
        std::env::set_var("SHELL", TEST_SHELL);
        let path = unique_socket_path();
        let daemon = start_daemon(&path).await;

        let pane_id = 42u32;
        let workspace_id = 7u32;

        // Phase A: create a session via shim A, then drop shim A. The
        // session must survive (slice 5 policy) so a fresh shim can
        // reattach.
        let shim_path_a = path.clone();
        tokio::task::spawn_blocking(move || {
            let mut shim = DaemonPty::new();
            connect_with_retry(&mut shim, &shim_path_a);
            let _reader = shim
                .spawn_in(pane_id, workspace_id, 80, 24, None)
                .expect("spawn_in");
        })
        .await
        .unwrap();

        // Phase B: open a fresh shim against the same daemon and call
        // attach_or_spawn with the same `(workspace_id, pane_id)`. It
        // must hit the cache populated during connect_to and return a
        // snapshot.
        let shim_path_b = path.clone();
        tokio::task::spawn_blocking(move || {
            let mut shim = DaemonPty::new();
            connect_with_retry(&mut shim, &shim_path_b);
            let (snapshot, mut reader) = shim
                .attach_or_spawn(pane_id, workspace_id, 80, 24, None)
                .expect("attach_or_spawn on survivor");
            assert!(
                snapshot.is_some(),
                "attach_or_spawn must reattach when a matching session survives"
            );
            assert!(shim.has(pane_id));

            // And a read does not immediately error. The reader may
            // not have any immediate bytes to deliver (no writes since
            // the first shell prompt) but it must not crash.
            let mut buf = [0u8; 64];
            match reader.read(&mut buf) {
                Ok(_) => {}
                Err(e) => panic!("reader erroring on attach survivor: {e}"),
            }
            shim.destroy(pane_id);
        })
        .await
        .unwrap();

        daemon.abort();
        let _ = daemon.await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn drop_shim_no_longer_kills_daemon_sessions() {
        std::env::set_var("SHELL", TEST_SHELL);
        let path = unique_socket_path();
        let daemon = start_daemon(&path).await;

        let shim_path_a = path.clone();
        let spawned_session = tokio::task::spawn_blocking(move || {
            let mut shim = DaemonPty::new();
            connect_with_retry(&mut shim, &shim_path_a);
            let _reader = shim.spawn_in(55, 9, 80, 24, None).expect("spawn");
            // Drop shim at end of scope. Slice 5: this does NOT kill
            // the daemon-side session.
            shim.session_id_for_pane(55)
                .expect("pane must be registered")
        })
        .await
        .unwrap();

        // Shim B: confirm the session survived shim A's drop.
        let shim_path_b = path.clone();
        tokio::task::spawn_blocking(move || {
            let mut shim = DaemonPty::new();
            connect_with_retry(&mut shim, &shim_path_b);
            let list = shim.list_sessions().expect("list_sessions");
            assert!(
                list.iter().any(|s| s.id == spawned_session && s.alive),
                "session {spawned_session} should survive shim drop; got {list:?}"
            );
        })
        .await
        .unwrap();

        daemon.abort();
        let _ = daemon.await;
    }
}
