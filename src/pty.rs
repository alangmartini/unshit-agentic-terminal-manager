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

use unshit_ptyd::client::{Client, SessionListSnapshot};
use unshit_ptyd::protocol::message::{SessionInfo, SNAPSHOT_MAX_SCROLLBACK_LINES};
use unshit_ptyd::protocol::{ProtocolError, Response, ServerEvent};
use unshit_terminal_core::Snapshot;

use crate::shell::ShellSpec;

/// Shim around the daemon client that keeps the old `PtyManager` API.
pub struct DaemonPty {
    inner: Option<Inner>,
    /// Local record of the cwd each pane was asked to spawn in. Lives
    /// outside `Inner` so the record survives before `connect_to` (unit
    /// tests that never connect) and across a disconnect. Populated by
    /// `spawn_in` and read by `spawn_cwd`.
    spawn_cwds: HashMap<u32, PathBuf>,
    /// Local record of the resolved shell each pane was asked to spawn
    /// with. Mirrors `spawn_cwds` so unit tests can assert which spawn
    /// sites forward `state.default_shell`. Only populated when the
    /// caller passes `Some(spec)`; missing key = "no shell requested",
    /// equivalent to letting the daemon's own `default_shell()` decide.
    spawn_shells: HashMap<u32, ShellSpec>,
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
    /// Receive end of the worker's async-write error channel. Drained
    /// by [`DaemonPty::take_write_errors`]; the bridge polls it and
    /// surfaces failures as toasts. Phase 2 of #135.
    write_error_rx: std_mpsc::Receiver<WriteError>,
}

/// Failure delivered from the worker for a fire-and-forget write that
/// the daemon could not accept. The `pane_id` is the UI-side pane the
/// write was issued for; the worker carries it through so the bridge
/// can mention it in the user-visible toast.
#[derive(Debug)]
pub struct WriteError {
    pub pane_id: u32,
    pub error: io::Error,
}

/// Opaque holder for the parked `cmd_rx` returned by
/// `DaemonPty::test_install_slow_daemon_inner`. Dropping the guard
/// closes the channel and turns subsequent `cmd_tx.send` calls into
/// errors, so callers must keep it alive for the duration of the
/// slow-daemon scenario. Exposed (hidden in docs) so the criterion
/// bench in `benches/` can drive the same scenario the unit test
/// uses without spinning up a real daemon. Callers outside this
/// crate should not depend on it.
#[doc(hidden)]
pub struct SlowDaemonGuard {
    _keep_alive: Box<dyn Send>,
}

enum Command {
    Spawn {
        cols: u16,
        rows: u16,
        cwd: Option<PathBuf>,
        shell: Option<String>,
        shell_args: Vec<String>,
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
    /// Fire-and-forget variant of [`Command::Write`]. The worker
    /// pushes failures to the shared `write_error_tx` so the UI never
    /// blocks waiting for the daemon. Phase 2 of #135.
    WriteAsync {
        session_id: u64,
        pane_id: u32,
        bytes: Vec<u8>,
    },
    Resize {
        session_id: u64,
        cols: u16,
        rows: u16,
    },
    Kill {
        session_id: u64,
    },
    /// Blocking kill: like `Kill`, but the daemon's response is mapped
    /// back through `reply` so the caller can tell whether the daemon
    /// actually saw the request. Used by the orphan-session kill path
    /// in `mutate_kill_session_id` so the user gets a toast on RPC
    /// failure instead of an optimistically-removed row.
    KillAck {
        session_id: u64,
        reply: std_mpsc::SyncSender<io::Result<()>>,
    },
    List {
        reply: std_mpsc::SyncSender<io::Result<SessionListSnapshot>>,
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
            spawn_shells: HashMap::new(),
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
        let (write_error_tx, write_error_rx) = std_mpsc::channel::<WriteError>();
        let socket_path = socket_path.to_path_buf();

        let worker = thread::Builder::new()
            .name("daemon-pty-worker".into())
            .spawn(move || {
                worker_main(socket_path, cmd_rx, ready_tx, write_error_tx);
            })
            .map_err(io::Error::other)?;

        match ready_rx.recv() {
            Ok(Ok(())) => {
                self.inner = Some(Inner {
                    cmd_tx,
                    sessions: HashMap::new(),
                    reattach_cache: HashMap::new(),
                    worker: Some(worker),
                    write_error_rx,
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
        self.spawn_in(pane_id, workspace_id, cols, rows, None, None)
    }

    pub fn spawn_in(
        &mut self,
        pane_id: u32,
        workspace_id: u32,
        cols: u16,
        rows: u16,
        cwd: Option<&Path>,
        shell: Option<&ShellSpec>,
    ) -> io::Result<Box<dyn io::Read + Send>> {
        self.spawn_in_named(pane_id, workspace_id, cols, rows, cwd, shell, None)
    }

    /// Like `spawn_in` but also forwards a human readable session
    /// `name` to the daemon. Used by the Quick Prompt tab so daemon
    /// inspection (e.g. `ptyctl list`) shows `qp: <prompt prefix>`
    /// instead of an opaque session id.
    pub fn spawn_in_named(
        &mut self,
        pane_id: u32,
        workspace_id: u32,
        cols: u16,
        rows: u16,
        cwd: Option<&Path>,
        shell: Option<&ShellSpec>,
        name: Option<&str>,
    ) -> io::Result<Box<dyn io::Read + Send>> {
        let cwd_owned = cwd.map(Path::to_path_buf);
        // Record the cwd before the IPC attempt so tests that never
        // connect, and production failures mid-handshake, still leave
        // a trace of what we asked for.
        if let Some(path) = cwd_owned.as_ref() {
            self.spawn_cwds.insert(pane_id, path.clone());
        }
        if let Some(spec) = shell.filter(|s| !s.is_empty()) {
            self.spawn_shells.insert(pane_id, spec.clone());
        }
        let (shell_program, shell_args) = shell_spec_to_wire(shell);
        let inner = self.inner.as_mut().ok_or_else(not_connected)?;
        let (byte_tx, byte_rx) = std_mpsc::channel::<Vec<u8>>();
        let (reply_tx, reply_rx) = std_mpsc::sync_channel::<io::Result<u64>>(1);
        let cmd = Command::Spawn {
            cols,
            rows,
            cwd: cwd_owned,
            shell: shell_program,
            shell_args,
            workspace_id,
            pane_id,
            name: name.map(|s| s.to_string()),
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

    /// Return the resolved shell this pane's session was asked to
    /// spawn with, if any. Mirrors `spawn_cwd`: missing key means the
    /// caller passed `None` (or an empty spec) and the daemon's own
    /// `default_shell()` decided.
    pub fn spawn_shell(&self, pane_id: u32) -> Option<&ShellSpec> {
        self.spawn_shells.get(&pane_id)
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
        shell: Option<&ShellSpec>,
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
        let reader = self.spawn_in(pane_id, workspace_id, cols, rows, cwd, shell)?;
        Ok((None, reader))
    }

    /// Synchronous write that round-trips through the daemon and waits
    /// for the reply. Suitable for tests, benchmarks, and any caller
    /// that needs to know the daemon accepted the bytes before
    /// returning. Render-thread callers MUST NOT use this; see
    /// [`write`](Self::write) for the fire-and-forget variant.
    pub fn write_blocking(&mut self, pane_id: u32, data: &[u8]) -> io::Result<()> {
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

    /// Fire-and-forget write. Queues the bytes on the worker's
    /// command channel and returns immediately, without waiting for
    /// the daemon's reply. The render thread uses this so a slow
    /// daemon round-trip cannot stall a frame.
    ///
    /// Returns `Ok(())` if the command was queued and `Err(_)` only
    /// for synchronous lookup failures (no connection, unknown pane).
    /// Daemon-side write failures are delivered asynchronously via
    /// [`take_write_errors`](Self::take_write_errors); the bridge
    /// drains that queue and surfaces failures as toasts. Phase 2
    /// of #135.
    pub fn write(&mut self, pane_id: u32, data: &[u8]) -> io::Result<()> {
        let inner = self.inner.as_mut().ok_or_else(not_connected)?;
        let session_id = *inner.sessions.get(&pane_id).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("no PTY for pane {pane_id}"),
            )
        })?;
        let cmd = Command::WriteAsync {
            session_id,
            pane_id,
            bytes: data.to_vec(),
        };
        inner.cmd_tx.send(cmd).map_err(|_| worker_gone())
    }

    /// Drain any pending fire-and-forget write failures the worker
    /// has reported since the last call. Returns an empty vector when
    /// the shim is not connected. Phase 2 of #135.
    pub fn take_write_errors(&mut self) -> Vec<WriteError> {
        let Some(inner) = self.inner.as_ref() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        while let Ok(err) = inner.write_error_rx.try_recv() {
            out.push(err);
        }
        out
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
        self.spawn_shells.remove(&pane_id);
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

    /// Blocking variant of [`kill_session_id`] that waits up to two
    /// seconds for the daemon's ack and surfaces the failure as an
    /// `io::Result`. Mirrors the contract of [`list_sessions`] and
    /// [`rename_session`]: callers in the orphan kill path use this
    /// so a disconnected or slow daemon shows up as a user-visible
    /// error instead of a silently-dropped row. Local pane bookkeeping
    /// is left to the caller to keep the success/failure split clean.
    pub fn kill_session_id_blocking(&mut self, session_id: u64) -> io::Result<()> {
        let inner = self.inner.as_mut().ok_or_else(not_connected)?;
        let (reply_tx, reply_rx) = std_mpsc::sync_channel::<io::Result<()>>(1);
        inner
            .cmd_tx
            .send(Command::KillAck {
                session_id,
                reply: reply_tx,
            })
            .map_err(|_| worker_gone())?;
        let result = reply_rx
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| worker_gone())?;
        if result.is_ok() {
            inner.sessions.retain(|_pane, sid| *sid != session_id);
        }
        result
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
        self.spawn_shells.clear();
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
        Ok(self.list_sessions_snapshot()?.sessions)
    }

    pub fn list_sessions_snapshot(&mut self) -> io::Result<SessionListSnapshot> {
        let inner = self.inner.as_mut().ok_or_else(not_connected)?;
        let (reply_tx, reply_rx) = std_mpsc::sync_channel::<io::Result<SessionListSnapshot>>(1);
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

    /// Test-only: install an `Inner` whose worker channel is already
    /// dead, with a single pane->session mapping pre-populated. Lets
    /// state.rs unit tests exercise the "session known, RPC fails"
    /// path that real disconnected mode does not reach (since
    /// `inner` is `None` and `session_id` returns `None`).
    #[cfg(test)]
    pub(crate) fn test_install_broken_inner_with_session(&mut self, pane_id: u32, session_id: u64) {
        let (cmd_tx, cmd_rx) = tokio_mpsc::unbounded_channel::<Command>();
        // Drop the receiver immediately so any send() returns an
        // SendError, which the shim maps via `worker_gone()` to a
        // BrokenPipe io::Error.
        drop(cmd_rx);
        let mut sessions = HashMap::new();
        sessions.insert(pane_id, session_id);
        let (_write_error_tx, write_error_rx) = std_mpsc::channel::<WriteError>();
        self.inner = Some(Inner {
            cmd_tx,
            sessions,
            reattach_cache: HashMap::new(),
            worker: None,
            write_error_rx,
        });
    }

    /// Install an `Inner` whose `cmd_tx` channel is alive but whose
    /// receiver is parked (held but never drained). Models a
    /// worst-case "infinitely slow daemon" so unit tests and the
    /// criterion bench in `benches/` can prove that fire-and-forget
    /// `write` returns immediately even when the round trip never
    /// completes. Returns an opaque guard that owns the parked
    /// receiver (dropping it would close the channel and turn
    /// `cmd_tx.send` into a `SendError`) and the worker-side error
    /// sender so the caller can simulate failures.
    ///
    /// Marked `#[doc(hidden)]` because it is only intended for the
    /// in-tree test and bench harnesses; outside callers should not
    /// depend on it.
    #[doc(hidden)]
    pub fn test_install_slow_daemon_inner(
        &mut self,
        pane_id: u32,
        session_id: u64,
    ) -> (SlowDaemonGuard, std_mpsc::Sender<WriteError>) {
        let (cmd_tx, cmd_rx) = tokio_mpsc::unbounded_channel::<Command>();
        let mut sessions = HashMap::new();
        sessions.insert(pane_id, session_id);
        let (write_error_tx, write_error_rx) = std_mpsc::channel::<WriteError>();
        self.inner = Some(Inner {
            cmd_tx,
            sessions,
            reattach_cache: HashMap::new(),
            worker: None,
            write_error_rx,
        });
        (
            SlowDaemonGuard {
                _keep_alive: Box::new(cmd_rx),
            },
            write_error_tx,
        )
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

/// Normalize a [`ShellSpec`] for the daemon wire. An empty spec
/// (no `program`) and `None` both map to `(None, vec![])` so the
/// daemon falls back to its own default, matching the additive
/// `shell_args` contract on `Request::SpawnSession`.
fn shell_spec_to_wire(spec: Option<&ShellSpec>) -> (Option<String>, Vec<String>) {
    match spec.filter(|s| !s.is_empty()) {
        Some(s) => (Some(s.program.clone()), s.args.clone()),
        None => (None, Vec::new()),
    }
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
    write_error_tx: std_mpsc::Sender<WriteError>,
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
                    shell,
                    shell_args,
                    workspace_id,
                    pane_id,
                    name,
                    byte_tx,
                    reply,
                } => {
                    let cwd_string = cwd.map(|p| p.display().to_string());
                    // Resolve the shell on the UI side so the daemon does
                    // not fall back to its own (potentially stale) default.
                    // The UI binary is rebuilt on every `cargo run`; the
                    // daemon survives across UI restarts. When the caller
                    // supplied an explicit shell (per-app or per-workspace
                    // setting from #144) we honour it; otherwise we resolve
                    // a sane default here so PowerShell / bash selection
                    // tracks the latest UI binary.
                    let shell = shell.or_else(|| Some(unshit_ptyd::pty::default_shell()));
                    let result = match client
                        .spawn_session(
                            cols,
                            rows,
                            cwd_string,
                            shell,
                            shell_args,
                            workspace_id,
                            pane_id,
                            name,
                        )
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
                Command::WriteAsync {
                    session_id,
                    pane_id,
                    bytes,
                } => {
                    let result: io::Result<()> = match client.write(session_id, bytes).await {
                        Ok(Response::Ack { .. }) => Ok(()),
                        Ok(Response::Error { code, message, .. }) => {
                            Err(io::Error::other(format!("{code}: {message}")))
                        }
                        Ok(other) => Err(io::Error::other(format!("unexpected: {other:?}"))),
                        Err(ProtocolError::Io(e)) => Err(e),
                        Err(other) => Err(io::Error::other(other.to_string())),
                    };
                    if let Err(e) = result {
                        log::warn!(
                            "DaemonPty::write (async) failed for pane {}: {}",
                            pane_id,
                            e
                        );
                        let _ = write_error_tx.send(WriteError { pane_id, error: e });
                    }
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
                Command::KillAck { session_id, reply } => {
                    if let Ok(mut guard) = sinks.lock() {
                        guard.remove(&session_id);
                    }
                    let result: io::Result<()> = match client.kill_session(session_id).await {
                        Ok(Response::Ack { .. }) => Ok(()),
                        Ok(Response::Error { code, message, .. }) => Err(io::Error::other(
                            format!("kill_session failed: {code}: {message}"),
                        )),
                        Ok(other) => Err(io::Error::other(format!("unexpected: {other:?}"))),
                        Err(ProtocolError::Io(e)) => Err(e),
                        Err(other) => Err(io::Error::other(other.to_string())),
                    };
                    let _ = reply.send(result);
                }
                Command::List { reply } => {
                    let result = match client.list_sessions_snapshot().await {
                        Ok(snapshot) => Ok(snapshot),
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

    // refs #130: blocking kill variant must follow list_sessions /
    // rename_session contract: NotConnected when no daemon attached.
    #[test]
    fn kill_session_id_blocking_returns_not_connected_when_disconnected() {
        let mut shim = DaemonPty::new();
        let err = shim
            .kill_session_id_blocking(42)
            .expect_err("blocking kill on unconnected shim must fail");
        assert_eq!(err.kind(), io::ErrorKind::NotConnected);
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
        let spawn_in_err = match shim.spawn_in(2, 1, 80, 24, None, None) {
            Err(e) => e,
            Ok(_) => panic!("spawn_in on unconnected shim must fail"),
        };
        assert_eq!(spawn_in_err.kind(), io::ErrorKind::NotConnected);
        let write_err = shim.write_blocking(1, b"hi").unwrap_err();
        assert_eq!(write_err.kind(), io::ErrorKind::NotConnected);
        // The fire-and-forget variant uses the same error contract for
        // synchronous lookup failures.
        let write_async_err = shim.write(1, b"hi").unwrap_err();
        assert_eq!(write_async_err.kind(), io::ErrorKind::NotConnected);
        let list_err = shim.list_sessions().unwrap_err();
        assert_eq!(list_err.kind(), io::ErrorKind::NotConnected);
        let attach_or_spawn_err = match shim.attach_or_spawn(1, 1, 80, 24, None, None) {
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
            let mut reader = shim
                .spawn_in(pane_id, 1, 80, 24, None, None)
                .expect("spawn_in");
            assert!(shim.has(pane_id));

            shim.write_blocking(pane_id, ECHO_CMD).expect("write");

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
            let _reader = shim
                .spawn_in(pane_id, 1, 80, 24, None, None)
                .expect("spawn_in");
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
            let _r1 = shim.spawn_in(1, 1, 80, 24, None, None).expect("spawn 1");
            let _r2 = shim.spawn_in(2, 1, 80, 24, None, None).expect("spawn 2");
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
            let _reader = shim.spawn_in(100, 1, 80, 24, None, None).expect("spawn_in");
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
            shim.write_blocking(200, ECHO_CMD)
                .expect("write via attached pane");
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
            let _reader_a = shim.spawn_in(10, 1, 80, 24, None, None).expect("spawn_in");
            let session_id = shim
                .session_id_for_pane(10)
                .expect("spawn_in must register pane");

            let (_snap, mut reader_b) = shim
                .attach_to(300, session_id, 0)
                .expect("attach_to live session");
            assert!(shim.has(300));

            // write + resize on the attached pane must not error.
            shim.write_blocking(300, ECHO_CMD)
                .expect("write via attached pane");
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
                .attach_or_spawn(1, 1, 80, 24, None, None)
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
                .spawn_in(pane_id, workspace_id, 80, 24, None, None)
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
                .attach_or_spawn(pane_id, workspace_id, 80, 24, None, None)
                .expect("attach_or_spawn on survivor");
            assert!(
                snapshot.is_some(),
                "attach_or_spawn must reattach when a matching session survives"
            );
            assert!(shim.has(pane_id));

            // And the attached reader stays live: write a bounded marker
            // through the reattached pane and wait for that output instead
            // of blocking forever on an idle shell.
            shim.write_blocking(pane_id, ECHO_CMD)
                .expect("write through reattached pane");
            let deadline = std::time::Instant::now() + Duration::from_millis(1500);
            let mut collected: Vec<u8> = Vec::new();
            let mut buf = [0u8; 64];
            while std::time::Instant::now() < deadline {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        collected.extend_from_slice(&buf[..n]);
                        if String::from_utf8_lossy(&collected).contains("shim-hi") {
                            break;
                        }
                    }
                    Err(e) => panic!("reader erroring on attach survivor: {e}"),
                }
            }
            let text = String::from_utf8_lossy(&collected).to_string();
            assert!(
                text.contains("shim-hi"),
                "reattached reader should receive marker output, got: {text:?}"
            );
            shim.destroy(pane_id);
        })
        .await
        .unwrap();

        daemon.abort();
        let _ = daemon.await;
    }

    #[test]
    fn shell_spec_to_wire_returns_none_for_no_spec() {
        assert_eq!(shell_spec_to_wire(None), (None, Vec::new()));
    }

    #[test]
    fn shell_spec_to_wire_treats_empty_program_as_none() {
        let spec = crate::shell::ShellSpec::default();
        assert_eq!(shell_spec_to_wire(Some(&spec)), (None, Vec::new()));
    }

    #[test]
    fn shell_spec_to_wire_returns_program_and_args_for_filled_spec() {
        let spec = crate::shell::ShellSpec {
            program: "/bin/bash".into(),
            args: vec!["--login".into(), "-i".into()],
        };
        assert_eq!(
            shell_spec_to_wire(Some(&spec)),
            (
                Some("/bin/bash".to_string()),
                vec!["--login".to_string(), "-i".to_string()],
            ),
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn spawn_in_with_shell_spec_routes_program_to_daemon() {
        const MARKER: &str = "spawn-in-shell-spec-marker-140";
        #[cfg(windows)]
        let shell = crate::shell::ShellSpec {
            program: "cmd.exe".into(),
            args: vec!["/C".into(), format!("echo {MARKER}")],
        };
        #[cfg(unix)]
        let shell = crate::shell::ShellSpec {
            program: "/bin/sh".into(),
            args: vec!["-c".into(), format!("echo {MARKER}")],
        };

        let path = unique_socket_path();
        let daemon = start_daemon(&path).await;

        let shim_path = path.clone();
        tokio::task::spawn_blocking(move || {
            let mut shim = DaemonPty::new();
            connect_with_retry(&mut shim, &shim_path);
            let mut reader = shim
                .spawn_in(11, 1, 80, 24, None, Some(&shell))
                .expect("spawn_in with shell spec");

            let deadline = std::time::Instant::now() + Duration::from_millis(2000);
            let mut collected: Vec<u8> = Vec::new();
            let mut buf = [0u8; 4096];
            while std::time::Instant::now() < deadline {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        collected.extend_from_slice(&buf[..n]);
                        if String::from_utf8_lossy(&collected).contains(MARKER) {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            let text = String::from_utf8_lossy(&collected).to_string();
            assert!(
                text.contains(MARKER),
                "expected {MARKER} in output, got: {text:?}"
            );
            shim.destroy(11);
        })
        .await
        .unwrap();

        daemon.abort();
        let _ = daemon.await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn spawn_in_named_forwards_name_to_daemon() {
        // Quick Prompt registers a friendly session name on the daemon
        // so `ptyctl list` shows `qp: <prompt prefix>` instead of an
        // opaque session id. We spawn with name=Some(...) and assert
        // list_sessions returns it.
        std::env::set_var("SHELL", TEST_SHELL);
        let path = unique_socket_path();
        let daemon = start_daemon(&path).await;

        let shim_path = path.clone();
        tokio::task::spawn_blocking(move || {
            let mut shim = DaemonPty::new();
            connect_with_retry(&mut shim, &shim_path);
            let pane_id = 313u32;
            let _reader = shim
                .spawn_in_named(pane_id, 1, 80, 24, None, None, Some("qp: do the thing"))
                .expect("spawn_in_named");

            let list = shim.list_sessions().expect("list_sessions");
            let names: Vec<_> = list.iter().filter_map(|s| s.name.as_deref()).collect();
            assert!(
                names.contains(&"qp: do the thing"),
                "expected name on daemon, got {list:?}"
            );
            shim.destroy(pane_id);
        })
        .await
        .unwrap();

        daemon.abort();
        let _ = daemon.await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn spawn_in_delegates_with_no_name() {
        // The unnamed entry point preserves the prior behavior: no
        // name reaches the daemon, list_sessions returns the session
        // with name=None.
        std::env::set_var("SHELL", TEST_SHELL);
        let path = unique_socket_path();
        let daemon = start_daemon(&path).await;

        let shim_path = path.clone();
        tokio::task::spawn_blocking(move || {
            let mut shim = DaemonPty::new();
            connect_with_retry(&mut shim, &shim_path);
            let pane_id = 314u32;
            let _reader = shim
                .spawn_in(pane_id, 1, 80, 24, None, None)
                .expect("spawn_in");
            let list = shim.list_sessions().expect("list_sessions");
            assert!(
                list.iter().any(|s| s.name.is_none()),
                "expected at least one session with no name, got {list:?}"
            );
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
            let _reader = shim.spawn_in(55, 9, 80, 24, None, None).expect("spawn");
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

    // refs #135 Phase 2: fire-and-forget write must return immediately
    // regardless of how slow the daemon side is. The slow daemon is
    // simulated by a parked `cmd_rx` that never consumes anything; the
    // call should still return in well under 100us per write because
    // `cmd_tx.send` on an unbounded tokio channel is non-blocking.
    #[test]
    fn write_returns_immediately_even_when_daemon_is_infinitely_slow() {
        let mut shim = DaemonPty::new();
        let (_guard, _parked_err_tx) = shim.test_install_slow_daemon_inner(7, 42);

        // Sample a batch of writes and assert the per-call cost stays
        // far below the 100us frame-budget guideline. We use 100 calls
        // so a one-off OS scheduling hiccup does not flake the test;
        // an average over 100 cleanly distinguishes microseconds (the
        // queue-on-channel cost) from milliseconds (a sync IPC round
        // trip on the render thread).
        let n = 100u32;
        let payload = b"x";
        let start = std::time::Instant::now();
        for _ in 0..n {
            shim.write(7, payload).expect("write must queue");
        }
        let elapsed = start.elapsed();
        let per_call = elapsed / n;
        assert!(
            per_call < Duration::from_micros(100),
            "fire-and-forget write took {per_call:?} per call (over 100 calls); \
             must be << 100us so it cannot block the render thread"
        );
    }

    // refs #135 Phase 2: synchronous lookup failure (unknown pane)
    // must still return an error from `write` so the caller does not
    // silently no-op.
    #[test]
    fn write_returns_not_found_when_pane_unknown() {
        let mut shim = DaemonPty::new();
        let (_guard, _parked_err_tx) = shim.test_install_slow_daemon_inner(7, 42);
        let err = shim.write(999, b"hi").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }

    // refs #135 Phase 2: when the worker reports an async-write
    // failure, it lands on the shared error queue and surfaces via
    // `take_write_errors`, NOT via `write`'s return value.
    #[test]
    fn write_failures_surface_via_take_write_errors_queue() {
        let mut shim = DaemonPty::new();
        let (_guard, parked_err_tx) = shim.test_install_slow_daemon_inner(7, 42);
        // Simulate the worker noticing a failed write for pane 7. In
        // production this happens inside the `Command::WriteAsync`
        // branch of `worker_main`. Here we drive it directly so the
        // test does not need a live daemon.
        parked_err_tx
            .send(WriteError {
                pane_id: 7,
                error: io::Error::new(io::ErrorKind::BrokenPipe, "daemon died"),
            })
            .expect("error channel must be alive");

        // The `write` call itself succeeded (it queued the bytes) and
        // does not surface async failures. Failures arrive only via
        // the drain method.
        shim.write(7, b"hi").expect("queue must succeed");
        let errors = shim.take_write_errors();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].pane_id, 7);
        assert_eq!(errors[0].error.kind(), io::ErrorKind::BrokenPipe);
    }

    // refs #135 Phase 2: `take_write_errors` is safe to call before
    // `connect_to` and returns an empty vec rather than panicking.
    #[test]
    fn take_write_errors_on_unconnected_shim_returns_empty() {
        let mut shim = DaemonPty::new();
        let errors = shim.take_write_errors();
        assert!(errors.is_empty());
    }
}
