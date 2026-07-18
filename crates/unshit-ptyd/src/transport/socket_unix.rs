//! Unix-domain-socket transport.
//!
//! Single-instance guard: probe-then-bind. If `connect` succeeds, a
//! daemon is already alive and we error out. If `connect` fails with
//! `ENOENT` or `ECONNREFUSED` we treat the socket file as stale, remove
//! it, and bind. Any other error propagates.

use std::io;
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

use tokio::net::{UnixListener, UnixStream};

pub type Connection = UnixStream;

/// Client-side connection returned by [`connect`]. On Unix the same
/// `UnixStream` type serves both ends; the alias exists to match the
/// Windows API shape.
pub type ClientConnection = UnixStream;

#[derive(Debug)]
pub struct Server {
    listener: UnixListener,
    path: PathBuf,
}

impl Server {
    /// Binds to `path` after a liveness probe.
    pub async fn bind(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        if path.exists() {
            match UnixStream::connect(&path).await {
                Ok(_) => {
                    return Err(io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        "another daemon owns this socket",
                    ));
                }
                Err(e)
                    if e.kind() == io::ErrorKind::NotFound
                        || e.kind() == io::ErrorKind::ConnectionRefused =>
                {
                    std::fs::remove_file(&path).ok();
                }
                Err(e) => return Err(e),
            }
        }
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let listener = UnixListener::bind(&path)?;
        if let Err(error) = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
        {
            drop(listener);
            let _ = std::fs::remove_file(&path);
            return Err(error);
        }
        Ok(Self { listener, path })
    }

    pub async fn accept(&mut self) -> io::Result<Connection> {
        loop {
            let (stream, _addr) = self.listener.accept().await?;
            match stream.peer_cred() {
                Ok(credentials)
                    if ensure_same_owner(super::current_euid(), credentials.uid()).is_ok() =>
                {
                    return Ok(stream);
                }
                Ok(_) => log::warn!(
                    "{{\"event\":\"ptyd.transport.peer_rejected\",\"level\":\"warn\",\"error_kind\":\"owner_mismatch\"}}"
                ),
                Err(_) => log::warn!(
                    "{{\"event\":\"ptyd.transport.peer_rejected\",\"level\":\"warn\",\"error_kind\":\"credential_unavailable\"}}"
                ),
            }
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        // Remove the socket file so the next daemon can bind cleanly.
        std::fs::remove_file(&self.path).ok();
    }
}

pub async fn connect(path: impl AsRef<Path>) -> io::Result<UnixStream> {
    let path = path.as_ref();
    let stream = UnixStream::connect(path).await?;
    verify_server_owner(&stream, path)?;
    Ok(stream)
}

/// Verify the connected process, not just the predictable socket pathname.
/// A different local account can create that name before startup, but cannot
/// forge the kernel-provided peer uid or chown its socket to this uid.
fn verify_server_owner(stream: &UnixStream, path: &Path) -> io::Result<()> {
    let expected_uid = super::current_euid();
    let peer_uid = stream.peer_cred()?.uid();
    ensure_same_owner(expected_uid, peer_uid)?;

    let metadata = std::fs::symlink_metadata(path)?;
    let private_socket = metadata.file_type().is_socket()
        && metadata.uid() == expected_uid
        && metadata.permissions().mode() & 0o077 == 0;
    if private_socket {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "local IPC endpoint is not an owner-only socket",
        ))
    }
}

fn ensure_same_owner(expected_uid: u32, actual_uid: u32) -> io::Result<()> {
    if expected_uid == actual_uid {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "local IPC server belongs to a different user",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn unique_socket_path() -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let pid = std::process::id();
        std::env::temp_dir().join(format!("unshit-ptyd-test-{pid}-{n}.sock"))
    }

    #[tokio::test]
    async fn client_and_server_exchange_bytes() {
        let path = unique_socket_path();
        let mut server = Server::bind(&path).await.unwrap();

        let client_path = path.clone();
        let client_task = tokio::spawn(async move {
            let mut c = connect(&client_path).await.unwrap();
            c.write_all(b"ping").await.unwrap();
            let mut buf = [0u8; 4];
            c.read_exact(&mut buf).await.unwrap();
            buf
        });

        let mut conn = server.accept().await.unwrap();
        let mut got = [0u8; 4];
        conn.read_exact(&mut got).await.unwrap();
        assert_eq!(&got, b"ping");
        conn.write_all(b"pong").await.unwrap();

        let client_got = client_task.await.unwrap();
        assert_eq!(&client_got, b"pong");
    }

    #[tokio::test]
    async fn stale_socket_file_is_replaced() {
        let path = unique_socket_path();
        // Create a stale regular file at the path: no listener.
        std::fs::write(&path, b"").unwrap();
        let _server = Server::bind(&path).await.unwrap();
    }

    #[tokio::test]
    async fn second_bind_with_live_server_is_rejected() {
        let path = unique_socket_path();
        let _first = Server::bind(&path).await.unwrap();
        let err = Server::bind(&path).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists, "{err:?}");
    }

    #[tokio::test]
    async fn bound_socket_is_accessible_only_to_its_owner() {
        let path = unique_socket_path();
        let _server = Server::bind(&path).await.unwrap();

        let permissions = std::fs::metadata(&path)
            .expect("socket metadata")
            .permissions();
        let mode = std::os::unix::fs::PermissionsExt::mode(&permissions) & 0o777;
        assert_eq!(mode, 0o600, "socket mode must be owner-only");
    }

    #[test]
    fn mismatched_socket_server_owner_is_rejected() {
        let error = ensure_same_owner(1000, 1001).expect_err("different uid must be rejected");
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    }
}
