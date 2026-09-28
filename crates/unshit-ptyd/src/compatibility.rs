//! Read-only installer preflight. Uses the same protocol range as the UI.

use std::{io, path::Path, time::Duration};

use crate::{client::Client, protocol::Response};

/// Versions 1 and 2 remain usable through the UI's existing feature gates.
/// Increase the minimum only when those fallback paths are removed. Unknown
/// newer protocols are rejected until their compatibility has been reviewed.
pub fn check_protocol(version: u32) -> io::Result<()> {
    if (1..=crate::protocol::PROTOCOL_VERSION).contains(&version) {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!("daemon protocol {version} is incompatible with this UI; leave the update pending until terminal sessions have finished"),
        ))
    }
}

/// A missing endpoint is safe: the UI can start its bundled daemon. An
/// occupied, unresponsive or inaccessible endpoint is never assumed absent.
pub async fn check_running_daemon(path: &Path) -> io::Result<()> {
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut attempts = 0;
        let mut client = loop {
            match Client::connect(path).await {
                Ok(client) => break client,
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
                Err(error) if attempts < 20 && retryable_connect(&error) => {
                    attempts += 1;
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
                Err(error) => return Err(error),
            }
        };
        match client.hello(crate::DAEMON_VERSION).await {
            Ok(Response::HelloAck {
                protocol_version, ..
            }) => check_protocol(protocol_version),
            Ok(_) => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "unexpected daemon greeting",
            )),
            Err(error) => Err(io::Error::other(error.to_string())),
        }
    })
    .await
    .map_err(|_| {
        io::Error::new(
            io::ErrorKind::TimedOut,
            "daemon compatibility check timed out",
        )
    })?
}

fn retryable_connect(error: &io::Error) -> bool {
    #[cfg(windows)]
    if error.raw_os_error() == Some(231) {
        // ERROR_PIPE_BUSY
        return true;
    }
    matches!(
        error.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::ConnectionRefused
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compatible_protocol_range_is_explicit() {
        assert!(check_protocol(0).is_err());
        assert!(check_protocol(1).is_ok());
        assert!(check_protocol(2).is_ok());
        assert!(check_protocol(crate::protocol::PROTOCOL_VERSION).is_ok());
        assert!(check_protocol(crate::protocol::PROTOCOL_VERSION + 1).is_err());
    }
}
