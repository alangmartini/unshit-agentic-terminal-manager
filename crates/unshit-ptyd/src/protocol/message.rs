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
use unshit_terminal_core::Snapshot;

use super::error::ProtocolError;
use super::frame::{KIND_CONTROL, KIND_EVENT, KIND_OUTPUT};
use super::{read_frame, write_frame};

/// Size of the session id prefix on `KIND_OUTPUT` frames.
pub const OUTPUT_SESSION_ID_SIZE: usize = 8;

/// Maximum number of scrollback lines an attach-session response
/// carries. The clamp is a v1 wire-format safety valve: with JSON
/// encoding and default blank-cell payloads this keeps the control
/// frame well under `MAX_FRAME_LEN` (1 MiB) for typical 80x24 grids.
/// TODO(slice 5 / polish): swap JSON for a compact binary format and
/// revisit this cap.
pub const SNAPSHOT_MAX_SCROLLBACK_LINES: usize = 100;

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
        /// Args forwarded to `shell` before any daemon side cwd args
        /// (e.g. the PowerShell `-NoExit -Command "Set-Location ..."`
        /// workaround). Additive in v1: defaults to empty for old
        /// clients, omitted from the wire when empty so old daemons
        /// see the same shape they always have.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        shell_args: Vec<String>,
        workspace_id: u32,
        pane_id: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
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
    /// Attach to an existing session and retrieve its authoritative
    /// snapshot (grid + scrollback tail).
    ///
    /// `scrollback_lines` is the requested number of most-recent
    /// scrollback rows to include. The daemon silently clamps this at
    /// [`SNAPSHOT_MAX_SCROLLBACK_LINES`] so an over-eager caller does
    /// not push the control frame past `MAX_FRAME_LEN`.
    AttachSession {
        id: u64,
        session_id: u64,
        scrollback_lines: u32,
    },
    /// Detach from a session. Slice 4 treats this as a no-op ack;
    /// slice 5 promotes it to "keep running" once cross-connection
    /// persistence lands.
    DetachSession { id: u64, session_id: u64 },
    /// Set or clear the display name of a session. Clearing uses
    /// `name: None`; the daemon treats an empty string the same as
    /// `None`. Responds with a generic `Ack`.
    RenameSession {
        id: u64,
        session_id: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
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
            Request::AttachSession { id, .. } => *id,
            Request::DetachSession { id, .. } => *id,
            Request::RenameSession { id, .. } => *id,
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
    pub workspace_id: u32,
    pub pane_id: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Daemon-to-client control responses and errors.
///
/// `Eq` is intentionally not derived: `SessionAttached` carries a
/// [`Snapshot`] whose cells only implement `PartialEq`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    /// Generic success ack used for write / resize / kill_session /
    /// detach_session.
    Ack {
        id: u64,
    },
    SessionList {
        id: u64,
        sessions: Vec<SessionInfo>,
    },
    SessionAttached {
        id: u64,
        snapshot: Snapshot,
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
            Response::SessionAttached { id, .. } => *id,
            Response::Error { id, .. } => *id,
        }
    }
}

/// Server-pushed unsolicited events.
///
/// `Output` rides on `KIND_OUTPUT` with a pure-binary payload of
/// `u64 session_id (LE) | raw bytes` so PTY output skips serde entirely
/// and is not inflated ~4x by JSON array-of-u8 encoding. Other variants
/// (none yet; session_exited and session_crashed arrive in slice 4)
/// ride on `KIND_EVENT` as JSON.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ServerEvent {
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

/// Reads one server-pushed frame and decodes it as a [`ServerEvent`].
///
/// Dispatches on frame kind: `KIND_OUTPUT` carries the binary payload
/// used for PTY bytes; `KIND_EVENT` carries JSON for future event
/// variants. A control frame surfacing here is a protocol violation
/// and maps to `UnknownKind`.
pub async fn read_server_event<R>(reader: &mut R) -> Result<Option<ServerEvent>, ProtocolError>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let Some(frame) = read_frame(reader).await? else {
        return Ok(None);
    };
    match frame.kind {
        KIND_OUTPUT => {
            let (session_id, bytes) = decode_output_payload(&frame.payload)?;
            Ok(Some(ServerEvent::Output {
                session_id,
                bytes: bytes.to_vec(),
            }))
        }
        KIND_EVENT => {
            let ev: ServerEvent = serde_json::from_slice(&frame.payload)?;
            Ok(Some(ev))
        }
        other => Err(ProtocolError::UnknownKind(other)),
    }
}

/// Writes `event` to the wire, using `KIND_OUTPUT` for `Output` and
/// `KIND_EVENT` JSON for everything else.
pub async fn write_server_event<W>(writer: &mut W, event: &ServerEvent) -> Result<(), ProtocolError>
where
    W: tokio::io::AsyncWrite + Unpin,
{
    match event {
        ServerEvent::Output { session_id, bytes } => {
            write_output_frame(writer, *session_id, bytes).await
        }
    }
}

/// Writes a `KIND_OUTPUT` frame directly from a session id and a slice
/// of raw bytes. Avoids the `ServerEvent::Output { .., bytes: bytes.to_vec() }`
/// clone that `write_server_event` would otherwise force on the caller.
pub async fn write_output_frame<W>(
    writer: &mut W,
    session_id: u64,
    bytes: &[u8],
) -> Result<(), ProtocolError>
where
    W: tokio::io::AsyncWrite + Unpin,
{
    let mut payload = Vec::with_capacity(OUTPUT_SESSION_ID_SIZE + bytes.len());
    payload.extend_from_slice(&session_id.to_le_bytes());
    payload.extend_from_slice(bytes);
    write_frame(writer, KIND_OUTPUT, &payload).await
}

/// Splits a `KIND_OUTPUT` frame payload into `(session_id, bytes)`.
///
/// The payload must begin with an 8-byte little-endian session id and
/// may carry zero or more trailing bytes. A shorter payload is a
/// protocol violation.
pub fn decode_output_payload(payload: &[u8]) -> Result<(u64, &[u8]), ProtocolError> {
    if payload.len() < OUTPUT_SESSION_ID_SIZE {
        return Err(ProtocolError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "output frame payload shorter than 8-byte session id",
        )));
    }
    let (id_bytes, rest) = payload.split_at(OUTPUT_SESSION_ID_SIZE);
    let id_array: [u8; OUTPUT_SESSION_ID_SIZE] = id_bytes
        .try_into()
        .expect("slice split_at guarantees length");
    Ok((u64::from_le_bytes(id_array), rest))
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
        let raw = br#"{"kind":"hibernate_session","id":1}"#;
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
            shell_args: vec![],
            workspace_id: 2,
            pane_id: 5,
            name: Some("scratch".into()),
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
    fn spawn_session_omits_name_when_none() {
        let req = Request::SpawnSession {
            id: 12,
            cols: 80,
            rows: 24,
            cwd: None,
            shell: None,
            shell_args: vec![],
            workspace_id: 0,
            pane_id: 0,
            name: None,
        };
        let s = serde_json::to_string(&req).unwrap();
        assert!(
            !s.contains("\"name\""),
            "None name must be omitted on the wire: {s}"
        );
        assert!(s.contains("\"workspace_id\":0"), "{s}");
        assert!(s.contains("\"pane_id\":0"), "{s}");
        let back: Request = serde_json::from_str(&s).unwrap();
        assert_eq!(req, back);
    }

    // refs #140: shell_args is an additive field. When set it must
    // round trip; when empty it must be omitted from the wire so old
    // daemons reading new payloads see the same shape they always have.
    #[test]
    fn spawn_session_round_trips_with_shell_args() {
        let req = Request::SpawnSession {
            id: 13,
            cols: 80,
            rows: 24,
            cwd: None,
            shell: Some("bash".into()),
            shell_args: vec!["--login".into(), "-i".into()],
            workspace_id: 0,
            pane_id: 0,
            name: None,
        };
        let s = serde_json::to_string(&req).unwrap();
        assert!(
            s.contains(r#""shell_args":["--login","-i"]"#),
            "shell_args must serialize when non empty: {s}"
        );
        let back: Request = serde_json::from_str(&s).unwrap();
        assert_eq!(req, back);
    }

    // refs #140: empty shell_args must not appear on the wire so the
    // change is invisible to old daemons that ignore unknown fields.
    #[test]
    fn spawn_session_omits_shell_args_when_empty() {
        let req = Request::SpawnSession {
            id: 14,
            cols: 80,
            rows: 24,
            cwd: None,
            shell: None,
            shell_args: vec![],
            workspace_id: 0,
            pane_id: 0,
            name: None,
        };
        let s = serde_json::to_string(&req).unwrap();
        assert!(
            !s.contains("\"shell_args\""),
            "empty shell_args must be omitted on the wire: {s}"
        );
    }

    // refs #140: an old wire payload (no shell_args key) must
    // deserialize with an empty vec so newer daemons keep accepting
    // requests from clients that have not been rebuilt yet.
    #[test]
    fn spawn_session_deserializes_with_default_shell_args_when_field_is_missing() {
        let json =
            r#"{"kind":"spawn_session","id":15,"cols":80,"rows":24,"workspace_id":0,"pane_id":0}"#;
        let back: Request = serde_json::from_str(json).unwrap();
        match back {
            Request::SpawnSession { shell_args, .. } => {
                assert!(
                    shell_args.is_empty(),
                    "missing shell_args field must deserialize to an empty vector"
                );
            }
            other => panic!("expected SpawnSession, got {other:?}"),
        }
    }

    #[test]
    fn session_info_omits_name_when_none() {
        let info = SessionInfo {
            id: 1,
            cols: 80,
            rows: 24,
            alive: true,
            pid: None,
            workspace_id: 7,
            pane_id: 3,
            name: None,
        };
        let s = serde_json::to_string(&info).unwrap();
        assert!(
            !s.contains("\"name\""),
            "None name must be omitted on the wire: {s}"
        );
        let back: SessionInfo = serde_json::from_str(&s).unwrap();
        assert_eq!(info, back);
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
                    workspace_id: 0,
                    pane_id: 1,
                    name: Some("shell".into()),
                },
                SessionInfo {
                    id: 2,
                    cols: 100,
                    rows: 30,
                    alive: false,
                    pid: None,
                    workspace_id: 1,
                    pane_id: 0,
                    name: None,
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
    async fn write_server_event_output_uses_kind_output_frame() {
        // Pin the on-wire choice: Output MUST ride KIND_OUTPUT with the
        // binary layout (u64 session_id LE + raw bytes), not KIND_EVENT
        // with JSON. Regressing this would re-inflate PTY output ~4x.
        let (mut a, mut b) = tokio::io::duplex(4096);
        let sent = ServerEvent::Output {
            session_id: 0x0102_0304_0506_0708,
            bytes: b"xy".to_vec(),
        };
        let writer = tokio::spawn(async move {
            write_server_event(&mut a, &sent).await.unwrap();
        });
        let frame = read_frame(&mut b).await.unwrap().expect("frame");
        writer.await.unwrap();
        assert_eq!(frame.kind, KIND_OUTPUT);
        assert_eq!(frame.payload.len(), OUTPUT_SESSION_ID_SIZE + 2);
        let (id, bytes) = decode_output_payload(&frame.payload).unwrap();
        assert_eq!(id, 0x0102_0304_0506_0708);
        assert_eq!(bytes, b"xy");
    }

    #[test]
    fn decode_output_payload_rejects_short_body() {
        // Seven bytes is one short of the 8-byte session-id prefix.
        let err = decode_output_payload(&[0u8; 7]).unwrap_err();
        match err {
            ProtocolError::Io(e) => assert_eq!(e.kind(), std::io::ErrorKind::InvalidData),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn decode_output_payload_accepts_empty_trailing_bytes() {
        // An 8-byte payload carrying only the session id (no trailing
        // bytes) is legal: the shell can send a zero-byte chunk.
        let payload = 17u64.to_le_bytes();
        let (id, rest) = decode_output_payload(&payload).unwrap();
        assert_eq!(id, 17);
        assert!(rest.is_empty());
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
                shell: None,
                shell_args: vec![],
                workspace_id: 0,
                pane_id: 0,
                name: None,
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
        assert_eq!(
            Request::AttachSession {
                id: 36,
                session_id: 1,
                scrollback_lines: 0,
            }
            .id(),
            36
        );
        assert_eq!(
            Request::DetachSession {
                id: 37,
                session_id: 1,
            }
            .id(),
            37
        );
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
        let snap = unshit_terminal_core::Terminal::new(3, 5, 10).snapshot(0);
        assert_eq!(
            Response::SessionAttached {
                id: 44,
                snapshot: snap,
            }
            .id(),
            44
        );
    }

    #[test]
    fn attach_session_request_round_trips() {
        let req = Request::AttachSession {
            id: 101,
            session_id: 7,
            scrollback_lines: 50,
        };
        let bytes = serde_json::to_vec(&req).unwrap();
        let back: Request = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(req, back);
    }

    #[test]
    fn detach_session_request_round_trips() {
        let req = Request::DetachSession {
            id: 102,
            session_id: 7,
        };
        let bytes = serde_json::to_vec(&req).unwrap();
        let back: Request = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(req, back);
    }

    #[test]
    fn session_attached_response_round_trips() {
        let snapshot = unshit_terminal_core::Terminal::new(3, 5, 10).snapshot(0);
        let resp = Response::SessionAttached { id: 103, snapshot };
        let bytes = serde_json::to_vec(&resp).unwrap();
        let back: Response = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(resp, back);
    }

    #[test]
    fn attach_session_request_serializes_scrollback_lines_field() {
        let req = Request::AttachSession {
            id: 1,
            session_id: 2,
            scrollback_lines: 7,
        };
        let s = serde_json::to_string(&req).unwrap();
        assert!(s.contains("\"kind\":\"attach_session\""), "{s}");
        assert!(s.contains("\"scrollback_lines\":7"), "{s}");
    }

    #[test]
    fn snapshot_max_scrollback_lines_is_one_hundred() {
        // Pin the v1 wire cap so a future refactor has to revisit the
        // deliberate choice documented in the constant's docstring.
        assert_eq!(SNAPSHOT_MAX_SCROLLBACK_LINES, 100);
    }
}
