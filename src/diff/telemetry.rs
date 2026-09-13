//! Structured JSONL telemetry for the diff review pane.
//!
//! Mirrors `crate::editor::telemetry`: bounded, rotating, queryable
//! records with a per-job correlation id so a request, its result and the
//! pane's later navigation can be reconstructed from the sink alone.
//!
//! Privacy invariant: records carry the repository root, the range, file
//! *counts* and sizes — never diff content, never the contents of a file,
//! never a commit message. The one path recorded is the file a user
//! explicitly opened from the diff, which the editor sink would record
//! anyway.

use std::path::Path;

use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct DiffEventRecord<'a> {
    pub timestamp_unix_ms: u64,
    /// Dotted event name, e.g. `diff.request`, `diff.ready`.
    pub event: &'static str,
    pub level: &'static str,
    /// One id per diff job; correlates request → ready/failed → nav.
    pub correlation_id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pane_id: Option<u32>,
    /// The validated range, e.g. `HEAD`, `main..feature`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo_root: Option<&'a str>,
    /// Where the request came from: `dialog`, `palette`, `flow`,
    /// `dispatch`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub files: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hunks: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rows: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stdout_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elapsed_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncated: Option<bool>,
    /// Machine-readable failure reason (`not_a_repo`, `bad_range`,
    /// `git_error`, `too_large`, `spawn`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<&'static str>,
    /// Path opened out of the diff, for `diff.open_file`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u64>,
    /// `hunk` or `file`, for `diff.nav`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<&'static str>,
    /// Hunk steps taken in this pane, reported once on `diff.closed`.
    /// Counted rather than logged per keystroke: `n`/`p`/`]`/`[` are bare
    /// keys, so a held key drove one synchronous file write per repeat.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hunk_steps: Option<u64>,
    /// File steps taken in this pane, reported alongside `hunk_steps`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_steps: Option<u64>,
}

impl<'a> DiffEventRecord<'a> {
    /// A record with only the always-present fields filled in; callers
    /// set what their event carries. Keeps twelve `None`s out of every
    /// call site.
    pub fn new(event: &'static str, level: &'static str, correlation_id: &'a str) -> Self {
        Self {
            timestamp_unix_ms: now_unix_ms(),
            event,
            level,
            correlation_id,
            pane_id: None,
            range: None,
            repo_root: None,
            origin: None,
            files: None,
            hunks: None,
            rows: None,
            stdout_bytes: None,
            elapsed_ms: None,
            truncated: None,
            reason: None,
            path: None,
            line: None,
            kind: None,
            hunk_steps: None,
            file_steps: None,
        }
    }
}

pub fn now_unix_ms() -> u64 {
    crate::telemetry_sink::now_unix_ms()
}

/// One id per diff job, shared by every record about it.
pub fn generate_job_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    format!(
        "diff-{}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed),
        now_unix_ms()
    )
}

/// Append a diff event to the bounded JSONL sink. Failures degrade to a
/// structured warn log so a broken sink never affects reviewing.
pub fn record_diff_event(record: &DiffEventRecord<'_>) {
    let Some(path) = default_path() else {
        return;
    };
    if record_to(&path, record).is_err() {
        log::warn!(
            "{{\"event\":\"diff.telemetry_write_failed\",\"level\":\"warn\",\"correlation_id\":{:?},\"source_event\":{:?}}}",
            record.correlation_id,
            record.event
        );
    }
}

pub fn default_path() -> Option<std::path::PathBuf> {
    crate::profile::config_dir().map(|directory| directory.join("diff-events.jsonl"))
}

fn record_to(path: &Path, event: &DiffEventRecord<'_>) -> std::io::Result<()> {
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
            "tm-diff-telemetry-{}-{}-{}",
            tag,
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("diff-events.jsonl")
    }

    fn remove_temp(path: &std::path::Path) {
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// A completed job must be reconstructable from the sink: which
    /// range, how big the result was, how long it took — and nothing
    /// about what the diff actually said.
    #[test]
    fn ready_record_is_queryable_and_carries_no_diff_content() {
        let path = unique_temp_path("ready");
        let mut record = DiffEventRecord::new("diff.ready", "info", "diff-test-1");
        record.pane_id = Some(7);
        record.range = Some("main..feature");
        record.repo_root = Some("C:/repo");
        record.files = Some(3);
        record.hunks = Some(9);
        record.rows = Some(412);
        record.stdout_bytes = Some(20_480);
        record.elapsed_ms = Some(84);
        record.truncated = Some(false);

        record_to(&path, &record).expect("write diff telemetry");
        let value: serde_json::Value = serde_json::from_str(
            std::fs::read_to_string(&path)
                .expect("read telemetry")
                .trim(),
        )
        .expect("valid JSONL record");

        assert_eq!(value["event"], "diff.ready");
        assert_eq!(value["correlation_id"], "diff-test-1");
        assert_eq!(value["range"], "main..feature");
        assert_eq!(value["files"], 3);
        assert_eq!(value["rows"], 412);
        // Privacy invariant: no content-bearing fields, ever.
        for forbidden in ["diff", "patch", "content", "lines", "text", "message"] {
            assert!(
                value.get(forbidden).is_none(),
                "{forbidden} must never be recorded"
            );
        }
        remove_temp(&path);
    }

    #[test]
    fn failure_record_carries_a_machine_readable_reason() {
        let path = unique_temp_path("fail");
        let mut record = DiffEventRecord::new("diff.failed", "warn", "diff-test-2");
        record.reason = Some("bad_range");
        record.range = Some("--output");
        record.elapsed_ms = Some(2);

        record_to(&path, &record).expect("write diff telemetry");
        let value: serde_json::Value = serde_json::from_str(
            std::fs::read_to_string(&path)
                .expect("read telemetry")
                .trim(),
        )
        .expect("valid JSONL record");
        assert_eq!(value["reason"], "bad_range");
        assert_eq!(value["level"], "warn");
        // Unset fields are omitted rather than serialised as null, so a
        // query for `files` does not match failures.
        assert!(value.get("files").is_none());
        remove_temp(&path);
    }

    #[test]
    fn job_ids_are_unique_within_a_process() {
        let a = generate_job_id();
        let b = generate_job_id();
        assert_ne!(a, b);
        assert!(a.starts_with("diff-"));
    }
}
