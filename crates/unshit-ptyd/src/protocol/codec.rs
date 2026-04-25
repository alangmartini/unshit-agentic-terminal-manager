//! Async frame codec over tokio streams.
//!
//! Tiny pair of helpers: [`read_frame`] and [`write_frame`]. We skip the
//! `tokio_util::codec` layer on purpose because the daemon only handles
//! one frame at a time per direction and does not need the sink/stream
//! machinery yet. If that changes we can swap this for a `FramedRead`
//! without touching callers, since the byte layout is pinned in
//! [`super::frame`].

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use super::error::ProtocolError;
use super::frame::{decode_length, encode_frame, split_header, LEN_PREFIX_SIZE};

/// A fully parsed inbound frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub kind: u8,
    pub payload: Vec<u8>,
}

/// Reads one frame from `reader`.
///
/// Returns `Ok(None)` only on clean EOF at a frame boundary (i.e. zero
/// bytes read before the stream closed). A peer that closed mid-header
/// surfaces as an `Io` error so callers can distinguish "polite hangup"
/// from "truncated".
pub async fn read_frame<R>(reader: &mut R) -> Result<Option<Frame>, ProtocolError>
where
    R: AsyncRead + Unpin,
{
    let mut len_buf = [0u8; LEN_PREFIX_SIZE];
    let mut got = 0usize;
    while got < LEN_PREFIX_SIZE {
        let n = reader.read(&mut len_buf[got..]).await?;
        if n == 0 {
            if got == 0 {
                return Ok(None);
            }
            return Err(
                std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "eof mid-header").into(),
            );
        }
        got += n;
    }

    let body_len = decode_length(len_buf)? as usize;
    let mut body = vec![0u8; body_len];
    reader.read_exact(&mut body).await?;

    let (header, payload) = split_header(&body)?;
    Ok(Some(Frame {
        kind: header.kind,
        payload: payload.to_vec(),
    }))
}

/// Writes one frame to `writer` and flushes it.
pub async fn write_frame<W>(writer: &mut W, kind: u8, payload: &[u8]) -> Result<(), ProtocolError>
where
    W: AsyncWrite + Unpin,
{
    let mut buf = Vec::with_capacity(LEN_PREFIX_SIZE + 1 + payload.len());
    encode_frame(kind, payload, &mut buf)?;
    writer.write_all(&buf).await?;
    writer.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::frame::KIND_CONTROL;
    use tokio::io::{duplex, AsyncWriteExt};

    #[tokio::test]
    async fn write_and_read_round_trips_payload() {
        let (mut a, mut b) = duplex(4096);
        let payload = b"{\"kind\":\"hello\",\"id\":1}".to_vec();
        let expected = payload.clone();

        let write_task = tokio::spawn(async move {
            write_frame(&mut a, KIND_CONTROL, &payload).await.unwrap();
        });

        let frame = read_frame(&mut b).await.unwrap().expect("frame");
        write_task.await.unwrap();

        assert_eq!(frame.kind, KIND_CONTROL);
        assert_eq!(frame.payload, expected);
    }

    #[tokio::test]
    async fn read_frame_assembles_split_writes() {
        let (mut a, mut b) = duplex(64);
        let payload = vec![42u8; 32];
        let expected = payload.clone();

        let writer = tokio::spawn(async move {
            // Encode once, then dribble bytes in with yield points so
            // the reader is forced to stitch the frame across polls.
            let mut buf = Vec::new();
            encode_frame(KIND_CONTROL, &payload, &mut buf).unwrap();
            for chunk in buf.chunks(4) {
                a.write_all(chunk).await.unwrap();
                tokio::task::yield_now().await;
            }
        });

        let frame = read_frame(&mut b).await.unwrap().expect("frame");
        writer.await.unwrap();
        assert_eq!(frame.payload, expected);
    }

    #[tokio::test]
    async fn read_frame_returns_none_on_clean_eof() {
        let (a, mut b) = duplex(16);
        drop(a);
        let result = read_frame(&mut b).await.unwrap();
        assert!(result.is_none(), "clean EOF must surface as Ok(None)");
    }

    #[tokio::test]
    async fn read_frame_errors_on_truncated_header() {
        let (mut a, mut b) = duplex(16);
        // Write two bytes then close; header read must fail with IO.
        a.write_all(&[1, 2]).await.unwrap();
        drop(a);
        let err = read_frame(&mut b).await.unwrap_err();
        assert!(matches!(err, ProtocolError::Io(_)), "{err:?}");
    }

    #[tokio::test]
    async fn read_frame_rejects_oversized_advertisement() {
        use super::super::error::MAX_FRAME_LEN;
        let (mut a, mut b) = duplex(16);
        let oversize: u32 = MAX_FRAME_LEN + 1;
        a.write_all(&oversize.to_le_bytes()).await.unwrap();
        // Do not send a body; the decoder must reject on the prefix.
        let err = read_frame(&mut b).await.unwrap_err();
        assert!(
            matches!(err, ProtocolError::FrameTooLarge { .. }),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn read_frame_rejects_zero_length() {
        let (mut a, mut b) = duplex(16);
        a.write_all(&0u32.to_le_bytes()).await.unwrap();
        let err = read_frame(&mut b).await.unwrap_err();
        assert!(matches!(err, ProtocolError::EmptyFrame), "{err:?}");
    }

    #[tokio::test]
    async fn read_frame_rejects_unknown_kind() {
        let (mut a, mut b) = duplex(16);
        // length = 1, kind = 0xFF, no payload.
        a.write_all(&1u32.to_le_bytes()).await.unwrap();
        a.write_all(&[0xFF]).await.unwrap();
        let err = read_frame(&mut b).await.unwrap_err();
        match err {
            ProtocolError::UnknownKind(k) => assert_eq!(k, 0xFF),
            other => panic!("unexpected error: {other:?}"),
        }
    }
}
