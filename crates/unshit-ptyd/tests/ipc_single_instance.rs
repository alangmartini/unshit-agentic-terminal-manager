//! Single-instance guard: a second daemon on the same path must fail.

use std::io;

use unshit_ptyd::daemon;

mod common;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn second_daemon_on_same_path_errors_cleanly() {
    let path = common::unique_socket_path();

    let p1 = path.clone();
    let first = tokio::spawn(async move { daemon::run(&p1).await });

    // Give the first daemon a moment to establish its listener. On
    // Windows we also need the pipe path to be created before the
    // second bind can detect it. Connect-with-retry is an easy probe.
    let _client = common::connect_with_retry(&path).await;

    // Second attempt must fail with AlreadyExists.
    let err = daemon::run(&path).await.unwrap_err();
    assert_eq!(
        err.kind(),
        io::ErrorKind::AlreadyExists,
        "second daemon should surface AlreadyExists: {err:?}"
    );

    // Clean up the first daemon.
    let mut cleanup_client = common::connect_with_retry(&path).await;
    cleanup_client.shutdown().await.unwrap();
    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), first)
        .await
        .expect("first daemon did not exit")
        .unwrap();
}
