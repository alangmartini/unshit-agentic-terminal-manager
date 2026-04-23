//! Launcher helpers for the `unshit-ptyd` daemon binary.
//!
//! See `SPEC.md` section 11 slice 3b. This module is pure utility: it
//! locates the daemon binary on disk and spawns it as a detached child
//! so the terminal-manager UI can connect to a running daemon on
//! startup. No UI state; no dependencies on the rest of the UI crate.

use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const DAEMON_BIN_NAME: &str = "unshit-ptyd";
const ENV_OVERRIDE: &str = "UNSHIT_PTYD_BINARY";
const CONNECT_TOTAL_DEADLINE: Duration = Duration::from_secs(3);
const CONNECT_INITIAL_BACKOFF: Duration = Duration::from_millis(25);
const CONNECT_MAX_BACKOFF: Duration = Duration::from_millis(200);

/// Resolves the daemon binary path.
///
/// 1. If `UNSHIT_PTYD_BINARY` env var is set and the path exists, use
///    it (dev / CI override).
/// 2. Otherwise, sibling of the current executable
///    (`std::env::current_exe()`'s parent directory with `unshit-ptyd`
///    plus the platform exe suffix appended). Returned regardless of
///    whether the file exists so tests can distinguish the resolution
///    step from the existence check.
pub fn locate_daemon_binary() -> io::Result<PathBuf> {
    if let Some(path) = env_override() {
        if path.exists() {
            return Ok(path);
        }
    }
    sibling_of_current_exe()
}

fn env_override() -> Option<PathBuf> {
    std::env::var_os(ENV_OVERRIDE).map(PathBuf::from)
}

fn sibling_of_current_exe() -> io::Result<PathBuf> {
    let exe = std::env::current_exe()?;
    let parent = exe.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "current_exe has no parent directory",
        )
    })?;
    let mut candidate = parent.join(DAEMON_BIN_NAME);
    let suffix = std::env::consts::EXE_SUFFIX;
    if !suffix.is_empty() {
        candidate.set_extension(suffix.trim_start_matches('.'));
    }
    Ok(candidate)
}

/// Launches the daemon as a detached child with null stdio.
///
/// On Windows, applies `CREATE_NO_WINDOW | DETACHED_PROCESS` creation
/// flags so the child is not tied to the parent console and no hidden
/// console pops up. On Unix, null stdio is enough; the child inherits
/// the session and survives parent death.
///
/// `socket_path` is forwarded as `--socket <path>` so tests and the
/// production UI can agree on a specific endpoint.
///
/// Returns the spawned `std::process::Child`. Dropping the handle does
/// NOT kill the child; it only relinquishes the parent's ability to
/// reap it.
pub fn spawn_daemon_detached(binary: &Path, socket_path: &Path) -> io::Result<Child> {
    let mut cmd = Command::new(binary);
    cmd.arg("--socket")
        .arg(socket_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    apply_detached_flags(&mut cmd);
    cmd.spawn().map_err(|e| {
        io::Error::new(
            e.kind(),
            format!(
                "failed to spawn daemon binary at {}: {}",
                binary.display(),
                e
            ),
        )
    })
}

#[cfg(windows)]
fn apply_detached_flags(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    cmd.creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS);
}

#[cfg(unix)]
fn apply_detached_flags(_cmd: &mut Command) {}

/// Connect-or-spawn convenience used by `main.rs` on startup.
///
/// 1. Try `unshit_ptyd::client::Client::connect(socket_path).await`.
///    The probe is retried once with a brief sleep so the Windows
///    named-pipe rebind window (a few ms between accepting one client
///    and creating the next pending instance) does not push us onto
///    the spawn path when the daemon is actually running.
/// 2. On failure, locate the binary, call [`spawn_daemon_detached`],
///    then retry connect with exponential backoff up to a bounded
///    deadline (~3 seconds).
/// 3. On connect success, drop the returned `Client` (the probe is the
///    only reason we opened it) and return `Ok(())`.
pub async fn connect_or_spawn(socket_path: &Path) -> io::Result<()> {
    if try_connect(socket_path).await.is_ok() {
        return Ok(());
    }
    tokio::time::sleep(Duration::from_millis(25)).await;
    if try_connect(socket_path).await.is_ok() {
        return Ok(());
    }

    let binary = locate_daemon_binary()?;
    let _child = spawn_daemon_detached(&binary, socket_path)?;

    let deadline = Instant::now() + CONNECT_TOTAL_DEADLINE;
    let mut backoff = CONNECT_INITIAL_BACKOFF;
    let mut last_err: Option<io::Error> = None;
    while Instant::now() < deadline {
        match try_connect(socket_path).await {
            Ok(()) => return Ok(()),
            Err(e) => last_err = Some(e),
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(CONNECT_MAX_BACKOFF);
    }

    let cause = last_err
        .map(|e| e.to_string())
        .unwrap_or_else(|| "timed out".to_string());
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        format!(
            "failed to connect to daemon at {} after spawning {}: {}",
            socket_path.display(),
            binary.display(),
            cause
        ),
    ))
}

async fn try_connect(socket_path: &Path) -> io::Result<()> {
    let client = unshit_ptyd::client::Client::connect(socket_path).await?;
    drop(client);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Serializes tests that mutate UNSHIT_PTYD_BINARY so they do not
    // race within a single test process.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    struct EnvGuard {
        key: &'static str,
        previous: Option<std::ffi::OsString>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &Path) -> Self {
            let previous = std::env::var_os(key);
            set_env(key, Some(value.as_os_str()));
            Self { key, previous }
        }

        fn remove(key: &'static str) -> Self {
            let previous = std::env::var_os(key);
            set_env(key, None);
            Self { key, previous }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match self.previous.take() {
                Some(v) => set_env(self.key, Some(v.as_os_str())),
                None => set_env(self.key, None),
            }
        }
    }

    fn set_env(key: &str, value: Option<&std::ffi::OsStr>) {
        // Edition 2021 keeps these APIs safe; wrapping the call site
        // centralizes the single-threaded invariant enforced above.
        match value {
            Some(v) => std::env::set_var(key, v),
            None => std::env::remove_var(key),
        }
    }

    fn unique_socket_path() -> std::path::PathBuf {
        static C: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = C.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let pid = std::process::id();
        #[cfg(windows)]
        {
            std::path::PathBuf::from(format!(r"\\.\pipe\unshit-ptyd-launcher-{pid}-{n}"))
        }
        #[cfg(unix)]
        {
            std::env::temp_dir().join(format!("unshit-ptyd-launcher-{pid}-{n}.sock"))
        }
    }

    #[test]
    fn locate_daemon_binary_uses_env_override_when_set() {
        let _guard = ENV_LOCK.lock().unwrap();

        let tmp_dir = std::env::temp_dir();
        let fake = tmp_dir.join(format!(
            "unshit-ptyd-locate-fixture-{}-{}{}",
            std::process::id(),
            line!(),
            std::env::consts::EXE_SUFFIX
        ));
        std::fs::write(&fake, b"#!/bin/sh\n").expect("write fixture");

        let _env = EnvGuard::set(ENV_OVERRIDE, &fake);
        let resolved = locate_daemon_binary().expect("locate should succeed");
        assert_eq!(resolved, fake);

        let _ = std::fs::remove_file(&fake);
    }

    #[test]
    fn locate_daemon_binary_returns_sibling_of_current_exe_when_env_absent() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _env = EnvGuard::remove(ENV_OVERRIDE);

        let resolved = locate_daemon_binary().expect("locate should succeed");
        let current = std::env::current_exe().expect("current_exe");
        let expected_parent = current.parent().expect("current_exe has parent");

        assert_eq!(
            resolved.parent(),
            Some(expected_parent),
            "sibling parent must match current_exe parent"
        );
        let file_name = resolved
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();
        assert!(
            file_name.starts_with(DAEMON_BIN_NAME),
            "sibling name must start with {DAEMON_BIN_NAME}: {file_name}"
        );
    }

    // Spawning a real daemon binary from the terminal-manager test
    // harness is only possible when a pre-built `unshit-ptyd` binary is
    // locatable. Cargo does not set `CARGO_BIN_EXE_unshit-ptyd` for
    // tests in sibling workspace packages, so we gate this test rather
    // than silently skip it.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "requires pre-built unshit-ptyd binary; run with `cargo build -p unshit-ptyd && cargo test -p terminal-manager -- --ignored`"]
    #[allow(clippy::await_holding_lock)]
    async fn spawn_daemon_detached_exits_when_asked() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _env = EnvGuard::remove(ENV_OVERRIDE);

        let binary = resolve_built_daemon_binary()
            .expect("unshit-ptyd binary must exist for this test; run cargo build -p unshit-ptyd");

        let socket = unique_socket_path();
        let mut child = spawn_daemon_detached(&binary, &socket).expect("spawn detached");

        let mut client = connect_retry(&socket, Duration::from_secs(3)).await;
        client.shutdown().await.expect("shutdown ack");

        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(_status) = child.try_wait().expect("try_wait") {
                return;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                panic!("daemon did not exit within 5s of shutdown");
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn connect_or_spawn_returns_ok_against_live_daemon() {
        let socket = unique_socket_path();
        let daemon_socket = socket.clone();
        let server = tokio::spawn(async move {
            unshit_ptyd::daemon::run(&daemon_socket).await.unwrap();
        });

        wait_until_listening(&socket, Duration::from_secs(3)).await;

        connect_or_spawn(&socket).await.expect("connect_or_spawn");

        let mut cleanup = connect_retry(&socket, Duration::from_secs(3)).await;
        cleanup.shutdown().await.expect("shutdown ack");
        let _ = tokio::time::timeout(Duration::from_secs(5), server).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[allow(clippy::await_holding_lock)]
    async fn connect_or_spawn_errors_clearly_when_binary_missing() {
        let _guard = ENV_LOCK.lock().unwrap();

        let bogus = std::env::temp_dir().join(format!(
            "unshit-ptyd-does-not-exist-{}-{}{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0),
            std::env::consts::EXE_SUFFIX
        ));
        assert!(!bogus.exists(), "fixture precondition");
        let _env = EnvGuard::set(ENV_OVERRIDE, &bogus);

        let socket = unique_socket_path();
        let err = connect_or_spawn(&socket)
            .await
            .expect_err("must fail when no daemon is reachable and binary cannot spawn");

        let message = err.to_string();
        let resolved = locate_daemon_binary().expect("locate resolves even when sibling missing");
        assert!(
            message.contains(&resolved.display().to_string())
                || message.contains("unshit-ptyd"),
            "error message must reference the daemon binary path so the user can tell the problem is launcher-related: {message}"
        );
    }

    // Helpers scoped to the tests above. Kept inside `mod tests` so
    // they do not leak into the public surface of the crate.

    fn resolve_built_daemon_binary() -> Option<PathBuf> {
        // Cargo may or may not have set CARGO_BIN_EXE_unshit-ptyd for
        // this crate depending on the build layout. If it is present at
        // compile time we prefer it; otherwise we walk up from the test
        // executable looking for `target/debug/unshit-ptyd`.
        if let Some(p) = option_env!("CARGO_BIN_EXE_unshit-ptyd") {
            let path = PathBuf::from(p);
            if path.exists() {
                return Some(path);
            }
        }
        let exe = std::env::current_exe().ok()?;
        let mut dir = exe.parent()?.to_path_buf();
        for _ in 0..4 {
            let mut candidate = dir.join(DAEMON_BIN_NAME);
            let suffix = std::env::consts::EXE_SUFFIX;
            if !suffix.is_empty() {
                candidate.set_extension(suffix.trim_start_matches('.'));
            }
            if candidate.exists() {
                return Some(candidate);
            }
            dir = dir.parent()?.to_path_buf();
        }
        None
    }

    async fn connect_retry(path: &Path, total: Duration) -> unshit_ptyd::client::Client {
        let deadline = Instant::now() + total;
        loop {
            match unshit_ptyd::client::Client::connect(path).await {
                Ok(c) => return c,
                Err(_) if Instant::now() < deadline => {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                Err(e) => panic!("client failed to connect: {e}"),
            }
        }
    }

    async fn wait_until_listening(path: &Path, total: Duration) {
        let deadline = Instant::now() + total;
        loop {
            if let Ok(c) = unshit_ptyd::client::Client::connect(path).await {
                drop(c);
                return;
            }
            if Instant::now() >= deadline {
                panic!("daemon never started listening on {}", path.display());
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}
