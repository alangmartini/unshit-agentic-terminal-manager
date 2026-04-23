//! Shared helpers for the IPC integration tests.
//!
//! Each test gets a unique socket path that encodes pid + process-local
//! counter so parallel runs of `cargo test` never collide.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use unshit_ptyd::client::Client;

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
