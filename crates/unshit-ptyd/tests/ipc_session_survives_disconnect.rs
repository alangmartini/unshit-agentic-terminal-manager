//! A session spawned on one connection SURVIVES that connection's
//! disconnect. A fresh client still sees the session in `list_sessions`.
//!
//! Slice 5 of the tmux-style persistence rollout: sessions only die on
//! explicit `KillSession` or daemon shutdown, never on client disconnect.

use unshit_ptyd::daemon;
use unshit_ptyd::protocol::Response;

mod common;

#[cfg(windows)]
const TEST_SHELL: &str = "cmd.exe";
#[cfg(windows)]
const LONG_RUNNING: &[u8] = b"ping -n 60 127.0.0.1 >nul\r\n";

#[cfg(unix)]
const TEST_SHELL: &str = "/bin/sh";
#[cfg(unix)]
const LONG_RUNNING: &[u8] = b"sleep 60\n";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn session_from_disconnected_client_stays_alive() {
    let path = common::unique_socket_path();
    let daemon_path = path.clone();
    let server_handle = tokio::spawn(async move {
        daemon::run(&daemon_path).await.unwrap();
    });

    let spawned_id;
    {
        let (mut client, _events) = common::connect_with_events_retry(&path).await;
        let Response::SessionSpawned { session_id, .. } = client
            .spawn_session(80, 24, None, Some(TEST_SHELL.into()), vec![], 0, 0, None)
            .await
            .unwrap()
        else {
            panic!("expected SessionSpawned");
        };
        spawned_id = session_id;
        client
            .write(session_id, LONG_RUNNING.to_vec())
            .await
            .unwrap();
    }

    // Poll on a fresh client: the session must still be alive for the
    // full window. Failing early if it disappears before the deadline.
    let (mut follow_up, _ev) = common::connect_with_events_retry(&path).await;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        let list = follow_up.list_sessions().await.unwrap();
        if !list.iter().any(|s| s.id == spawned_id) {
            panic!("session {spawned_id} must survive client disconnect: {list:?}");
        }
        if std::time::Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    // Explicit kill drains the survivor so the daemon can shut down
    // cleanly (shutdown is refused while sessions are alive).
    follow_up.kill_session(spawned_id).await.unwrap();
    follow_up.shutdown().await.unwrap();
    common::await_daemon(server_handle).await;
}
