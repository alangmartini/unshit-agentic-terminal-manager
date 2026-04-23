//! Per-connection request handler.
//!
//! Reads requests, dispatches them, writes responses. For slice 2 the
//! vocabulary is just hello and shutdown; additional request kinds slot
//! in without touching the outer loop.
//!
//! A panic or error inside a handler must not bring the daemon down.
//! The outer `Daemon::run` wraps every connection task in
//! `catch_unwind` and logs.

use std::io;

use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::broadcast;

use crate::protocol::{
    message::{read_request, write_response, Request, Response},
    ProtocolError, PROTOCOL_VERSION,
};
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
    S: AsyncRead + AsyncWrite + Unpin,
{
    let (read_half, write_half) = tokio::io::split(stream);
    let mut reader = read_half;
    let mut writer = write_half;
    let mut shutdown_rx = shutdown.subscribe();

    loop {
        tokio::select! {
            // Another handler pulled the shutdown lever; finish gracefully.
            _ = shutdown_rx.recv() => return Ok(()),
            req = read_request(&mut reader) => {
                let req = match req? {
                    Some(r) => r,
                    None => return Ok(()),
                };
                match handle(req, &mut writer).await? {
                    PostRequest::Continue => continue,
                    PostRequest::ShutdownRequested => {
                        // Tell sibling connections to wrap up.
                        let _ = shutdown.send(());
                        return Ok(());
                    }
                }
            }
        }
    }
}

async fn handle<W>(req: Request, writer: &mut W) -> Result<PostRequest, ProtocolError>
where
    W: AsyncWrite + Unpin,
{
    match req {
        Request::Hello { id, client_version } => {
            log::debug!("hello from client_version={client_version} id={id}");
            let resp = Response::HelloAck {
                id,
                server_version: DAEMON_VERSION.to_string(),
                protocol_version: PROTOCOL_VERSION,
            };
            write_response(writer, &resp).await?;
            Ok(PostRequest::Continue)
        }
        Request::Shutdown { id } => {
            // Slice 2: no sessions exist yet, so shutdown always succeeds.
            // Slice 3 will gate this on the session registry.
            let resp = Response::ShutdownAck {
                id,
                ok: true,
                reason: None,
            };
            write_response(writer, &resp).await?;
            Ok(PostRequest::ShutdownRequested)
        }
    }
}

/// Converts a protocol error into an IO error for loop-level logging.
///
/// Callers that want the original variant should keep the
/// `ProtocolError` instead; this helper exists so the outer supervisor
/// can uniformly log with `io::Error`.
pub fn protocol_to_io(err: ProtocolError) -> io::Error {
    match err {
        ProtocolError::Io(e) => e,
        other => io::Error::new(io::ErrorKind::InvalidData, other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::duplex;

    #[tokio::test]
    async fn hello_elicits_hello_ack_with_echoed_id() {
        let (client, server) = duplex(4096);
        let (shutdown_tx, _shutdown_rx) = broadcast::channel(4);

        let server_task = tokio::spawn(async move {
            serve_connection(server, shutdown_tx).await.unwrap();
        });

        let (mut client_read, mut client_write) = tokio::io::split(client);
        use crate::protocol::message::{read_response, write_request};
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

        // Close the duplex so the server sees EOF on its read side and
        // exits the accept loop. `tokio::io::split` hands out two halves
        // that jointly own the stream via a shared lock: dropping just
        // `client_write` is not enough because `client_read` keeps the
        // stream alive. Both halves must go.
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
        use crate::protocol::message::{read_response, write_request};
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
}
