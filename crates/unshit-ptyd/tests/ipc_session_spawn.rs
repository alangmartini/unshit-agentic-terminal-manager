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
        .spawn_session(80, 24, None, Some(TEST_SHELL.into()), vec![], 0, 0, None)
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

// refs #140: when the client supplies shell_args, those args must reach
// the spawned process. We prove it by spawning a one shot command: on
// Unix `/bin/sh -c "echo MARKER"`, on Windows `cmd.exe /C echo MARKER`.
// If shell_args are forwarded all the way to CommandBuilder, the marker
// appears in the output stream without us writing anything to the
// session's stdin.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn spawn_session_forwards_shell_args_to_spawned_process() {
    const MARKER: &str = "shell-args-marker-140";

    #[cfg(windows)]
    let shell_args = vec!["/C".to_string(), format!("echo {MARKER}")];
    #[cfg(unix)]
    let shell_args = vec!["-c".to_string(), format!("echo {MARKER}")];

    let path = common::unique_socket_path();
    let daemon_path = path.clone();
    let server_handle = tokio::spawn(async move {
        daemon::run(&daemon_path).await.unwrap();
    });

    let (mut client, mut events) = common::connect_with_events_retry(&path).await;
    let resp = client
        .spawn_session(
            80,
            24,
            None,
            Some(TEST_SHELL.into()),
            shell_args,
            0,
            0,
            None,
        )
        .await
        .unwrap();
    let session_id = match resp {
        Response::SessionSpawned { session_id, .. } => session_id,
        other => panic!("expected SessionSpawned, got {other:?}"),
    };

    let collected =
        common::collect_output_for(&mut events, session_id, Duration::from_secs(3)).await;
    let text = String::from_utf8_lossy(&collected);
    assert!(
        text.contains(MARKER),
        "expected shell_args to be forwarded so {MARKER:?} reaches the output stream, \
         got: {text:?}"
    );

    let _ = client.kill_session(session_id).await;
    let _ = client.shutdown().await;
    common::await_daemon(server_handle).await;
}
