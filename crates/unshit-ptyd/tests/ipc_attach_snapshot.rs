//! Attach / detach RPCs round-trip a `Snapshot` through the daemon.
//!
//! Covers slice 4c of the tmux-style persistence plan (see SPEC.md
//! section 4): `attach_session` must return the authoritative grid +
//! scrollback the daemon has parsed so far; `detach_session` is a
//! no-op ack in slice 4; unknown session ids surface as a protocol
//! error; and the scrollback cap (`SNAPSHOT_MAX_SCROLLBACK_LINES`) is
//! enforced silently on the server side.

use std::time::Duration;

use unshit_ptyd::daemon;
use unshit_ptyd::protocol::{ProtocolError, Response, SNAPSHOT_MAX_SCROLLBACK_LINES};
use unshit_terminal_core::Snapshot;

mod common;

#[cfg(windows)]
const TEST_SHELL: &str = "cmd.exe";
#[cfg(unix)]
const TEST_SHELL: &str = "/bin/sh";

#[cfg(windows)]
const ECHO_CMD: &[u8] = b"echo attach-marker-zz\r\n";
#[cfg(unix)]
const ECHO_CMD: &[u8] = b"echo attach-marker-zz\n";

fn grid_text(snap: &Snapshot) -> String {
    let grid = &snap.grid;
    let mut s = String::new();
    for r in 0..grid.rows() {
        if let Some(row) = grid.row(r) {
            for cell in row {
                s.push(cell.ch);
            }
            s.push('\n');
        }
    }
    s
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn attach_after_write_returns_snapshot_with_recent_output() {
    let path = common::unique_socket_path();
    let daemon_path = path.clone();
    let server_handle = tokio::spawn(async move {
        daemon::run(&daemon_path).await.unwrap();
    });

    let (mut client, mut events) = common::connect_with_events_retry(&path).await;
    let Response::SessionSpawned { session_id, .. } = client
        .spawn_session(80, 24, None, Some(TEST_SHELL.into()), vec![], 0, 0, None)
        .await
        .unwrap()
    else {
        panic!("expected SessionSpawned");
    };

    client.write(session_id, ECHO_CMD.to_vec()).await.unwrap();

    // Drain the event stream so the daemon-side terminal has time to
    // parse every chunk before we snapshot.
    let _ = common::collect_output_for(&mut events, session_id, Duration::from_millis(1500)).await;

    let snapshot = client.attach_session(session_id, 0).await.unwrap();
    assert_eq!(snapshot.grid.rows(), 24);
    assert_eq!(snapshot.grid.cols(), 80);

    let rendered = grid_text(&snapshot);
    assert!(
        rendered.contains("attach-marker-zz"),
        "expected echo marker in snapshot grid, got:\n{rendered}"
    );

    client.shutdown().await.unwrap();
    common::await_daemon(server_handle).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn attach_unknown_session_returns_error_response() {
    let path = common::unique_socket_path();
    let daemon_path = path.clone();
    let server_handle = tokio::spawn(async move {
        daemon::run(&daemon_path).await.unwrap();
    });

    let (mut client, _events) = common::connect_with_events_retry(&path).await;
    let err = client.attach_session(9999, 0).await.unwrap_err();
    match err {
        ProtocolError::Io(e) => {
            let msg = e.to_string();
            assert!(
                msg.contains("session_not_found"),
                "expected session_not_found code in message, got: {msg}"
            );
        }
        other => panic!("unexpected error variant: {other:?}"),
    }

    client.shutdown().await.unwrap();
    common::await_daemon(server_handle).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn detach_session_acks_on_live_session() {
    let path = common::unique_socket_path();
    let daemon_path = path.clone();
    let server_handle = tokio::spawn(async move {
        daemon::run(&daemon_path).await.unwrap();
    });

    let (mut client, _events) = common::connect_with_events_retry(&path).await;
    let Response::SessionSpawned { session_id, .. } = client
        .spawn_session(80, 24, None, Some(TEST_SHELL.into()), vec![], 0, 0, None)
        .await
        .unwrap()
    else {
        panic!("expected SessionSpawned");
    };

    let ack = client.detach_session(session_id).await.unwrap();
    assert!(
        matches!(ack, Response::Ack { .. }),
        "expected plain Ack, got {ack:?}"
    );

    // Detach is a no-op in slice 4: the session must still exist.
    let list = client.list_sessions().await.unwrap();
    assert!(
        list.iter().any(|s| s.id == session_id),
        "detach must not remove the session: {list:?}"
    );

    client.kill_session(session_id).await.unwrap();
    client.shutdown().await.unwrap();
    common::await_daemon(server_handle).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn attach_clamps_scrollback_lines_at_cap() {
    let path = common::unique_socket_path();
    let daemon_path = path.clone();
    let server_handle = tokio::spawn(async move {
        daemon::run(&daemon_path).await.unwrap();
    });

    let (mut client, mut events) = common::connect_with_events_retry(&path).await;
    let Response::SessionSpawned { session_id, .. } = client
        .spawn_session(80, 24, None, Some(TEST_SHELL.into()), vec![], 0, 0, None)
        .await
        .unwrap()
    else {
        panic!("expected SessionSpawned");
    };

    // Push plenty of lines directly through the daemon's terminal
    // parser. Avoids shell quirks by writing the bytes in one chunk.
    let big: Vec<u8> = (0..(SNAPSHOT_MAX_SCROLLBACK_LINES + 200))
        .map(|_| b'\n')
        .collect();
    client.write(session_id, big).await.unwrap();

    let _ = common::collect_output_for(&mut events, session_id, Duration::from_millis(500)).await;

    let snapshot = client
        .attach_session(session_id, 10_000)
        .await
        .expect("snapshot");
    assert!(
        snapshot.scrollback.len() <= SNAPSHOT_MAX_SCROLLBACK_LINES,
        "scrollback len {} exceeds cap {}",
        snapshot.scrollback.len(),
        SNAPSHOT_MAX_SCROLLBACK_LINES
    );

    client.kill_session(session_id).await.unwrap();
    client.shutdown().await.unwrap();
    common::await_daemon(server_handle).await;
}
