//! Platform IPC transport.
//!
//! The daemon and the client share one trait-object-free API: each
//! platform module exposes a `Server` that yields connections and a
//! `connect` free function used by clients. Callers thread the same
//! `socket_path: &Path` through both.

use std::path::PathBuf;

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
    default_socket_path_for_instance(None)
}

/// Default socket path namespaced by an instance profile.
///
/// `None` is the shared per-user default. `Some(name)` appends a
/// sanitized `-{name}` so parallel instances (an installed app, a dev
/// build, a test run) each get their own daemon instead of attaching
/// to one another's sessions.
pub fn default_socket_path_for_instance(instance: Option<&str>) -> PathBuf {
    #[cfg(windows)]
    {
        default_named_pipe_path(instance)
    }
    #[cfg(unix)]
    {
        let name = match instance.map(sanitize_socket_component) {
            Some(tag) if !tag.is_empty() => format!("unshit-ptyd-{tag}"),
            _ => "unshit-ptyd".to_string(),
        };
        if let Ok(dir) = std::env::var("XDG_RUNTIME_DIR") {
            return PathBuf::from(dir).join(format!("{name}.sock"));
        }
        std::env::temp_dir().join(format!("{name}-{}.sock", current_euid()))
    }
}

#[cfg(windows)]
fn default_named_pipe_path(instance: Option<&str>) -> PathBuf {
    let user = windows_user_pipe_suffix(
        std::env::var_os("USERDOMAIN").as_deref(),
        std::env::var_os("USERNAME").as_deref(),
    );
    match instance.map(sanitize_socket_component) {
        Some(tag) if !tag.is_empty() => {
            PathBuf::from(format!(r"\\.\pipe\unshit-ptyd-{user}-{tag}"))
        }
        _ => PathBuf::from(format!(r"\\.\pipe\unshit-ptyd-{user}")),
    }
}

/// Instance tags come from user-controlled env (`TM_PROFILE`), so they
/// go through the same alphanumeric-only sanitizer as the user suffix.
fn sanitize_socket_component(raw: &str) -> String {
    sanitize_pipe_component(raw)
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

    #[cfg(windows)]
    #[test]
    fn windows_instance_suffix_namespaces_the_pipe() {
        let base = default_socket_path();
        let dev = default_socket_path_for_instance(Some("dev"));
        assert_eq!(
            dev.to_string_lossy(),
            format!("{}-dev", base.to_string_lossy()),
            "instance pipe must be the user pipe plus a -instance suffix"
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_instance_suffix_is_sanitized() {
        let dev = default_socket_path_for_instance(Some("Test Run/7"));
        let text = dev.to_string_lossy();
        assert!(
            text.ends_with("-test_run_7"),
            "instance tag must be pipe-safe: {text}"
        );
    }

    #[test]
    fn empty_or_unset_instance_is_the_shared_default() {
        assert_eq!(
            default_socket_path_for_instance(None),
            default_socket_path()
        );
        assert_eq!(
            default_socket_path_for_instance(Some("  ")),
            default_socket_path(),
            "an instance that sanitizes to empty must not add a dangling suffix"
        );
    }
}
