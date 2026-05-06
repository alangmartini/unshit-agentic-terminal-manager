//! Platform IPC transport.
//!
//! The daemon and the client share one trait-object-free API: each
//! platform module exposes a `Server` that yields connections and a
//! `connect` free function used by clients. Callers thread the same
//! `socket_path: &Path` through both.

use std::path::PathBuf;

use crate::protocol::PROTOCOL_VERSION;

#[cfg(windows)]
pub mod pipe_windows;
#[cfg(windows)]
pub use pipe_windows::{connect, ClientConnection, Connection, Server};

#[cfg(unix)]
pub mod socket_unix;
#[cfg(unix)]
pub use socket_unix::{connect, ClientConnection, Connection, Server};

/// Default path the daemon binds when the caller does not override it.
///
/// On Windows this is a user-scoped named pipe; on Unix a filesystem
/// socket under `$XDG_RUNTIME_DIR` or `$TMPDIR`. See SPEC.md section 4.
pub fn default_socket_path() -> PathBuf {
    #[cfg(windows)]
    {
        default_named_pipe_path()
    }
    #[cfg(unix)]
    {
        if let Ok(dir) = std::env::var("XDG_RUNTIME_DIR") {
            return PathBuf::from(dir).join(format!("unshit-ptyd-v{PROTOCOL_VERSION}.sock"));
        }
        std::env::temp_dir().join(format!(
            "unshit-ptyd-v{PROTOCOL_VERSION}-{}.sock",
            current_euid()
        ))
    }
}

#[cfg(windows)]
fn default_named_pipe_path() -> PathBuf {
    let user = windows_user_pipe_suffix(
        std::env::var_os("USERDOMAIN").as_deref(),
        std::env::var_os("USERNAME").as_deref(),
    );
    PathBuf::from(format!(r"\\.\pipe\unshit-ptyd-v{PROTOCOL_VERSION}-{user}"))
}

#[cfg(windows)]
fn windows_user_pipe_suffix(
    domain: Option<&std::ffi::OsStr>,
    username: Option<&std::ffi::OsStr>,
) -> String {
    let raw = [domain, username]
        .into_iter()
        .flatten()
        .map(|part| part.to_string_lossy())
        .filter(|part| !part.trim().is_empty())
        .collect::<Vec<_>>()
        .join("-");
    sanitize_pipe_component(if raw.is_empty() { "user" } else { &raw })
}

#[cfg(windows)]
fn sanitize_pipe_component(raw: &str) -> String {
    let sanitized: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    sanitized.trim_matches('_').to_string()
}

#[cfg(unix)]
fn current_euid() -> u32 {
    // Avoid pulling the libc crate for a single syscall. `geteuid` is
    // in every libc on every Unix we care about and its signature is
    // stable across platforms.
    extern "C" {
        fn geteuid() -> u32;
    }
    unsafe { geteuid() }
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    use super::*;

    #[cfg(windows)]
    #[test]
    fn windows_default_socket_path_is_user_scoped() {
        let path = default_socket_path();
        let text = path.to_string_lossy();
        assert!(
            text.starts_with(r"\\.\pipe\unshit-ptyd-"),
            "default pipe must include a user suffix: {text}"
        );
        assert_ne!(
            text.as_ref(),
            r"\\.\pipe\unshit-ptyd",
            "default pipe must not be machine-global"
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_user_pipe_suffix_is_stable_and_pipe_safe() {
        let suffix = windows_user_pipe_suffix(
            Some(std::ffi::OsStr::new("DESKTOP-PHC7C66")),
            Some(std::ffi::OsStr::new("Alan Beelink")),
        );
        assert_eq!(suffix, "desktop_phc7c66_alan_beelink");
    }
}
