//! Length-prefixed framing: `u32 length (LE) | u8 kind | payload`.
//!
//! `length` covers the kind byte and the payload, so the minimum legal
//! frame on the wire is 5 bytes (4 length + 1 kind) with an empty
//! payload. The cap on `length` lives in [`super::error::MAX_FRAME_LEN`]
//! so the caller can reject oversize headers without allocating.
//!
//! This module is sync and buffer-only. The async [`super::codec`]
//! layer adapts it to tokio streams.

use super::error::{ProtocolError, MAX_FRAME_LEN};

/// Kind byte for JSON control messages (requests, responses, errors).
pub const KIND_CONTROL: u8 = 0;

/// Kind byte reserved for binary PTY output chunks. Not used in slice 2;
/// the constant is pinned here so later slices cannot collide with it.
pub const KIND_OUTPUT: u8 = 1;

/// Size of the length prefix in bytes.
pub const LEN_PREFIX_SIZE: usize = 4;

/// Appends an encoded frame to `out`.
///
/// `payload.len() + 1` must fit in `u32` and must not exceed
/// [`MAX_FRAME_LEN`].
pub fn encode_frame(kind: u8, payload: &[u8], out: &mut Vec<u8>) -> Result<(), ProtocolError> {
    let body_len = payload
        .len()
        .checked_add(1)
        .and_then(|v| u32::try_from(v).ok())
        .ok_or(ProtocolError::FrameTooLarge {
            advertised: u32::MAX,
            cap: MAX_FRAME_LEN,
        })?;
    if body_len > MAX_FRAME_LEN {
        return Err(ProtocolError::FrameTooLarge {
            advertised: body_len,
            cap: MAX_FRAME_LEN,
        });
    }
    out.extend_from_slice(&body_len.to_le_bytes());
    out.push(kind);
    out.extend_from_slice(payload);
    Ok(())
}

/// Decoded view of one frame's header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameHeader {
    pub kind: u8,
    pub payload_len: u32,
}

/// Parses a 4-byte length prefix, rejecting the oversize and zero cases.
///
/// Zero is illegal because every frame must carry at least the kind byte.
pub fn decode_length(prefix: [u8; LEN_PREFIX_SIZE]) -> Result<u32, ProtocolError> {
    let body_len = u32::from_le_bytes(prefix);
    if body_len == 0 {
        return Err(ProtocolError::EmptyFrame);
    }
    if body_len > MAX_FRAME_LEN {
        return Err(ProtocolError::FrameTooLarge {
            advertised: body_len,
            cap: MAX_FRAME_LEN,
        });
    }
    Ok(body_len)
}

/// Splits the first kind byte off a body buffer.
///
/// `body` must already exclude the length prefix and be exactly
/// `body_len` bytes.
pub fn split_header(body: &[u8]) -> Result<(FrameHeader, &[u8]), ProtocolError> {
    let (kind_byte, payload) = body.split_first().ok_or(ProtocolError::EmptyFrame)?;
    let kind = *kind_byte;
    if kind != KIND_CONTROL && kind != KIND_OUTPUT {
        return Err(ProtocolError::UnknownKind(kind));
    }
    let payload_len = u32::try_from(payload.len()).map_err(|_| ProtocolError::FrameTooLarge {
        advertised: u32::MAX,
        cap: MAX_FRAME_LEN,
    })?;
    Ok((FrameHeader { kind, payload_len }, payload))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_control_frame_preserves_payload() {
        let payload = br#"{"kind":"hello","id":1,"client_version":"x"}"#;
        let mut buf = Vec::new();
        encode_frame(KIND_CONTROL, payload, &mut buf).unwrap();

        // First four bytes are the length prefix and must match payload+kind.
        let expected_len: u32 = (payload.len() + 1) as u32;
        assert_eq!(
            buf[..LEN_PREFIX_SIZE],
            expected_len.to_le_bytes(),
            "length prefix mismatch"
        );
        assert_eq!(buf[LEN_PREFIX_SIZE], KIND_CONTROL);
        assert_eq!(&buf[LEN_PREFIX_SIZE + 1..], payload);
    }

    #[test]
    fn empty_payload_round_trips_as_five_bytes() {
        let mut buf = Vec::new();
        encode_frame(KIND_CONTROL, &[], &mut buf).unwrap();
        assert_eq!(buf.len(), LEN_PREFIX_SIZE + 1);

        let len = decode_length(buf[..LEN_PREFIX_SIZE].try_into().unwrap()).unwrap();
        assert_eq!(len, 1, "length should cover only the kind byte");

        let (header, payload) = split_header(&buf[LEN_PREFIX_SIZE..]).unwrap();
        assert_eq!(header.kind, KIND_CONTROL);
        assert_eq!(header.payload_len, 0);
        assert!(payload.is_empty());
    }

    #[test]
    fn decode_length_rejects_zero() {
        let err = decode_length([0, 0, 0, 0]).unwrap_err();
        assert!(matches!(err, ProtocolError::EmptyFrame), "{err:?}");
    }

    #[test]
    fn decode_length_rejects_over_one_mib() {
        let oversized: u32 = MAX_FRAME_LEN + 1;
        let err = decode_length(oversized.to_le_bytes()).unwrap_err();
        match err {
            ProtocolError::FrameTooLarge { advertised, cap } => {
                assert_eq!(advertised, oversized);
                assert_eq!(cap, MAX_FRAME_LEN);
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn decode_length_accepts_exact_cap() {
        let len = decode_length(MAX_FRAME_LEN.to_le_bytes()).unwrap();
        assert_eq!(len, MAX_FRAME_LEN);
    }

    #[test]
    fn encode_rejects_payload_larger_than_cap() {
        let payload = vec![0u8; MAX_FRAME_LEN as usize];
        let mut buf = Vec::new();
        let err = encode_frame(KIND_CONTROL, &payload, &mut buf).unwrap_err();
        assert!(
            matches!(err, ProtocolError::FrameTooLarge { .. }),
            "{err:?}"
        );
        assert!(buf.is_empty(), "buffer must be unchanged on encode failure");
    }

    #[test]
    fn split_header_rejects_empty_body() {
        let err = split_header(&[]).unwrap_err();
        assert!(matches!(err, ProtocolError::EmptyFrame), "{err:?}");
    }

    #[test]
    fn split_header_rejects_unknown_kind() {
        let err = split_header(&[0xFF]).unwrap_err();
        match err {
            ProtocolError::UnknownKind(k) => assert_eq!(k, 0xFF),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn split_header_accepts_output_kind() {
        let (header, payload) = split_header(&[KIND_OUTPUT, 1, 2, 3]).unwrap();
        assert_eq!(header.kind, KIND_OUTPUT);
        assert_eq!(header.payload_len, 3);
        assert_eq!(payload, &[1, 2, 3]);
    }

    #[test]
    fn two_frames_concatenate_cleanly() {
        let mut buf = Vec::new();
        encode_frame(KIND_CONTROL, b"first", &mut buf).unwrap();
        encode_frame(KIND_CONTROL, b"second", &mut buf).unwrap();

        // Peel frame one.
        let len1 = decode_length(buf[..LEN_PREFIX_SIZE].try_into().unwrap()).unwrap() as usize;
        let end1 = LEN_PREFIX_SIZE + len1;
        let (h1, p1) = split_header(&buf[LEN_PREFIX_SIZE..end1]).unwrap();
        assert_eq!(h1.kind, KIND_CONTROL);
        assert_eq!(p1, b"first");

        // Peel frame two from the remaining buffer.
        let tail = &buf[end1..];
        let len2 = decode_length(tail[..LEN_PREFIX_SIZE].try_into().unwrap()).unwrap() as usize;
        let (h2, p2) = split_header(&tail[LEN_PREFIX_SIZE..LEN_PREFIX_SIZE + len2]).unwrap();
        assert_eq!(h2.kind, KIND_CONTROL);
        assert_eq!(p2, b"second");
    }
}
