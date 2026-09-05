//! Structured JSONL telemetry for agent classification and launches.
//!
//! Sink: `<config_dir>/agent-events.jsonl`, bounded and rotating like the
//! Flow Explorer and editor sinks. Every record carries the UI run's
//! correlation id plus workspace/pane ids so one pane's history can be
//! reconstructed from interleaved lines.
//!
//! Privacy invariant: records carry profile ids, sources, match reasons
//! and outcomes, and **never** the guest title text — Claude Code puts
//! the task summary in its window title. Events fire on transitions only
//! (tag gained, tag lost, launch, CLI request), never per title update.

use std::path::PathBuf;

use serde::Serialize;

pub use crate::telemetry_sink::now_unix_ms;

#[derive(Debug, Serialize)]
pub struct AgentEventRecord<'a> {
    pub timestamp_unix_ms: u64,
    /// Dotted event name: `agent.classified`, `agent.untagged`,
    /// `agent.launch`, `agent.launch_failed`, `agent.cli`.
    pub event: &'static str,
    pub level: &'static str,
    /// `AppState::restore_correlation_id`: one id per UI run.
    pub correlation_id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pane_id: Option<u32>,
    /// `AgentProfile::id`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<&'a str>,
    /// Tag source (`launched` / `hook` / `title`) or launch origin
    /// (`command` / `ctx_menu` / `cli` / `quick_prompt`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<&'static str>,
    /// Machine-readable reason: `glyph`, `needle`, `title_cleared`,
    /// `unknown_profile`, `workspace_not_found`, ...
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_kind: Option<&'static str>,
}

impl<'a> AgentEventRecord<'a> {
    pub fn new(event: &'static str, level: &'static str, correlation_id: &'a str) -> Self {
        Self {
            timestamp_unix_ms: now_unix_ms(),
            event,
            level,
            correlation_id,
            workspace_id: None,
            pane_id: None,
            profile: None,
            source: None,
            reason: None,
            outcome: None,
            error_kind: None,
        }
    }
}

pub fn default_path() -> Option<PathBuf> {
    crate::profile::config_dir().map(|directory| directory.join("agent-events.jsonl"))
}

/// Append one record to the sink. A broken sink degrades to a structured
/// warn log and never affects the feature.
pub fn record(record: &AgentEventRecord<'_>) {
    // Same rule as the agent-restore sink: unit tests exercise the
    // mutators thousands of times and must not append to the developer's
    // real profile directory. Tests that want a file go through
    // `telemetry_sink::append_rotating_jsonl` with their own path.
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
            "{{\"event\":\"agent.telemetry_write_failed\",\"level\":\"warn\",\"correlation_id\":{:?},\"source_event\":{:?}}}",
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
            "tm-agent-telemetry-{}-{}-{}",
            tag,
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("agent-events.jsonl")
    }

    #[test]
    fn classified_record_is_queryable_and_carries_no_title() {
        let path = unique_temp_path("classified");
        let mut record = AgentEventRecord::new("agent.classified", "info", "run-1");
        record.workspace_id = Some(3);
        record.pane_id = Some(7);
        record.profile = Some("claude");
        record.source = Some("title");
        record.reason = Some("glyph");
        crate::telemetry_sink::append_rotating_jsonl(&path, &record, 512 * 1024).unwrap();
        let value: serde_json::Value =
            serde_json::from_str(std::fs::read_to_string(&path).unwrap().trim()).unwrap();
        assert_eq!(value["event"], "agent.classified");
        assert_eq!(value["correlation_id"], "run-1");
        assert_eq!(value["pane_id"], 7);
        assert_eq!(value["profile"], "claude");
        assert_eq!(value["reason"], "glyph");
        assert!(value.get("outcome").is_none());
        for forbidden in ["title", "text", "summary", "prompt", "cwd", "path"] {
            assert!(value.get(forbidden).is_none(), "{forbidden}");
        }
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn failure_record_carries_error_kind() {
        let path = unique_temp_path("failed");
        let mut record = AgentEventRecord::new("agent.launch_failed", "error", "run-2");
        record.profile = Some("codex");
        record.outcome = Some("rejected");
        record.error_kind = Some("spawn_rejected");
        crate::telemetry_sink::append_rotating_jsonl(&path, &record, 512 * 1024).unwrap();
        let value: serde_json::Value =
            serde_json::from_str(std::fs::read_to_string(&path).unwrap().trim()).unwrap();
        assert_eq!(value["level"], "error");
        assert_eq!(value["error_kind"], "spawn_rejected");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
