//! Windows named-pipe transport.
//!
//! Tokio's named-pipe server works in rounds: each accept requires a
//! fresh `NamedPipeServer` built with the same path. The first instance
//! is created with `first_pipe_instance(true)` so a second bind on the
//! same path fails with `ERROR_ACCESS_DENIED`. That is our single-
//! instance guard per SPEC.md section 2.

use std::io;
use std::path::{Path, PathBuf};

use tokio::net::windows::named_pipe::{
    ClientOptions, NamedPipeClient, NamedPipeServer, ServerOptions,
};

/// Server-side connection handed out by `Server::accept`.
pub type Connection = NamedPipeServer;

/// Client-side connection returned by [`connect`].
pub type ClientConnection = NamedPipeClient;

/// Listens on a named pipe and yields connections one at a time.
///
/// Holds the path so we can keep re-creating server instances across
/// accepts; the pipe is torn down when `Server` is dropped.
pub struct Server {
    path: PathBuf,
    // Pending instance waiting for a client. `None` between `accept`
    // calls, populated again on the next call.
    pending: Option<NamedPipeServer>,
}

impl Server {
    /// Binds to `path`. Fails with `AlreadyExists` if another daemon
    /// already owns this pipe.
    pub fn bind(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        let server = create_instance(&path, true)?;
        Ok(Self {
            path,
            pending: Some(server),
        })
    }

    /// Waits for a client to connect and returns the resulting
    /// connection. The next accept rebuilds a fresh pending instance
    /// so we keep serving.
    pub async fn accept(&mut self) -> io::Result<Connection> {
        let server = self
            .pending
            .take()
            .expect("pending instance must always be populated between accepts");
        server.connect().await?;
        // Prepare the next instance so the path stays owned by us.
        self.pending = Some(create_instance(&self.path, false)?);
        Ok(server)
    }
}

fn create_instance(path: &Path, first: bool) -> io::Result<NamedPipeServer> {
    let mut opts = ServerOptions::new();
    opts.first_pipe_instance(first);
    opts.create(path).map_err(|e| {
        if first && e.kind() == io::ErrorKind::PermissionDenied {
            io::Error::new(
                io::ErrorKind::AlreadyExists,
                "another daemon owns this pipe",
            )
        } else {
            e
        }
    })
}

/// Connects to a daemon already listening on `path`.
pub async fn connect(path: impl AsRef<Path>) -> io::Result<ClientConnection> {
    ClientOptions::new().open(path.as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn unique_pipe_path() -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let pid = std::process::id();
        PathBuf::from(format!(r"\\.\pipe\unshit-ptyd-test-{pid}-{n}"))
    }

    #[tokio::test]
    async fn client_and_server_exchange_bytes() {
        let path = unique_pipe_path();
        let mut server = Server::bind(&path).unwrap();

        let client_path = path.clone();
        let client_task = tokio::spawn(async move {
            // Retry briefly because the server might not be waiting yet.
            for _ in 0..50 {
                match connect(&client_path).await {
                    Ok(mut c) => {
                        c.write_all(b"ping").await.unwrap();
                        let mut buf = [0u8; 4];
                        c.read_exact(&mut buf).await.unwrap();
                        return buf;
                    }
                    Err(_) => tokio::time::sleep(std::time::Duration::from_millis(5)).await,
                }
            }
            panic!("client could not connect");
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
    async fn second_bind_on_same_path_is_rejected() {
        let path = unique_pipe_path();
        let _first = Server::bind(&path).unwrap();
        match Server::bind(&path) {
            Ok(_) => panic!("second bind should have failed"),
            Err(e) => assert_eq!(
                e.kind(),
                io::ErrorKind::AlreadyExists,
                "second bind should surface AlreadyExists: {e:?}"
            ),
        }
    }
}
