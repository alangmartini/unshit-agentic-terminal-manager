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
const DAEMON_PACKAGE_NAME: &str = "unshit-ptyd";
const ENV_OVERRIDE: &str = "UNSHIT_PTYD_BINARY";
const CONNECT_TOTAL_DEADLINE: Duration = Duration::from_secs(3);
/// First pause after spawning the daemon. A freshly spawned `unshit-ptyd`
/// publishes its endpoint in single-digit milliseconds, so the old 25ms floor
/// was almost entirely dead time on the cold-start path: the daemon was ready
/// and the UI was still asleep. Backoff grows geometrically, so an unhealthy
/// daemon still costs only a handful of probes before the deadline.
const CONNECT_INITIAL_BACKOFF: Duration = Duration::from_micros(500);
const CONNECT_MAX_BACKOFF: Duration = Duration::from_millis(50);
/// Pause before re-probing an endpoint that exists but refused the connection.
/// This is the Windows named-pipe rebind window: the daemon has accepted one
/// client and has not yet created the next pending instance.
const REBIND_RETRY_PAUSE: Duration = Duration::from_millis(5);
const REBIND_RETRY_ATTEMPTS: u32 = 8;
const CARGO_BUILD_OUTPUT_LIMIT: usize = 4096;

/// Resolves the daemon binary path.
///
/// 1. If `UNSHIT_PTYD_BINARY` env var is set and the path exists, use
///    it (dev / CI override).
/// 2. The immutable `daemons/<UI version>/` install, when present.
/// 3. Otherwise, sibling of the current executable
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
    bundled_daemon_binary()
}

fn ensure_daemon_binary() -> io::Result<PathBuf> {
    if let Some(path) = env_override() {
        if path.exists() {
            return Ok(path);
        }
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "{ENV_OVERRIDE} points to a missing daemon binary at {}",
                path.display()
            ),
        ));
    }

    let binary = bundled_daemon_binary()?;
    if binary.exists() {
        return Ok(binary);
    }

    maybe_build_daemon_from_workspace(&binary)?;
    Ok(binary)
}

fn env_override() -> Option<PathBuf> {
    std::env::var_os(ENV_OVERRIDE).map(PathBuf::from)
}

fn bundled_daemon_binary() -> io::Result<PathBuf> {
    let sibling = sibling_of_current_exe()?;
    Ok(bundled_daemon_at(&sibling, env!("CARGO_PKG_VERSION")))
}

fn bundled_daemon_at(sibling: &Path, version: &str) -> PathBuf {
    let versioned = sibling
        .parent()
        .unwrap_or(Path::new("."))
        .join("daemons")
        .join(version)
        .join(sibling.file_name().unwrap_or_default());
    if versioned.is_file() {
        versioned
    } else {
        sibling.to_path_buf()
    }
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

fn maybe_build_daemon_from_workspace(expected_binary: &Path) -> io::Result<()> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    if !manifest_dir.join("Cargo.toml").is_file()
        || !manifest_dir
            .join("crates")
            .join(DAEMON_PACKAGE_NAME)
            .join("Cargo.toml")
            .is_file()
    {
        return Ok(());
    }

    let Some(exe_dir) = expected_binary.parent() else {
        return Ok(());
    };

    let profile = exe_dir
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("debug");
    let target_dir = exe_dir.parent();

    log::info!(
        "daemon binary missing at {}; building {DAEMON_PACKAGE_NAME}",
        expected_binary.display()
    );

    let mut cmd = Command::new(cargo_command());
    cmd.current_dir(&manifest_dir)
        .arg("build")
        .arg("-p")
        .arg(DAEMON_PACKAGE_NAME)
        .arg("--bin")
        .arg(DAEMON_BIN_NAME);

    if profile == "release" {
        cmd.arg("--release");
    } else if profile != "debug" {
        cmd.arg("--profile").arg(profile);
    }

    if let Some(target_dir) = target_dir {
        cmd.arg("--target-dir").arg(target_dir);
    }

    let output = cmd.output().map_err(|e| {
        io::Error::new(
            e.kind(),
            format!("failed to run cargo build for {DAEMON_PACKAGE_NAME}: {e}"),
        )
    })?;

    if !output.status.success() {
        return Err(io::Error::other(format!(
            "failed to build {DAEMON_PACKAGE_NAME} with cargo (status {}):{}{}",
            output.status,
            format_command_output("stdout", &output.stdout),
            format_command_output("stderr", &output.stderr),
        )));
    }

    if !expected_binary.exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "cargo build completed but daemon binary is still missing at {}",
                expected_binary.display()
            ),
        ));
    }

    Ok(())
}

fn cargo_command() -> PathBuf {
    std::env::var_os("CARGO")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("cargo"))
}

fn format_command_output(label: &str, bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let text = text.trim();
    if text.is_empty() {
        return String::new();
    }
    let mut truncated = text
        .chars()
        .take(CARGO_BUILD_OUTPUT_LIMIT)
        .collect::<String>();
    if text.chars().count() > CARGO_BUILD_OUTPUT_LIMIT {
        truncated.push_str("\n...");
    }
    format!("\n{label}:\n{truncated}")
}

/// Launches the daemon as a detached child with null stdio.
///
/// On Windows, applies `CREATE_NO_WINDOW | DETACHED_PROCESS` creation
/// flags so the child is not tied to the parent console and no hidden
/// console pops up. On Unix, the daemon gets its own process group as well as
/// null stdio. That lets it survive the UI process exiting and prevents an
/// interactive terminal's Ctrl+C from reaching the daemon with the UI.
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
fn apply_detached_flags(cmd: &mut Command) {
    use std::os::unix::process::CommandExt;

    // Put the daemon in a fresh process group before exec. The UI deliberately
    // drops the Child handle after the connect probe; the daemon owns sessions
    // independently and must not share the launcher's terminal signal group.
    cmd.process_group(0);
}

/// Outcome of [`connect_or_spawn`], so startup telemetry can separate a launch
/// that attached to a live daemon from one that had to pay process-spawn cost.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DaemonStartup {
    /// A daemon was already listening; no process was created.
    Attached,
    /// No daemon was listening, so one was spawned and waited for.
    Spawned,
}

/// Connect-or-spawn convenience used by `main.rs` on startup.
///
/// 1. Connect and check protocol compatibility. For a newer bundled release,
///    request retirement only when the daemon supports the atomic v3 gate.
/// 2. If that failed *because the endpoint exists but refused us*, retry
///    briefly: that is the Windows named-pipe rebind window (a few ms between
///    accepting one client and creating the next pending instance), and
///    spawning a second daemon over a healthy one would be wrong.
///    A "no such endpoint" error means no daemon is running and is not retried
///    — on the cold-start path every millisecond spent re-probing an endpoint
///    that provably does not exist is pure latency.
/// 3. For an absent or retired daemon, call [`spawn_daemon_detached`], then retry
///    connect with exponential backoff up to a bounded deadline (~3 seconds).
/// 4. On connect success, drop the returned `Client` (the probe is the only
///    reason we opened it).
pub async fn connect_or_spawn(socket_path: &Path) -> io::Result<DaemonStartup> {
    for attempt in 0..=REBIND_RETRY_ATTEMPTS {
        match probe_daemon(socket_path, true).await {
            Ok(true) => return Ok(DaemonStartup::Attached),
            Ok(false) => break, // Retirement accepted and endpoint released.
            Err(e) if e.kind() == io::ErrorKind::NotFound => break,
            Err(e) if retryable_startup_error(&e) && attempt < REBIND_RETRY_ATTEMPTS => {
                tokio::time::sleep(REBIND_RETRY_PAUSE).await;
            }
            // A crashed Unix daemon may leave its socket on disk. The server
            // bind path checks that it is stale before replacing it.
            #[cfg(unix)]
            Err(e) if e.kind() == io::ErrorKind::ConnectionRefused => break,
            Err(e) => return Err(e),
        }
    }

    let binary = ensure_daemon_binary()?;
    let _child = spawn_daemon_detached(&binary, socket_path)?;

    let deadline = Instant::now() + CONNECT_TOTAL_DEADLINE;
    let mut backoff = CONNECT_INITIAL_BACKOFF;
    let mut last_err: Option<io::Error> = None;
    while Instant::now() < deadline {
        match probe_daemon(socket_path, false).await {
            Ok(_) => return Ok(DaemonStartup::Spawned),
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

async fn probe_daemon(socket_path: &Path, allow_upgrade: bool) -> io::Result<bool> {
    use unshit_ptyd::protocol::{Response, RETIRE_PROTOCOL_VERSION};
    tokio::time::timeout(CONNECT_TOTAL_DEADLINE, async {
        let mut client = unshit_ptyd::client::Client::connect(socket_path).await?;
        let Response::HelloAck {
            protocol_version,
            executable,
            ..
        } = client
            .hello(env!("CARGO_PKG_VERSION"))
            .await
            .map_err(protocol_io_error)?
        else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "unexpected daemon greeting",
            ));
        };
        unshit_ptyd::compatibility::check_protocol(protocol_version)?;
        if allow_upgrade && protocol_version >= RETIRE_PROTOCOL_VERSION && env_override().is_none()
        {
            let bundled = bundled_daemon_binary()?;
            if executable
                .as_deref()
                .is_some_and(|running| upgrade_pending(Path::new(running), &bundled))
            {
                match client.retire_if_idle().await.map_err(protocol_io_error)? {
                    Response::ShutdownAck { ok: true, .. } => {
                        drop(client);
                        // Ack precedes listener teardown. Do not mistake the
                        // retiring listener for the newly installed daemon.
                        loop {
                            match unshit_ptyd::client::Client::connect(socket_path).await {
                                Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
                                Err(e) if !retryable_startup_error(&e) => return Err(e),
                                Ok(mut successor) => {
                                    // Another launcher can publish the new daemon
                                    // before this probe observes a missing pipe.
                                    if let Ok(Response::HelloAck {
                                        protocol_version,
                                        executable: successor_exe,
                                        ..
                                    }) = successor.hello(env!("CARGO_PKG_VERSION")).await
                                    {
                                        if successor_exe != executable {
                                            unshit_ptyd::compatibility::check_protocol(
                                                protocol_version,
                                            )?;
                                            return Ok(true);
                                        }
                                    }
                                }
                                Err(_) => {}
                            }
                            tokio::time::sleep(REBIND_RETRY_PAUSE).await;
                        }
                    }
                    Response::ShutdownAck { ok: false, .. } => {}
                    _ => {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "unexpected daemon retirement response",
                        ))
                    }
                }
            }
        }
        Ok(true)
    })
    .await
    .map_err(|_| {
        io::Error::new(
            io::ErrorKind::TimedOut,
            "daemon handshake or retirement timed out",
        )
    })?
}

fn protocol_io_error(error: unshit_ptyd::protocol::ProtocolError) -> io::Error {
    match error {
        unshit_ptyd::protocol::ProtocolError::Io(error) => error,
        error => io::Error::new(io::ErrorKind::InvalidData, error),
    }
}

/// Bridge startup follows a short-lived launcher probe. Another UI can retire
/// an idle daemon between those connections, so retry the entire read-only
/// handshake while the successor takes over. No session request is retried.
pub async fn connect_ui_client(
    socket_path: &Path,
) -> io::Result<(
    unshit_ptyd::client::Client,
    tokio::sync::mpsc::Receiver<unshit_ptyd::protocol::ServerEvent>,
    u32,
)> {
    use unshit_ptyd::{client::Client, protocol::Response};
    tokio::time::timeout(CONNECT_TOTAL_DEADLINE, async {
        loop {
            let result = async {
                let (mut client, events) = Client::connect_with_events(socket_path).await?;
                match client
                    .hello(env!("CARGO_PKG_VERSION"))
                    .await
                    .map_err(protocol_io_error)?
                {
                    Response::HelloAck {
                        protocol_version, ..
                    } => {
                        unshit_ptyd::compatibility::check_protocol(protocol_version)?;
                        Ok((client, events, protocol_version))
                    }
                    _ => Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "unexpected daemon greeting",
                    )),
                }
            }
            .await;
            match result {
                Err(error)
                    if error.kind() == io::ErrorKind::NotFound
                        || retryable_startup_error(&error) =>
                {
                    tokio::time::sleep(REBIND_RETRY_PAUSE).await;
                }
                result => return result,
            }
        }
    })
    .await
    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "daemon bridge handshake timed out"))?
}

fn retryable_startup_error(error: &io::Error) -> bool {
    endpoint_exists_but_refused(error)
        || matches!(
            error.kind(),
            io::ErrorKind::BrokenPipe
                | io::ErrorKind::UnexpectedEof
                | io::ErrorKind::ConnectionAborted
                | io::ErrorKind::ConnectionReset
                // macOS reports a Unix socket whose peer closed before any
                // byte was exchanged as ENOTCONN (57) on the first write or
                // read, where Linux and Windows report a reset or EOF.
                | io::ErrorKind::NotConnected
        )
}

/// Only move forward within this installation. Development overrides, other
/// installs and rollback UIs must never retire a newer daemon.
fn upgrade_pending(running: &Path, bundled: &Path) -> bool {
    let (Ok(running), Ok(bundled)) = (running.canonicalize(), bundled.canonicalize()) else {
        return false;
    };
    let Some(release_dir) = bundled.parent() else {
        return false;
    };
    let Some(daemons_dir) = release_dir.parent() else {
        return false;
    };
    if daemons_dir.file_name().is_none_or(|name| name != "daemons") || running == bundled {
        return false;
    }
    if running.parent() == daemons_dir.parent() {
        return true; // Transition from a legacy sibling installation.
    }
    let Some(old_dir) = running.parent() else {
        return false;
    };
    if old_dir.parent() != Some(daemons_dir) {
        return false;
    }
    match (
        old_dir
            .file_name()
            .and_then(|s| s.to_str())
            .and_then(|s| semver::Version::parse(s).ok()),
        release_dir
            .file_name()
            .and_then(|s| s.to_str())
            .and_then(|s| semver::Version::parse(s).ok()),
    ) {
        (Some(old), Some(new)) => new > old,
        _ => false,
    }
}

/// Whether a failed connect means "a daemon is there, try again in a moment"
/// rather than "nothing is listening, go spawn one".
///
/// Windows reports the rebind window as `ERROR_PIPE_BUSY` (231) and a missing
/// pipe as `ERROR_FILE_NOT_FOUND` (2); Unix reports a missing socket as
/// `NotFound` and a listening-but-saturated socket as `ConnectionRefused`.
/// Other errors are returned to the caller; permission and protocol failures
/// must not be mistaken for an absent daemon.
fn endpoint_exists_but_refused(err: &io::Error) -> bool {
    #[cfg(windows)]
    const ERROR_PIPE_BUSY: i32 = 231;
    #[cfg(windows)]
    if err.raw_os_error() == Some(ERROR_PIPE_BUSY) {
        return true;
    }
    matches!(
        err.kind(),
        io::ErrorKind::ConnectionRefused | io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Serializes tests that mutate UNSHIT_PTYD_BINARY so they do not
    // race within a single test process.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn ui_handshake_retries_a_connection_closed_during_retirement() {
        let path = unique_socket_path();
        #[cfg(windows)]
        let mut listener = unshit_ptyd::transport::Server::bind(&path).unwrap();
        #[cfg(unix)]
        let mut listener = unshit_ptyd::transport::Server::bind(&path).await.unwrap();
        let peer = tokio::spawn(async move {
            // The retiring daemon rejects this connection before its Hello.
            drop(listener.accept().await.unwrap());
            let successor = listener.accept().await.unwrap();
            let (shutdown, _rx) = tokio::sync::broadcast::channel(4);
            unshit_ptyd::daemon::handler::serve_connection(
                successor,
                shutdown,
                std::sync::Arc::new(unshit_ptyd::session::registry::SessionRegistry::new()),
            )
            .await
            .unwrap();
        });
        let (mut client, _events, protocol) = connect_ui_client(&path).await.unwrap();
        assert_eq!(protocol, unshit_ptyd::protocol::PROTOCOL_VERSION);
        client.shutdown().await.unwrap();
        peer.await.unwrap();
    }

    #[test]
    fn installed_daemon_selection_and_upgrade_direction() {
        let root = std::env::temp_dir().join(format!(
            "tm-daemon-layout-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let sibling = root.join("unshit-ptyd.exe");
        let old = root.join("daemons/0.5.0/unshit-ptyd.exe");
        let new = root.join("daemons/0.6.1/unshit-ptyd.exe");
        let other = root.join("other/daemons/0.6.1/unshit-ptyd.exe");
        for file in [&sibling, &old, &new, &other] {
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, b"fixture").unwrap();
        }
        assert_eq!(bundled_daemon_at(&sibling, "0.6.1"), new);
        assert_eq!(bundled_daemon_at(&sibling, "0.4.0"), sibling);
        assert!(upgrade_pending(&old, &new));
        assert!(upgrade_pending(&sibling, &new));
        assert!(!upgrade_pending(&new, &old));
        assert!(!upgrade_pending(&new, &new));
        assert!(!upgrade_pending(&old, &other));
        assert!(!upgrade_pending(&old, &sibling));
        std::fs::remove_dir_all(root).unwrap();
    }

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

        let outcome = connect_or_spawn(&socket).await.expect("connect_or_spawn");
        assert_eq!(
            outcome,
            DaemonStartup::Attached,
            "a live daemon must be attached to, never re-spawned"
        );

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
