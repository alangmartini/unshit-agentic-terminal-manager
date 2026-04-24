//! `rename_session` sets and clears the display name on the daemon.

use unshit_ptyd::daemon;
use unshit_ptyd::protocol::Response;

mod common;

#[cfg(windows)]
const TEST_SHELL: &str = "cmd.exe";
#[cfg(unix)]
const TEST_SHELL: &str = "/bin/sh";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rename_session_updates_list_output() {
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

    // Fresh session has no name.
    let before = client.list_sessions().await.unwrap();
    let entry = before
        .iter()
        .find(|s| s.id == session_id)
        .expect("session in list");
    assert_eq!(entry.name, None);

    client
        .rename_session(session_id, Some("my build server".to_string()))
        .await
        .unwrap();

    let named = client.list_sessions().await.unwrap();
    let entry = named
        .iter()
        .find(|s| s.id == session_id)
        .expect("session in list after rename");
    assert_eq!(entry.name.as_deref(), Some("my build server"));

    // Clearing via empty string behaves like None.
    client
        .rename_session(session_id, Some(String::new()))
        .await
        .unwrap();

    let cleared = client.list_sessions().await.unwrap();
    let entry = cleared
        .iter()
        .find(|s| s.id == session_id)
        .expect("session in list after clear");
    assert_eq!(entry.name, None);

    client.kill_session(session_id).await.unwrap();
    client.shutdown().await.unwrap();
    common::await_daemon(server_handle).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rename_session_unknown_id_errors() {
    let path = common::unique_socket_path();
    let daemon_path = path.clone();
    let server_handle = tokio::spawn(async move {
        daemon::run(&daemon_path).await.unwrap();
    });

    let (mut client, _events) = common::connect_with_events_retry(&path).await;

    let err = client
        .rename_session(9999, Some("nope".to_string()))
        .await
        .expect_err("rename on unknown id must fail");
    let msg = err.to_string();
    assert!(
        msg.contains("rename_session failed"),
        "expected rename error message, got: {msg}"
    );

    client.shutdown().await.unwrap();
    common::await_daemon(server_handle).await;
}
