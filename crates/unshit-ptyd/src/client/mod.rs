//! Daemon client used by the `--shutdown` flag and by tests.
//!
//! One connection per instance. The monotonic correlation id starts at
//! 1; we never reuse ids on the same connection.

use std::io;
use std::path::Path;

use crate::protocol::{
    message::{read_response, write_request, Request, Response},
    ProtocolError,
};
use crate::transport::{connect, ClientConnection};

/// Monotonic u64 generator starting at 1. Broken out from [`Client`]
/// so tests can cover the wrap-around behavior without a transport.
#[derive(Debug, Clone, Copy)]
pub struct RequestIds {
    next: u64,
}

impl Default for RequestIds {
    fn default() -> Self {
        Self { next: 1 }
    }
}

impl RequestIds {
    pub fn next(&mut self) -> u64 {
        let id = self.next;
        // Saturation avoids id reuse at u64::MAX. Reaching it takes
        // billions of years of uptime so this is defensive, not a real
        // concern.
        self.next = self.next.saturating_add(1);
        id
    }

    pub fn peek(&self) -> u64 {
        self.next
    }
}

/// Sequentially issues requests over one transport connection.
pub struct Client {
    stream: ClientConnection,
    ids: RequestIds,
}

impl Client {
    /// Opens a connection to the daemon listening on `path`.
    pub async fn connect(path: &Path) -> io::Result<Self> {
        let stream = connect(path).await?;
        Ok(Self {
            stream,
            ids: RequestIds::default(),
        })
    }

    /// Returns the next correlation id without consuming it; exposed
    /// for the tests that assert monotonicity.
    pub fn peek_next_id(&self) -> u64 {
        self.ids.peek()
    }

    fn alloc_id(&mut self) -> u64 {
        self.ids.next()
    }

    /// Sends a Hello and waits for the matching HelloAck.
    pub async fn hello(&mut self, client_version: &str) -> Result<Response, ProtocolError> {
        let id = self.alloc_id();
        let req = Request::Hello {
            id,
            client_version: client_version.to_string(),
        };
        self.roundtrip(req, id).await
    }

    /// Sends a Shutdown and waits for the matching ShutdownAck.
    pub async fn shutdown(&mut self) -> Result<Response, ProtocolError> {
        let id = self.alloc_id();
        self.roundtrip(Request::Shutdown { id }, id).await
    }

    async fn roundtrip(
        &mut self,
        req: Request,
        expected_id: u64,
    ) -> Result<Response, ProtocolError> {
        write_request(&mut self.stream, &req).await?;
        let resp = read_response(&mut self.stream).await?.ok_or_else(|| {
            ProtocolError::Io(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "daemon closed connection before responding",
            ))
        })?;
        if resp.id() != expected_id {
            return Err(ProtocolError::Io(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "response id {} does not match request id {}",
                    resp.id(),
                    expected_id
                ),
            )));
        }
        Ok(resp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_ids_start_at_one_and_increment() {
        let mut ids = RequestIds::default();
        assert_eq!(ids.peek(), 1);
        assert_eq!(ids.next(), 1);
        assert_eq!(ids.next(), 2);
        assert_eq!(ids.next(), 3);
        assert_eq!(
            ids.peek(),
            4,
            "peek must report the next id, not the last handed out"
        );
    }

    #[test]
    fn request_ids_saturate_instead_of_wrapping() {
        let mut ids = RequestIds { next: u64::MAX };
        assert_eq!(ids.next(), u64::MAX);
        assert_eq!(ids.next(), u64::MAX, "must not wrap to zero");
    }
}
