use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use serde_json::{json, Value};
use terminal_manager_diagnostics::{
    DiagnosticEnvelope, DiagnosticEvent, DiagnosticEventFamily, EVENT_SCHEMA_VERSION,
};

const DEFAULT_EVENT_CAPACITY: usize = 4_096;

#[derive(Debug, Clone)]
pub struct DiagnosticEventStore {
    inner: Arc<Mutex<EventStoreInner>>,
    started_at: Instant,
    capacity: usize,
}

#[derive(Debug)]
struct EventStoreInner {
    next_seq: u64,
    current_step_id: Option<String>,
    dropped_events: u64,
    events: VecDeque<DiagnosticEnvelope<DiagnosticEvent>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EventDrain {
    pub events: Vec<DiagnosticEnvelope<DiagnosticEvent>>,
    pub dropped_events: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventFlushSummary {
    pub visible_events: u64,
    pub dropped_events: u64,
}

impl Default for DiagnosticEventStore {
    fn default() -> Self {
        Self::with_capacity(DEFAULT_EVENT_CAPACITY)
    }
}

impl DiagnosticEventStore {
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(EventStoreInner {
                next_seq: 1,
                current_step_id: None,
                dropped_events: 0,
                events: VecDeque::with_capacity(capacity),
            })),
            started_at: Instant::now(),
            capacity,
        }
    }

    pub fn mark_step(
        &self,
        id: impl Into<String>,
        label: impl Into<String>,
    ) -> DiagnosticEnvelope<DiagnosticEvent> {
        let id = id.into();
        let label = label.into();
        let mut inner = self.inner.lock().unwrap_or_else(|err| err.into_inner());
        inner.current_step_id = Some(id.clone());
        self.push_locked(
            &mut inner,
            Some(id),
            DiagnosticEventFamily::TestStep,
            "diagnostics.step",
            "marked",
            json!({ "label": label }),
            None,
        )
    }

    pub fn clear_step(&self, reason: Option<String>) -> DiagnosticEnvelope<DiagnosticEvent> {
        let mut inner = self.inner.lock().unwrap_or_else(|err| err.into_inner());
        let previous_step_id = inner.current_step_id.take();
        self.push_locked(
            &mut inner,
            previous_step_id.clone(),
            DiagnosticEventFamily::TestStep,
            "diagnostics.step",
            "cleared",
            json!({
                "previous_step_id": previous_step_id,
                "reason": reason,
            }),
            None,
        )
    }

    pub fn record_event(
        &self,
        family: DiagnosticEventFamily,
        target: impl Into<String>,
        kind: impl Into<String>,
        fields: Value,
    ) -> DiagnosticEnvelope<DiagnosticEvent> {
        let mut inner = self.inner.lock().unwrap_or_else(|err| err.into_inner());
        let step_id = inner.current_step_id.clone();
        self.push_locked(&mut inner, step_id, family, target, kind, fields, None)
    }

    pub fn record_log(
        &self,
        level: impl Into<String>,
        target: impl Into<String>,
        kind: impl Into<String>,
        fields: Value,
    ) -> DiagnosticEnvelope<DiagnosticEvent> {
        let level = level.into();
        let mut log_fields = match fields {
            Value::Object(map) => map,
            value => {
                let mut map = serde_json::Map::new();
                map.insert("value".to_owned(), value);
                map
            }
        };
        log_fields.insert("level".to_owned(), Value::String(level));
        self.record_event(
            DiagnosticEventFamily::Log,
            target,
            kind,
            Value::Object(log_fields),
        )
    }

    pub fn flush_summary(&self) -> EventFlushSummary {
        let inner = self.inner.lock().unwrap_or_else(|err| err.into_inner());
        EventFlushSummary {
            visible_events: inner.events.len() as u64,
            dropped_events: inner.dropped_events,
        }
    }

    pub fn drain(&self, limit: Option<usize>) -> EventDrain {
        let mut inner = self.inner.lock().unwrap_or_else(|err| err.into_inner());
        let requested = limit.unwrap_or(inner.events.len());
        let count = requested.min(inner.events.len());
        let events = inner.events.drain(..count).collect();
        EventDrain {
            events,
            dropped_events: inner.dropped_events,
        }
    }

    fn push_locked(
        &self,
        inner: &mut EventStoreInner,
        step_id: Option<String>,
        family: DiagnosticEventFamily,
        target: impl Into<String>,
        kind: impl Into<String>,
        fields: Value,
        correlation_id: Option<String>,
    ) -> DiagnosticEnvelope<DiagnosticEvent> {
        let event = DiagnosticEnvelope {
            schema_version: EVENT_SCHEMA_VERSION.to_owned(),
            seq: inner.next_seq,
            timestamp_utc: now_utc_string(),
            monotonic_ms: self
                .started_at
                .elapsed()
                .as_millis()
                .min(u128::from(u64::MAX)) as u64,
            test_step_id: step_id,
            correlation_id,
            payload: DiagnosticEvent {
                family,
                thread: current_thread_label(),
                target: target.into(),
                kind: kind.into(),
                fields,
            },
        };
        inner.next_seq = inner.next_seq.saturating_add(1);
        if self.capacity == 0 {
            inner.dropped_events = inner.dropped_events.saturating_add(1);
            return event;
        }
        if inner.events.len() == self.capacity {
            inner.events.pop_front();
            inner.dropped_events = inner.dropped_events.saturating_add(1);
        }
        inner.events.push_back(event.clone());
        event
    }
}

fn current_thread_label() -> String {
    let thread = std::thread::current();
    thread
        .name()
        .map(str::to_owned)
        .unwrap_or_else(|| format!("{:?}", thread.id()))
}

fn now_utc_string() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format_unix_seconds_utc(seconds)
}

fn format_unix_seconds_utc(seconds: u64) -> String {
    let days = (seconds / 86_400) as i64;
    let seconds_of_day = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    let hour = seconds_of_day / 3_600;
    let minute = (seconds_of_day % 3_600) / 60;
    let second = seconds_of_day % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

fn civil_from_days(days_since_unix_epoch: i64) -> (i64, u32, u32) {
    let z = days_since_unix_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    (year, month as u32, day as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_are_sequenced_and_jsonl_serializable() {
        let events = DiagnosticEventStore::with_capacity(8);

        let first = events.record_event(
            DiagnosticEventFamily::Window,
            "window.main",
            "created",
            json!({ "focused": true }),
        );
        let second = events.record_event(
            DiagnosticEventFamily::Render,
            "renderer.surface",
            "frame_presented",
            json!({ "frame": 1 }),
        );

        assert_eq!(first.seq, 1);
        assert_eq!(second.seq, 2);
        assert_eq!(first.schema_version, EVENT_SCHEMA_VERSION);
        assert!(second.monotonic_ms >= first.monotonic_ms);
        assert!(!first.timestamp_utc.is_empty());

        let line = serde_json::to_string(&second).unwrap();
        assert!(line.contains("\"family\":\"render\""));
        assert!(!line.contains('\n'));
    }

    #[test]
    fn step_marker_correlates_subsequent_events_until_cleared() {
        let events = DiagnosticEventStore::with_capacity(8);

        events.mark_step("snap-left", "Snap left");
        let correlated = events.record_event(
            DiagnosticEventFamily::Layout,
            "layout.root",
            "computed",
            json!({}),
        );
        events.clear_step(Some("done".to_owned()));
        let uncorrelated = events.record_event(
            DiagnosticEventFamily::Input,
            "input.keyboard",
            "key",
            json!({ "key": "Escape" }),
        );

        assert_eq!(correlated.test_step_id.as_deref(), Some("snap-left"));
        assert!(uncorrelated.test_step_id.is_none());
    }

    #[test]
    fn capped_queue_reports_dropped_events() {
        let events = DiagnosticEventStore::with_capacity(2);

        events.record_event(DiagnosticEventFamily::Window, "window", "one", json!({}));
        events.record_event(DiagnosticEventFamily::Window, "window", "two", json!({}));
        events.record_event(DiagnosticEventFamily::Window, "window", "three", json!({}));

        let summary = events.flush_summary();
        assert_eq!(summary.visible_events, 2);
        assert_eq!(summary.dropped_events, 1);

        let drained = events.drain(None);
        assert_eq!(drained.dropped_events, 1);
        assert_eq!(drained.events.len(), 2);
        assert_eq!(drained.events[0].payload.kind, "two");
        assert_eq!(drained.events[1].payload.kind, "three");
    }

    #[test]
    fn all_initial_event_families_can_be_recorded() {
        let events = DiagnosticEventStore::with_capacity(16);

        for family in DiagnosticEventFamily::all() {
            events.record_event(*family, "diagnostics.synthetic", "observed", json!({}));
        }

        let drained = events.drain(None);
        for family in DiagnosticEventFamily::all() {
            assert!(drained
                .events
                .iter()
                .any(|event| event.payload.family == *family));
        }
    }

    #[test]
    fn warning_and_error_logs_are_structured_log_events() {
        let events = DiagnosticEventStore::with_capacity(8);

        events.record_log(
            "warning",
            "diagnostics.logger",
            "warning",
            json!({ "message": "diagnostic warning" }),
        );
        events.record_log(
            "error",
            "diagnostics.logger",
            "error",
            json!({ "message": "diagnostic error" }),
        );

        let drained = events.drain(None);
        assert_eq!(drained.events.len(), 2);
        assert!(drained.events.iter().all(|event| {
            event.payload.family == DiagnosticEventFamily::Log
                && event.payload.target == "diagnostics.logger"
        }));
        assert_eq!(drained.events[0].payload.fields["level"], "warning");
        assert_eq!(drained.events[1].payload.fields["level"], "error");
    }
}
