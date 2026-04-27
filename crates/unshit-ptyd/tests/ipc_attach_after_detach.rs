//! Detach then reattach: the terminal must retain the grid state written
//! before the detach, and the reattached connection must receive live
//! bytes from subsequent writes.
//!
//! Slice 5 regression: `detach` must not wipe the terminal, and `attach`
//! on the same session must resume streaming without restarting the
//! child.

use std::time::Duration;

use unshit_ptyd::daemon;
use unshit_ptyd::protocol::Response;
use unshit_terminal_core::Snapshot;

mod common;

#[cfg(windows)]
const TEST_SHELL: &str = "cmd.exe";
#[cfg(unix)]
const TEST_SHELL: &str = "/bin/sh";

#[cfg(windows)]
const FIRST_ECHO: &[u8] = b"echo pre-detach-marker\r\n";
#[cfg(unix)]
const FIRST_ECHO: &[u8] = b"echo pre-detach-marker\n";

#[cfg(windows)]
const SECOND_ECHO: &[u8] = b"echo post-attach-marker\r\n";
#[cfg(unix)]
const SECOND_ECHO: &[u8] = b"echo post-attach-marker\n";

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
async fn attach_after_detach_preserves_terminal_and_resumes_streaming() {
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

    client.write(session_id, FIRST_ECHO.to_vec()).await.unwrap();
    let _ = common::collect_output_for(&mut events, session_id, Duration::from_millis(1500)).await;

    let ack = client.detach_session(session_id).await.unwrap();
    assert!(matches!(ack, Response::Ack { .. }));

    let snapshot = client.attach_session(session_id, 0).await.unwrap();
    let rendered = grid_text(&snapshot);
    assert!(
        rendered.contains("pre-detach-marker"),
        "detach must not wipe terminal, got:\n{rendered}"
    );

    client
        .write(session_id, SECOND_ECHO.to_vec())
        .await
        .unwrap();
    let collected =
        common::collect_output_for(&mut events, session_id, Duration::from_secs(2)).await;
    let live_text = String::from_utf8_lossy(&collected);
    assert!(
        live_text.contains("post-attach-marker"),
        "reattach must resume streaming live bytes, got: {live_text:?}"
    );

    client.kill_session(session_id).await.unwrap();
    client.shutdown().await.unwrap();
    common::await_daemon(server_handle).await;
}
