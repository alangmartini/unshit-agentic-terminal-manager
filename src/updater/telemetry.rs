//! Structured JSONL telemetry for the self-update flow.
//!
//! Sink: `<config_dir>/update-events.jsonl`, bounded and rotating like the
//! agent and resource sinks. Every record carries the UI run's correlation
//! id so one check → prompt → download → install chain can be reconstructed
//! from interleaved lines, plus the running version so a log from an old
//! build reads unambiguously after the upgrade.
//!
//! Privacy invariant: records carry versions, sources, outcomes, error kinds,
//! byte counts, process ids and the install scope. They never carry URLs,
//! file paths, response bodies or release notes.
//!
//! Event names (`event`): `update.init`, `update.startup_check_skipped`,
//! `update.check_started`, `update.check_completed`, `update.check_failed`,
//! `update.prompt_shown`, `update.prompt_skipped`, `update.prompt_dismissed`,
//! `update.startup_check_toggled`, `update.release_page_opened`,
//! `update.release_page_failed`, `update.install_redirected`,
//! `update.download_started`, `update.download_completed`,
//! `update.download_failed`, `update.layout_persisted`,
//! `update.install_launched`, `update.install_failed`,
//! `update.daemon_shutdown`, `update.exiting`, `update.stale_downloads_removed`,
//! `update.worker_spawn_failed`.

use std::path::PathBuf;

use serde::Serialize;

pub use crate::telemetry_sink::now_unix_ms;

#[derive(Debug, Serialize)]
pub struct UpdateEventRecord<'a> {
    pub timestamp_unix_ms: u64,
    pub event: &'static str,
    pub level: &'static str,
    /// `AppState::restore_correlation_id`: one id per UI run.
    pub correlation_id: &'a str,
    /// Version of the binary writing the record.
    pub current_version: &'static str,
    /// `startup` / `manual` / `install`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_kind: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub http_status: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elapsed_ms: Option<u64>,
    /// `current_user` / `all_users` for installed copies.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<&'static str>,
    /// Installer process id on `update.install_launched`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    /// `true` when `TM_UPDATE_FEED_URL` replaced the GitHub feed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub feed_overridden: Option<bool>,
}

impl<'a> UpdateEventRecord<'a> {
    pub fn new(event: &'static str, level: &'static str, correlation_id: &'a str) -> Self {
        Self {
            timestamp_unix_ms: now_unix_ms(),
            event,
            level,
            correlation_id,
            current_version: env!("CARGO_PKG_VERSION"),
            source: None,
            latest_version: None,
            outcome: None,
            reason: None,
            error_kind: None,
            http_status: None,
            bytes: None,
            total_bytes: None,
            elapsed_ms: None,
            scope: None,
            pid: None,
            feed_overridden: None,
        }
    }
}

pub fn default_path() -> Option<PathBuf> {
    crate::profile::config_dir().map(|directory| directory.join("update-events.jsonl"))
}

/// Append one record to the sink. A broken sink degrades to a structured
/// warn log and never affects the feature.
pub fn record(record: &UpdateEventRecord<'_>) {
    // Unit tests drive the state machine thousands of times and must not
    // append to the developer's real profile directory. Tests that want a
    // file go through `telemetry_sink::append_rotating_jsonl` directly.
    if cfg!(test) {
        return;
    }
    let Some(path) = default_path() else {
        return;
    };
    if crate::telemetry_sink::append_rotating_jsonl(
        &path,
        record,
        crate::telemetry_sink::DEFAULT_MAX_LOG_BYTES,
    )
    .is_err()
    {
        log::warn!(
            "{{\"event\":\"update.telemetry_write_failed\",\"level\":\"warn\",\"correlation_id\":{:?},\"source_event\":{:?}}}",
            record.correlation_id,
            record.event
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn unique_temp_path(tag: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "tm-update-telemetry-{}-{}-{}",
            tag,
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("update-events.jsonl")
    }

    #[test]
    fn check_record_is_queryable_and_carries_no_urls_or_paths() {
        let path = unique_temp_path("check");
        let mut record = UpdateEventRecord::new("update.check_completed", "info", "run-1");
        record.source = Some("startup");
        record.latest_version = Some("0.5.0".to_string());
        record.outcome = Some("available");
        crate::telemetry_sink::append_rotating_jsonl(&path, &record, 512 * 1024).unwrap();
        let value: serde_json::Value =
            serde_json::from_str(std::fs::read_to_string(&path).unwrap().trim()).unwrap();
        assert_eq!(value["event"], "update.check_completed");
        assert_eq!(value["correlation_id"], "run-1");
        assert_eq!(value["current_version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(value["latest_version"], "0.5.0");
        assert_eq!(value["outcome"], "available");
        assert!(value.get("error_kind").is_none());
        for forbidden in ["url", "path", "body", "notes", "installer"] {
            assert!(value.get(forbidden).is_none(), "{forbidden}");
        }
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn failure_record_carries_error_kind_and_http_status() {
        let path = unique_temp_path("failed");
        let mut record = UpdateEventRecord::new("update.check_failed", "warn", "run-2");
        record.error_kind = Some("http");
        record.http_status = Some(403);
        crate::telemetry_sink::append_rotating_jsonl(&path, &record, 512 * 1024).unwrap();
        let value: serde_json::Value =
            serde_json::from_str(std::fs::read_to_string(&path).unwrap().trim()).unwrap();
        assert_eq!(value["level"], "warn");
        assert_eq!(value["error_kind"], "http");
        assert_eq!(value["http_status"], 403);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
