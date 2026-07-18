//! Per-connection request handler.
//!
//! Each connection gets its own [`SessionRegistry`]. Slice 5 promotes
//! sessions to survive the client that spawned them: a disconnect drains
//! forwarders and detaches every session, but the children keep running
//! and their terminals keep parsing. Sessions only die on explicit
//! `KillSession` or when the daemon itself exits.
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
use crate::session::{registry::SessionRegistry, AttachmentToken};
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
/// that the daemon is stopping, so they can close out cleanly. The
/// `registry` is shared across every connection so sessions survive
/// individual client disconnects.
pub async fn serve_connection<S>(
    stream: S,
    shutdown: broadcast::Sender<()>,
    registry: Arc<SessionRegistry>,
) -> Result<(), ProtocolError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (read_half, write_half) = tokio::io::split(stream);
    let mut reader = read_half;
    let writer = Arc::new(Mutex::new(write_half));
    let mut shutdown_rx = shutdown.subscribe();

    // Keyed by `(session_id, attachment_token)`. The token lets cleanup
    // prove that this connection still owns the session's output sink;
    // a stale connection must never detach a newer attachment.
    let mut forwarders: HashMap<(u64, AttachmentToken), JoinHandle<()>> = HashMap::new();

    let result = loop {
        tokio::select! {
            _ = shutdown_rx.recv() => break Ok(()),
            req = read_request(&mut reader) => {
                let req = match req {
                    Ok(Some(request)) => request,
                    Ok(None) => break Ok(()),
                    Err(error) => break Err(error),
                };
                match handle(req, writer.clone(), registry.clone(), &mut forwarders).await {
                    Ok(PostRequest::Continue) => continue,
                    Ok(PostRequest::ShutdownRequested) => {
                        let _ = shutdown.send(());
                        break Ok(());
                    }
                    Err(error) => break Err(error),
                }
            }
        }
    };

    // Sessions survive client disconnect (slice 5). Abort the per-session
    // forwarder tasks so they release the writer, then detach each
    // session so a later attach sees a fresh channel. The `Terminal`
    // inside each session keeps parsing PTY output so reattaches observe
    // the authoritative grid plus scrollback.
    cleanup_forwarders(&mut forwarders, &registry).await;

    result
}

type SharedWriter<W> = Arc<Mutex<W>>;

async fn handle<W>(
    req: Request,
    writer: SharedWriter<W>,
    registry: Arc<SessionRegistry>,
    forwarders: &mut HashMap<(u64, AttachmentToken), JoinHandle<()>>,
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
        Request::Shutdown { id, force } => {
            if force {
                let killed = registry.kill_all().await;
                if !killed.is_empty() {
                    log::info!(
                        "force shutdown: killed {} session(s): {killed:?}",
                        killed.len()
                    );
                }
            }
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
            shell_args,
            workspace_id,
            pane_id,
            name,
            restore_correlation_id,
        } => {
            let cwd_path = cwd.as_deref().map(PathBuf::from);
            let shell_ref = shell.as_deref();
            let spawn_res = registry
                .spawn_with_context(
                    cols,
                    rows,
                    cwd_path.as_deref(),
                    shell_ref,
                    &shell_args,
                    workspace_id,
                    pane_id,
                    name,
                    restore_correlation_id.as_deref(),
                )
                .await;
            match spawn_res {
                Ok((session_id, attachment_token, hook_capability, rx)) => {
                    let handle = tokio::spawn(forward_output(session_id, rx, writer.clone()));
                    forwarders.insert((session_id, attachment_token), handle);
                    send_response(
                        &writer,
                        Response::SessionSpawned {
                            id,
                            session_id,
                            hook_capability,
                        },
                    )
                    .await?;
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
            abort_forwarders_for_session(forwarders, session_id).await;
            send_response(&writer, Response::Ack { id }).await?;
            Ok(PostRequest::Continue)
        }
        Request::ListSessions { id } => {
            let sessions = registry.list().await;
            send_response(
                &writer,
                Response::SessionList {
                    id,
                    sessions,
                    daemon_pid: Some(std::process::id()),
                    daemon_memory_rss_bytes: crate::memory::current_resident_set_bytes(),
                },
            )
            .await?;
            Ok(PostRequest::Continue)
        }
        Request::EnsureSession {
            id,
            cols,
            rows,
            cwd,
            shell,
            shell_args,
            workspace_id,
            pane_id,
            name,
            restore_correlation_id,
            scrollback_lines,
        } => {
            let cwd_path = cwd.as_deref().map(PathBuf::from);
            let clamped = (scrollback_lines as usize).min(SNAPSHOT_MAX_SCROLLBACK_LINES);
            match registry
                .ensure_with_context(
                    cols,
                    rows,
                    cwd_path.as_deref(),
                    shell.as_deref(),
                    &shell_args,
                    workspace_id,
                    pane_id,
                    name,
                    clamped,
                    restore_correlation_id.as_deref(),
                )
                .await
            {
                Ok(ensured) => {
                    let session_id = ensured.session_id;
                    abort_forwarders_for_session(forwarders, session_id).await;
                    let handle =
                        tokio::spawn(forward_output(session_id, ensured.output, writer.clone()));
                    forwarders.insert((session_id, ensured.attachment_token), handle);
                    send_response(
                        &writer,
                        Response::SessionEnsured {
                            id,
                            session_id,
                            disposition: ensured.disposition,
                            snapshot: ensured.snapshot,
                            hook_capability: ensured.hook_capability,
                        },
                    )
                    .await?;
                }
                Err(e) => {
                    let code = if e.kind() == io::ErrorKind::AlreadyExists {
                        error_code(&e)
                    } else {
                        "spawn_failed"
                    };
                    send_err(&writer, id, code, &e).await?;
                }
            }
            Ok(PostRequest::Continue)
        }
        Request::AttachSession {
            id,
            session_id,
            scrollback_lines,
        } => {
            let clamped = (scrollback_lines as usize).min(SNAPSHOT_MAX_SCROLLBACK_LINES);
            let attachment = registry.attach_with_snapshot(session_id, clamped).await;
            match attachment {
                Some((attachment_token, hook_capability, snapshot, rx)) => {
                    abort_forwarders_for_session(forwarders, session_id).await;
                    let handle = tokio::spawn(forward_output(session_id, rx, writer.clone()));
                    forwarders.insert((session_id, attachment_token), handle);
                    send_response(
                        &writer,
                        Response::SessionAttached {
                            id,
                            snapshot,
                            hook_capability,
                        },
                    )
                    .await?;
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
        Request::DetachSession { id, session_id } => {
            let attachment_keys: Vec<_> = forwarders
                .keys()
                .copied()
                .filter(|(attached_session_id, _)| *attached_session_id == session_id)
                .collect();
            for (attached_session_id, attachment_token) in attachment_keys {
                if let Some(handle) = forwarders.remove(&(attached_session_id, attachment_token)) {
                    handle.abort();
                    let _ = handle.await;
                }
                registry.detach(attached_session_id, attachment_token).await;
            }
            send_response(&writer, Response::Ack { id }).await?;
            Ok(PostRequest::Continue)
        }
        Request::RenameSession {
            id,
            session_id,
            name,
        } => {
            if registry.rename(session_id, name).await {
                send_response(&writer, Response::Ack { id }).await?;
            } else {
                let err = io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("no session for id {session_id}"),
                );
                send_err(&writer, id, error_code(&err), &err).await?;
            }
            Ok(PostRequest::Continue)
        }
    }
}

async fn abort_forwarders_for_session(
    forwarders: &mut HashMap<(u64, AttachmentToken), JoinHandle<()>>,
    session_id: u64,
) {
    let attachment_keys: Vec<_> = forwarders
        .keys()
        .copied()
        .filter(|(attached_session_id, _)| *attached_session_id == session_id)
        .collect();
    for attachment_key in attachment_keys {
        if let Some(handle) = forwarders.remove(&attachment_key) {
            handle.abort();
            let _ = handle.await;
        }
    }
}

async fn cleanup_forwarders(
    forwarders: &mut HashMap<(u64, AttachmentToken), JoinHandle<()>>,
    registry: &SessionRegistry,
) {
    for ((session_id, attachment_token), handle) in forwarders.drain() {
        handle.abort();
        let _ = handle.await;
        registry.detach(session_id, attachment_token).await;
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
        io::ErrorKind::AlreadyExists => "session_key_ambiguous",
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
    use tokio::io::{duplex, AsyncWriteExt};

    #[tokio::test]
    async fn hello_elicits_hello_ack_with_echoed_id() {
        let (client, server) = duplex(4096);
        let (shutdown_tx, _shutdown_rx) = broadcast::channel(4);
        let registry = Arc::new(SessionRegistry::new());

        let server_task = tokio::spawn(async move {
            serve_connection(server, shutdown_tx, registry)
                .await
                .unwrap();
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
        let registry = Arc::new(SessionRegistry::new());

        let server_task = tokio::spawn(async move {
            serve_connection(server, shutdown_tx, registry)
                .await
                .unwrap();
        });

        let (mut client_read, mut client_write) = tokio::io::split(client);
        write_request(
            &mut client_write,
            &Request::Shutdown {
                id: 3,
                force: false,
            },
        )
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

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn stale_connection_cleanup_preserves_newer_attachment() {
        #[cfg(windows)]
        let test_shell = "cmd.exe";
        #[cfg(unix)]
        let test_shell = "/bin/sh";

        let registry = Arc::new(SessionRegistry::new());
        let writer_a = Arc::new(Mutex::new(tokio::io::sink()));
        let writer_b = Arc::new(Mutex::new(tokio::io::sink()));
        let mut forwarders_a = HashMap::new();
        let mut forwarders_b = HashMap::new();

        handle(
            Request::SpawnSession {
                id: 1,
                cols: 80,
                rows: 24,
                cwd: None,
                shell: Some(test_shell.into()),
                shell_args: vec![],
                workspace_id: 7,
                pane_id: 3,
                name: None,
                restore_correlation_id: None,
            },
            writer_a,
            registry.clone(),
            &mut forwarders_a,
        )
        .await
        .expect("spawn request");
        let session_id = registry.list().await[0].id;

        handle(
            Request::AttachSession {
                id: 2,
                session_id,
                scrollback_lines: 0,
            },
            writer_b,
            registry.clone(),
            &mut forwarders_b,
        )
        .await
        .expect("attach request");
        let current_token = forwarders_b
            .keys()
            .find_map(|(id, token)| (*id == session_id).then_some(*token))
            .expect("new connection attachment token");

        cleanup_forwarders(&mut forwarders_a, &registry).await;

        assert!(
            registry.detach(session_id, current_token).await,
            "cleanup from the original connection must not detach the replacement"
        );

        cleanup_forwarders(&mut forwarders_b, &registry).await;
        registry.remove(session_id).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn protocol_error_still_detaches_the_connections_attachment() {
        #[cfg(windows)]
        let (test_shell, shell_args) = (
            "cmd.exe",
            vec![
                "/Q".into(),
                "/D".into(),
                "/C".into(),
                "ping -n 30 127.0.0.1 >nul".into(),
            ],
        );
        #[cfg(unix)]
        let (test_shell, shell_args) = ("/bin/sh", vec!["-c".into(), "sleep 30".into()]);

        let (client, server) = duplex(16 * 1024);
        let (shutdown_tx, _shutdown_rx) = broadcast::channel(4);
        let registry = Arc::new(SessionRegistry::new());
        let server_registry = registry.clone();
        let server_task =
            tokio::spawn(
                async move { serve_connection(server, shutdown_tx, server_registry).await },
            );

        let (mut client_read, mut client_write) = tokio::io::split(client);
        write_request(
            &mut client_write,
            &Request::SpawnSession {
                id: 9,
                cols: 80,
                rows: 24,
                cwd: None,
                shell: Some(test_shell.into()),
                shell_args,
                workspace_id: 7,
                pane_id: 4,
                name: None,
                restore_correlation_id: None,
            },
        )
        .await
        .expect("spawn request");
        let response = read_response(&mut client_read)
            .await
            .expect("spawn response frame")
            .expect("spawn response");
        let session_id = match response {
            Response::SessionSpawned { session_id, .. } => session_id,
            other => panic!("unexpected spawn response: {other:?}"),
        };

        // A one-byte frame body whose kind byte is not part of the
        // protocol forces `read_request` down its error path.
        client_write
            .write_all(&[0, 0, 0, 1, 0xff])
            .await
            .expect("malformed frame write");
        drop(client_write);
        drop(client_read);

        assert!(
            server_task.await.expect("server task").is_err(),
            "the malformed frame must surface a protocol error"
        );
        assert!(
            !registry.detach(session_id, 1).await,
            "error-path cleanup must already detach the connection's initial attachment"
        );
        registry.remove(session_id).await;
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
