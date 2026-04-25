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
/// On Windows this is a named pipe; on Unix a filesystem socket under
/// `$XDG_RUNTIME_DIR` or `$TMPDIR`. See SPEC.md section 4.
pub fn default_socket_path() -> PathBuf {
    #[cfg(windows)]
    {
        PathBuf::from(r"\\.\pipe\unshit-ptyd")
    }
    #[cfg(unix)]
    {
        if let Ok(dir) = std::env::var("XDG_RUNTIME_DIR") {
            return PathBuf::from(dir).join("unshit-ptyd.sock");
        }
        std::env::temp_dir().join(format!("unshit-ptyd-{}.sock", current_euid()))
    }
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
