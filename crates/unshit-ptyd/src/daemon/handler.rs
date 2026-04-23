//! Per-connection request handler.
//!
//! Each connection gets its own [`SessionRegistry`]. A session spawned
//! from a connection is implicitly cleaned up when that connection
//! closes, matching the current in-process UI behavior (close UI,
//! shells die). Slice 5 will introduce cross-connection persistence.
//!
//! The handler holds the write half of the connection behind a tokio
//! mutex because both the request-reply loop and the per-session output
//! forwarders need to write frames. Serializing them on a mutex keeps
//! frame bytes from interleaving on the wire.

use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{broadcast, mpsc, Mutex};
use tokio::task::JoinHandle;

use crate::protocol::{
    message::{
        read_request, write_output_frame, write_response, Request, Response,
        SNAPSHOT_MAX_SCROLLBACK_LINES,
    },
    ProtocolError, PROTOCOL_VERSION,
};
use crate::session::registry::SessionRegistry;
use crate::DAEMON_VERSION;

/// Outcome the outer loop uses to decide whether to keep serving.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PostRequest {
    /// Keep serving further requests on this connection.
    Continue,
    /// The client asked us to shut down; stop the outer accept loop
    /// after the current handler returns.
    ShutdownRequested,
}

/// Drives the request loop on a single connection.
///
/// The `shutdown` broadcast is used to notify other in-flight handlers
/// that the daemon is stopping, so they can close out cleanly.
pub async fn serve_connection<S>(
    stream: S,
    shutdown: broadcast::Sender<()>,
) -> Result<(), ProtocolError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (read_half, write_half) = tokio::io::split(stream);
    let mut reader = read_half;
    let writer = Arc::new(Mutex::new(write_half));
    let mut shutdown_rx = shutdown.subscribe();

    let registry = Arc::new(SessionRegistry::new());
    // Keyed by session_id so KillSession can abort the matching
    // forwarder without a linear scan, and per-connection cleanup
    // can drain them all on close.
    let mut forwarders: HashMap<u64, JoinHandle<()>> = HashMap::new();

    let result = loop {
        tokio::select! {
            _ = shutdown_rx.recv() => break Ok(()),
            req = read_request(&mut reader) => {
                let req = match req? {
                    Some(r) => r,
                    None => break Ok(()),
                };
                match handle(req, writer.clone(), registry.clone(), &mut forwarders).await? {
                    PostRequest::Continue => continue,
                    PostRequest::ShutdownRequested => {
                        let _ = shutdown.send(());
                        break Ok(());
                    }
                }
            }
        }
    };

    // Clean up every session spawned on this connection, then await the
    // per-session forwarder tasks so they exit before we drop the
    // writer.
    registry.kill_all().await;
    for (_id, handle) in forwarders.drain() {
        handle.abort();
        let _ = handle.await;
    }

    result
}

type SharedWriter<W> = Arc<Mutex<W>>;

async fn handle<W>(
    req: Request,
    writer: SharedWriter<W>,
    registry: Arc<SessionRegistry>,
    forwarders: &mut HashMap<u64, JoinHandle<()>>,
) -> Result<PostRequest, ProtocolError>
where
    W: AsyncWrite + Unpin + Send + 'static,
{
    match req {
        Request::Hello { id, client_version } => {
            log::debug!("hello from client_version={client_version} id={id}");
            send_response(
                &writer,
                Response::HelloAck {
                    id,
                    server_version: DAEMON_VERSION.to_string(),
                    protocol_version: PROTOCOL_VERSION,
                },
            )
            .await?;
            Ok(PostRequest::Continue)
        }
        Request::Shutdown { id } => {
            let alive = registry.len().await;
            if alive > 0 {
                // Slice 3 policy: refuse shutdown while this connection
                // still owns live sessions. Slice 5 reworks this gate
                // against the global registry instead of per-connection.
                send_response(
                    &writer,
                    Response::ShutdownAck {
                        id,
                        ok: false,
                        reason: Some(format!("{alive} sessions alive")),
                    },
                )
                .await?;
                Ok(PostRequest::Continue)
            } else {
                send_response(
                    &writer,
                    Response::ShutdownAck {
                        id,
                        ok: true,
                        reason: None,
                    },
                )
                .await?;
                Ok(PostRequest::ShutdownRequested)
            }
        }
        Request::SpawnSession {
            id,
            cols,
            rows,
            cwd,
            shell,
        } => {
            let cwd_path = cwd.as_deref().map(PathBuf::from);
            let shell_ref = shell.as_deref();
            let spawn_res = registry
                .spawn(cols, rows, cwd_path.as_deref(), shell_ref)
                .await;
            match spawn_res {
                Ok((session_id, rx)) => {
                    let handle = tokio::spawn(forward_output(session_id, rx, writer.clone()));
                    forwarders.insert(session_id, handle);
                    send_response(&writer, Response::SessionSpawned { id, session_id }).await?;
                }
                Err(e) => {
                    send_err(&writer, id, "spawn_failed", &e).await?;
                }
            }
            Ok(PostRequest::Continue)
        }
        Request::Write {
            id,
            session_id,
            bytes,
        } => {
            match registry.write(session_id, &bytes).await {
                Ok(()) => send_response(&writer, Response::Ack { id }).await?,
                Err(e) => send_err(&writer, id, error_code(&e), &e).await?,
            }
            Ok(PostRequest::Continue)
        }
        Request::Resize {
            id,
            session_id,
            cols,
            rows,
        } => {
            match registry.resize(session_id, cols, rows).await {
                Ok(()) => send_response(&writer, Response::Ack { id }).await?,
                Err(e) => send_err(&writer, id, error_code(&e), &e).await?,
            }
            Ok(PostRequest::Continue)
        }
        Request::KillSession { id, session_id } => {
            registry.remove(session_id).await;
            if let Some(h) = forwarders.remove(&session_id) {
                h.abort();
            }
            send_response(&writer, Response::Ack { id }).await?;
            Ok(PostRequest::Continue)
        }
        Request::ListSessions { id } => {
            let sessions = registry.list().await;
            send_response(&writer, Response::SessionList { id, sessions }).await?;
            Ok(PostRequest::Continue)
        }
        Request::AttachSession {
            id,
            session_id,
            scrollback_lines,
        } => {
            let clamped = (scrollback_lines as usize).min(SNAPSHOT_MAX_SCROLLBACK_LINES);
            match registry.snapshot(session_id, clamped).await {
                Some(snapshot) => {
                    send_response(&writer, Response::SessionAttached { id, snapshot }).await?;
                }
                None => {
                    let err = io::Error::new(
                        io::ErrorKind::NotFound,
                        format!("no session for id {session_id}"),
                    );
                    send_err(&writer, id, error_code(&err), &err).await?;
                }
            }
            Ok(PostRequest::Continue)
        }
        Request::DetachSession {
            id,
            session_id: _session_id,
        } => {
            // Slice 4 policy: detach is a no-op; sessions die on
            // connection close (see slice 3a per-connection cleanup).
            // Slice 5 promotes detach to "keep running" and wires the
            // cross-connection persistence path.
            send_response(&writer, Response::Ack { id }).await?;
            Ok(PostRequest::Continue)
        }
    }
}

async fn send_response<W>(writer: &SharedWriter<W>, resp: Response) -> Result<(), ProtocolError>
where
    W: AsyncWrite + Unpin,
{
    let mut guard = writer.lock().await;
    write_response(&mut *guard, &resp).await
}

async fn send_err<W>(
    writer: &SharedWriter<W>,
    id: u64,
    code: impl Into<String>,
    e: &io::Error,
) -> Result<(), ProtocolError>
where
    W: AsyncWrite + Unpin,
{
    send_response(
        writer,
        Response::Error {
            id,
            code: code.into(),
            message: e.to_string(),
        },
    )
    .await
}

/// Forwards every byte chunk from `rx` as a `KIND_OUTPUT` frame on
/// `writer`, tagging the chunk with `session_id`. Exits when the
/// session drops its sender or the writer errors out.
async fn forward_output<W>(
    session_id: u64,
    mut rx: mpsc::Receiver<Vec<u8>>,
    writer: SharedWriter<W>,
) where
    W: AsyncWrite + Unpin,
{
    while let Some(bytes) = rx.recv().await {
        let mut guard = writer.lock().await;
        if write_output_frame(&mut *guard, session_id, &bytes)
            .await
            .is_err()
        {
            return;
        }
    }
}

fn error_code(e: &io::Error) -> &'static str {
    match e.kind() {
        io::ErrorKind::NotFound => "session_not_found",
        io::ErrorKind::NotConnected => "session_dead",
        _ => "io_error",
    }
}

/// Converts a protocol error into an IO error for loop-level logging.
pub fn protocol_to_io(err: ProtocolError) -> io::Error {
    match err {
        ProtocolError::Io(e) => e,
        other => io::Error::new(io::ErrorKind::InvalidData, other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::message::{read_response, write_request};
    use tokio::io::duplex;

    #[tokio::test]
    async fn hello_elicits_hello_ack_with_echoed_id() {
        let (client, server) = duplex(4096);
        let (shutdown_tx, _shutdown_rx) = broadcast::channel(4);

        let server_task = tokio::spawn(async move {
            serve_connection(server, shutdown_tx).await.unwrap();
        });

        let (mut client_read, mut client_write) = tokio::io::split(client);
        write_request(
            &mut client_write,
            &Request::Hello {
                id: 7,
                client_version: "test".into(),
            },
        )
        .await
        .unwrap();
        let resp = read_response(&mut client_read)
            .await
            .unwrap()
            .expect("hello_ack");
        match resp {
            Response::HelloAck {
                id,
                protocol_version,
                ..
            } => {
                assert_eq!(id, 7);
                assert_eq!(protocol_version, PROTOCOL_VERSION);
            }
            other => panic!("unexpected response: {other:?}"),
        }

        // Closing the duplex: see note in slice 2. Both halves must go.
        drop(client_write);
        drop(client_read);
        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn shutdown_returns_shutdown_ack_and_drops_connection() {
        let (client, server) = duplex(4096);
        let (shutdown_tx, _shutdown_rx) = broadcast::channel(4);

        let server_task = tokio::spawn(async move {
            serve_connection(server, shutdown_tx).await.unwrap();
        });

        let (mut client_read, mut client_write) = tokio::io::split(client);
        write_request(&mut client_write, &Request::Shutdown { id: 3 })
            .await
            .unwrap();
        let resp = read_response(&mut client_read)
            .await
            .unwrap()
            .expect("shutdown_ack");
        assert_eq!(
            resp,
            Response::ShutdownAck {
                id: 3,
                ok: true,
                reason: None,
            }
        );
        server_task.await.unwrap();
    }

    #[test]
    fn protocol_to_io_preserves_io_kind() {
        let e = ProtocolError::Io(io::Error::new(io::ErrorKind::ConnectionReset, "x"));
        let io_err = protocol_to_io(e);
        assert_eq!(io_err.kind(), io::ErrorKind::ConnectionReset);
    }

    #[test]
    fn protocol_to_io_wraps_non_io_variants_as_invalid_data() {
        let e = ProtocolError::EmptyFrame;
        let io_err = protocol_to_io(e);
        assert_eq!(io_err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn error_code_maps_not_found_to_session_not_found() {
        let e = io::Error::new(io::ErrorKind::NotFound, "x");
        assert_eq!(error_code(&e), "session_not_found");
    }

    #[test]
    fn error_code_falls_back_to_io_error() {
        let e = io::Error::other("x");
        assert_eq!(error_code(&e), "io_error");
    }
}
