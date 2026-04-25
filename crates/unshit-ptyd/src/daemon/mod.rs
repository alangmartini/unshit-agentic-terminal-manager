//! Daemon event loop.
//!
//! Binds the transport, then accepts connections in a loop and hands
//! each one to `handler::serve_connection`. A shutdown-requested signal
//! from any handler tells the outer loop to stop accepting and exit
//! once in-flight handlers finish.
//!
//! The loop never panics. Per-handler failures are logged and do not
//! affect sibling connections or the supervisor; see SPEC.md section
//! 8 ("The daemon must never panic out of its main loop").

use std::io;
use std::path::Path;
use std::sync::Arc;

use tokio::sync::broadcast;

use crate::session::registry::SessionRegistry;
use crate::transport::Server;

pub mod handler;

/// Runs the daemon until a shutdown is requested or the listener dies.
///
/// The listener lives for the duration of this call. A second
/// `Daemon::run` on the same path fails with `AlreadyExists` from
/// [`Server::bind`], which is the single-instance guard.
pub async fn run(socket_path: &Path) -> io::Result<()> {
    let mut server = bind_server(socket_path).await?;
    log::info!("unshit-ptyd listening on {}", socket_path.display());

    // Capacity of 16 is plenty: the only thing we ever send is the
    // shutdown nudge, and receivers consume it once.
    let (shutdown_tx, mut shutdown_rx) = broadcast::channel::<()>(16);

    // Slice 5: one registry shared across every connection so sessions
    // survive client disconnect. Each handler attaches and detaches
    // against this shared state.
    let registry = Arc::new(SessionRegistry::new());

    loop {
        tokio::select! {
            _ = shutdown_rx.recv() => {
                log::info!("shutdown requested; stopping accept loop");
                return Ok(());
            }
            accept = server.accept() => {
                match accept {
                    Ok(conn) => {
                        let tx = shutdown_tx.clone();
                        let reg = registry.clone();
                        tokio::spawn(async move {
                            if let Err(e) = handler::serve_connection(conn, tx, reg).await {
                                log::warn!("connection error: {}", e);
                            }
                        });
                    }
                    Err(e) => {
                        log::error!("accept failed: {}", e);
                        return Err(e);
                    }
                }
            }
        }
    }
}

#[cfg(windows)]
async fn bind_server(path: &Path) -> io::Result<Server> {
    Server::bind(path)
}

#[cfg(unix)]
async fn bind_server(path: &Path) -> io::Result<Server> {
    Server::bind(path).await
}
