//! Updates may disconnect the UI, but must keep the daemon and shells alive.
use std::time::Duration;
use unshit_ptyd::{compatibility, daemon, protocol::Response};

mod common;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn compatibility_and_retirement_preserve_live_shell_across_disconnect() {
    let path = common::unique_socket_path();
    let server_path = path.clone();
    let server = tokio::spawn(async move { daemon::run(&server_path).await.unwrap() });
    let mut client = common::connect_with_retry(&path).await;
    #[cfg(windows)]
    let shell = "cmd.exe";
    #[cfg(unix)]
    let shell = "/bin/sh";
    let Response::SessionSpawned { session_id, .. } = client
        .spawn_session(80, 24, None, Some(shell.into()), vec![], 1, 1, None)
        .await
        .unwrap()
    else {
        panic!("spawn response")
    };
    let before = client.list_sessions().await.unwrap();
    compatibility::check_running_daemon(&path).await.unwrap();
    assert!(matches!(
        client.retire_if_idle().await.unwrap(),
        Response::ShutdownAck { ok: false, .. }
    ));
    drop(client);

    let (mut reattached, mut events) = common::connect_with_events_retry(&path).await;
    let after = reattached.list_sessions().await.unwrap();
    assert_eq!(after[0].id, before[0].id);
    assert_eq!(after[0].pid, before[0].pid);
    assert!(after[0].alive);
    #[cfg(windows)]
    let command = b"echo update-^survived\r\n".to_vec();
    #[cfg(unix)]
    let command = b"echo update-'survived'\n".to_vec();
    reattached.attach_session(session_id, 0).await.unwrap();
    reattached.write(session_id, command).await.unwrap();
    let output = common::collect_output_for(&mut events, session_id, Duration::from_secs(2)).await;
    let text = String::from_utf8_lossy(&output);
    reattached.kill_session(session_id).await.unwrap();
    assert!(matches!(
        reattached.retire_if_idle().await.unwrap(),
        Response::ShutdownAck { ok: true, .. }
    ));
    common::await_daemon(server).await;
    assert!(
        text.contains("update-survived"),
        "shell did not execute command: {text:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retirement_waits_for_other_clients_to_disconnect() {
    let path = common::unique_socket_path();
    let server_path = path.clone();
    let server = tokio::spawn(async move { daemon::run(&server_path).await.unwrap() });
    let mut ui = common::connect_with_retry(&path).await;
    ui.hello("old-ui").await.unwrap();
    let mut updater = common::connect_with_retry(&path).await;
    assert!(matches!(
        updater.retire_if_idle().await.unwrap(),
        Response::ShutdownAck { ok: false, .. }
    ));
    ui.hello("still-connected").await.unwrap();
    drop(ui);
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if matches!(
                updater.retire_if_idle().await.unwrap(),
                Response::ShutdownAck { ok: true, .. }
            ) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    common::await_daemon(server).await;
}

#[tokio::test]
async fn compatibility_accepts_absent_daemon_without_starting_one() {
    let path = common::unique_socket_path();
    compatibility::check_running_daemon(&path).await.unwrap();
    assert!(unshit_ptyd::client::Client::connect(&path).await.is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn installer_preflight_accepts_old_protocols_and_rejects_unknown_ones() {
    use unshit_ptyd::protocol::{read_request, write_response};
    for version in [0, 1, 2, 3, 4] {
        let path = common::unique_socket_path();
        #[cfg(windows)]
        let mut listener = unshit_ptyd::transport::Server::bind(&path).unwrap();
        #[cfg(unix)]
        let mut listener = unshit_ptyd::transport::Server::bind(&path).await.unwrap();
        let server = tokio::spawn(async move {
            let mut conn = listener.accept().await.unwrap();
            let hello = read_request(&mut conn).await.unwrap().unwrap();
            write_response(
                &mut conn,
                &Response::HelloAck {
                    id: hello.id(),
                    server_version: "fixture".into(),
                    protocol_version: version,
                    executable: None,
                },
            )
            .await
            .unwrap();
            // A preflight must never send Shutdown, Spawn or any mutation.
            assert!(read_request(&mut conn).await.unwrap().is_none());
        });
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_unshit-ptyd"))
            .arg("--check-compatible")
            .arg("--socket")
            .arg(&path)
            .output()
            .unwrap();
        assert_eq!(
            output.status.success(),
            (1..=3).contains(&version),
            "{output:?}"
        );
        server.await.unwrap();
    }
}
