//! Client spawns a session, receives the echoed output via a
//! `ServerEvent::Output`, then kills the session explicitly and
//! confirms it is gone. Slice 5 moves session-lifetime ownership off
//! the connection, so cleanup now requires an explicit kill.

use std::time::Duration;

use unshit_ptyd::daemon;
use unshit_ptyd::protocol::Response;

mod common;

#[cfg(windows)]
const TEST_SHELL: &str = "cmd.exe";
#[cfg(unix)]
const TEST_SHELL: &str = "/bin/sh";

#[cfg(windows)]
const ECHO_PAYLOAD: &[u8] = b"echo session-spawn-hi\r\n";
#[cfg(unix)]
const ECHO_PAYLOAD: &[u8] = b"echo session-spawn-hi\n";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn spawn_session_yields_output_event_containing_payload() {
    let path = common::unique_socket_path();
    let daemon_path = path.clone();
    let server_handle = tokio::spawn(async move {
        daemon::run(&daemon_path).await.unwrap();
    });

    let (mut client, mut events) = common::connect_with_events_retry(&path).await;
    let resp = client
        .spawn_session(80, 24, None, Some(TEST_SHELL.into()), 0, 0, None)
        .await
        .unwrap();
    let session_id = match resp {
        Response::SessionSpawned { session_id, .. } => session_id,
        other => panic!("expected SessionSpawned, got {other:?}"),
    };
    assert!(session_id >= 1, "ids start at 1: got {session_id}");

    // Drive the shell: ask it to echo a distinctive string.
    client
        .write(session_id, ECHO_PAYLOAD.to_vec())
        .await
        .unwrap();

    let collected =
        common::collect_output_for(&mut events, session_id, Duration::from_secs(2)).await;
    let text = String::from_utf8_lossy(&collected);
    assert!(
        text.contains("session-spawn-hi"),
        "expected echo output to contain marker, got: {text:?}"
    );

    // Explicit kill drops the session from the shared registry.
    client.kill_session(session_id).await.unwrap();
    drop(client);
    drop(events);

    let (mut follow_up, _ev) = common::connect_with_events_retry(&path).await;
    let list = follow_up.list_sessions().await.unwrap();
    assert!(
        list.is_empty(),
        "killed session must not appear on follow-up list, got {list:?}"
    );

    follow_up.shutdown().await.unwrap();
    common::await_daemon(server_handle).await;
}
