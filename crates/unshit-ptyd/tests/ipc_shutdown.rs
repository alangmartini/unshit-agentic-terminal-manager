//! End-to-end: client sends Shutdown, server exits cleanly and the
//! socket path is released so a fresh daemon can bind again.

use unshit_ptyd::daemon;
use unshit_ptyd::protocol::Response;

mod common;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shutdown_stops_the_server() {
    let path = common::unique_socket_path();
    let daemon_path = path.clone();
    let server_handle = tokio::spawn(async move {
        daemon::run(&daemon_path).await.unwrap();
    });

    assert_shutdown_ack(common::shutdown_with_retry(&path).await);

    // Daemon task must complete on its own since the accept loop stops
    // when the shutdown signal fires.
    let join = tokio::time::timeout(std::time::Duration::from_secs(5), server_handle)
        .await
        .expect("daemon did not exit within timeout");
    join.expect("daemon task panicked");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fresh_daemon_can_bind_after_shutdown() {
    let path = common::unique_socket_path();

    // First daemon.
    let p1 = path.clone();
    let first = tokio::spawn(async move { daemon::run(&p1).await.unwrap() });
    assert_shutdown_ack(common::shutdown_with_retry(&path).await);
    tokio::time::timeout(std::time::Duration::from_secs(5), first)
        .await
        .expect("first daemon did not exit")
        .unwrap();

    // Second daemon on the same path must start cleanly.
    let p2 = path.clone();
    let second = tokio::spawn(async move { daemon::run(&p2).await.unwrap() });
    assert_shutdown_ack(common::shutdown_with_retry(&path).await);
    tokio::time::timeout(std::time::Duration::from_secs(5), second)
        .await
        .expect("second daemon did not exit")
        .unwrap();
}

fn assert_shutdown_ack(resp: Response) {
    match resp {
        Response::ShutdownAck { ok, .. } => assert!(ok, "shutdown must ack with ok=true"),
        other => panic!("expected ShutdownAck, got {other:?}"),
    }
}
