//! Structured JSONL telemetry for terminal panes.
//!
//! Mirrors `crate::editor::telemetry`: bounded, rotating, queryable
//! records keyed by pane id, in `<config_dir>/terminal-events.jsonl`.
//! Read it FIRST on a selection or scrolling bug report: it answers the
//! two questions such reports hinge on — which DEC private modes the
//! running program toggled (alternate screen, mouse reporting), and what
//! the selection edge auto-scroll did or why it refused.
//!
//! Privacy invariant: records carry pane ids, mode numbers, line counts
//! and machine-readable reasons — **never terminal content**. Mode events
//! fire only on actual transitions (a TUI re-asserting its modes on every
//! repaint costs nothing), and auto-scroll is summarized once per drag,
//! so nothing here sits on the per-frame or per-byte hot path.

use std::path::Path;

use serde::Serialize;

#[derive(Debug, Default, Serialize)]
pub struct TerminalEventRecord {
    pub timestamp_unix_ms: u64,
    /// Dotted event name: `terminal.mode_changed`,
    /// `terminal.selection_autoscroll`,
    /// `terminal.selection_autoscroll_blocked`.
    pub event: &'static str,
    pub level: &'static str,
    /// Pane id; correlates with the `pane=` fields of the pty and agent
    /// sinks.
    pub pane: u32,
    /// DEC private mode number for `terminal.mode_changed`: `1049` for any
    /// alternate-screen alias (47/1047/1049), `1000`/`1002`/`1003` mouse
    /// tracking, `1006` SGR mouse encoding.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// Net scroll direction of an auto-scroll summary: `up` (toward older
    /// history) or `down` (toward the live screen).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub direction: Option<&'static str>,
    /// Signed lines applied over the whole drag (positive = toward older).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lines: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// Why the drag's auto-scroll ended (`released`, `reentered`) or was
    /// refused (`mouse_reporting`: the program owns the mouse and scrolls
    /// its own viewport; `alt_screen`: no scrollback to reveal).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<&'static str>,
}

pub fn now_unix_ms() -> u64 {
    crate::telemetry_sink::now_unix_ms()
}

/// Append a terminal event to the bounded JSONL sink. Failures degrade to
/// a structured warn log so a broken sink never affects the terminal.
pub fn record_terminal_event(record: &TerminalEventRecord) {
    let Some(path) = default_path() else {
        return;
    };
    if record_to(&path, record).is_err() {
        log::warn!(
            "{{\"event\":\"terminal.telemetry_write_failed\",\"level\":\"warn\",\"pane\":{},\"source_event\":{:?}}}",
            record.pane,
            record.event
        );
    }
}

pub fn default_path() -> Option<std::path::PathBuf> {
    crate::profile::config_dir().map(|directory| directory.join("terminal-events.jsonl"))
}

fn record_to(path: &Path, event: &TerminalEventRecord) -> std::io::Result<()> {
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
            "tm-terminal-telemetry-{}-{}-{}",
            tag,
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        // PID reuse makes this name reachable again in a later run, and
        // `record_to` appends, so a stale leftover would corrupt the read.
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("terminal-events.jsonl")
    }

    fn remove_temp(path: &std::path::Path) {
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    fn read_record(path: &std::path::Path) -> serde_json::Value {
        serde_json::from_str(
            std::fs::read_to_string(path)
                .expect("read telemetry")
                .trim(),
        )
        .expect("valid JSONL record")
    }

    #[test]
    fn mode_change_record_is_queryable_by_pane_and_mode() {
        let path = unique_temp_path("mode");
        let record = TerminalEventRecord {
            timestamp_unix_ms: 123,
            event: "terminal.mode_changed",
            level: "info",
            pane: 7,
            mode: Some(1049),
            enabled: Some(true),
            ..Default::default()
        };
        record_to(&path, &record).expect("write terminal telemetry");
        let value = read_record(&path);
        assert_eq!(value["event"], "terminal.mode_changed");
        assert_eq!(value["pane"], 7);
        assert_eq!(value["mode"], 1049);
        assert_eq!(value["enabled"], true);
        // Auto-scroll-only fields are omitted, not nulled.
        assert!(value.get("lines").is_none());
        assert!(value.get("reason").is_none());
        remove_temp(&path);
    }

    #[test]
    fn autoscroll_summary_carries_direction_lines_and_reason_but_no_content() {
        let path = unique_temp_path("autoscroll");
        let record = TerminalEventRecord {
            timestamp_unix_ms: 5,
            event: "terminal.selection_autoscroll",
            level: "info",
            pane: 3,
            direction: Some("up"),
            lines: Some(42),
            duration_ms: Some(900),
            reason: Some("released"),
            ..Default::default()
        };
        record_to(&path, &record).expect("write terminal telemetry");
        let value = read_record(&path);
        assert_eq!(value["direction"], "up");
        assert_eq!(value["lines"], 42);
        assert_eq!(value["duration_ms"], 900);
        assert_eq!(value["reason"], "released");
        // Privacy invariant: no content-bearing fields, ever.
        assert!(value.get("text").is_none());
        assert!(value.get("content").is_none());
        assert!(value.get("selection").is_none());
        remove_temp(&path);
    }
}
