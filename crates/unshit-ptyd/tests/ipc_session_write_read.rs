//! Interactive round-trip: spawn a shell, write to it, collect the
//! echoed output via the event stream.

use std::time::Duration;

use unshit_ptyd::daemon;
use unshit_ptyd::protocol::Response;

mod common;

#[cfg(windows)]
const TEST_SHELL: &str = "cmd.exe";
#[cfg(unix)]
const TEST_SHELL: &str = "/bin/sh";

#[cfg(windows)]
const ECHO_CMD: &[u8] = b"echo hello\r\n";
#[cfg(unix)]
const ECHO_CMD: &[u8] = b"echo hello\n";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn write_produces_hello_in_output_stream() {
    let path = common::unique_socket_path();
    let daemon_path = path.clone();
    let server_handle = tokio::spawn(async move {
        daemon::run(&daemon_path).await.unwrap();
    });

    let (mut client, mut events) = common::connect_with_events_retry(&path).await;
    let Response::SessionSpawned { session_id, .. } = client
        .spawn_session(80, 24, None, Some(TEST_SHELL.into()))
        .await
        .unwrap()
    else {
        panic!("expected SessionSpawned");
    };

    client.write(session_id, ECHO_CMD.to_vec()).await.unwrap();

    let collected =
        common::collect_output_for(&mut events, session_id, Duration::from_secs(2)).await;
    let text = String::from_utf8_lossy(&collected);
    assert!(
        text.contains("hello"),
        "expected 'hello' in output: {text:?}"
    );

    // Kill the session and shut down.
    let ack = client.kill_session(session_id).await.unwrap();
    assert!(matches!(ack, Response::Ack { .. }), "got {ack:?}");

    client.shutdown().await.unwrap();
    common::await_daemon(server_handle).await;
}
