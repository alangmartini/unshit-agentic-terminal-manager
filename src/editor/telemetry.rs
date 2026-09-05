//! Structured JSONL telemetry for the built-in file editor.
//!
//! Mirrors `crate::renderer_telemetry`: bounded, rotating, queryable
//! records with a per-editor-pane correlation id. Privacy invariant:
//! records carry paths, sizes, and reasons — **never file content**.
//! Events are emitted only on lifecycle transitions (open/save/close),
//! keeping telemetry off the keystroke hot path.

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
    crate::telemetry_sink::now_unix_ms()
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
    crate::telemetry_sink::append_rotating_jsonl(
        path,
        event,
        crate::telemetry_sink::DEFAULT_MAX_LOG_BYTES,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn unique_temp_path(tag: &str) -> std::path::PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "tm-editor-telemetry-{}-{}-{}",
            tag,
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        // PID reuse makes this name reachable again in a later run, and
        // `record_to` appends, so a stale leftover would corrupt the read.
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("editor-events.jsonl")
    }

    fn remove_temp(path: &std::path::Path) {
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn editor_event_is_queryable_and_contains_no_file_content() {
        let path = unique_temp_path("open");
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
        remove_temp(&path);
    }

    #[test]
    fn failure_record_carries_reason() {
        let path = unique_temp_path("fail");
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
        remove_temp(&path);
    }
}
