//! Crash isolation: one session dying must not affect any other.
//!
//! Covers slice 6 / F4 of the SPEC: "a shell exiting (exit / process
//! crash) only ends that session. Other sessions and the daemon itself
//! stay alive." The companion unit test
//! `session::tests::run_reader_catches_panic_from_reader` covers F4.2
//! (parser panic containment) at the function level without needing a
//! feature flag on production code. This integration test drives the
//! full daemon plus a live shell per session so the real reader task
//! and registry machinery participate.

use std::time::Duration;

use unshit_ptyd::daemon;
use unshit_ptyd::protocol::Response;

mod common;

#[cfg(windows)]
const TEST_SHELL: &str = "cmd.exe";
#[cfg(unix)]
const TEST_SHELL: &str = "/bin/sh";

#[cfg(windows)]
const EXIT_SELF: &[u8] = b"exit\r\n";
#[cfg(windows)]
const ECHO_SURVIVOR: &[u8] = b"echo survivor-still-responsive\r\n";
#[cfg(unix)]
const EXIT_SELF: &[u8] = b"exit\n";
#[cfg(unix)]
const ECHO_SURVIVOR: &[u8] = b"echo survivor-still-responsive\n";

/// F4.1: one shell exiting (the `exit` builtin, not an external kill)
/// only ends that session. The daemon keeps serving, the surviving
/// session keeps streaming, and `list_sessions` drops the dead one.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shell_exit_in_one_session_does_not_kill_the_other() {
    let path = common::unique_socket_path();
    let daemon_path = path.clone();
    let server_handle = tokio::spawn(async move {
        daemon::run(&daemon_path).await.unwrap();
    });

    let (mut client, mut events) = common::connect_with_events_retry(&path).await;

    let Response::SessionSpawned {
        session_id: victim, ..
    } = client
        .spawn_session(80, 24, None, Some(TEST_SHELL.into()), 0, 0, None)
        .await
        .unwrap()
    else {
        panic!("expected SessionSpawned");
    };
    let Response::SessionSpawned {
        session_id: survivor,
        ..
    } = client
        .spawn_session(80, 24, None, Some(TEST_SHELL.into()), 0, 1, None)
        .await
        .unwrap()
    else {
        panic!("expected SessionSpawned");
    };

    // Let both shells emit their prompts so we're sure the reader tasks
    // are live before we tell one to quit.
    let _ = common::collect_output_for(&mut events, survivor, Duration::from_millis(500)).await;

    client.write(victim, EXIT_SELF.to_vec()).await.unwrap();

    // Give the child process and reader task time to notice EOF and
    // drop the session's alive flag. `cmd.exe` is slower than /bin/sh
    // so we sample for up to two seconds.
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    let mut victim_alive = true;
    while victim_alive && std::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
        let list = client.list_sessions().await.unwrap();
        victim_alive = list
            .iter()
            .find(|s| s.id == victim)
            .map(|s| s.alive)
            .unwrap_or(false);
    }
    assert!(
        !victim_alive,
        "victim session should report alive=false after shell exit"
    );

    // Survivor still registered, still alive, still writable.
    let list = client.list_sessions().await.unwrap();
    let survivor_info = list
        .iter()
        .find(|s| s.id == survivor)
        .expect("survivor must still be listed");
    assert!(
        survivor_info.alive,
        "survivor must still be alive; info: {survivor_info:?}"
    );

    client
        .write(survivor, ECHO_SURVIVOR.to_vec())
        .await
        .unwrap();
    let out = common::collect_output_for(&mut events, survivor, Duration::from_secs(2)).await;
    assert!(
        String::from_utf8_lossy(&out).contains("survivor-still-responsive"),
        "survivor must still echo after the other session's shell exited: {:?}",
        String::from_utf8_lossy(&out)
    );

    client.kill_session(victim).await.ok();
    client.kill_session(survivor).await.unwrap();
    client.shutdown().await.unwrap();
    common::await_daemon(server_handle).await;
}
