//! Local notification IPC and CLI entry points.
//!
//! The running UI owns a local pipe/socket. Child processes inside managed
//! terminals call `terminal-manager notify ...`; the short-lived CLI process
//! sends a JSON request to that endpoint and exits. The UI subscription mutates
//! app state and yields framework events to repaint or activate the window.

use std::ffi::OsString;
use std::io;
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CliMode {
    Notify,
    Activate,
}

#[derive(Default)]
struct CliFields {
    title: Option<String>,
    text: Option<String>,
    socket: Option<PathBuf>,
    workspace_id: Option<u32>,
    pane_id: Option<u32>,
}

pub fn handle_cli_from_env<I, S>(args: I) -> Option<i32>
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    let command = match parse_cli_args(args, |key| std::env::var(key).ok()) {
        Ok(Some(command)) => command,
        Ok(None) => return None,
        Err(e) => {
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
    };

    match result {
        Ok(()) => Some(0),
        Err(e) => {
            eprintln!("terminal-manager notification error: {e}");
            Some(1)
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
        _ => return Ok(None),
    };

    let mut fields = CliFields::default();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => return Err(notification_usage().to_string()),
            "--title" => fields.title = Some(take_value(&mut args, "--title")?),
            "--text" | "--body" | "--message" => {
                fields.text = Some(take_value(&mut args, arg.as_str())?)
            }
            "--socket" => fields.socket = Some(PathBuf::from(take_value(&mut args, "--socket")?)),
            "--workspace-id" | "--workspace" => {
                fields.workspace_id = Some(parse_u32_flag(&mut args, arg.as_str())?)
            }
            "--pane-id" | "--pane" => {
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
                _ => return Err(format!("unexpected positional argument {positional:?}")),
            },
        }
    }

    let socket = fields
        .socket
        .or_else(|| get_env(ENV_NOTIFY_SOCKET).map(PathBuf::from))
        .unwrap_or_else(default_notification_socket_path);
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

    match mode {
        CliMode::Notify => Ok(Some(CliCommand::Notify {
            socket,
            title: require_non_empty(fields.title, "--title")?,
            text: require_non_empty(fields.text, "--text")?,
            target,
        })),
        CliMode::Activate => Ok(Some(CliCommand::Activate { socket, target })),
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
    "usage: terminal-manager notify --title <title> --text <text> [--workspace-id <id>] [--pane-id <id>] [--socket <path>]\n       terminal-manager activate [--workspace-id <id>] [--pane-id <id>] [--socket <path>]"
}

fn send_cli_request_blocking(socket: &Path, request: NotificationIpcRequest) -> io::Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(io::Error::other)?;
    runtime.block_on(send_cli_request(socket, &request))
}

async fn send_cli_request(socket: &Path, request: &NotificationIpcRequest) -> io::Result<()> {
    let mut conn = unshit_ptyd::transport::connect(socket).await?;
    let bytes = serde_json::to_vec(request).map_err(io::Error::other)?;
    conn.write_all(&bytes).await?;
    conn.flush().await
}

pub fn default_notification_socket_path() -> PathBuf {
    #[cfg(windows)]
    {
        PathBuf::from(r"\\.\pipe\terminal-manager-notify")
    }
    #[cfg(unix)]
    {
        if let Ok(dir) = std::env::var("XDG_RUNTIME_DIR") {
            return PathBuf::from(dir).join("terminal-manager-notify.sock");
        }
        std::env::temp_dir().join(format!("terminal-manager-notify-{}.sock", current_euid()))
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
                            continue;
                        }
                    };

                    let effect = apply_ipc_request(&shared, request, &socket);
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
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).await?;
    serde_json::from_slice::<NotificationIpcRequest>(&bytes)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

#[derive(Default)]
struct IpcEffect {
    rebuild: bool,
    activate_window: bool,
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
            if !focused {
                log::warn!(
                    "notification activation target not found: workspace_id={} pane_id={}",
                    workspace_id,
                    pane_id
                );
            }
        }
    }
    effect
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
        client.await.unwrap();

        assert_eq!(got, request);
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
}
