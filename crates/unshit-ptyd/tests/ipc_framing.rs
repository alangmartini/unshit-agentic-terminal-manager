//! Framing invariants enforced over a live connection:
//! an oversize length advertisement is rejected and the connection
//! is dropped so the daemon stays healthy for other clients.

use tokio::io::AsyncWriteExt;
use unshit_ptyd::daemon;
use unshit_ptyd::protocol::MAX_FRAME_LEN;
use unshit_ptyd::transport::connect;

mod common;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn oversize_frame_drops_connection_but_daemon_survives() {
    let path = common::unique_socket_path();
    let daemon_path = path.clone();
    let server_handle = tokio::spawn(async move {
        daemon::run(&daemon_path).await.unwrap();
    });

    // Make sure the daemon is listening before we send the garbage.
    let _warmup = common::connect_with_retry(&path).await;
    drop(_warmup);

    // Low-level connection bypassing the Client helper: we intentionally
    // send a header that advertises more than the cap.
    let mut raw = wait_for_raw_connect(&path).await;
    let oversize: u32 = MAX_FRAME_LEN + 1;
    raw.write_all(&oversize.to_le_bytes()).await.unwrap();
    // Give the daemon a chance to read the header and drop us.
    let _ = raw.shutdown().await;
    drop(raw);

    // Daemon must still serve further clients, then shut down normally.
    let mut client = common::connect_with_retry(&path).await;
    let resp = client.hello("framing-test").await.unwrap();
    assert_eq!(resp.id(), 1, "hello must still work after garbage client");

    client.shutdown().await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), server_handle)
        .await
        .expect("daemon did not exit")
        .unwrap();
}

#[cfg(windows)]
async fn wait_for_raw_connect(
    path: &std::path::Path,
) -> tokio::net::windows::named_pipe::NamedPipeClient {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(2000);
    loop {
        match connect(path).await {
            Ok(c) => return c,
            Err(_) if std::time::Instant::now() < deadline => {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            Err(e) => panic!("raw connect failed: {e}"),
        }
    }
}

#[cfg(unix)]
async fn wait_for_raw_connect(path: &std::path::Path) -> tokio::net::UnixStream {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(2000);
    loop {
        match connect(path).await {
            Ok(c) => return c,
            Err(_) if std::time::Instant::now() < deadline => {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            Err(e) => panic!("raw connect failed: {e}"),
        }
    }
}
