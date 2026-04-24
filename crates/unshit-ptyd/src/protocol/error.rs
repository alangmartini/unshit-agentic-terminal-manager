//! Typed errors at the protocol boundary.
//!
//! Internal helpers propagate `io::Error` until they cross the public
//! frame/codec surface. At that surface we lift to these variants so
//! callers can distinguish "wire was malformed" from "connection was
//! truncated" without string-matching.

use std::io;

/// Maximum size in bytes of a single frame's kind+payload section.
/// A frame whose declared length exceeds this cap is rejected and the
/// owning connection must be dropped (see SPEC.md section 4).
pub const MAX_FRAME_LEN: u32 = 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    /// The advertised frame length exceeded [`MAX_FRAME_LEN`].
    #[error("frame length {advertised} exceeds cap {cap}")]
    FrameTooLarge { advertised: u32, cap: u32 },

    /// A frame declared non-zero length but carried no kind byte.
    #[error("frame length cannot be zero")]
    EmptyFrame,

    /// The kind byte did not match any known frame kind.
    #[error("unknown frame kind: {0}")]
    UnknownKind(u8),

    /// JSON payload could not be decoded into a protocol message.
    #[error("malformed control payload: {0}")]
    MalformedJson(#[from] serde_json::Error),

    /// Wrapped I/O failure, typically a truncated stream.
    #[error("io error: {0}")]
    Io(#[from] io::Error),
}
