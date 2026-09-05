//! Structured JSONL telemetry for the Flow Explorer.
//!
//! Mirrors the editor sink: bounded, rotating, queryable records keyed by a
//! per-document `flow_id`. Privacy invariant: records carry the flow id,
//! the source path, mode, counts, reasons and timings, and **never** node
//! names, descriptions or source text. Events fire on lifecycle
//! transitions only (launch, ready, open, close, view change), never per
//! frame.

use std::path::PathBuf;

use serde::Serialize;

pub use crate::telemetry_sink::now_unix_ms;

#[derive(Debug, Serialize)]
pub struct FlowEventRecord<'a> {
    pub timestamp_unix_ms: u64,
    /// Dotted event name, e.g. `flow.open`, `flow.parse_failed`.
    pub event: &'static str,
    pub level: &'static str,
    /// Correlates launch → ready → open → close for one flow document.
    pub flow_id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edge_count: Option<u64>,
    /// Machine-readable failure reason (`invalid_json`, `validation`, ...).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elapsed_ms: Option<u64>,
    /// Raw OS error code for `io` failures.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub os_error: Option<i32>,
    /// View name for `flow.view_changed`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub view: Option<&'static str>,
}

impl<'a> FlowEventRecord<'a> {
    /// A record with every optional field unset; callers fill what applies.
    pub fn new(event: &'static str, level: &'static str, flow_id: &'a str) -> Self {
        Self {
            timestamp_unix_ms: now_unix_ms(),
            event,
            level,
            flow_id,
            path: None,
            mode: None,
            agent: None,
            node_count: None,
            edge_count: None,
            reason: None,
            elapsed_ms: None,
            os_error: None,
            view: None,
        }
    }
}

pub fn default_path() -> Option<PathBuf> {
    crate::profile::config_dir().map(|directory| directory.join("flow-events.jsonl"))
}

/// Append a flow event to the bounded JSONL sink. Failures degrade to a
/// structured warn log so a broken sink never affects the feature.
pub fn record_flow_event(record: &FlowEventRecord<'_>) {
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
            "{{\"event\":\"flow.telemetry_write_failed\",\"level\":\"warn\",\"flow_id\":{:?},\"source_event\":{:?}}}",
            record.flow_id,
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
            "tm-flow-telemetry-{}-{}-{}",
            tag,
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("flow-events.jsonl")
    }

    #[test]
    fn open_event_is_queryable_and_carries_no_content() {
        let path = unique_temp_path("open");
        let mut record = FlowEventRecord::new("flow.open", "info", "send-a-prompt");
        record.path = Some("C:/flows/send-a-prompt.json");
        record.mode = Some("explain");
        record.node_count = Some(11);
        record.edge_count = Some(10);
        crate::telemetry_sink::append_rotating_jsonl(&path, &record, 512 * 1024).unwrap();
        let value: serde_json::Value =
            serde_json::from_str(std::fs::read_to_string(&path).unwrap().trim()).unwrap();
        assert_eq!(value["event"], "flow.open");
        assert_eq!(value["flow_id"], "send-a-prompt");
        assert_eq!(value["node_count"], 11);
        assert_eq!(value["mode"], "explain");
        assert!(value.get("reason").is_none());
        // Privacy invariant: no content-bearing fields, ever.
        for forbidden in [
            "title",
            "summary",
            "nodes",
            "name",
            "description",
            "source",
            "text",
        ] {
            assert!(value.get(forbidden).is_none(), "{forbidden}");
        }
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn failure_record_carries_reason_and_os_error() {
        let path = unique_temp_path("fail");
        let mut record = FlowEventRecord::new("flow.open_failed", "warn", "broken");
        record.reason = Some("invalid_json");
        record.os_error = Some(2);
        crate::telemetry_sink::append_rotating_jsonl(&path, &record, 512 * 1024).unwrap();
        let value: serde_json::Value =
            serde_json::from_str(std::fs::read_to_string(&path).unwrap().trim()).unwrap();
        assert_eq!(value["reason"], "invalid_json");
        assert_eq!(value["os_error"], 2);
        assert_eq!(value["level"], "warn");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
