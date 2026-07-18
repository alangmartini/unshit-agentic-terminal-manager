//! Wire protocol: framing, codec, message types, and typed errors.
//!
//! The layering mirrors SPEC.md section 4:
//! - [`frame`] owns the byte layout.
//! - [`codec`] adapts frames to tokio `AsyncRead`/`AsyncWrite`.
//! - [`message`] defines the JSON control vocabulary.
//! - [`error`] is the shared error type across the three.

pub mod codec;
pub mod error;
pub mod frame;
pub mod message;

pub use codec::{read_frame, write_frame, Frame};
pub use error::{ProtocolError, MAX_FRAME_LEN};
pub use frame::{FrameHeader, KIND_CONTROL, KIND_EVENT, KIND_OUTPUT, LEN_PREFIX_SIZE};
pub use message::{
    decode_output_payload, read_request, read_response, read_server_event, write_output_frame,
    write_request, write_response, write_server_event, Request, Response, ServerEvent, SessionInfo,
    SNAPSHOT_MAX_SCROLLBACK_LINES,
};

/// Wire protocol version advertised in `HelloAck`. Bump whenever a client
/// must feature-detect a request or response before using it, including new
/// variants that an older peer cannot safely ignore (see SPEC.md section 10).
pub const PROTOCOL_VERSION: u32 = 2;

/// First protocol version whose daemon understands the atomic
/// `EnsureSession` request. New clients can still reattach to sessions on a
/// v1 daemon, but must not send the unknown variant or assume a cache miss is
/// proof that no live session exists.
pub const ENSURE_SESSION_PROTOCOL_VERSION: u32 = 2;
