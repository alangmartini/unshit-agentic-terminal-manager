//! Structured JSONL telemetry for the resource monitor.
//!
//! Uses the shared rotating sink. Records fire on lifecycle transitions
//! (start, daemon list failing or recovering) and as a one-per-minute
//! summary, never per tick, so a quiet app does not churn the file.
//! Privacy invariant: counts, durations and error kinds only. No pids of
//! other users' processes, no command lines, no paths.

use std::path::PathBuf;

use serde::Serialize;

pub use crate::telemetry_sink::now_unix_ms;

#[derive(Debug, Serialize)]
pub struct ResourceEventRecord {
    pub timestamp_unix_ms: u64,
    /// Dotted event name, e.g. `resource.monitor_started`,
    /// `resource.summary`, `resource.list_failed`.
    pub event: &'static str,
    pub level: &'static str,
    /// `process-<ui pid>`: correlates one UI instance's records.
    pub correlation_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interval_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logical_cpus: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform_supported: Option<bool>,
    /// Ticks folded into a summary record.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ticks: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sessions: Option<usize>,
    /// Processes attributed to any tree on the last tick.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub processes: Option<usize>,
    /// Attributed processes that could not be opened on the last tick;
    /// explains a total that reads low.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unsampled: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tick_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mean_tick_ms: Option<f64>,
    /// Daemon list requests that failed inside the summary window.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub list_failures: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_pct: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mem_bytes: Option<u64>,
    /// `io::ErrorKind` of a failed daemon list, as text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl ResourceEventRecord {
    pub fn new(event: &'static str, level: &'static str) -> Self {
        Self {
            timestamp_unix_ms: now_unix_ms(),
            event,
            level,
            correlation_id: format!("process-{}", std::process::id()),
            interval_ms: None,
            logical_cpus: None,
            platform_supported: None,
            ticks: None,
            sessions: None,
            processes: None,
            unsampled: None,
            max_tick_ms: None,
            mean_tick_ms: None,
            list_failures: None,
            cpu_pct: None,
            mem_bytes: None,
            reason: None,
        }
    }
}

pub fn default_path() -> Option<PathBuf> {
    crate::profile::config_dir().map(|directory| directory.join("resource-events.jsonl"))
}

/// Append a record to the bounded JSONL sink and mirror it to the process
/// log. Sink failures degrade to a warn log so a broken sink never affects
/// sampling.
pub fn record(record: &ResourceEventRecord) {
    match serde_json::to_string(record) {
        Ok(line) => match record.level {
            "warn" => log::warn!("{line}"),
            "debug" => log::debug!("{line}"),
            _ => log::info!("{line}"),
        },
        Err(_) => {}
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
            "{{\"event\":\"resource.telemetry_write_failed\",\"level\":\"warn\",\"correlation_id\":{:?},\"source_event\":{:?}}}",
            record.correlation_id,
            record.event
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unset_fields_are_omitted_from_the_line() {
        let mut record = ResourceEventRecord::new("resource.summary", "info");
        record.ticks = Some(60);
        record.cpu_pct = Some(3.5);
        let line = serde_json::to_string(&record).unwrap();
        assert!(line.contains("\"event\":\"resource.summary\""));
        assert!(line.contains("\"ticks\":60"));
        assert!(!line.contains("reason"), "{line}");
        assert!(!line.contains("sessions"), "{line}");
        assert!(record.correlation_id.starts_with("process-"));
    }
}
