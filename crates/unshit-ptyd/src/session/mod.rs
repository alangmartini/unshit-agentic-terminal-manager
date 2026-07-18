//! Session ownership: one PTY child per session, one registry per daemon.
//!
//! A [`Session`] is the daemon-side counterpart of a UI pane. It owns a
//! spawned shell child through [`crate::pty::PtyPair`], tags itself with
//! a monotonic `u64` id, and pipes output bytes through a
//! [`tokio::sync::mpsc::Sender`] so the handler can forward them as
//! `ServerEvent::Output` frames.
//!
//! Slice 4b: the session also owns an `unshit_terminal_core::Terminal`
//! and feeds every PTY chunk through it in the reader task, so the
//! daemon maintains authoritative grid plus scrollback state. Scrollback
//! persistence and the attach RPC arrive in slice 4c.

pub mod registry;

use std::io::{Read, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use unshit_terminal_core::{Snapshot, Terminal};

/// Default scrollback cap per session. Matches SPEC.md section 3 F3.
const DEFAULT_SCROLLBACK: usize = 10_000;

/// Size of the read buffer fed into the mpsc. Matches the value used
/// elsewhere in the UI bridge so throughput characteristics do not drift
/// between slice 3a (daemon owns PTYs, UI still in-process) and later
/// slices.
const READ_BUF_LEN: usize = 4096;

pub const ENV_NOTIFY_SOCKET: &str = "TM_NOTIFY_SOCKET";
pub const ENV_WORKSPACE_ID: &str = "TM_WORKSPACE_ID";
pub const ENV_PANE_ID: &str = "TM_PANE_ID";
pub const ENV_AGENT_HOOK_CAPABILITY: &str = "TM_AGENT_HOOK_CAPABILITY";
pub const ENV_AGENT_RESTORE_CORRELATION_ID: &str = "TM_AGENT_RESTORE_CORRELATION_ID";

/// Session-scoped generation that identifies the current output attachment.
pub type AttachmentToken = u64;

/// Environment variables that select a non-default Claude Code profile,
/// override the Anthropic provider/model, or mark an in-progress Claude
/// Code agent session.
///
/// The daemon is long-lived and inherits the environment of whatever
/// launched it. When that launcher is a Claude Code session or a
/// provider-override wrapper (e.g. a z.ai/GLM profile that exports
/// `ANTHROPIC_BASE_URL` + `CLAUDE_CONFIG_DIR`), those vars would otherwise
/// propagate into every spawned pane. A pane is meant to be a clean
/// interactive shell, so a `claude`/`cc` started inside one must fall back
/// to the user's default config rather than inheriting the launcher's
/// profile. We strip these before spawn so a single tainted launch can't
/// poison every pane until the daemon restarts.
pub(crate) const INHERITED_AGENT_ENV_VARS: &[&str] = &[
    // Config-directory / profile selection.
    "CLAUDE_CONFIG_DIR",
    // Provider + model overrides (set by GLM/z.ai-style wrappers).
    "ANTHROPIC_BASE_URL",
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_MODEL",
    "ANTHROPIC_SMALL_FAST_MODEL",
    "ANTHROPIC_DEFAULT_OPUS_MODEL",
    "ANTHROPIC_DEFAULT_SONNET_MODEL",
    "ANTHROPIC_DEFAULT_HAIKU_MODEL",
    // Markers of an in-progress Claude Code session.
    "CLAUDECODE",
    "CLAUDE_CODE_ENTRYPOINT",
    "CLAUDE_CODE_SSE_PORT",
    "CLAUDE_CODE_SESSION_ID",
    "CLAUDE_CODE_TMPDIR",
    "CLAUDE_CODE_EXECPATH",
    "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC",
];

/// Remove the [`INHERITED_AGENT_ENV_VARS`] from a spawn command so panes
/// don't inherit the daemon launcher's Claude Code profile or provider
/// override. `CommandBuilder::new` pre-seeds the command with the daemon's
/// base environment, so `env_remove` here drops inherited values, not just
/// ones set explicitly on this builder.
pub(crate) fn strip_inherited_agent_env(cmd: &mut CommandBuilder) {
    for key in INHERITED_AGENT_ENV_VARS {
        cmd.env_remove(key);
    }
}

/// Owns one PTY child and the reader task that fans its bytes into the
/// outbound mpsc.
pub struct Session {
    /// Monotonic id assigned by the registry.
    pub id: u64,
    /// Underlying PTY state (master, child, writer).
    pty: Option<PtyPair>,
    /// Last known geometry. Kept for `ListSessions`.
    cols: u16,
    rows: u16,
    /// PID of the child shell at spawn time.
    pid: Option<u32>,
    /// Reader task handle; aborted on drop so the blocking read does not
    /// keep the child's master alive.
    reader_task: Option<JoinHandle<()>>,
    /// Polls the primary child independently from the PTY reader. ConPTY can
    /// keep a master read pending after a one-shot child exits, so reader EOF
    /// alone is not a reliable lifecycle signal on Windows.
    child_watch_task: Option<JoinHandle<()>>,
    /// Daemon-side terminal emulator. Every PTY chunk is parsed into
    /// this in the reader task before being forwarded to the mpsc, so
    /// `snapshot()` always reflects bytes already observed by clients.
    terminal: Arc<Mutex<Terminal>>,
    /// Swappable output sink plus its session-scoped generation token.
    /// `None` when no client is attached; the reader task still parses
    /// bytes into `terminal`, but nothing is forwarded. `attach()` swaps
    /// in a fresh sender with a new token. `detach(token)` only clears the
    /// sender when the token still owns the attachment, so cleanup from a
    /// stale connection cannot detach a newer connection.
    output: Arc<Mutex<OutputState>>,
    /// Workspace id tag used by the UI to match sessions back to panes
    /// after a restart. Opaque to the daemon.
    workspace_id: u32,
    /// Pane id tag within the workspace. Opaque to the daemon.
    pane_id: u32,
    /// Optional human-friendly name for the session.
    name: Option<String>,
    /// Unpredictable bearer capability injected only into this PTY's
    /// environment and echoed to the attached UI over the daemon protocol.
    hook_capability: String,
}

/// Internal representation of one PTY child, mirrored after the
/// `PtyPair` used by the old UI-side manager but without per-pane
/// bookkeeping.
struct PtyPair {
    child: Arc<Mutex<Box<dyn Child + Send + Sync>>>,
    writer: Box<dyn Write + Send>,
    master: Box<dyn MasterPty + Send>,
}

struct OutputState {
    current: Option<OutputSink>,
    next_token: u64,
    /// False once the sole PTY reader reaches EOF, errors, or panics. The
    /// flag shares the output lock so attach and reader shutdown form one
    /// atomic decision: an attach either installs its sender before shutdown
    /// clears it, or observes a closed reader and fails.
    reader_open: bool,
}

struct OutputSink {
    token: AttachmentToken,
    tx: mpsc::Sender<Vec<u8>>,
}

impl Session {
    /// Spawns a shell with the requested geometry and starts the reader
    /// task. On success the caller receives the `Session`, the initial
    /// attachment token, and a `Receiver` it can poll for outbound bytes.
    ///
    /// `shell` overrides the platform default when `Some`; falling back
    /// to `SHELL` + platform default when `None`. `shell_args` are
    /// forwarded to the program before any daemon side cwd args (the
    /// PowerShell `-NoExit -Command "Set-Location ..."` workaround).
    pub fn spawn(
        id: u64,
        cols: u16,
        rows: u16,
        cwd: Option<&Path>,
        shell: Option<&str>,
        shell_args: &[String],
        workspace_id: u32,
        pane_id: u32,
        name: Option<String>,
    ) -> std::io::Result<(Self, AttachmentToken, mpsc::Receiver<Vec<u8>>)> {
        Self::spawn_with_context(
            id,
            cols,
            rows,
            cwd,
            shell,
            shell_args,
            workspace_id,
            pane_id,
            name,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn spawn_with_context(
        id: u64,
        cols: u16,
        rows: u16,
        cwd: Option<&Path>,
        shell: Option<&str>,
        shell_args: &[String],
        workspace_id: u32,
        pane_id: u32,
        name: Option<String>,
        restore_correlation_id: Option<&str>,
    ) -> std::io::Result<(Self, AttachmentToken, mpsc::Receiver<Vec<u8>>)> {
        let hook_capability = generate_hook_capability()?;
        let pty_system = native_pty_system();
        let size = PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        };
        let pty = pty_system.openpty(size).map_err(std::io::Error::other)?;

        let shell = shell
            .map(|s| s.to_string())
            .unwrap_or_else(crate::pty::default_shell);

        let mut cmd = CommandBuilder::new(&shell);
        for arg in crate::pty::build_spawn_args(&shell, shell_args, cwd) {
            cmd.arg(arg);
        }
        if let Some(dir) = cwd {
            cmd.cwd(dir);
        } else if let Some(home) = dirs::home_dir() {
            cmd.cwd(home);
        }
        // Advertise xterm capabilities so TUI apps (Claude Code, vim,
        // less, htop, etc) enable alt-screen, true color, and other
        // modern features instead of falling back to a degraded inline
        // rendering path. Without TERM set, ink-based apps treat us as
        // a dumb terminal and skip DECSET 1049 / 256-color escapes.
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        for (key, value) in terminal_manager_session_env_with_correlation(
            workspace_id,
            pane_id,
            &hook_capability,
            restore_correlation_id,
        ) {
            cmd.env(key, value);
        }
        // Panes are fresh interactive shells, not children of whatever
        // Claude Code / provider profile launched the long-lived daemon.
        // Strip the leaked profile/session vars so e.g. `cc` uses the
        // user's default config instead of inheriting a GLM/z.ai profile.
        strip_inherited_agent_env(&mut cmd);

        let child = pty.slave.spawn_command(cmd).map_err(|_| {
            // portable-pty's Windows error includes the full CreateProcess
            // command line. Agent prompts can be argv values, so neither the
            // daemon response nor logs may forward that raw provider error.
            log::error!(
                r#"{{"event":"pty_child_spawn_failed","workspace_id":{workspace_id},"pane_id":{pane_id},"error_kind":"spawn_command"}}"#
            );
            std::io::Error::other("PTY child process could not be started")
        })?;
        let pid = child.process_id();
        let child = Arc::new(Mutex::new(child));

        let reader = pty
            .master
            .try_clone_reader()
            .map_err(std::io::Error::other)?;
        let writer = pty.master.take_writer().map_err(std::io::Error::other)?;

        let (tx, rx) = mpsc::channel::<Vec<u8>>(64);

        let terminal = Arc::new(Mutex::new(Terminal::new(
            rows as usize,
            cols as usize,
            DEFAULT_SCROLLBACK,
        )));
        let reader_terminal = Arc::clone(&terminal);

        let attachment_token = 1;
        let output = Arc::new(Mutex::new(OutputState {
            current: Some(OutputSink {
                token: attachment_token,
                tx,
            }),
            next_token: 2,
            reader_open: true,
        }));
        let reader_output = Arc::clone(&output);

        let reader_task = tokio::task::spawn_blocking(move || {
            run_reader(reader, reader_output, reader_terminal);
        });
        let child_watch_task =
            tokio::spawn(watch_child_exit(Arc::clone(&child), Arc::clone(&output)));

        let session = Self {
            id,
            pty: Some(PtyPair {
                child,
                writer,
                master: pty.master,
            }),
            cols,
            rows,
            pid,
            reader_task: Some(reader_task),
            child_watch_task: Some(child_watch_task),
            terminal,
            output,
            workspace_id,
            pane_id,
            name,
            hook_capability,
        };

        Ok((session, attachment_token, rx))
    }

    /// Replaces the current output sender with a fresh channel and
    /// returns the matching receiver. Any prior receiver is dropped;
    /// the reader stops forwarding to it on the next chunk.
    pub fn attach(&self) -> Option<(AttachmentToken, mpsc::Receiver<Vec<u8>>)> {
        self.attach_with_snapshot(0)
            .map(|(token, _snapshot, output)| (token, output))
    }

    /// Atomically establishes a new output boundary and snapshots everything
    /// before it. The reader takes the same `terminal -> output` lock order,
    /// so a byte chunk is either represented in this snapshot and delivered
    /// only to the old attachment, or absent from the snapshot and delivered
    /// to the new attachment. It can never be replayed through both surfaces.
    pub fn attach_with_snapshot(
        &self,
        scrollback_lines: usize,
    ) -> Option<(AttachmentToken, Snapshot, mpsc::Receiver<Vec<u8>>)> {
        let (tx, rx) = mpsc::channel::<Vec<u8>>(64);
        let terminal = self
            .terminal
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut output = self
            .output
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !output.reader_open || !self.child_running() {
            output.reader_open = false;
            output.current = None;
            return None;
        }
        let token = output.next_token;
        output.next_token = token
            .checked_add(1)
            .expect("session attachment token space exhausted");
        output.current = Some(OutputSink { token, tx });
        let snapshot = terminal.snapshot(scrollback_lines);
        Some((token, snapshot, rx))
    }

    /// Clears the output sender only when `token` still owns it. Future
    /// PTY output still lands in the terminal but is not forwarded
    /// anywhere. Returns `false` for a stale or already-detached token.
    pub fn detach(&self, token: AttachmentToken) -> bool {
        let mut guard = self
            .output
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if guard.current.as_ref().map(|sink| sink.token) == Some(token) {
            guard.current = None;
            true
        } else {
            false
        }
    }

    /// Writes `bytes` to the PTY stdin. Uses `spawn_blocking` because
    /// the `Write` impl from portable-pty is blocking.
    pub async fn write(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        let pty = self
            .pty
            .as_mut()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotConnected, "session dead"))?;
        pty.writer.write_all(bytes)?;
        pty.writer.flush()
    }

    /// Resizes the PTY and the daemon-side terminal. Best-effort: if
    /// the PTY resize call fails we keep the old dimensions so later
    /// accessors do not lie about reality, and we do not touch the
    /// terminal either.
    pub fn resize(&mut self, cols: u16, rows: u16) {
        if let Some(pty) = self.pty.as_mut() {
            let new_size = PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            };
            if pty.master.resize(new_size).is_ok() {
                self.cols = cols;
                self.rows = rows;
                if let Ok(mut term) = self.terminal.lock() {
                    term.resize(rows as usize, cols as usize);
                }
            }
        }
    }

    /// Returns a snapshot of the current grid plus up to
    /// `scrollback_lines` most-recent scrollback rows. Never panics on a
    /// poisoned mutex; returns a fresh snapshot sized to the current
    /// dimensions in that case.
    pub fn snapshot(&self, scrollback_lines: usize) -> Snapshot {
        match self.terminal.lock() {
            Ok(term) => term.snapshot(scrollback_lines),
            Err(_) => Terminal::new(self.rows as usize, self.cols as usize, 0).snapshot(0),
        }
    }

    /// Kills the child and aborts the reader task.
    ///
    /// Safe to call multiple times; subsequent calls are no-ops.
    pub fn kill(&mut self) {
        if let Some(handle) = self.child_watch_task.take() {
            handle.abort();
        }
        if let Some(pty) = self.pty.take() {
            if let Ok(mut child) = pty.child.lock() {
                let _ = child.kill();
                let _ = child.wait();
            }
            // Explicitly drop the writer/master so the reader sees EOF.
            drop(pty.writer);
            drop(pty.master);
        }
        if let Some(handle) = self.reader_task.take() {
            handle.abort();
        }
        close_output(&self.output);
    }

    /// Reports whether the child is still running.
    pub fn alive(&self) -> bool {
        let reader_open = self
            .output
            .lock()
            .map(|output| output.reader_open)
            .unwrap_or(false);
        if !reader_open {
            return false;
        }
        self.child_running()
    }

    fn child_running(&self) -> bool {
        let Some(pty) = self.pty.as_ref() else {
            return false;
        };
        let Ok(mut child) = pty.child.lock() else {
            return false;
        };
        matches!(child.try_wait(), Ok(None))
    }

    pub fn cols(&self) -> u16 {
        self.cols
    }

    pub fn rows(&self) -> u16 {
        self.rows
    }

    pub fn pid(&self) -> Option<u32> {
        self.pid
    }

    pub fn workspace_id(&self) -> u32 {
        self.workspace_id
    }

    pub fn pane_id(&self) -> u32 {
        self.pane_id
    }

    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    pub fn hook_capability(&self) -> &str {
        &self.hook_capability
    }

    /// Set or clear the display name. An empty string is treated the
    /// same as `None` so the UI does not have to care which it sends.
    pub fn set_name(&mut self, name: Option<String>) {
        self.name = name.filter(|s| !s.is_empty());
    }
}

pub fn terminal_manager_session_env(
    workspace_id: u32,
    pane_id: u32,
    hook_capability: &str,
) -> Vec<(&'static str, String)> {
    terminal_manager_session_env_with_correlation(workspace_id, pane_id, hook_capability, None)
}

fn terminal_manager_session_env_with_correlation(
    workspace_id: u32,
    pane_id: u32,
    hook_capability: &str,
    restore_correlation_id: Option<&str>,
) -> Vec<(&'static str, String)> {
    let restore_correlation_id = restore_correlation_id
        .filter(|value| is_valid_correlation_id(value))
        .map(str::to_owned)
        .or_else(|| {
            std::env::var(ENV_AGENT_RESTORE_CORRELATION_ID)
                .ok()
                .filter(|value| is_valid_correlation_id(value))
        })
        .unwrap_or_default();
    vec![
        (ENV_NOTIFY_SOCKET, notification_socket_path_from_env()),
        (ENV_WORKSPACE_ID, workspace_id.to_string()),
        (ENV_PANE_ID, pane_id.to_string()),
        (ENV_AGENT_HOOK_CAPABILITY, hook_capability.to_string()),
        (ENV_AGENT_RESTORE_CORRELATION_ID, restore_correlation_id),
    ]
}

fn is_valid_correlation_id(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| match index {
            8 | 13 | 18 | 23 => byte == b'-',
            _ => byte.is_ascii_hexdigit(),
        })
}

fn generate_hook_capability() -> std::io::Result<String> {
    let mut bytes = [0u8; 32];
    fill_secure_random(&mut bytes)?;
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    Ok(encoded)
}

#[cfg(windows)]
fn fill_secure_random(bytes: &mut [u8]) -> std::io::Result<()> {
    use windows_sys::Win32::Security::Cryptography::{
        BCryptGenRandom, BCRYPT_USE_SYSTEM_PREFERRED_RNG,
    };

    // SAFETY: the pointer and byte count describe the live mutable slice;
    // a null algorithm handle is required with SYSTEM_PREFERRED_RNG.
    let status = unsafe {
        BCryptGenRandom(
            std::ptr::null_mut(),
            bytes.as_mut_ptr(),
            bytes.len().try_into().expect("capability size fits u32"),
            BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        )
    };
    if status >= 0 {
        Ok(())
    } else {
        Err(std::io::Error::other("system random generator failed"))
    }
}

#[cfg(unix)]
fn fill_secure_random(bytes: &mut [u8]) -> std::io::Result<()> {
    std::fs::File::open("/dev/urandom")?.read_exact(bytes)
}

fn notification_socket_path_from_env() -> String {
    std::env::var_os(ENV_NOTIFY_SOCKET)
        .filter(|v| !v.is_empty())
        .map(|v| v.to_string_lossy().to_string())
        .unwrap_or_else(|| {
            default_notification_socket_path()
                .to_string_lossy()
                .to_string()
        })
}

fn default_notification_socket_path() -> std::path::PathBuf {
    #[cfg(windows)]
    {
        std::path::PathBuf::from(r"\\.\pipe\terminal-manager-notify")
    }
    #[cfg(unix)]
    {
        if let Ok(dir) = std::env::var("XDG_RUNTIME_DIR") {
            return std::path::PathBuf::from(dir).join("terminal-manager-notify.sock");
        }
        std::env::temp_dir().join(format!("terminal-manager-notify-{}.sock", current_euid()))
    }
}

#[cfg(unix)]
fn current_euid() -> u32 {
    extern "C" {
        fn geteuid() -> u32;
    }
    unsafe { geteuid() }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.kill();
    }
}

fn run_reader(
    reader: Box<dyn Read + Send>,
    output: Arc<Mutex<OutputState>>,
    terminal: Arc<Mutex<Terminal>>,
) {
    // The reader body can panic if the VTE parser hits a bug on
    // malformed input, if process_bytes indexes out-of-bounds after a
    // resize race, or if any other internal invariant is violated.
    // Catching here fulfils the slice-6 "a panic in one session's
    // parser thread must not take the daemon down" acceptance
    // criterion: the task exits cleanly, the child stays killable
    // through the normal Session::kill path, and every other session's
    // reader keeps streaming on its own thread.
    let reader_output = Arc::clone(&output);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        run_reader_inner(reader, reader_output, terminal);
    }));
    if let Err(payload) = result {
        let msg = panic_payload_str(&payload);
        log::error!("session reader panicked: {msg}; reader exiting");
    }
    close_output(&output);
}

async fn watch_child_exit(
    child: Arc<Mutex<Box<dyn Child + Send + Sync>>>,
    output: Arc<Mutex<OutputState>>,
) {
    loop {
        let exited = match child.lock() {
            Ok(mut child) => !matches!(child.try_wait(), Ok(None)),
            Err(_) => true,
        };
        if exited {
            // Give the reader a bounded window to drain output the child
            // wrote immediately before exit. Unix PTYs usually reach EOF in
            // this window and close themselves; ConPTY may keep the read
            // pending until the pseudoconsole is torn down, so the watcher
            // still provides the eventual close required by the UI.
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            close_output(&output);
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

fn close_output(output: &Arc<Mutex<OutputState>>) {
    let mut output = output
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    output.reader_open = false;
    output.current = None;
}

fn run_reader_inner(
    mut reader: Box<dyn Read + Send>,
    output: Arc<Mutex<OutputState>>,
    terminal: Arc<Mutex<Terminal>>,
) {
    let mut buf = vec![0u8; READ_BUF_LEN];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => return,
            Ok(n) => {
                let tx_opt = {
                    let Ok(mut term) = terminal.lock() else {
                        continue;
                    };
                    term.process_bytes(&buf[..n]);
                    output
                        .lock()
                        .ok()
                        .and_then(|guard| guard.current.as_ref().map(|sink| sink.tx.clone()))
                };
                if let Some(tx) = tx_opt {
                    // Non-blocking: if the current client is slow or gone
                    // we drop the chunk and rely on the terminal plus
                    // scrollback as the source of truth. Never exit the
                    // reader when the receiver is gone; a later attach
                    // should still observe live output.
                    let _ = tx.try_send(buf[..n].to_vec());
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => return,
        }
    }
}

fn panic_payload_str(payload: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<&'static str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "<non-string panic payload>".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn test_shell() -> &'static str {
        #[cfg(windows)]
        {
            "cmd.exe"
        }
        #[cfg(unix)]
        {
            "/bin/sh"
        }
    }

    /// Drains the receiver for up to `timeout` ms, returning the
    /// accumulated bytes. Stops early when the channel closes.
    async fn drain_for(rx: &mut mpsc::Receiver<Vec<u8>>, timeout: Duration) -> Vec<u8> {
        let deadline = tokio::time::Instant::now() + timeout;
        let mut collected = Vec::new();
        while let Ok(chunk) = tokio::time::timeout_at(deadline, rx.recv()).await {
            match chunk {
                Some(bytes) => collected.extend(bytes),
                None => break,
            }
        }
        collected
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn spawn_emits_output_from_echo() {
        let (mut session, _token, mut rx) =
            Session::spawn(1, 80, 24, None, Some(test_shell()), &[], 0, 0, None)
                .expect("spawn session");

        #[cfg(windows)]
        let payload = b"echo session-hi\r\n";
        #[cfg(unix)]
        let payload = b"echo session-hi\n";
        session.write(payload).await.expect("write");

        let got = drain_for(&mut rx, Duration::from_millis(1500)).await;
        let text = String::from_utf8_lossy(&got);
        assert!(
            text.contains("session-hi"),
            "expected echo output to contain marker, got: {text:?}"
        );

        session.kill();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn resize_updates_recorded_dimensions() {
        let (mut session, _token, _rx) =
            Session::spawn(2, 80, 24, None, Some(test_shell()), &[], 0, 0, None)
                .expect("spawn session");
        assert_eq!(session.cols(), 80);
        assert_eq!(session.rows(), 24);

        session.resize(120, 40);
        assert_eq!(session.cols(), 120);
        assert_eq!(session.rows(), 40);

        session.kill();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn set_name_stores_and_clears_display_name() {
        let (mut session, _token, _rx) =
            Session::spawn(10, 80, 24, None, Some(test_shell()), &[], 0, 0, None).expect("spawn");
        assert_eq!(session.name(), None);

        session.set_name(Some("build".to_string()));
        assert_eq!(session.name(), Some("build"));

        // Empty string must behave the same as clearing so the UI can
        // send whatever the input field contains without branching.
        session.set_name(Some(String::new()));
        assert_eq!(session.name(), None);

        session.set_name(Some("again".to_string()));
        session.set_name(None);
        assert_eq!(session.name(), None);

        session.kill();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn kill_is_idempotent_and_marks_session_dead() {
        let (mut session, _token, _rx) =
            Session::spawn(3, 80, 24, None, Some(test_shell()), &[], 0, 0, None)
                .expect("spawn session");
        assert!(
            session.alive(),
            "session must be alive immediately after spawn"
        );
        session.kill();
        // Second kill is a no-op and must not panic.
        session.kill();
        assert!(!session.alive(), "dead session must not report alive");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn drop_kills_child_and_closes_receiver() {
        let (session, _token, mut rx) =
            Session::spawn(4, 80, 24, None, Some(test_shell()), &[], 0, 0, None)
                .expect("spawn session");
        drop(session);

        // With the session gone the reader task should stop and the
        // channel should close. Allow a small grace window.
        let closed = tokio::time::timeout(Duration::from_millis(1500), async {
            while rx.recv().await.is_some() {}
        })
        .await;
        assert!(closed.is_ok(), "receiver should close once session dropped");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn natural_child_exit_closes_receiver_while_session_is_retained() {
        #[cfg(windows)]
        let shell_args = vec!["/Q".into(), "/D".into(), "/C".into(), "exit /B 0".into()];
        #[cfg(unix)]
        let shell_args = vec!["-c".into(), "exit 0".into()];

        let (mut session, _token, mut rx) =
            Session::spawn(5, 80, 24, None, Some(test_shell()), &shell_args, 0, 0, None)
                .expect("spawn one-shot session");

        let closed = tokio::time::timeout(Duration::from_secs(3), async {
            while rx.recv().await.is_some() {}
        })
        .await;
        assert!(
            closed.is_ok(),
            "natural PTY EOF must close the output channel even while the Session remains registered"
        );

        session.kill();
    }

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
    async fn snapshot_reflects_bytes_written_to_pty() {
        let (mut session, _token, mut rx) =
            Session::spawn(10, 80, 24, None, Some(test_shell()), &[], 0, 0, None)
                .expect("spawn session");

        #[cfg(windows)]
        let payload = b"echo snapmarker\r\n";
        #[cfg(unix)]
        let payload = b"echo snapmarker\n";
        session.write(payload).await.expect("write");

        let _ = drain_for(&mut rx, Duration::from_millis(1500)).await;
        let snap = session.snapshot(0);
        let rendered = grid_text(&snap);
        assert!(
            rendered.contains("snapmarker"),
            "expected snapshot to contain marker, got: {rendered:?}"
        );

        session.kill();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn snapshot_is_empty_for_fresh_session() {
        let (mut session, _token, _rx) =
            Session::spawn(11, 80, 24, None, Some(test_shell()), &[], 0, 0, None)
                .expect("spawn session");
        let snap = session.snapshot(0);
        assert_eq!(snap.grid.rows(), 24);
        assert_eq!(snap.grid.cols(), 80);
        assert_eq!(snap.grid.cursor(), (0, 0));
        session.kill();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn resize_propagates_to_terminal() {
        let (mut session, _token, _rx) =
            Session::spawn(12, 80, 24, None, Some(test_shell()), &[], 0, 0, None)
                .expect("spawn session");
        let snap = session.snapshot(0);
        assert_eq!(snap.grid.rows(), 24);
        assert_eq!(snap.grid.cols(), 80);

        session.resize(120, 40);
        let snap = session.snapshot(0);
        assert_eq!(snap.grid.rows(), 40);
        assert_eq!(snap.grid.cols(), 120);
        session.kill();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn snapshot_on_dead_session_returns_empty_grid() {
        let (mut session, _token, _rx) =
            Session::spawn(13, 80, 24, None, Some(test_shell()), &[], 0, 0, None)
                .expect("spawn session");
        session.kill();
        let snap = session.snapshot(0);
        assert_eq!(snap.grid.rows(), 24);
        assert_eq!(snap.grid.cols(), 80);
        let (cr, cc) = snap.grid.cursor();
        assert!(cr < snap.grid.rows(), "cursor row out of grid: {cr}");
        assert!(cc < snap.grid.cols(), "cursor col out of grid: {cc}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn spawn_records_workspace_and_pane_metadata() {
        let (session, _token, _rx) = Session::spawn(
            20,
            80,
            24,
            None,
            Some(test_shell()),
            &[],
            7,
            3,
            Some("scratch".into()),
        )
        .expect("spawn session");
        assert_eq!(session.workspace_id(), 7);
        assert_eq!(session.pane_id(), 3);
        assert_eq!(session.name(), Some("scratch"));
    }

    #[test]
    fn strip_inherited_agent_env_removes_profile_and_session_vars() {
        let mut cmd = CommandBuilder::new("pwsh");
        // Simulate the leaked GLM/z.ai profile plus Claude Code session
        // markers that a tainted daemon launch would propagate.
        cmd.env("CLAUDE_CONFIG_DIR", r"C:\Users\x\.claude-glm");
        cmd.env("ANTHROPIC_BASE_URL", "https://api.z.ai/api/anthropic");
        cmd.env("ANTHROPIC_MODEL", "glm-5.2");
        cmd.env("CLAUDE_CODE_SESSION_ID", "0b453326");
        cmd.env("CLAUDECODE", "1");
        // A benign var the user actually wants must survive.
        cmd.env("MY_SAFE_VAR", "keep-me");

        strip_inherited_agent_env(&mut cmd);

        for key in INHERITED_AGENT_ENV_VARS {
            assert!(
                cmd.get_env(key).is_none(),
                "{key} should have been stripped from the spawn env"
            );
        }
        assert_eq!(
            cmd.get_env("MY_SAFE_VAR"),
            Some(std::ffi::OsStr::new("keep-me")),
            "non-agent env vars must be preserved"
        );
    }

    #[test]
    fn session_env_includes_notification_target_metadata() {
        let env = terminal_manager_session_env(7, 3, "capability");
        let find = |key: &str| {
            env.iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| v.as_str())
                .unwrap_or("")
        };

        assert!(!find(ENV_NOTIFY_SOCKET).is_empty());
        assert_eq!(find(ENV_WORKSPACE_ID), "7");
        assert_eq!(find(ENV_PANE_ID), "3");
        assert_eq!(find(ENV_AGENT_HOOK_CAPABILITY), "capability");
    }

    #[test]
    fn per_spawn_correlation_overrides_long_lived_daemon_environment() {
        const CURRENT_UI: &str = "019f75b0-e94f-71f0-b1ea-f478f8438c1a";
        let env =
            terminal_manager_session_env_with_correlation(7, 3, "capability", Some(CURRENT_UI));
        let correlation = env
            .iter()
            .find(|(key, _)| *key == ENV_AGENT_RESTORE_CORRELATION_ID)
            .map(|(_, value)| value.as_str());

        assert_eq!(correlation, Some(CURRENT_UI));
    }

    #[test]
    fn generated_hook_capabilities_are_valid_and_distinct() {
        let first = generate_hook_capability().expect("first capability");
        let second = generate_hook_capability().expect("second capability");

        assert_eq!(first.len(), 64);
        assert!(first.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_eq!(second.len(), 64);
        assert!(second.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_ne!(first, second);
    }

    #[test]
    fn child_spawn_failure_never_exposes_command_arguments() {
        const SECRET_PROMPT: &str = "sentinel-prompt-that-must-not-escape";
        let shell_args = vec![SECRET_PROMPT.to_string()];
        let error = match Session::spawn(
            91,
            80,
            24,
            None,
            Some("terminal-manager-deliberately-missing-executable"),
            &shell_args,
            4,
            7,
            None,
        ) {
            Ok(_) => panic!("missing executable unexpectedly spawned"),
            Err(error) => error,
        };
        let message = error.to_string();

        assert_eq!(message, "PTY child process could not be started");
        assert!(!message.contains(SECRET_PROMPT));
        assert!(!message.contains("terminal-manager-deliberately-missing-executable"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn attach_replaces_prior_receiver_and_drops_it() {
        let (mut session, _original_token, original_rx) =
            Session::spawn(21, 80, 24, None, Some(test_shell()), &[], 0, 0, None)
                .expect("spawn session");

        let (_new_token, mut new_rx) = session.attach().expect("live session attaches");

        #[cfg(windows)]
        let payload = b"echo reattach-marker\r\n";
        #[cfg(unix)]
        let payload = b"echo reattach-marker\n";
        session.write(payload).await.expect("write");

        let got = drain_for(&mut new_rx, Duration::from_millis(1500)).await;
        let text = String::from_utf8_lossy(&got);
        assert!(
            text.contains("reattach-marker"),
            "new receiver should observe live bytes, got: {text:?}"
        );

        // Original receiver was dropped and replaced on attach; any bytes
        // that slipped through before the swap cannot include the marker.
        drop(original_rx);

        session.kill();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn detach_clears_tx_but_keeps_terminal_parsing() {
        let (mut session, token, rx) =
            Session::spawn(22, 80, 24, None, Some(test_shell()), &[], 0, 0, None)
                .expect("spawn session");
        drop(rx);
        assert!(session.detach(token));

        #[cfg(windows)]
        let payload = b"echo detachmarker\r\n";
        #[cfg(unix)]
        let payload = b"echo detachmarker\n";
        session.write(payload).await.expect("write");

        tokio::time::sleep(Duration::from_millis(1500)).await;
        let snap = session.snapshot(0);
        let rendered = grid_text(&snap);
        assert!(
            rendered.contains("detachmarker"),
            "terminal must keep parsing while detached, got: {rendered:?}"
        );

        session.kill();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn reader_does_not_exit_when_output_receiver_dropped() {
        let (mut session, _token, rx) =
            Session::spawn(23, 80, 24, None, Some(test_shell()), &[], 0, 0, None)
                .expect("spawn session");
        drop(rx);

        #[cfg(windows)]
        let payload = b"echo livemarker\r\n";
        #[cfg(unix)]
        let payload = b"echo livemarker\n";
        session.write(payload).await.expect("write");

        tokio::time::sleep(Duration::from_millis(1500)).await;
        let snap = session.snapshot(0);
        let rendered = grid_text(&snap);
        assert!(
            rendered.contains("livemarker"),
            "reader must not exit because the client went away, got: {rendered:?}"
        );

        session.kill();
    }

    /// Regression for F4.2 (crash isolation): a panic inside
    /// `run_reader_inner` must be trapped by `run_reader`'s
    /// `catch_unwind` wrapper, never propagating to the task that
    /// spawned it. Without the wrapper a VTE parser bug would take the
    /// daemon's spawn_blocking task with it, killing every other
    /// session running on the shared blocking pool.
    #[test]
    fn run_reader_catches_panic_from_reader() {
        struct PanickingReader;
        impl Read for PanickingReader {
            fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
                panic!("synthetic reader panic");
            }
        }

        let terminal = Arc::new(Mutex::new(Terminal::new(24, 80, 0)));
        let (tx, mut rx) = mpsc::channel::<Vec<u8>>(4);
        let output = Arc::new(Mutex::new(OutputState {
            current: Some(OutputSink { token: 1, tx }),
            next_token: 2,
            reader_open: true,
        }));

        // Call directly on the current thread so any escaped panic
        // would fail the test; catch_unwind inside run_reader must
        // swallow it.
        run_reader(
            Box::new(PanickingReader),
            Arc::clone(&output),
            Arc::clone(&terminal),
        );

        assert_eq!(
            rx.try_recv(),
            Err(mpsc::error::TryRecvError::Disconnected),
            "a failed reader must close its output channel"
        );

        // Terminal state is still accessible: the Mutex is not poisoned
        // (the panic happened before any terminal lock was acquired, and
        // catch_unwind does not mark Mutexes we never held).
        let guard = terminal.lock().expect("terminal mutex must be unpoisoned");
        assert_eq!(guard.grid().rows(), 24);
    }
}
