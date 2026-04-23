//! JSON control-message vocabulary.
//!
//! Serde tags the variants on a `kind` field to match SPEC.md section 4.
//! Each request / response / error carries a client-allocated `id` so
//! the client can correlate responses without relying on stream order.
//!
//! Only hello / shutdown are modelled in slice 2. Later slices add the
//! session-lifecycle variants; adding new `kind` values is additive and
//! does not bump `PROTOCOL_VERSION`.

use serde::{Deserialize, Serialize};

use super::error::ProtocolError;
use super::frame::KIND_CONTROL;
use super::{read_frame, write_frame};

/// Client-to-daemon control requests.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Request {
    /// Opening handshake. The daemon replies with `hello_ack`.
    Hello { id: u64, client_version: String },
    /// Graceful daemon shutdown. Only succeeds with zero sessions alive.
    /// In slice 2 the session count is trivially zero.
    Shutdown { id: u64 },
}

impl Request {
    /// The correlation id the client attached to this request.
    pub fn id(&self) -> u64 {
        match self {
            Request::Hello { id, .. } => *id,
            Request::Shutdown { id } => *id,
        }
    }
}

/// Daemon-to-client control responses and errors.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Response {
    HelloAck {
        id: u64,
        server_version: String,
        protocol_version: u32,
    },
    ShutdownAck {
        id: u64,
        ok: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    Error {
        id: u64,
        code: String,
        message: String,
    },
}

impl Response {
    pub fn id(&self) -> u64 {
        match self {
            Response::HelloAck { id, .. } => *id,
            Response::ShutdownAck { id, .. } => *id,
            Response::Error { id, .. } => *id,
        }
    }
}

/// Reads one control frame and decodes it as a [`Request`].
///
/// Returns `Ok(None)` on clean EOF. Binary (output) frames are not
/// expected from a client in slice 2 and surface as `UnknownKind`.
pub async fn read_request<R>(reader: &mut R) -> Result<Option<Request>, ProtocolError>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let Some(frame) = read_frame(reader).await? else {
        return Ok(None);
    };
    if frame.kind != KIND_CONTROL {
        return Err(ProtocolError::UnknownKind(frame.kind));
    }
    let req: Request = serde_json::from_slice(&frame.payload)?;
    Ok(Some(req))
}

/// Reads one control frame and decodes it as a [`Response`].
pub async fn read_response<R>(reader: &mut R) -> Result<Option<Response>, ProtocolError>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let Some(frame) = read_frame(reader).await? else {
        return Ok(None);
    };
    if frame.kind != KIND_CONTROL {
        return Err(ProtocolError::UnknownKind(frame.kind));
    }
    let resp: Response = serde_json::from_slice(&frame.payload)?;
    Ok(Some(resp))
}

/// Serializes `req` as JSON and writes it as a control frame.
pub async fn write_request<W>(writer: &mut W, req: &Request) -> Result<(), ProtocolError>
where
    W: tokio::io::AsyncWrite + Unpin,
{
    let bytes = serde_json::to_vec(req)?;
    write_frame(writer, KIND_CONTROL, &bytes).await
}

/// Serializes `resp` as JSON and writes it as a control frame.
pub async fn write_response<W>(writer: &mut W, resp: &Response) -> Result<(), ProtocolError>
where
    W: tokio::io::AsyncWrite + Unpin,
{
    let bytes = serde_json::to_vec(resp)?;
    write_frame(writer, KIND_CONTROL, &bytes).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hello_request_round_trips() {
        let req = Request::Hello {
            id: 7,
            client_version: "0.1.0".into(),
        };
        let bytes = serde_json::to_vec(&req).unwrap();
        let back: Request = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(req, back);
    }

    #[test]
    fn shutdown_request_round_trips() {
        let req = Request::Shutdown { id: 42 };
        let bytes = serde_json::to_vec(&req).unwrap();
        let back: Request = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(req, back);
    }

    #[test]
    fn hello_ack_round_trips() {
        let resp = Response::HelloAck {
            id: 1,
            server_version: "0.2.0".into(),
            protocol_version: 1,
        };
        let bytes = serde_json::to_vec(&resp).unwrap();
        let back: Response = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(resp, back);
    }

    #[test]
    fn shutdown_ack_with_reason_round_trips() {
        let resp = Response::ShutdownAck {
            id: 2,
            ok: false,
            reason: Some("sessions alive".into()),
        };
        let bytes = serde_json::to_vec(&resp).unwrap();
        let back: Response = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(resp, back);
    }

    #[test]
    fn shutdown_ack_without_reason_omits_field() {
        // reason = None must not appear in the JSON output so older
        // clients never see a `reason: null` they do not expect.
        let resp = Response::ShutdownAck {
            id: 3,
            ok: true,
            reason: None,
        };
        let s = serde_json::to_string(&resp).unwrap();
        assert!(!s.contains("reason"), "omit empty reason: {s}");

        let back: Response = serde_json::from_str(&s).unwrap();
        assert_eq!(resp, back);
    }

    #[test]
    fn error_response_round_trips() {
        let resp = Response::Error {
            id: 4,
            code: "shutdown_denied".into(),
            message: "2 sessions alive".into(),
        };
        let bytes = serde_json::to_vec(&resp).unwrap();
        let back: Response = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(resp, back);
    }

    #[test]
    fn unknown_request_kind_is_rejected() {
        let raw = br#"{"kind":"list_sessions","id":1}"#;
        let err = serde_json::from_slice::<Request>(raw).unwrap_err();
        assert!(err.is_data(), "unknown kind should be a data error: {err}");
    }

    #[test]
    fn request_kind_is_snake_case_on_wire() {
        // Pin the wire spelling so a future refactor cannot silently
        // switch it (which would break deployed clients).
        let s = serde_json::to_string(&Request::Shutdown { id: 9 }).unwrap();
        assert!(s.contains("\"kind\":\"shutdown\""), "{s}");
    }

    #[test]
    fn request_id_accessor_returns_attached_id() {
        assert_eq!(
            Request::Hello {
                id: 11,
                client_version: "v".into(),
            }
            .id(),
            11
        );
        assert_eq!(Request::Shutdown { id: 12 }.id(), 12);
    }

    #[test]
    fn response_id_accessor_returns_attached_id() {
        assert_eq!(
            Response::HelloAck {
                id: 21,
                server_version: "v".into(),
                protocol_version: 1,
            }
            .id(),
            21
        );
        assert_eq!(
            Response::ShutdownAck {
                id: 22,
                ok: true,
                reason: None,
            }
            .id(),
            22
        );
        assert_eq!(
            Response::Error {
                id: 23,
                code: "x".into(),
                message: "y".into(),
            }
            .id(),
            23
        );
    }

    #[tokio::test]
    async fn read_write_request_over_duplex() {
        let (mut a, mut b) = tokio::io::duplex(4096);
        let req = Request::Hello {
            id: 99,
            client_version: "x".into(),
        };
        let sent = req.clone();
        let writer = tokio::spawn(async move {
            write_request(&mut a, &sent).await.unwrap();
        });
        let got = read_request(&mut b).await.unwrap().expect("request");
        writer.await.unwrap();
        assert_eq!(got, req);
    }

    #[tokio::test]
    async fn read_write_response_over_duplex() {
        let (mut a, mut b) = tokio::io::duplex(4096);
        let resp = Response::HelloAck {
            id: 1,
            server_version: "0.1.0".into(),
            protocol_version: 1,
        };
        let sent = resp.clone();
        let writer = tokio::spawn(async move {
            write_response(&mut a, &sent).await.unwrap();
        });
        let got = read_response(&mut b).await.unwrap().expect("response");
        writer.await.unwrap();
        assert_eq!(got, resp);
    }
}
