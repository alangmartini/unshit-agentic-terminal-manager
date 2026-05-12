//! Shared helpers for the IPC integration tests.
//!
//! Each test gets a unique socket path that encodes pid + process-local
//! counter so parallel runs of `cargo test` never collide.
//!
//! `#[allow(dead_code)]` is applied at the module level because each
//! test file imports this as `mod common;` and only calls a subset of
//! the helpers, which rustc otherwise flags per test crate.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tokio::sync::mpsc;
use unshit_ptyd::client::Client;
use unshit_ptyd::protocol::{Response, ServerEvent};

static COUNTER: AtomicU64 = AtomicU64::new(0);

pub fn unique_socket_path() -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let pid = std::process::id();
    #[cfg(windows)]
    {
        PathBuf::from(format!(r"\\.\pipe\unshit-ptyd-test-{pid}-{n}"))
    }
    #[cfg(unix)]
    {
        std::env::temp_dir().join(format!("unshit-ptyd-test-{pid}-{n}.sock"))
    }
}

/// Connects to the daemon, retrying briefly while it wires up its
/// listener. A spawned daemon can race the connect, so we wait up to a
/// generous window instead of failing on the first `ConnectionRefused`
/// / `FileNotFound`.
pub async fn connect_with_retry(path: &Path) -> Client {
    let deadline = std::time::Instant::now() + Duration::from_millis(2000);
    loop {
        match Client::connect(path).await {
            Ok(c) => return c,
            Err(_) if std::time::Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            Err(e) => panic!("client failed to connect within deadline: {e}"),
        }
    }
}

/// Sends shutdown, retrying the complete connect/request roundtrip.
///
/// On Windows a named-pipe open can briefly succeed against an instance
/// that is being closed during daemon restart. Retrying only `connect`
/// is not enough for shutdown/rebind tests because the first write can
/// still hit `BrokenPipe`.
pub async fn shutdown_with_retry(path: &Path) -> Response {
    let deadline = std::time::Instant::now() + Duration::from_millis(2000);
    loop {
        match Client::connect(path).await {
            Ok(mut client) => match client.shutdown().await {
                Ok(resp) => return resp,
                Err(_) if std::time::Instant::now() < deadline => {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                Err(e) => panic!("client failed to shutdown within deadline: {e}"),
            },
            Err(_) if std::time::Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            Err(e) => panic!("client failed to connect for shutdown within deadline: {e}"),
        }
    }
}

/// Same as [`connect_with_retry`] but also hands back the server-event
/// receiver. Used by session tests that need to observe pushed output.
pub async fn connect_with_events_retry(path: &Path) -> (Client, mpsc::Receiver<ServerEvent>) {
    let deadline = std::time::Instant::now() + Duration::from_millis(2000);
    loop {
        match Client::connect_with_events(path).await {
            Ok(pair) => return pair,
            Err(_) if std::time::Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            Err(e) => panic!("client failed to connect within deadline: {e}"),
        }
    }
}

/// Drains the event stream for up to `timeout`, concatenating every
/// `Output` payload whose `session_id` matches `id`. Ignores other
/// sessions so multi-session tests can share the stream.
pub async fn collect_output_for(
    rx: &mut mpsc::Receiver<ServerEvent>,
    id: u64,
    timeout: Duration,
) -> Vec<u8> {
    let deadline = tokio::time::Instant::now() + timeout;
    let mut out = Vec::new();
    while let Ok(Some(ev)) = tokio::time::timeout_at(deadline, rx.recv()).await {
        match ev {
            ServerEvent::Output { session_id, bytes } if session_id == id => out.extend(bytes),
            _ => continue,
        }
    }
    out
}

/// Waits for the daemon to shut down, after a manual shutdown call, so
/// the OS can release the pipe / socket before the next test reuses it
/// on overlapping counters.
pub async fn await_daemon(handle: tokio::task::JoinHandle<()>) {
    let _ = tokio::time::timeout(Duration::from_secs(5), handle).await;
}
