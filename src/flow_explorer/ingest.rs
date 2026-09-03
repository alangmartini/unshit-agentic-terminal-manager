//! Reading a flow document off disk: size cap, parse, validate.
//!
//! Shared by the manual `flow.open:<path>` path and the launch poller so
//! both surface the same reasons in toasts and telemetry.

use std::path::Path;

use super::model::{parse_flow, Flow, FlowParseError};

/// A flow document larger than this is refused before it is read.
pub const MAX_FLOW_JSON_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Debug)]
pub enum IngestError {
    TooLarge(u64),
    Io(std::io::Error),
    Parse(FlowParseError),
}

impl IngestError {
    /// Machine-readable reason for telemetry.
    pub fn reason(&self) -> &'static str {
        match self {
            IngestError::TooLarge(_) => "too_large",
            IngestError::Io(e) if e.kind() == std::io::ErrorKind::NotFound => "not_found",
            IngestError::Io(_) => "io",
            IngestError::Parse(e) => e.reason(),
        }
    }

    pub fn os_error(&self) -> Option<i32> {
        match self {
            IngestError::Io(e) => e.raw_os_error(),
            _ => None,
        }
    }

    /// Human-readable message for the failure toast.
    pub fn message(&self, path: &Path) -> String {
        let name = display_name(path);
        match self {
            IngestError::TooLarge(bytes) => format!(
                "{} is too large for a flow ({} MiB, limit {} MiB)",
                name,
                bytes / (1024 * 1024),
                MAX_FLOW_JSON_BYTES / (1024 * 1024)
            ),
            IngestError::Io(e) if e.kind() == std::io::ErrorKind::NotFound => {
                format!("flow file not found: {}", name)
            }
            IngestError::Io(e) => format!("could not read {}: {}", name, e),
            IngestError::Parse(e) => format!("{}: {}", name, e),
        }
    }
}

/// File name for titles and toasts; falls back to the full path string.
pub fn display_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

pub fn ingest_file(path: &Path) -> Result<Flow, IngestError> {
    ingest_file_with_cap(path, MAX_FLOW_JSON_BYTES)
}

/// `ingest_file` with an explicit size cap (tests).
pub fn ingest_file_with_cap(path: &Path, max_bytes: u64) -> Result<Flow, IngestError> {
    let metadata = std::fs::metadata(path).map_err(IngestError::Io)?;
    if metadata.len() > max_bytes {
        return Err(IngestError::TooLarge(metadata.len()));
    }
    let bytes = std::fs::read(path).map_err(IngestError::Io)?;
    parse_flow(&bytes).map_err(IngestError::Parse)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow_explorer::test_support::fixture_path;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn temp_flow(tag: &str, contents: &[u8]) -> std::path::PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "tm-flow-ingest-{}-{}-{}",
            tag,
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("flow.json");
        std::fs::write(&path, contents).unwrap();
        path
    }

    fn cleanup(path: &Path) {
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn fixture_ingests() {
        let flow = ingest_file(&fixture_path()).unwrap();
        assert_eq!(flow.nodes.len(), 11);
    }

    #[test]
    fn missing_file_is_not_found() {
        let path = Path::new("C:/definitely/not/here/flow.json");
        let err = ingest_file(path).unwrap_err();
        assert_eq!(err.reason(), "not_found");
        assert!(err.message(path).contains("flow.json"));
    }

    #[test]
    fn oversized_file_is_refused_before_parsing() {
        let err = ingest_file_with_cap(&fixture_path(), 16).unwrap_err();
        assert_eq!(err.reason(), "too_large");
        assert!(err.message(&fixture_path()).contains("too large"));
    }

    #[test]
    fn broken_json_and_producer_errors_keep_their_reasons() {
        let broken = temp_flow("broken", b"{ not json");
        let err = ingest_file(&broken).unwrap_err();
        assert_eq!(err.reason(), "invalid_json");
        assert!(err.os_error().is_none());
        cleanup(&broken);

        let failed = temp_flow(
            "producer",
            br#"{"schema_version":1,"error":"no entry points matched the request"}"#,
        );
        let err = ingest_file(&failed).unwrap_err();
        assert_eq!(err.reason(), "producer_error");
        assert!(err.message(&failed).contains("no entry points"));
        cleanup(&failed);
    }
}
