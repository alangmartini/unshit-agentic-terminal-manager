//! A session spawned on one connection does NOT survive that
//! connection's disconnect. A fresh client sees an empty session list.

use unshit_ptyd::daemon;
use unshit_ptyd::protocol::Response;

mod common;

#[cfg(windows)]
const TEST_SHELL: &str = "cmd.exe";
// On Windows, ping with -n 60 keeps the child alive long enough for
// this test without relying on sleep semantics that differ across
// shells.
#[cfg(windows)]
const LONG_RUNNING: &[u8] = b"ping -n 60 127.0.0.1 >nul\r\n";

#[cfg(unix)]
const TEST_SHELL: &str = "/bin/sh";
#[cfg(unix)]
const LONG_RUNNING: &[u8] = b"sleep 60\n";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn session_from_disconnected_client_is_reaped() {
    let path = common::unique_socket_path();
    let daemon_path = path.clone();
    let server_handle = tokio::spawn(async move {
        daemon::run(&daemon_path).await.unwrap();
    });

    // First client: spawn a long-running session, then drop the client.
    {
        let (mut client, _events) = common::connect_with_events_retry(&path).await;
        let Response::SessionSpawned { session_id, .. } = client
            .spawn_session(80, 24, None, Some(TEST_SHELL.into()))
            .await
            .unwrap()
        else {
            panic!("expected SessionSpawned");
        };
        // Start a long-running command; we do not need output.
        client
            .write(session_id, LONG_RUNNING.to_vec())
            .await
            .unwrap();
        // Dropping the client closes its transport, which should also
        // kill every session spawned on that connection.
    }

    // Give the daemon a moment to observe the disconnect and reap.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let (mut follow_up, _ev) = common::connect_with_events_retry(&path).await;
    let list = follow_up.list_sessions().await.unwrap();
    assert!(
        list.is_empty(),
        "session from prior connection must have been reaped: {list:?}"
    );

    follow_up.shutdown().await.unwrap();
    common::await_daemon(server_handle).await;
}
