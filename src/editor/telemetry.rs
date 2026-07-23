//! Structured JSONL telemetry for the built-in file editor.
//!
//! Mirrors `crate::renderer_telemetry`: bounded, rotating, queryable
//! records with a per-editor-pane correlation id. Privacy invariant:
//! records carry paths, sizes, and reasons — **never file content**.
//! Events are emitted only on lifecycle transitions (open/save/close),
//! keeping telemetry off the keystroke hot path.

use std::io::Write;
use std::path::Path;

use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct EditorEventRecord<'a> {
    pub timestamp_unix_ms: u64,
    /// Dotted event name, e.g. `editor.open`, `editor.save_failed`.
    pub event: &'static str,
    pub level: &'static str,
    /// One id per editor pane instance; correlates open→save→close.
    pub correlation_id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_count: Option<u64>,
    /// Machine-readable failure reason (`too_large`, `invalid_utf8`, `io`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<&'static str>,
    /// Raw OS error code for `io` failures, so the sink alone can tell
    /// readonly (5) from disk-full (112) from sharing violations (32).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub os_error: Option<i32>,
}

pub fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

/// Append an editor event to the bounded JSONL sink. Failures degrade to
/// a structured warn log so a broken sink never affects editing.
pub fn record_editor_event(record: &EditorEventRecord<'_>) {
    let Some(path) = default_path() else {
        return;
    };
    if record_to(&path, record).is_err() {
        log::warn!(
            "{{\"event\":\"editor.telemetry_write_failed\",\"level\":\"warn\",\"correlation_id\":{:?},\"source_event\":{:?}}}",
            record.correlation_id,
            record.event
        );
    }
}

pub fn default_path() -> Option<std::path::PathBuf> {
    crate::profile::config_dir().map(|directory| directory.join("editor-events.jsonl"))
}

fn record_to(path: &Path, event: &EditorEventRecord<'_>) -> std::io::Result<()> {
    const MAX_LOG_BYTES: u64 = 512 * 1024;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if path
        .metadata()
        .map(|metadata| metadata.len() >= MAX_LOG_BYTES)
        .unwrap_or(false)
    {
        let rotated = path.with_extension("jsonl.1");
        if rotated.exists() {
            std::fs::remove_file(&rotated)?;
        }
        std::fs::rename(path, rotated)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    let mut line = serde_json::to_vec(event).map_err(std::io::Error::other)?;
    line.push(b'\n');
    file.write_all(&line)?;
    file.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[test]
    fn editor_event_is_queryable_and_contains_no_file_content() {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir()
            .join(format!(
                "tm-editor-telemetry-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            ))
            .join("editor-events.jsonl");
        let record = EditorEventRecord {
            timestamp_unix_ms: 123,
            event: "editor.open",
            level: "info",
            correlation_id: "editor-test",
            path: Some("C:/tmp/example.rs"),
            file_bytes: Some(42),
            line_count: Some(3),
            reason: None,
            os_error: None,
        };

        record_to(&path, &record).expect("write editor telemetry");
        let value: serde_json::Value = serde_json::from_str(
            std::fs::read_to_string(&path)
                .expect("read telemetry")
                .trim(),
        )
        .expect("valid JSONL record");
        assert_eq!(value["event"], "editor.open");
        assert_eq!(value["correlation_id"], "editor-test");
        assert_eq!(value["file_bytes"], 42);
        // Privacy invariant: no content-bearing fields, ever.
        assert!(value.get("text").is_none());
        assert!(value.get("content").is_none());
        assert!(value.get("lines").is_none());
    }

    #[test]
    fn failure_record_carries_reason() {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir()
            .join(format!(
                "tm-editor-telemetry-fail-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            ))
            .join("editor-events.jsonl");
        let record = EditorEventRecord {
            timestamp_unix_ms: 5,
            event: "editor.open_failed",
            level: "warn",
            correlation_id: "editor-test-fail",
            path: Some("C:/tmp/huge.bin"),
            file_bytes: Some(999_999_999),
            line_count: None,
            reason: Some("too_large"),
            os_error: None,
        };
        record_to(&path, &record).expect("write editor telemetry");
        let value: serde_json::Value = serde_json::from_str(
            std::fs::read_to_string(&path)
                .expect("read telemetry")
                .trim(),
        )
        .expect("valid JSONL record");
        assert_eq!(value["reason"], "too_large");
    }
}
