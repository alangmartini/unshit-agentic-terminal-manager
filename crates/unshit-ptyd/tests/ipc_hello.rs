//! End-to-end: client sends Hello, daemon responds with HelloAck.

use unshit_ptyd::daemon;
use unshit_ptyd::protocol::{Response, PROTOCOL_VERSION};

mod common;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hello_round_trip() {
    let path = common::unique_socket_path();
    let daemon_path = path.clone();
    let server_handle = tokio::spawn(async move {
        daemon::run(&daemon_path).await.unwrap();
    });

    let mut client = common::connect_with_retry(&path).await;
    let resp = client.hello("integration-test").await.unwrap();

    match resp {
        Response::HelloAck {
            id,
            protocol_version,
            ..
        } => {
            assert_eq!(id, 1, "first client id must be 1");
            assert_eq!(protocol_version, PROTOCOL_VERSION);
        }
        other => panic!("unexpected response: {other:?}"),
    }

    // Monotonic ids: a second hello yields id 2.
    let second = client.hello("integration-test").await.unwrap();
    assert_eq!(second.id(), 2);

    // Clean up: send shutdown so the daemon task terminates.
    let _ = client.shutdown().await;
    let _ = server_handle.await;
}
