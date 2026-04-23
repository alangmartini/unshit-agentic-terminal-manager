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
use super::frame::{KIND_CONTROL, KIND_EVENT};
use super::{read_frame, write_frame};

/// Client-to-daemon control requests.
///
/// Binary payloads carried in [`Request::Write`] serialise as a JSON
/// array of u8 values. That is verbose on the wire but keeps the whole
/// vocabulary in a single JSON envelope so we can skip pulling in a
/// base64 dep for v1. If profiling shows overhead matters the plan is
/// to switch to `serde_bytes` + base64 without bumping the protocol
/// version (the change would be a non-breaking tag swap since variant
/// names stay stable).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Request {
    /// Opening handshake. The daemon replies with `hello_ack`.
    Hello { id: u64, client_version: String },
    /// Graceful daemon shutdown. Only succeeds with zero sessions alive.
    Shutdown { id: u64 },
    /// Spawn a new session running `shell` (or the platform default).
    SpawnSession {
        id: u64,
        cols: u16,
        rows: u16,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cwd: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        shell: Option<String>,
    },
    /// Write `bytes` to the PTY stdin of `session_id`.
    Write {
        id: u64,
        session_id: u64,
        bytes: Vec<u8>,
    },
    /// Resize the PTY of `session_id`.
    Resize {
        id: u64,
        session_id: u64,
        cols: u16,
        rows: u16,
    },
    /// Kill the child of `session_id` and remove it from the registry.
    KillSession { id: u64, session_id: u64 },
    /// List every session on the daemon.
    ListSessions { id: u64 },
}

impl Request {
    /// The correlation id the client attached to this request.
    pub fn id(&self) -> u64 {
        match self {
            Request::Hello { id, .. } => *id,
            Request::Shutdown { id } => *id,
            Request::SpawnSession { id, .. } => *id,
            Request::Write { id, .. } => *id,
            Request::Resize { id, .. } => *id,
            Request::KillSession { id, .. } => *id,
            Request::ListSessions { id } => *id,
        }
    }
}

/// Snapshot of session state returned by `list_sessions`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionInfo {
    pub id: u64,
    pub cols: u16,
    pub rows: u16,
    pub alive: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
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
    SessionSpawned {
        id: u64,
        session_id: u64,
    },
    /// Generic success ack used for write / resize / kill_session.
    Ack {
        id: u64,
    },
    SessionList {
        id: u64,
        sessions: Vec<SessionInfo>,
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
            Response::SessionSpawned { id, .. } => *id,
            Response::Ack { id } => *id,
            Response::SessionList { id, .. } => *id,
            Response::Error { id, .. } => *id,
        }
    }
}

/// Server-pushed unsolicited events. Carried on `KIND_EVENT` frames so
/// a client can distinguish them from solicited responses while sharing
/// the same connection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ServerEvent {
    /// Raw PTY output bytes from `session_id`. Bytes are a JSON array
    /// of u8 for the same reasons described on [`Request::Write`].
    Output { session_id: u64, bytes: Vec<u8> },
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

/// Reads one frame and decodes it as a [`ServerEvent`].
///
/// Event frames are pushed by the daemon on its own schedule, so the
/// client typically runs this in a dedicated reader task. A control
/// frame surfacing here is a protocol violation from the client's
/// perspective and maps to `UnknownKind(KIND_CONTROL)`.
pub async fn read_server_event<R>(reader: &mut R) -> Result<Option<ServerEvent>, ProtocolError>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let Some(frame) = read_frame(reader).await? else {
        return Ok(None);
    };
    if frame.kind != KIND_EVENT {
        return Err(ProtocolError::UnknownKind(frame.kind));
    }
    let ev: ServerEvent = serde_json::from_slice(&frame.payload)?;
    Ok(Some(ev))
}

/// Serializes `event` as JSON and writes it as a `KIND_EVENT` frame.
pub async fn write_server_event<W>(writer: &mut W, event: &ServerEvent) -> Result<(), ProtocolError>
where
    W: tokio::io::AsyncWrite + Unpin,
{
    let bytes = serde_json::to_vec(event)?;
    write_frame(writer, KIND_EVENT, &bytes).await
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
        // Pick a variant name we have not defined yet so this test
        // stays meaningful as new request kinds are added.
        let raw = br#"{"kind":"attach_session","id":1}"#;
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

    #[test]
    fn spawn_session_round_trips_with_optional_fields() {
        let req = Request::SpawnSession {
            id: 11,
            cols: 120,
            rows: 40,
            cwd: Some("/tmp".into()),
            shell: None,
        };
        let s = serde_json::to_string(&req).unwrap();
        assert!(
            !s.contains("\"shell\""),
            "None shell must be omitted on the wire: {s}"
        );
        let back: Request = serde_json::from_str(&s).unwrap();
        assert_eq!(req, back);
    }

    #[test]
    fn write_request_preserves_byte_payload() {
        let bytes: Vec<u8> = vec![0, 1, 2, 0xff, b'a', b'\n'];
        let req = Request::Write {
            id: 5,
            session_id: 3,
            bytes: bytes.clone(),
        };
        let s = serde_json::to_string(&req).unwrap();
        let back: Request = serde_json::from_str(&s).unwrap();
        match back {
            Request::Write { bytes: got, .. } => assert_eq!(got, bytes),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn session_spawned_response_round_trips() {
        let resp = Response::SessionSpawned {
            id: 1,
            session_id: 42,
        };
        let s = serde_json::to_string(&resp).unwrap();
        let back: Response = serde_json::from_str(&s).unwrap();
        assert_eq!(resp, back);
    }

    #[test]
    fn ack_response_round_trips() {
        let resp = Response::Ack { id: 99 };
        let s = serde_json::to_string(&resp).unwrap();
        let back: Response = serde_json::from_str(&s).unwrap();
        assert_eq!(resp, back);
    }

    #[test]
    fn session_list_response_round_trips() {
        let resp = Response::SessionList {
            id: 1,
            sessions: vec![
                SessionInfo {
                    id: 1,
                    cols: 80,
                    rows: 24,
                    alive: true,
                    pid: Some(1234),
                },
                SessionInfo {
                    id: 2,
                    cols: 100,
                    rows: 30,
                    alive: false,
                    pid: None,
                },
            ],
        };
        let s = serde_json::to_string(&resp).unwrap();
        let back: Response = serde_json::from_str(&s).unwrap();
        assert_eq!(resp, back);
    }

    #[test]
    fn server_event_output_round_trips() {
        let ev = ServerEvent::Output {
            session_id: 7,
            bytes: b"hello\n".to_vec(),
        };
        let s = serde_json::to_string(&ev).unwrap();
        let back: ServerEvent = serde_json::from_str(&s).unwrap();
        assert_eq!(ev, back);
    }

    #[tokio::test]
    async fn read_write_server_event_over_duplex() {
        let (mut a, mut b) = tokio::io::duplex(4096);
        let ev = ServerEvent::Output {
            session_id: 11,
            bytes: vec![1, 2, 3, 4],
        };
        let sent = ev.clone();
        let writer = tokio::spawn(async move {
            write_server_event(&mut a, &sent).await.unwrap();
        });
        let got = read_server_event(&mut b).await.unwrap().expect("event");
        writer.await.unwrap();
        assert_eq!(got, ev);
    }

    #[tokio::test]
    async fn read_server_event_rejects_control_frame() {
        // Regression: a KIND_CONTROL frame slipping into the event
        // channel must surface as UnknownKind so callers can drop the
        // connection instead of silently corrupting the event stream.
        let (mut a, mut b) = tokio::io::duplex(4096);
        write_response(&mut a, &Response::Ack { id: 1 })
            .await
            .unwrap();
        let err = read_server_event(&mut b).await.unwrap_err();
        match err {
            ProtocolError::UnknownKind(k) => assert_eq!(k, KIND_CONTROL),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn request_id_accessor_handles_new_variants() {
        assert_eq!(
            Request::SpawnSession {
                id: 31,
                cols: 80,
                rows: 24,
                cwd: None,
                shell: None
            }
            .id(),
            31
        );
        assert_eq!(
            Request::Write {
                id: 32,
                session_id: 1,
                bytes: vec![]
            }
            .id(),
            32
        );
        assert_eq!(
            Request::Resize {
                id: 33,
                session_id: 1,
                cols: 1,
                rows: 1
            }
            .id(),
            33
        );
        assert_eq!(
            Request::KillSession {
                id: 34,
                session_id: 1
            }
            .id(),
            34
        );
        assert_eq!(Request::ListSessions { id: 35 }.id(), 35);
    }

    #[test]
    fn response_id_accessor_handles_new_variants() {
        assert_eq!(
            Response::SessionSpawned {
                id: 41,
                session_id: 2
            }
            .id(),
            41
        );
        assert_eq!(Response::Ack { id: 42 }.id(), 42);
        assert_eq!(
            Response::SessionList {
                id: 43,
                sessions: vec![]
            }
            .id(),
            43
        );
    }
}
