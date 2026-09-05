//! Local notification IPC and CLI entry points.
//!
//! The running UI owns a local pipe/socket. Child processes inside managed
//! terminals call `terminal-manager notify ...`; the short-lived CLI process
//! sends a JSON request to that endpoint and exits. The UI subscription mutates
//! app state and yields framework events to repaint or activate the window.

use std::ffi::OsString;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::pin::Pin;
#[cfg(windows)]
use std::process::{Command, Stdio};

use futures_core::Stream;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use unshit::app::{EventSink, ExternalEvent, Subscription};

use crate::state::{
    focus_workspace_pane_by_num, mutate_with, push_notification_toast, SharedState,
};

pub const ENV_NOTIFY_SOCKET: &str = "TM_NOTIFY_SOCKET";
pub const ENV_WORKSPACE_ID: &str = "TM_WORKSPACE_ID";
pub const ENV_PANE_ID: &str = "TM_PANE_ID";
pub const ENV_AGENT_HOOK_CAPABILITY: &str = "TM_AGENT_HOOK_CAPABILITY";
pub const ENV_AGENT_RESTORE_CORRELATION_ID: &str = "TM_AGENT_RESTORE_CORRELATION_ID";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NotificationIpcRequest {
    Notify {
        title: String,
        text: String,
        workspace_id: u32,
        pane_id: u32,
    },
    Activate {
        workspace_id: u32,
        pane_id: u32,
    },
    AgentSessionObserved {
        agent: crate::agent_restore::AgentKind,
        session_id: String,
        cwd: PathBuf,
        source: String,
        workspace_id: u32,
        pane_id: u32,
        capability: String,
    },
    /// `terminal-manager agent [<profile>] [--workspace-id N]`: open a
    /// new agent tab. `profile` is a `crate::agents` id (default agent
    /// when absent); `workspace_id` is the daemon routing id (active
    /// workspace when absent).
    NewAgent {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        profile: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        workspace_id: Option<u32>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationTarget {
    pub workspace_id: u32,
    pub pane_id: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliCommand {
    Notify {
        socket: PathBuf,
        title: String,
        text: String,
        target: NotificationTarget,
    },
    Activate {
        socket: PathBuf,
        target: NotificationTarget,
    },
    SessionHook {
        socket: PathBuf,
        agent: crate::agent_restore::AgentKind,
        target: Option<NotificationTarget>,
        capability: Option<String>,
    },
    NewAgent {
        socket: PathBuf,
        profile: Option<String>,
        workspace_id: Option<u32>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CliMode {
    Notify,
    Activate,
    SessionHook(crate::agent_restore::AgentKind),
    NewAgent,
}

#[derive(Default)]
struct CliFields {
    title: Option<String>,
    text: Option<String>,
    socket: Option<PathBuf>,
    workspace_id: Option<u32>,
    pane_id: Option<u32>,
    profile: Option<String>,
}

pub fn handle_cli_from_env<I, S>(args: I) -> Option<i32>
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    let args: Vec<OsString> = args.into_iter().map(Into::into).collect();
    let is_session_hook = args
        .first()
        .is_some_and(|arg| arg.to_string_lossy() == "session-hook");
    let hook_agent = session_hook_agent_from_args(&args);
    let command = match parse_cli_args(args, |key| std::env::var(key).ok()) {
        Ok(Some(command)) => command,
        Ok(None) => return None,
        Err(e) => {
            if is_session_hook {
                record_hook_cli_failure(hook_agent, "cli_parse", "invalid_arguments");
                return Some(0);
            }
            eprintln!("terminal-manager notification error: {e}");
            eprintln!("{}", notification_usage());
            return Some(2);
        }
    };

    let result = match command {
        CliCommand::Notify {
            socket,
            title,
            text,
            target,
        } => send_cli_request_blocking(
            &socket,
            NotificationIpcRequest::Notify {
                title,
                text,
                workspace_id: target.workspace_id,
                pane_id: target.pane_id,
            },
        ),
        CliCommand::Activate { socket, target } => send_cli_request_blocking(
            &socket,
            NotificationIpcRequest::Activate {
                workspace_id: target.workspace_id,
                pane_id: target.pane_id,
            },
        ),
        CliCommand::SessionHook {
            socket,
            agent,
            target,
            capability,
        } => {
            let (Some(target), Some(capability)) = (target, capability) else {
                return Some(0);
            };
            if !trusted_hook_socket(&socket) {
                record_hook_cli_failure(Some(agent), "hook_transport", "untrusted_endpoint");
                return Some(0);
            }
            let observation = match read_session_hook_input(std::io::stdin().lock()) {
                Ok(observation) => observation,
                Err(_) => {
                    record_hook_cli_failure(Some(agent), "hook_payload", "invalid_payload");
                    return Some(0);
                }
            };
            send_hook_request_blocking(
                &socket,
                NotificationIpcRequest::AgentSessionObserved {
                    agent,
                    session_id: observation.session_id,
                    cwd: observation.cwd,
                    source: observation.source,
                    workspace_id: target.workspace_id,
                    pane_id: target.pane_id,
                    capability,
                },
            )
        }
        CliCommand::NewAgent {
            socket,
            profile,
            workspace_id,
        } => send_cli_request_blocking(
            &socket,
            NotificationIpcRequest::NewAgent {
                profile,
                workspace_id,
            },
        ),
    };

    match result {
        Ok(()) => Some(0),
        Err(e) => {
            if is_session_hook {
                record_hook_cli_failure(
                    hook_agent,
                    "hook_transport",
                    hook_transport_error_kind(&e),
                );
                Some(0)
            } else {
                eprintln!("terminal-manager notification error: {e}");
                Some(1)
            }
        }
    }
}

pub fn parse_cli_args<I, S, F>(args: I, get_env: F) -> Result<Option<CliCommand>, String>
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
    F: Fn(&str) -> Option<String>,
{
    let mut args = args
        .into_iter()
        .map(|s| s.into().to_string_lossy().to_string());
    let Some(first) = args.next() else {
        return Ok(None);
    };

    let mode = match first.as_str() {
        "notify" | "--notify" => CliMode::Notify,
        "activate" | "--activate" => CliMode::Activate,
        "session-hook" => {
            let provider = args
                .next()
                .ok_or_else(|| "session-hook requires a provider".to_string())?;
            let agent = match provider.as_str() {
                "claude" => crate::agent_restore::AgentKind::Claude,
                "codex" => crate::agent_restore::AgentKind::Codex,
                _ => return Err("session-hook provider must be claude or codex".to_string()),
            };
            CliMode::SessionHook(agent)
        }
        "agent" | "new-agent" => CliMode::NewAgent,
        _ => return Ok(None),
    };

    let mut fields = CliFields::default();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => return Err(notification_usage().to_string()),
            "--title" => {
                if matches!(mode, CliMode::SessionHook(_)) {
                    return Err("session-hook does not accept notification content".into());
                }
                fields.title = Some(take_value(&mut args, "--title")?);
            }
            "--text" | "--body" | "--message" => {
                if matches!(mode, CliMode::SessionHook(_)) {
                    return Err("session-hook does not accept notification content".into());
                }
                fields.text = Some(take_value(&mut args, arg.as_str())?)
            }
            "--socket" => fields.socket = Some(PathBuf::from(take_value(&mut args, "--socket")?)),
            "--workspace-id" | "--workspace" => {
                if matches!(mode, CliMode::SessionHook(_)) {
                    return Err("session-hook target comes only from terminal environment".into());
                }
                fields.workspace_id = Some(parse_u32_flag(&mut args, arg.as_str())?)
            }
            "--pane-id" | "--pane" => {
                if matches!(mode, CliMode::SessionHook(_)) {
                    return Err("session-hook target comes only from terminal environment".into());
                }
                fields.pane_id = Some(parse_u32_flag(&mut args, arg.as_str())?)
            }
            other if other.starts_with('-') => {
                return Err(format!("unknown flag {other:?}"));
            }
            positional => match mode {
                CliMode::Notify if fields.title.is_none() => {
                    fields.title = Some(positional.to_string());
                }
                CliMode::Notify if fields.text.is_none() => {
                    fields.text = Some(positional.to_string());
                }
                CliMode::NewAgent if fields.profile.is_none() => {
                    if crate::agents::parse_launchable_id(positional).is_none() {
                        let known: Vec<&str> =
                            crate::agents::launchable_profiles().map(|p| p.id).collect();
                        return Err(format!(
                            "unknown agent {positional:?}; known agents: {}",
                            known.join(", ")
                        ));
                    }
                    fields.profile = Some(positional.to_string());
                }
                CliMode::SessionHook(_) => {
                    return Err(format!("unexpected positional argument {positional:?}"));
                }
                _ => return Err(format!("unexpected positional argument {positional:?}")),
            },
        }
    }

    let socket = fields
        .socket
        .or_else(|| get_env(ENV_NOTIFY_SOCKET).map(PathBuf::from))
        .unwrap_or_else(default_notification_socket_path);
    let env_target = || {
        let workspace_id = parse_env_u32(&get_env, ENV_WORKSPACE_ID)?;
        let pane_id = parse_env_u32(&get_env, ENV_PANE_ID)?;
        Some(NotificationTarget {
            workspace_id,
            pane_id,
        })
    };

    match mode {
        CliMode::Notify => {
            let target = NotificationTarget {
                workspace_id: fields
                    .workspace_id
                    .or_else(|| parse_env_u32(&get_env, ENV_WORKSPACE_ID))
                    .ok_or_else(|| format!("missing --workspace-id or {ENV_WORKSPACE_ID}"))?,
                pane_id: fields
                    .pane_id
                    .or_else(|| parse_env_u32(&get_env, ENV_PANE_ID))
                    .ok_or_else(|| format!("missing --pane-id or {ENV_PANE_ID}"))?,
            };
            Ok(Some(CliCommand::Notify {
                socket,
                title: require_non_empty(fields.title, "--title")?,
                text: require_non_empty(fields.text, "--text")?,
                target,
            }))
        }
        CliMode::Activate => {
            let target = NotificationTarget {
                workspace_id: fields
                    .workspace_id
                    .or_else(|| parse_env_u32(&get_env, ENV_WORKSPACE_ID))
                    .ok_or_else(|| format!("missing --workspace-id or {ENV_WORKSPACE_ID}"))?,
                pane_id: fields
                    .pane_id
                    .or_else(|| parse_env_u32(&get_env, ENV_PANE_ID))
                    .ok_or_else(|| format!("missing --pane-id or {ENV_PANE_ID}"))?,
            };
            Ok(Some(CliCommand::Activate { socket, target }))
        }
        CliMode::SessionHook(agent) => Ok(Some(CliCommand::SessionHook {
            socket,
            agent,
            target: env_target(),
            capability: get_env(ENV_AGENT_HOOK_CAPABILITY)
                .filter(|value| is_valid_hook_capability(value)),
        })),
        CliMode::NewAgent => {
            if fields.title.is_some() || fields.text.is_some() {
                return Err("agent does not accept notification content".into());
            }
            Ok(Some(CliCommand::NewAgent {
                socket,
                profile: fields.profile,
                // Inside a managed terminal the environment names the
                // calling workspace, so the tab opens next to the caller.
                workspace_id: fields
                    .workspace_id
                    .or_else(|| parse_env_u32(&get_env, ENV_WORKSPACE_ID)),
            }))
        }
    }
}

fn take_value<I>(args: &mut I, flag: &str) -> Result<String, String>
where
    I: Iterator<Item = String>,
{
    args.next()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("{flag} requires a value"))
}

fn parse_u32_flag<I>(args: &mut I, flag: &str) -> Result<u32, String>
where
    I: Iterator<Item = String>,
{
    let raw = take_value(args, flag)?;
    raw.parse::<u32>()
        .map_err(|_| format!("{flag} must be an unsigned integer, got {raw:?}"))
}

fn parse_env_u32<F>(get_env: &F, key: &str) -> Option<u32>
where
    F: Fn(&str) -> Option<String>,
{
    get_env(key).and_then(|raw| raw.parse::<u32>().ok())
}

fn require_non_empty(value: Option<String>, field: &str) -> Result<String, String> {
    value
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("missing {field}"))
}

fn notification_usage() -> &'static str {
    "usage: terminal-manager notify --title <title> --text <text> [--workspace-id <id>] [--pane-id <id>] [--socket <path>]\n       terminal-manager activate [--workspace-id <id>] [--pane-id <id>] [--socket <path>]\n       terminal-manager agent [claude|codex|gemini|opencode|aider|copilot] [--workspace-id <id>] [--socket <path>]\n       terminal-manager session-hook <claude|codex>"
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
struct SessionHookInput {
    session_id: String,
    cwd: PathBuf,
    hook_event_name: String,
    source: String,
}

#[derive(Debug, PartialEq, Eq)]
struct AgentSessionObservation {
    session_id: String,
    cwd: PathBuf,
    source: String,
}

fn read_session_hook_input<R: Read>(reader: R) -> Result<AgentSessionObservation, String> {
    const MAX_HOOK_STDIN_BYTES: u64 = 64 * 1024;
    let mut bytes = Vec::new();
    reader
        .take(MAX_HOOK_STDIN_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "could not read hook input".to_string())?;
    if bytes.len() as u64 > MAX_HOOK_STDIN_BYTES {
        return Err("hook input exceeds size limit".into());
    }
    let input: SessionHookInput =
        serde_json::from_slice(&bytes).map_err(|_| "invalid hook JSON".to_string())?;
    if input.hook_event_name != "SessionStart" {
        return Err("unsupported hook event".into());
    }
    if !matches!(
        input.source.as_str(),
        "startup" | "resume" | "clear" | "compact"
    ) {
        return Err("unsupported session source".into());
    }
    if !crate::agent_restore::is_valid_session_id(&input.session_id) {
        return Err("invalid session id".into());
    }
    if !input.cwd.is_absolute() {
        return Err("hook cwd must be absolute".into());
    }
    Ok(AgentSessionObservation {
        session_id: input.session_id.to_ascii_lowercase(),
        cwd: input.cwd,
        source: input.source,
    })
}

fn send_cli_request_blocking(socket: &Path, request: NotificationIpcRequest) -> io::Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(io::Error::other)?;
    runtime.block_on(send_cli_request(socket, &request))
}

fn send_hook_request_blocking(socket: &Path, request: NotificationIpcRequest) -> io::Result<()> {
    // Provider hook configurations allow five seconds. Keep one bounded retry
    // deadline just below that ceiling so a SessionStart emitted while the UI
    // is still building can wait for the subscription listener to bind, while
    // still returning control before the provider kills the hook process.
    const HOOK_IPC_DEADLINE: std::time::Duration = std::time::Duration::from_millis(4500);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(io::Error::other)?;
    runtime.block_on(async {
        let mut last_error = None;
        let deadline = tokio::time::Instant::now() + HOOK_IPC_DEADLINE;
        let mut retry_delay = std::time::Duration::from_millis(50);
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            match tokio::time::timeout(remaining, send_cli_request(socket, &request)).await {
                Ok(Ok(())) => return Ok(()),
                Ok(Err(error)) => last_error = Some(error),
                Err(_) => {
                    last_error = Some(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "hook IPC retry deadline elapsed",
                    ));
                    break;
                }
            }

            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            tokio::time::sleep(retry_delay.min(remaining)).await;
            retry_delay = retry_delay
                .saturating_mul(2)
                .min(std::time::Duration::from_millis(250));
        }
        Err(last_error.unwrap_or_else(|| io::Error::other("hook IPC unavailable")))
    })
}

async fn send_cli_request(socket: &Path, request: &NotificationIpcRequest) -> io::Result<()> {
    let mut conn = unshit_ptyd::transport::connect(socket).await?;
    let bytes = serde_json::to_vec(request).map_err(io::Error::other)?;
    conn.write_all(&bytes).await?;
    conn.flush().await?;
    let mut ack = [0u8; 1];
    tokio::time::timeout(std::time::Duration::from_secs(2), conn.read_exact(&mut ack))
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "notification IPC ack timed out"))??;
    if ack[0] == 1 {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "notification IPC request rejected",
        ))
    }
}

pub fn default_notification_socket_path() -> PathBuf {
    // Namespace by instance profile: a dev/test instance must not bind
    // (or deliver activations to) the installed app's notify pipe.
    let name = match crate::profile::active_profile() {
        Some(tag) => format!("terminal-manager-notify-{tag}"),
        None => "terminal-manager-notify".to_string(),
    };
    #[cfg(windows)]
    {
        PathBuf::from(format!(r"\\.\pipe\{name}"))
    }
    #[cfg(unix)]
    {
        if let Ok(dir) = std::env::var("XDG_RUNTIME_DIR") {
            return PathBuf::from(dir).join(format!("{name}.sock"));
        }
        std::env::temp_dir().join(format!("{name}-{}.sock", current_euid()))
    }
}

fn is_valid_hook_capability(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn trusted_hook_socket(path: &Path) -> bool {
    let expected = default_notification_socket_path();
    #[cfg(windows)]
    {
        path.to_string_lossy()
            .eq_ignore_ascii_case(&expected.to_string_lossy())
    }
    #[cfg(unix)]
    {
        path == expected
    }
}

pub fn notification_socket_path() -> PathBuf {
    std::env::var_os(ENV_NOTIFY_SOCKET)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(default_notification_socket_path)
}

pub fn spawn_desktop_notification_for_target(
    title: impl Into<String>,
    text: impl Into<String>,
    workspace_id: u32,
    pane_id: u32,
) -> io::Result<()> {
    let desktop = DesktopNotification {
        title: title.into(),
        text: text.into(),
        workspace_id,
        pane_id,
        socket: notification_socket_path(),
    };
    spawn_desktop_notification(&desktop)
}

#[cfg(unix)]
fn current_euid() -> u32 {
    extern "C" {
        fn geteuid() -> u32;
    }
    unsafe { geteuid() }
}

pub fn notification_subscription(shared: SharedState) -> Subscription {
    Subscription::new(
        "notification-ipc".to_string(),
        move |_sink: EventSink| -> Pin<Box<dyn Stream<Item = ExternalEvent> + Send>> {
            let shared = shared.clone();
            Box::pin(async_stream::stream! {
                let socket = notification_socket_path();
                let mut server = match bind_notification_server(&socket).await {
                    Ok(server) => server,
                    Err(e) => {
                        log::warn!(
                            "notification IPC disabled; failed to bind {}: {}",
                            socket.display(),
                            e
                        );
                        return;
                    }
                };
                log::info!("terminal-manager notification IPC listening on {}", socket.display());

                loop {
                    let mut conn = match server.accept().await {
                        Ok(conn) => conn,
                        Err(e) => {
                            log::warn!("notification IPC accept failed: {e}");
                            continue;
                        }
                    };
                    let request = match read_request_to_end(&mut conn).await {
                        Ok(request) => request,
                        Err(e) => {
                            log::warn!("notification IPC request failed: {e}");
                            let _ = conn.write_all(&[0]).await;
                            continue;
                        }
                    };

                    let effect = apply_ipc_request(&shared, request, &socket);
                    let _ = conn.write_all(&[u8::from(effect.accepted)]).await;
                    let _ = conn.flush().await;
                    if effect.activate_window {
                        yield ExternalEvent::ActivateWindow;
                    }
                    if effect.rebuild {
                        yield ExternalEvent::RequestRebuild;
                    }
                }
            })
        },
    )
}

#[cfg(windows)]
async fn bind_notification_server(path: &Path) -> io::Result<unshit_ptyd::transport::Server> {
    unshit_ptyd::transport::Server::bind(path)
}

#[cfg(unix)]
async fn bind_notification_server(path: &Path) -> io::Result<unshit_ptyd::transport::Server> {
    unshit_ptyd::transport::Server::bind(path).await
}

async fn read_request_to_end<R>(reader: &mut R) -> io::Result<NotificationIpcRequest>
where
    R: AsyncRead + Unpin,
{
    const MAX_IPC_REQUEST_BYTES: usize = 128 * 1024;
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let count =
            tokio::time::timeout(std::time::Duration::from_secs(2), reader.read(&mut chunk))
                .await
                .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "IPC request timed out"))??;
        if count == 0 {
            break;
        }
        if bytes.len().saturating_add(count) > MAX_IPC_REQUEST_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "notification IPC request exceeds size limit",
            ));
        }
        bytes.extend_from_slice(&chunk[..count]);
        match serde_json::from_slice::<NotificationIpcRequest>(&bytes) {
            Ok(request) => return Ok(request),
            Err(error) if error.is_eof() => {}
            Err(error) => return Err(io::Error::new(io::ErrorKind::InvalidData, error)),
        }
    }
    serde_json::from_slice::<NotificationIpcRequest>(&bytes)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

#[derive(Default)]
struct IpcEffect {
    rebuild: bool,
    activate_window: bool,
    accepted: bool,
}

fn apply_ipc_request(
    shared: &SharedState,
    request: NotificationIpcRequest,
    socket: &Path,
) -> IpcEffect {
    let mut effect = IpcEffect::default();
    match request {
        NotificationIpcRequest::Notify {
            title,
            text,
            workspace_id,
            pane_id,
        } => {
            let desktop = DesktopNotification {
                title: title.clone(),
                text: text.clone(),
                workspace_id,
                pane_id,
                socket: socket.to_path_buf(),
            };
            mutate_with(shared, |state| {
                push_notification_toast(state, title, text, workspace_id, pane_id);
            });
            if let Err(e) = spawn_desktop_notification(&desktop) {
                log::warn!("desktop notification failed: {e}");
            }
            effect.rebuild = true;
            effect.accepted = true;
        }
        NotificationIpcRequest::Activate {
            workspace_id,
            pane_id,
        } => {
            let focused = mutate_with(shared, |state| {
                focus_workspace_pane_by_num(state, workspace_id, pane_id)
            });
            effect.rebuild = focused;
            effect.activate_window = true;
            effect.accepted = true;
            if !focused {
                log::warn!(
                    "notification activation target not found: workspace_id={} pane_id={}",
                    workspace_id,
                    pane_id
                );
            }
        }
        NotificationIpcRequest::AgentSessionObserved {
            agent,
            session_id,
            cwd,
            source,
            workspace_id,
            pane_id,
            capability,
        } => {
            let Some(source) = normalized_hook_source(&source) else {
                return effect;
            };
            let (changed, saved) = mutate_with(shared, |state| {
                if !pane_belongs_to_workspace(state, workspace_id, pane_id)
                    || !crate::agent_restore::is_valid_session_id(&session_id)
                    || !cwd.is_absolute()
                    || !is_valid_hook_capability(&capability)
                    || state.pty_manager.hook_capability(pane_id) != Some(capability.as_str())
                {
                    let mut event = crate::agent_restore::telemetry::RestoreEvent::new(
                        &state.restore_correlation_id,
                        crate::agent_restore::telemetry::EventName::HookRejected,
                        crate::agent_restore::telemetry::Level::Warn,
                    );
                    event.provider = Some(agent);
                    event.workspace_id = Some(workspace_id);
                    event.pane_id = Some(pane_id);
                    event.source = Some(source);
                    event.outcome = Some("rejected");
                    event.error_kind = Some("invalid_target_or_metadata");
                    crate::agent_restore::telemetry::record(&event);
                    return (false, false);
                }
                observe_session_with_persistence(
                    state,
                    pane_id,
                    workspace_id,
                    agent,
                    &session_id,
                    &cwd,
                    source,
                    |state| {
                        crate::state::persist_agent_metadata(
                            state,
                            agent,
                            workspace_id,
                            pane_id,
                            source,
                        )
                    },
                )
            });
            effect.rebuild = changed;
            effect.accepted = saved;
        }
        NotificationIpcRequest::NewAgent {
            profile,
            workspace_id,
        } => {
            let outcome = mutate_with(shared, |state| {
                apply_new_agent_request(state, profile.as_deref(), workspace_id)
            });
            effect.rebuild = outcome.is_ok();
            effect.activate_window = outcome.is_ok();
            effect.accepted = outcome.is_ok();
            if let Err(reason) = outcome {
                log::warn!(
                    "{{\"event\":\"agent.cli\",\"level\":\"warn\",\"outcome\":\"rejected\",\"reason\":{reason:?},\"workspace_id\":{workspace_id:?}}}"
                );
            }
        }
    }
    effect
}

/// Resolve and run a `NewAgent` IPC request against live state. Returns
/// the machine-readable rejection reason so both the log line and the
/// telemetry record name the same cause.
fn apply_new_agent_request(
    state: &mut crate::state::AppState,
    profile: Option<&str>,
    workspace_id: Option<u32>,
) -> Result<(), &'static str> {
    let correlation_id = state.restore_correlation_id.clone();
    let mut event =
        crate::agents::telemetry::AgentEventRecord::new("agent.cli", "info", &correlation_id);
    event.source = Some("cli");
    event.workspace_id = workspace_id;
    let ws_idx = match workspace_id {
        Some(id) => state.workspaces.iter().position(|w| w.num == id),
        None => Some(state.active_workspace),
    };
    let Some(ws_idx) = ws_idx else {
        event.level = "warn";
        event.outcome = Some("rejected");
        event.reason = Some("workspace_not_found");
        crate::agents::telemetry::record(&event);
        return Err("workspace_not_found");
    };
    let profile = match profile {
        None => crate::agents::default_profile(),
        Some(raw) => match crate::agents::parse_launchable_id(raw) {
            Some(profile) => profile,
            None => {
                event.level = "warn";
                event.outcome = Some("rejected");
                event.reason = Some("unknown_profile");
                crate::agents::telemetry::record(&event);
                return Err("unknown_profile");
            }
        },
    };
    event.profile = Some(profile.id);
    if ws_idx != state.active_workspace {
        crate::state::mutate_switch_workspace(state, ws_idx);
    }
    if !crate::state::mutate_add_agent_tab(state, profile, "cli") {
        event.level = "warn";
        event.outcome = Some("rejected");
        event.reason = Some("not_launchable");
        crate::agents::telemetry::record(&event);
        return Err("not_launchable");
    }
    crate::persist::save_workspaces(state);
    event.outcome = Some("opened");
    event.pane_id = Some(state.active_pane.0);
    crate::agents::telemetry::record(&event);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn observe_session_with_persistence(
    state: &mut crate::state::AppState,
    pane_id: u32,
    workspace_id: u32,
    agent: crate::agent_restore::AgentKind,
    session_id: &str,
    cwd: &Path,
    source: &'static str,
    persist: impl FnOnce(&mut crate::state::AppState) -> bool,
) -> (bool, bool) {
    // `observe_session` intentionally returns true for a valid duplicate.
    // This makes an ACK-lost delivery, or a retry after a transient write
    // failure, execute persistence again before receiving a positive ACK.
    let observed = crate::agent_restore::observe_session(
        state,
        pane_id,
        workspace_id,
        agent,
        session_id,
        cwd,
        source,
    );
    let saved = observed && persist(state);
    (observed, saved)
}

fn normalized_hook_source(source: &str) -> Option<&'static str> {
    match source {
        "startup" => Some("startup"),
        "resume" => Some("resume"),
        "clear" => Some("clear"),
        "compact" => Some("compact"),
        _ => None,
    }
}

fn session_hook_agent_from_args(args: &[OsString]) -> Option<crate::agent_restore::AgentKind> {
    if args.first()?.to_string_lossy() != "session-hook" {
        return None;
    }
    match args.get(1)?.to_string_lossy().as_ref() {
        "claude" => Some(crate::agent_restore::AgentKind::Claude),
        "codex" => Some(crate::agent_restore::AgentKind::Codex),
        _ => None,
    }
}

fn hook_transport_error_kind(error: &io::Error) -> &'static str {
    match error.kind() {
        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused => "listener_unavailable",
        io::ErrorKind::TimedOut => "timeout",
        io::ErrorKind::BrokenPipe | io::ErrorKind::ConnectionReset => "transport_lost",
        _ => "io_error",
    }
}

fn record_hook_cli_failure(
    agent: Option<crate::agent_restore::AgentKind>,
    source: &'static str,
    error_kind: &'static str,
) {
    let correlation_id = std::env::var(ENV_AGENT_RESTORE_CORRELATION_ID)
        .ok()
        .filter(|value| crate::agent_restore::is_valid_session_id(value))
        .unwrap_or_else(crate::agent_restore::generate_session_id);
    let mut event = crate::agent_restore::telemetry::RestoreEvent::new(
        &correlation_id,
        crate::agent_restore::telemetry::EventName::HookRejected,
        crate::agent_restore::telemetry::Level::Warn,
    );
    event.provider = agent;
    event.workspace_id = std::env::var(ENV_WORKSPACE_ID)
        .ok()
        .and_then(|value| value.parse().ok());
    event.pane_id = std::env::var(ENV_PANE_ID)
        .ok()
        .and_then(|value| value.parse().ok());
    event.source = Some(source);
    event.outcome = Some("ignored");
    event.error_kind = Some(error_kind);
    crate::agent_restore::telemetry::record(&event);
}

fn pane_belongs_to_workspace(
    state: &crate::state::AppState,
    workspace_id: u32,
    pane_id: u32,
) -> bool {
    let Some((workspace_index, workspace)) = state
        .workspaces
        .iter()
        .enumerate()
        .find(|(_, workspace)| workspace.num == workspace_id)
    else {
        return false;
    };
    if workspace_index == state.active_workspace {
        if state
            .panes
            .iter()
            .flatten()
            .any(|pane| pane.id.0 == pane_id)
        {
            return true;
        }
        return state.tabs.iter().enumerate().any(|(tab_index, tab)| {
            tab_index != state.active_tab
                && tab.panes.iter().flatten().any(|pane| pane.id.0 == pane_id)
        });
    }
    workspace
        .tabs
        .iter()
        .flat_map(|tab| tab.panes.iter().flatten())
        .any(|pane| pane.id.0 == pane_id)
}

struct DesktopNotification {
    title: String,
    text: String,
    workspace_id: u32,
    pane_id: u32,
    socket: PathBuf,
}

#[cfg(windows)]
fn spawn_desktop_notification(notification: &DesktopNotification) -> io::Result<()> {
    use std::os::windows::process::CommandExt;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const SCRIPT: &str = r#"
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
$notify = New-Object System.Windows.Forms.NotifyIcon
$notify.Icon = [System.Drawing.SystemIcons]::Information
$notify.BalloonTipTitle = $env:TM_NOTIFY_TITLE
$notify.BalloonTipText = $env:TM_NOTIFY_TEXT
$notify.Visible = $true
$script:clicked = $false
$activate = {
  $argsList = @('activate', '--socket', $env:TM_NOTIFY_CLICK_SOCKET, '--workspace-id', $env:TM_NOTIFY_WORKSPACE_ID, '--pane-id', $env:TM_NOTIFY_PANE_ID)
  Start-Process -FilePath $env:TM_NOTIFY_CLICK_EXE -ArgumentList $argsList -WindowStyle Hidden
  $script:clicked = $true
}
$notify.add_BalloonTipClicked($activate)
$notify.add_Click($activate)
$notify.ShowBalloonTip(10000)
$deadline = (Get-Date).AddSeconds(12)
while ((Get-Date) -lt $deadline -and -not $script:clicked) {
  [System.Windows.Forms.Application]::DoEvents()
  Start-Sleep -Milliseconds 100
}
$notify.Visible = $false
$notify.Dispose()
"#;

    let exe = std::env::current_exe()?;
    Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-WindowStyle",
            "Hidden",
            "-Command",
            SCRIPT,
        ])
        .env("TM_NOTIFY_CLICK_EXE", exe)
        .env("TM_NOTIFY_CLICK_SOCKET", &notification.socket)
        .env("TM_NOTIFY_TITLE", &notification.title)
        .env("TM_NOTIFY_TEXT", &notification.text)
        .env(
            "TM_NOTIFY_WORKSPACE_ID",
            notification.workspace_id.to_string(),
        )
        .env("TM_NOTIFY_PANE_ID", notification.pane_id.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map(|_| ())
}

#[cfg(not(windows))]
fn spawn_desktop_notification(notification: &DesktopNotification) -> io::Result<()> {
    log::info!(
        "desktop notification requested: title={:?} text={:?} workspace_id={} pane_id={} socket={}",
        notification.title,
        notification.text,
        notification.workspace_id,
        notification.pane_id,
        notification.socket.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicU64, Ordering};

    const CAPABILITY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn env_map(values: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = values
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        move |key| map.get(key).cloned()
    }

    fn unique_socket_path() -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let pid = std::process::id();
        #[cfg(windows)]
        {
            PathBuf::from(format!(r"\\.\pipe\terminal-manager-notify-test-{pid}-{n}"))
        }
        #[cfg(unix)]
        {
            std::env::temp_dir().join(format!("terminal-manager-notify-test-{pid}-{n}.sock"))
        }
    }

    #[test]
    fn parse_notify_uses_explicit_values() {
        let parsed = parse_cli_args(
            [
                "notify",
                "--title",
                "Done",
                "--text",
                "Agent finished",
                "--workspace-id",
                "7",
                "--pane-id",
                "3",
                "--socket",
                "custom.sock",
            ],
            env_map(&[]),
        )
        .expect("parse")
        .expect("command");

        assert_eq!(
            parsed,
            CliCommand::Notify {
                socket: PathBuf::from("custom.sock"),
                title: "Done".to_string(),
                text: "Agent finished".to_string(),
                target: NotificationTarget {
                    workspace_id: 7,
                    pane_id: 3,
                },
            }
        );
    }

    #[test]
    fn parse_notify_falls_back_to_terminal_env() {
        let parsed = parse_cli_args(
            ["notify", "--title", "Done", "--text", "Agent finished"],
            env_map(&[
                (ENV_NOTIFY_SOCKET, "from-env.sock"),
                (ENV_WORKSPACE_ID, "4"),
                (ENV_PANE_ID, "9"),
            ]),
        )
        .expect("parse")
        .expect("command");

        assert_eq!(
            parsed,
            CliCommand::Notify {
                socket: PathBuf::from("from-env.sock"),
                title: "Done".to_string(),
                text: "Agent finished".to_string(),
                target: NotificationTarget {
                    workspace_id: 4,
                    pane_id: 9,
                },
            }
        );
    }

    #[test]
    fn parse_activate_requires_target() {
        let err = parse_cli_args(["activate"], env_map(&[])).expect_err("target required");
        assert!(err.contains(ENV_WORKSPACE_ID), "{err}");
    }

    #[test]
    fn ipc_request_round_trips() {
        let request = NotificationIpcRequest::Notify {
            title: "Build".to_string(),
            text: "Done".to_string(),
            workspace_id: 1,
            pane_id: 2,
        };
        let bytes = serde_json::to_vec(&request).unwrap();
        let back: NotificationIpcRequest = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(back, request);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn send_cli_request_delivers_ipc_payload() {
        let socket = unique_socket_path();
        let mut server = bind_notification_server(&socket).await.unwrap();
        let request = NotificationIpcRequest::Activate {
            workspace_id: 9,
            pane_id: 4,
        };

        let client_socket = socket.clone();
        let sent = request.clone();
        let client = tokio::spawn(async move {
            send_cli_request(&client_socket, &sent).await.unwrap();
        });

        let mut conn = server.accept().await.unwrap();
        let got = read_request_to_end(&mut conn).await.unwrap();
        conn.write_all(&[1]).await.unwrap();
        conn.flush().await.unwrap();
        client.await.unwrap();

        assert_eq!(got, request);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn session_hook_waits_for_listener_during_cold_start() {
        let socket = unique_socket_path();
        let request = NotificationIpcRequest::AgentSessionObserved {
            agent: crate::agent_restore::AgentKind::Claude,
            session_id: "019f75b0-e94f-71f0-b1ea-f478f8438c1a".into(),
            cwd: std::env::current_dir().expect("cwd"),
            source: "startup".into(),
            workspace_id: 1,
            pane_id: 1,
            capability: CAPABILITY.into(),
        };
        let client_socket = socket.clone();
        let client = tokio::task::spawn_blocking(move || {
            send_hook_request_blocking(&client_socket, request)
        });

        // This exceeds the old ten-immediate-attempt window (~900 ms) and
        // models restoration work delaying subscription startup.
        tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
        let mut server = bind_notification_server(&socket)
            .await
            .expect("delayed listener bind");
        let mut conn = server.accept().await.expect("hook connection");
        let received = read_request_to_end(&mut conn).await.expect("hook request");
        conn.write_all(&[1]).await.expect("hook ack");
        conn.flush().await.expect("flush ack");

        assert!(matches!(
            received,
            NotificationIpcRequest::AgentSessionObserved { .. }
        ));
        client
            .await
            .expect("hook client task")
            .expect("hook delivery");
        drop(server);
        #[cfg(unix)]
        let _ = std::fs::remove_file(socket);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn session_hook_retries_nack_until_metadata_is_durably_accepted() {
        let socket = unique_socket_path();
        let mut server = bind_notification_server(&socket).await.expect("listener");
        let request = NotificationIpcRequest::AgentSessionObserved {
            agent: crate::agent_restore::AgentKind::Codex,
            session_id: "019f75b0-e94f-71f0-b1ea-f478f8438c1a".into(),
            cwd: std::env::current_dir().expect("cwd"),
            source: "resume".into(),
            workspace_id: 1,
            pane_id: 1,
            capability: CAPABILITY.into(),
        };
        let client_socket = socket.clone();
        let client = tokio::task::spawn_blocking(move || {
            send_hook_request_blocking(&client_socket, request)
        });

        let mut first = server.accept().await.expect("first hook attempt");
        let _ = read_request_to_end(&mut first)
            .await
            .expect("first request");
        first.write_all(&[0]).await.expect("transient nack");
        first.flush().await.expect("flush nack");
        drop(first);

        let mut retry = server.accept().await.expect("retried hook attempt");
        let _ = read_request_to_end(&mut retry)
            .await
            .expect("retried request");
        retry.write_all(&[1]).await.expect("durable ack");
        retry.flush().await.expect("flush ack");

        client.await.expect("hook client task").expect("hook retry");
        drop(server);
        #[cfg(unix)]
        let _ = std::fs::remove_file(socket);
    }

    #[test]
    fn notification_socket_path_uses_env_override() {
        let parsed = parse_cli_args(
            ["activate", "--workspace-id", "1", "--pane-id", "2"],
            env_map(&[(ENV_NOTIFY_SOCKET, "override.sock")]),
        )
        .expect("parse")
        .expect("command");

        assert_eq!(
            parsed,
            CliCommand::Activate {
                socket: PathBuf::from("override.sock"),
                target: NotificationTarget {
                    workspace_id: 1,
                    pane_id: 2,
                },
            }
        );
    }

    #[test]
    fn parse_session_hook_uses_only_terminal_target_environment() {
        let parsed = parse_cli_args(
            ["session-hook", "claude"],
            env_map(&[
                (ENV_NOTIFY_SOCKET, "hook.sock"),
                (ENV_WORKSPACE_ID, "4"),
                (ENV_PANE_ID, "9"),
                (ENV_AGENT_HOOK_CAPABILITY, CAPABILITY),
            ]),
        )
        .expect("parse")
        .expect("command");
        assert_eq!(
            parsed,
            CliCommand::SessionHook {
                socket: PathBuf::from("hook.sock"),
                agent: crate::agent_restore::AgentKind::Claude,
                target: Some(NotificationTarget {
                    workspace_id: 4,
                    pane_id: 9,
                }),
                capability: Some(CAPABILITY.to_string()),
            }
        );
        assert!(parse_cli_args(
            ["session-hook", "claude", "--pane-id", "9"],
            env_map(&[(ENV_WORKSPACE_ID, "4"), (ENV_PANE_ID, "9")]),
        )
        .is_err());
        assert!(parse_cli_args(
            ["session-hook", "claude", "--text", "must-not-be-accepted"],
            env_map(&[(ENV_WORKSPACE_ID, "4"), (ENV_PANE_ID, "9")]),
        )
        .is_err());
        assert_eq!(
            session_hook_agent_from_args(&["session-hook".into(), "codex".into()]),
            Some(crate::agent_restore::AgentKind::Codex)
        );
    }

    #[test]
    fn session_hook_outside_terminal_manager_is_a_silent_noop_target() {
        let parsed = parse_cli_args(["session-hook", "codex"], env_map(&[]))
            .expect("parse")
            .expect("command");
        assert!(matches!(
            parsed,
            CliCommand::SessionHook {
                agent: crate::agent_restore::AgentKind::Codex,
                target: None,
                ..
            }
        ));
    }

    #[test]
    fn hook_input_extracts_only_allowlisted_session_start_fields() {
        let cwd = std::env::current_dir().expect("cwd");
        let body = serde_json::json!({
            "session_id": "24C31FC8-8200-4773-8A0B-0447BD64BCDC",
            "cwd": cwd,
            "hook_event_name": "SessionStart",
            "source": "resume",
            "transcript_path": "secret.jsonl",
            "prompt": "do not retain this"
        })
        .to_string();
        let observed = read_session_hook_input(body.as_bytes()).expect("valid hook");
        assert_eq!(observed.session_id, "24c31fc8-8200-4773-8a0b-0447bd64bcdc");
        assert_eq!(observed.source, "resume");
    }

    #[test]
    fn hook_input_rejects_wrong_event_relative_cwd_and_oversize_body() {
        let wrong_event = serde_json::json!({
            "session_id": "24c31fc8-8200-4773-8a0b-0447bd64bcdc",
            "cwd": std::env::current_dir().expect("cwd"),
            "hook_event_name": "Stop",
            "source": "startup"
        })
        .to_string();
        assert!(read_session_hook_input(wrong_event.as_bytes()).is_err());

        let relative = serde_json::json!({
            "session_id": "24c31fc8-8200-4773-8a0b-0447bd64bcdc",
            "cwd": "relative/path",
            "hook_event_name": "SessionStart",
            "source": "startup"
        })
        .to_string();
        assert!(read_session_hook_input(relative.as_bytes()).is_err());
        assert!(read_session_hook_input(vec![b'x'; 64 * 1024 + 1].as_slice()).is_err());
    }

    #[test]
    fn agent_session_ipc_has_no_transcript_or_prompt_fields() {
        let request = NotificationIpcRequest::AgentSessionObserved {
            agent: crate::agent_restore::AgentKind::Codex,
            session_id: "24c31fc8-8200-4773-8a0b-0447bd64bcdc".into(),
            cwd: std::env::current_dir().expect("cwd"),
            source: "startup".into(),
            workspace_id: 1,
            pane_id: 1,
            capability: CAPABILITY.into(),
        };
        let body = serde_json::to_string(&request).expect("serialize");
        assert!(!body.contains("transcript"));
        assert!(!body.contains("prompt"));
        assert_eq!(
            serde_json::from_str::<NotificationIpcRequest>(&body).expect("deserialize"),
            request
        );
    }

    #[test]
    fn observed_session_updates_matching_pane_once_and_rejects_stale_target() {
        let mut state = crate::state::seed_state();
        state.pty_manager.test_set_hook_capability(1, CAPABILITY);
        let shared = std::sync::Arc::new(std::sync::Mutex::new(state));
        let cwd = std::env::current_dir().expect("cwd");
        let request = NotificationIpcRequest::AgentSessionObserved {
            agent: crate::agent_restore::AgentKind::Claude,
            session_id: "24c31fc8-8200-4773-8a0b-0447bd64bcdc".into(),
            cwd: cwd.clone(),
            source: "startup".into(),
            workspace_id: 1,
            pane_id: 1,
            capability: CAPABILITY.into(),
        };
        let socket = Path::new("unused.sock");
        let mut forged = request.clone();
        if let NotificationIpcRequest::AgentSessionObserved { capability, .. } = &mut forged {
            *capability = "f".repeat(64);
        }
        assert!(!apply_ipc_request(&shared, forged, socket).accepted);
        assert!(!shared
            .lock()
            .expect("state")
            .agent_restarts
            .contains_key(&1));
        let first = apply_ipc_request(&shared, request.clone(), socket);
        assert!(first.rebuild);
        assert!(first.accepted);
        let duplicate_after_ack_loss = apply_ipc_request(&shared, request, socket);
        assert!(duplicate_after_ack_loss.rebuild);
        assert!(duplicate_after_ack_loss.accepted);
        let guard = shared.lock().expect("state");
        assert_eq!(
            guard.agent_restarts[&1].session_id.as_deref(),
            Some("24c31fc8-8200-4773-8a0b-0447bd64bcdc")
        );
        drop(guard);

        let stale = NotificationIpcRequest::AgentSessionObserved {
            agent: crate::agent_restore::AgentKind::Claude,
            session_id: "3b9436f8-7a22-4289-bf8a-bce97853cb79".into(),
            cwd,
            source: "startup".into(),
            workspace_id: 1,
            pane_id: 999,
            capability: CAPABILITY.into(),
        };
        assert!(!apply_ipc_request(&shared, stale, socket).rebuild);
        assert!(!shared
            .lock()
            .expect("state")
            .agent_restarts
            .contains_key(&999));
    }

    #[test]
    fn hook_retry_persists_again_after_first_save_failure() {
        let mut state = crate::state::seed_state();
        let cwd = std::env::current_dir().expect("cwd");
        let session_id = "019f75b0-e94f-71f0-b1ea-f478f8438c1a";

        let first = observe_session_with_persistence(
            &mut state,
            1,
            1,
            crate::agent_restore::AgentKind::Claude,
            session_id,
            &cwd,
            "startup",
            |_| false,
        );
        assert_eq!(first, (true, false));

        let retry = observe_session_with_persistence(
            &mut state,
            1,
            1,
            crate::agent_restore::AgentKind::Claude,
            session_id,
            &cwd,
            "startup",
            |_| true,
        );
        assert_eq!(retry, (true, true));
        assert_eq!(
            state.agent_restarts[&1].session_id.as_deref(),
            Some(session_id)
        );
    }
}

#[cfg(test)]
mod agents_tab_tests {
    use super::*;
    use std::collections::HashMap;

    fn env_map(values: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = values
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        move |key| map.get(key).cloned()
    }

    #[test]
    fn parse_agent_command_takes_profile_and_workspace_from_flags_or_env() {
        let parsed = parse_cli_args(
            [
                "agent",
                "codex",
                "--workspace-id",
                "3",
                "--socket",
                "s.sock",
            ],
            |_| None,
        )
        .expect("parse")
        .expect("command");
        assert_eq!(
            parsed,
            CliCommand::NewAgent {
                socket: PathBuf::from("s.sock"),
                profile: Some("codex".into()),
                workspace_id: Some(3),
            }
        );

        let parsed = parse_cli_args(
            ["agent"],
            env_map(&[
                (ENV_NOTIFY_SOCKET, "env.sock"),
                (ENV_WORKSPACE_ID, "7"),
                (ENV_PANE_ID, "2"),
            ]),
        )
        .expect("parse")
        .expect("command");
        assert_eq!(
            parsed,
            CliCommand::NewAgent {
                socket: PathBuf::from("env.sock"),
                profile: None,
                workspace_id: Some(7),
            }
        );

        let outside = parse_cli_args(["new-agent"], |_| None)
            .expect("parse")
            .expect("command");
        assert!(matches!(
            outside,
            CliCommand::NewAgent {
                profile: None,
                workspace_id: None,
                ..
            }
        ));
    }

    #[test]
    fn parse_agent_command_rejects_unknown_profiles_and_notification_flags() {
        let err = parse_cli_args(["agent", "nope"], |_| None).expect_err("unknown agent");
        assert!(err.contains("unknown agent"), "{err}");
        assert!(err.contains("claude"), "{err}");
        assert!(parse_cli_args(["agent", "claude", "--title", "x"], |_| None).is_err());
        assert!(parse_cli_args(["agent", "claude", "codex"], |_| None).is_err());
        assert!(parse_cli_args(["agent", "openrouter"], |_| None).is_err());
    }

    #[test]
    fn new_agent_ipc_round_trips_with_snake_case_kind() {
        let request = NotificationIpcRequest::NewAgent {
            profile: Some("claude".into()),
            workspace_id: Some(2),
        };
        let body = serde_json::to_string(&request).expect("serialize");
        assert!(body.contains(r#""kind":"new_agent""#), "{body}");
        assert_eq!(
            serde_json::from_str::<NotificationIpcRequest>(&body).expect("deserialize"),
            request
        );
        let bare: NotificationIpcRequest =
            serde_json::from_str(r#"{"kind":"new_agent"}"#).expect("bare request");
        assert_eq!(
            bare,
            NotificationIpcRequest::NewAgent {
                profile: None,
                workspace_id: None,
            }
        );
    }

    #[test]
    fn new_agent_ipc_opens_a_tab_in_the_named_workspace_and_activates_the_window() {
        let state = crate::state::seed_state();
        let target_num = state.workspaces[1].num;
        let shared = std::sync::Arc::new(std::sync::Mutex::new(state));
        let effect = apply_ipc_request(
            &shared,
            NotificationIpcRequest::NewAgent {
                profile: Some("codex".into()),
                workspace_id: Some(target_num),
            },
            Path::new("unused.sock"),
        );
        assert!(effect.accepted);
        assert!(effect.rebuild);
        assert!(effect.activate_window);
        let guard = shared.lock().expect("state");
        assert_eq!(guard.active_workspace, 1);
        assert_eq!(guard.pane_agents[&guard.active_pane.0].profile, "codex");
    }

    #[test]
    fn new_agent_ipc_rejects_unknown_workspace_or_title_only_profile() {
        let shared = std::sync::Arc::new(std::sync::Mutex::new(crate::state::seed_state()));
        let tabs_before = shared.lock().expect("state").tabs.len();
        let effect = apply_ipc_request(
            &shared,
            NotificationIpcRequest::NewAgent {
                profile: None,
                workspace_id: Some(999),
            },
            Path::new("unused.sock"),
        );
        assert!(!effect.accepted);
        assert!(!effect.activate_window);
        let effect = apply_ipc_request(
            &shared,
            NotificationIpcRequest::NewAgent {
                profile: Some("openrouter".into()),
                workspace_id: None,
            },
            Path::new("unused.sock"),
        );
        assert!(!effect.accepted);
        assert_eq!(shared.lock().expect("state").tabs.len(), tabs_before);
    }
}
