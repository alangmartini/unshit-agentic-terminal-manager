//! Regression: the client must not deadlock when the daemon closes a
//! connection without responding.
//!
//! Backstory: a daemon-side `FrameTooLarge` on an oversized
//! `SessionAttached` response would propagate up through the handler,
//! drop the connection, and leave the client's `reader_loop` to exit.
//! But the reader left the pending oneshot senders alive in the shared
//! `PendingMap`, so the waiting `roundtrip` blocked forever on
//! `rx.await`. The UI hung at startup on the third launch because the
//! eager `attach_or_spawn` never returned.
//!
//! The fix: `reader_loop` drains `pending` on exit so every in-flight
//! `rx.await` returns `Err`, which `roundtrip` maps to an
//! `UnexpectedEof` IO error the caller can react to.

use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use unshit_ptyd::client::Client;
use unshit_ptyd::protocol::ProtocolError;
use unshit_ptyd::transport::Server;

mod common;

#[cfg(windows)]
async fn bind_test_server(path: &std::path::Path) -> Server {
    Server::bind(path).expect("bind")
}

#[cfg(unix)]
async fn bind_test_server(path: &std::path::Path) -> Server {
    Server::bind(path).await.expect("bind")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn client_returns_err_when_server_drops_connection_mid_roundtrip() {
    let path = common::unique_socket_path();
    let mut server = bind_test_server(&path).await;
    let server_task = tokio::spawn(async move {
        let mut conn = server.accept().await.expect("accept");
        let mut prefix = [0u8; 4];
        if conn.read_exact(&mut prefix).await.is_err() {
            return;
        }
        let body_len = u32::from_le_bytes(prefix) as usize;
        let mut body = vec![0u8; body_len];
        let _ = conn.read_exact(&mut body).await;
        conn.shutdown().await.ok();
    });

    let client_path = path.clone();
    let mut client = Client::connect(&client_path).await.expect("client connect");

    let outcome = tokio::time::timeout(Duration::from_secs(3), client.hello("regression")).await;
    let inner = outcome.expect("client deadlocked after server dropped connection");
    match inner {
        Err(ProtocolError::Io(_)) => {}
        Err(other) => panic!("expected IO error, got {other:?}"),
        Ok(resp) => panic!("expected Err, got {resp:?}"),
    }

    drop(client);
    let _ = tokio::time::timeout(Duration::from_secs(3), server_task).await;
}
