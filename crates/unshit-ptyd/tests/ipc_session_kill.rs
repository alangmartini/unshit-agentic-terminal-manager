//! Killing a session removes it from `list_sessions`.

use unshit_ptyd::daemon;
use unshit_ptyd::protocol::Response;

mod common;

#[cfg(windows)]
const TEST_SHELL: &str = "cmd.exe";
#[cfg(unix)]
const TEST_SHELL: &str = "/bin/sh";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn kill_session_removes_it_from_list() {
    let path = common::unique_socket_path();
    let daemon_path = path.clone();
    let server_handle = tokio::spawn(async move {
        daemon::run(&daemon_path).await.unwrap();
    });

    let (mut client, _events) = common::connect_with_events_retry(&path).await;
    let Response::SessionSpawned { session_id, .. } = client
        .spawn_session(80, 24, None, Some(TEST_SHELL.into()), 0, 0, None)
        .await
        .unwrap()
    else {
        panic!("expected SessionSpawned");
    };

    // Before kill: list shows the session.
    let before = client.list_sessions().await.unwrap();
    assert!(
        before.iter().any(|s| s.id == session_id),
        "session must appear in list before kill: {before:?}"
    );

    let ack = client.kill_session(session_id).await.unwrap();
    assert!(matches!(ack, Response::Ack { .. }), "got {ack:?}");

    let after = client.list_sessions().await.unwrap();
    assert!(
        after.iter().all(|s| s.id != session_id),
        "session must NOT appear in list after kill: {after:?}"
    );

    client.shutdown().await.unwrap();
    common::await_daemon(server_handle).await;
}
