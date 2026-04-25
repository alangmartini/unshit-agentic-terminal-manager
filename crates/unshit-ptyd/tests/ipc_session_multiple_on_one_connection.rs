//! Two sessions on one connection: writes and kills are isolated.

use std::time::Duration;

use unshit_ptyd::daemon;
use unshit_ptyd::protocol::Response;

mod common;

#[cfg(windows)]
const TEST_SHELL: &str = "cmd.exe";
#[cfg(unix)]
const TEST_SHELL: &str = "/bin/sh";

#[cfg(windows)]
const ECHO_ONE: &[u8] = b"echo first\r\n";
#[cfg(windows)]
const ECHO_TWO: &[u8] = b"echo second\r\n";
#[cfg(unix)]
const ECHO_ONE: &[u8] = b"echo first\n";
#[cfg(unix)]
const ECHO_TWO: &[u8] = b"echo second\n";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_sessions_work_independently_on_one_connection() {
    let path = common::unique_socket_path();
    let daemon_path = path.clone();
    let server_handle = tokio::spawn(async move {
        daemon::run(&daemon_path).await.unwrap();
    });

    let (mut client, mut events) = common::connect_with_events_retry(&path).await;

    let Response::SessionSpawned { session_id: a, .. } = client
        .spawn_session(80, 24, None, Some(TEST_SHELL.into()), 0, 0, None)
        .await
        .unwrap()
    else {
        panic!("expected SessionSpawned");
    };
    let Response::SessionSpawned { session_id: b, .. } = client
        .spawn_session(80, 24, None, Some(TEST_SHELL.into()), 0, 1, None)
        .await
        .unwrap()
    else {
        panic!("expected SessionSpawned");
    };
    assert_ne!(a, b, "two spawns must yield distinct ids");

    client.write(a, ECHO_ONE.to_vec()).await.unwrap();
    client.write(b, ECHO_TWO.to_vec()).await.unwrap();

    // Drain the event stream for a short window and filter by id. The
    // order of the two output streams is unspecified.
    let deadline = Duration::from_secs(2);
    let mut seen_first = Vec::new();
    let mut seen_second = Vec::new();
    let start = tokio::time::Instant::now();
    while start.elapsed() < deadline {
        match tokio::time::timeout(deadline - start.elapsed(), events.recv()).await {
            Ok(Some(unshit_ptyd::protocol::ServerEvent::Output { session_id, bytes })) => {
                if session_id == a {
                    seen_first.extend(bytes);
                } else if session_id == b {
                    seen_second.extend(bytes);
                }
                if String::from_utf8_lossy(&seen_first).contains("first")
                    && String::from_utf8_lossy(&seen_second).contains("second")
                {
                    break;
                }
            }
            _ => break,
        }
    }

    assert!(
        String::from_utf8_lossy(&seen_first).contains("first"),
        "session a output missing 'first': {:?}",
        String::from_utf8_lossy(&seen_first)
    );
    assert!(
        String::from_utf8_lossy(&seen_second).contains("second"),
        "session b output missing 'second': {:?}",
        String::from_utf8_lossy(&seen_second)
    );

    // Kill one; the other must still work.
    client.kill_session(a).await.unwrap();
    let list = client.list_sessions().await.unwrap();
    assert!(
        list.iter().all(|s| s.id != a),
        "killed session should be gone: {list:?}"
    );
    assert!(
        list.iter().any(|s| s.id == b),
        "other session must still be listed: {list:?}"
    );

    // Second session still writable.
    #[cfg(windows)]
    let follow = b"echo still-alive\r\n";
    #[cfg(unix)]
    let follow = b"echo still-alive\n";
    client.write(b, follow.to_vec()).await.unwrap();
    let more = common::collect_output_for(&mut events, b, Duration::from_secs(2)).await;
    assert!(
        String::from_utf8_lossy(&more).contains("still-alive"),
        "surviving session must still echo: {:?}",
        String::from_utf8_lossy(&more)
    );

    client.kill_session(b).await.unwrap();
    client.shutdown().await.unwrap();
    common::await_daemon(server_handle).await;
}
