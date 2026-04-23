//! Daemon client used by the `--shutdown` flag, the UI bridge, and
//! tests.
//!
//! The client splits the transport into a write half owned by the
//! [`Client`] and a read half owned by a background task. The reader
//! classifies frames: control frames become [`Response`]s routed back
//! to the matching request via a pending-map; event frames become
//! [`ServerEvent`]s pushed onto an mpsc the caller drains in its own
//! task.

use std::collections::HashMap;
use std::io;
use std::path::Path;
use std::sync::Arc;

use tokio::io::{AsyncRead, WriteHalf};
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio::task::JoinHandle;

use unshit_terminal_core::Snapshot;

use crate::protocol::{
    message::{decode_output_payload, write_request, Request, Response, ServerEvent, SessionInfo},
    read_frame, ProtocolError, KIND_CONTROL, KIND_EVENT, KIND_OUTPUT,
};
use crate::transport::{connect, ClientConnection};

/// Monotonic u64 generator starting at 1. Broken out from [`Client`]
/// so tests can cover the wrap-around behavior without a transport.
#[derive(Debug, Clone, Copy)]
pub struct RequestIds {
    next: u64,
}

impl Default for RequestIds {
    fn default() -> Self {
        Self { next: 1 }
    }
}

impl RequestIds {
    pub fn next(&mut self) -> u64 {
        let id = self.next;
        // Saturation avoids id reuse at u64::MAX.
        self.next = self.next.saturating_add(1);
        id
    }

    pub fn peek(&self) -> u64 {
        self.next
    }
}

type PendingMap = Arc<Mutex<HashMap<u64, oneshot::Sender<Response>>>>;

/// Sequentially issues requests over one transport connection and
/// exposes a stream of server-pushed events.
pub struct Client {
    writer: WriteHalf<ClientConnection>,
    ids: RequestIds,
    pending: PendingMap,
    reader_task: Option<JoinHandle<()>>,
}

impl Client {
    /// Opens a connection to the daemon listening on `path`.
    ///
    /// A background reader task is spawned to dispatch inbound frames.
    /// When the connection dies the task exits and any pending request
    /// yields an IO error.
    pub async fn connect(path: &Path) -> io::Result<Self> {
        let (client, _events) = Self::connect_with_events(path).await?;
        Ok(client)
    }

    /// Variant of [`Client::connect`] that also hands back the
    /// receiver end of the server-event stream.
    pub async fn connect_with_events(
        path: &Path,
    ) -> io::Result<(Self, mpsc::Receiver<ServerEvent>)> {
        let stream = connect(path).await?;
        let (reader, writer) = tokio::io::split(stream);
        let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));
        let (event_tx, event_rx) = mpsc::channel::<ServerEvent>(64);

        let pending_for_reader = pending.clone();
        let reader_task = tokio::spawn(async move {
            reader_loop(reader, pending_for_reader, event_tx).await;
        });

        Ok((
            Self {
                writer,
                ids: RequestIds::default(),
                pending,
                reader_task: Some(reader_task),
            },
            event_rx,
        ))
    }

    /// Returns the next correlation id without consuming it; exposed
    /// for the tests that assert monotonicity.
    pub fn peek_next_id(&self) -> u64 {
        self.ids.peek()
    }

    fn alloc_id(&mut self) -> u64 {
        self.ids.next()
    }

    /// Sends a Hello and waits for the matching HelloAck.
    pub async fn hello(&mut self, client_version: &str) -> Result<Response, ProtocolError> {
        let id = self.alloc_id();
        let req = Request::Hello {
            id,
            client_version: client_version.to_string(),
        };
        self.roundtrip(req, id).await
    }

    /// Sends a Shutdown and waits for the matching ShutdownAck.
    pub async fn shutdown(&mut self) -> Result<Response, ProtocolError> {
        let id = self.alloc_id();
        self.roundtrip(Request::Shutdown { id }, id).await
    }

    /// Spawns a session on the daemon.
    pub async fn spawn_session(
        &mut self,
        cols: u16,
        rows: u16,
        cwd: Option<String>,
        shell: Option<String>,
    ) -> Result<Response, ProtocolError> {
        let id = self.alloc_id();
        let req = Request::SpawnSession {
            id,
            cols,
            rows,
            cwd,
            shell,
        };
        self.roundtrip(req, id).await
    }

    /// Writes `bytes` to the PTY stdin of `session_id`.
    pub async fn write(
        &mut self,
        session_id: u64,
        bytes: Vec<u8>,
    ) -> Result<Response, ProtocolError> {
        let id = self.alloc_id();
        let req = Request::Write {
            id,
            session_id,
            bytes,
        };
        self.roundtrip(req, id).await
    }

    /// Resizes the PTY of `session_id`.
    pub async fn resize(
        &mut self,
        session_id: u64,
        cols: u16,
        rows: u16,
    ) -> Result<Response, ProtocolError> {
        let id = self.alloc_id();
        let req = Request::Resize {
            id,
            session_id,
            cols,
            rows,
        };
        self.roundtrip(req, id).await
    }

    /// Kills the session with the given id.
    pub async fn kill_session(&mut self, session_id: u64) -> Result<Response, ProtocolError> {
        let id = self.alloc_id();
        self.roundtrip(Request::KillSession { id, session_id }, id)
            .await
    }

    /// Returns the current list of sessions.
    pub async fn list_sessions(&mut self) -> Result<Vec<SessionInfo>, ProtocolError> {
        let id = self.alloc_id();
        let resp = self.roundtrip(Request::ListSessions { id }, id).await?;
        match resp {
            Response::SessionList { sessions, .. } => Ok(sessions),
            Response::Error { code, message, .. } => Err(ProtocolError::Io(io::Error::other(
                format!("list_sessions failed: {code}: {message}"),
            ))),
            other => Err(ProtocolError::Io(io::Error::other(format!(
                "unexpected response: {other:?}"
            )))),
        }
    }

    /// Fetches the daemon's current snapshot for `session_id`.
    ///
    /// `scrollback_lines` is clamped server-side at
    /// [`crate::protocol::SNAPSHOT_MAX_SCROLLBACK_LINES`] so an
    /// over-eager request simply returns that many lines rather than
    /// erroring.
    pub async fn attach_session(
        &mut self,
        session_id: u64,
        scrollback_lines: u32,
    ) -> Result<Snapshot, ProtocolError> {
        let id = self.alloc_id();
        let req = Request::AttachSession {
            id,
            session_id,
            scrollback_lines,
        };
        let resp = self.roundtrip(req, id).await?;
        match resp {
            Response::SessionAttached { snapshot, .. } => Ok(snapshot),
            Response::Error { code, message, .. } => Err(ProtocolError::Io(io::Error::other(
                format!("attach_session failed: {code}: {message}"),
            ))),
            other => Err(ProtocolError::Io(io::Error::other(format!(
                "unexpected response: {other:?}"
            )))),
        }
    }

    /// Slice 4 no-op that will mean "keep running" in slice 5. Exposed
    /// now so the UI can start calling it today without churn later.
    pub async fn detach_session(&mut self, session_id: u64) -> Result<Response, ProtocolError> {
        let id = self.alloc_id();
        self.roundtrip(Request::DetachSession { id, session_id }, id)
            .await
    }

    async fn roundtrip(
        &mut self,
        req: Request,
        expected_id: u64,
    ) -> Result<Response, ProtocolError> {
        let (tx, rx) = oneshot::channel::<Response>();
        {
            let mut guard = self.pending.lock().await;
            guard.insert(expected_id, tx);
        }
        if let Err(e) = write_request(&mut self.writer, &req).await {
            // Clean up the pending slot so we never leak a oneshot.
            let mut guard = self.pending.lock().await;
            guard.remove(&expected_id);
            return Err(e);
        }
        match rx.await {
            Ok(resp) => Ok(resp),
            Err(_) => Err(ProtocolError::Io(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "daemon closed connection before responding",
            ))),
        }
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        if let Some(handle) = self.reader_task.take() {
            handle.abort();
        }
    }
}

async fn reader_loop<R>(mut reader: R, pending: PendingMap, events: mpsc::Sender<ServerEvent>)
where
    R: AsyncRead + Unpin,
{
    loop {
        let frame = match read_frame(&mut reader).await {
            Ok(Some(frame)) => frame,
            Ok(None) => return,
            Err(_) => return,
        };
        match frame.kind {
            KIND_CONTROL => {
                let resp: Response = match serde_json::from_slice(&frame.payload) {
                    Ok(r) => r,
                    Err(_) => return,
                };
                let id = resp.id();
                let mut guard = pending.lock().await;
                if let Some(tx) = guard.remove(&id) {
                    let _ = tx.send(resp);
                }
            }
            KIND_OUTPUT => {
                let (session_id, bytes) = match decode_output_payload(&frame.payload) {
                    Ok(pair) => pair,
                    Err(_) => return,
                };
                let event = ServerEvent::Output {
                    session_id,
                    bytes: bytes.to_vec(),
                };
                if events.send(event).await.is_err() {
                    return;
                }
            }
            KIND_EVENT => {
                let event: ServerEvent = match serde_json::from_slice(&frame.payload) {
                    Ok(ev) => ev,
                    Err(_) => return,
                };
                if events.send(event).await.is_err() {
                    return;
                }
            }
            _ => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_ids_start_at_one_and_increment() {
        let mut ids = RequestIds::default();
        assert_eq!(ids.peek(), 1);
        assert_eq!(ids.next(), 1);
        assert_eq!(ids.next(), 2);
        assert_eq!(ids.next(), 3);
        assert_eq!(
            ids.peek(),
            4,
            "peek must report the next id, not the last handed out"
        );
    }

    #[test]
    fn request_ids_saturate_instead_of_wrapping() {
        let mut ids = RequestIds { next: u64::MAX };
        assert_eq!(ids.next(), u64::MAX);
        assert_eq!(ids.next(), u64::MAX, "must not wrap to zero");
    }
}
