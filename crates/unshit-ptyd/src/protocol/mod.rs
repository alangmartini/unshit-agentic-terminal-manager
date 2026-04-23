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
pub use frame::{FrameHeader, KIND_CONTROL, KIND_OUTPUT, LEN_PREFIX_SIZE};
pub use message::{read_request, read_response, write_request, write_response, Request, Response};

/// Wire protocol version advertised in `HelloAck`. Bump on any
/// non-additive change (see SPEC.md section 10).
pub const PROTOCOL_VERSION: u32 = 1;
