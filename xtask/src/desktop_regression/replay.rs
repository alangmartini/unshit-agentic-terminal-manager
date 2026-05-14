use std::collections::BTreeSet;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Component, Path};
use std::sync::Mutex;
use std::time::{Instant, SystemTime};

use serde::{Deserialize, Serialize};
use terminal_manager_diagnostics::{
    is_supported_runner_action_schema_version, RunnerAction, RunnerActionKind, RunnerActionTarget,
    RUNNER_ACTION_SCHEMA_VERSION,
};

use crate::desktop_regression::artifacts::format_utc_timestamp;

pub const ACTION_TRACE_FILE: &str = "runner.actions.jsonl";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionTraceEntry {
    pub schema_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suite_id: Option<String>,
    #[serde(flatten)]
    pub action: RunnerAction,
}

impl ActionTraceEntry {
    fn new(suite_id: Option<&str>, action: RunnerAction) -> Self {
        Self {
            schema_version: RUNNER_ACTION_SCHEMA_VERSION.to_owned(),
            suite_id: suite_id.map(ToOwned::to_owned),
            action,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedTrace {
    pub entries: Vec<ActionTraceEntry>,
    pub suite_ids: Vec<String>,
}

impl ValidatedTrace {
    pub fn action_count(&self) -> u32 {
        self.entries.len() as u32
    }

    pub fn actions_for_suite(&self, suite_id: &str) -> Vec<RunnerAction> {
        let trace_has_suite_ids = !self.suite_ids.is_empty();
        self.entries
            .iter()
            .filter(|entry| {
                if trace_has_suite_ids {
                    entry.suite_id.as_deref() == Some(suite_id)
                } else {
                    entry.suite_id.is_none()
                }
            })
            .map(|entry| entry.action.clone())
            .collect()
    }
}

struct ActionRecorderState {
    file: File,
    entries: Vec<ActionTraceEntry>,
    next_seq: u64,
}

pub struct ActionRecorder {
    start: Instant,
    state: Mutex<ActionRecorderState>,
}

impl ActionRecorder {
    pub fn create(path: &Path) -> Result<Self, String> {
        let file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(path)
            .map_err(|e| format!("failed to create action trace {}: {e}", path.display()))?;

        Ok(Self {
            start: Instant::now(),
            state: Mutex::new(ActionRecorderState {
                file,
                entries: Vec::new(),
                next_seq: 1,
            }),
        })
    }

    pub fn record(
        &self,
        suite_id: Option<&str>,
        step_id: Option<&str>,
        target: RunnerActionTarget,
        kind: RunnerActionKind,
    ) -> Result<(), String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "action trace recorder lock is poisoned".to_owned())?;
        let action = RunnerAction {
            seq: state.next_seq,
            timestamp_utc: format_utc_timestamp(SystemTime::now()),
            monotonic_ms: self.start.elapsed().as_millis() as u64,
            step_id: step_id.map(ToOwned::to_owned),
            target,
            kind,
        };
        state.next_seq += 1;
        let entry = ActionTraceEntry::new(suite_id, action);
        validate_entry(&entry)?;
        let line = serde_json::to_string(&entry)
            .map_err(|e| format!("failed to serialize action trace line: {e}"))?;
        writeln!(state.file, "{line}")
            .map_err(|e| format!("failed to write action trace line: {e}"))?;
        state.entries.push(entry);
        Ok(())
    }

    pub fn actions_for_suite(&self, suite_id: &str) -> Vec<RunnerAction> {
        let Ok(state) = self.state.lock() else {
            return Vec::new();
        };
        state
            .entries
            .iter()
            .filter(|entry| entry.suite_id.as_deref() == Some(suite_id))
            .map(|entry| entry.action.clone())
            .collect()
    }
}

pub fn validate_trace_file(path: &Path) -> Result<ValidatedTrace, String> {
    let file = File::open(path)
        .map_err(|e| format!("failed to open replay trace {}: {e}", path.display()))?;
    let reader = BufReader::new(file);
    let mut entries = Vec::new();
    let mut suite_ids = BTreeSet::new();
    let mut last_seq = 0_u64;

    for (index, line) in reader.lines().enumerate() {
        let line_no = index + 1;
        let line = line.map_err(|e| format!("failed to read replay trace line {line_no}: {e}"))?;
        if line.trim().is_empty() {
            continue;
        }
        let entry: ActionTraceEntry = serde_json::from_str(&line)
            .map_err(|e| format!("invalid replay trace line {line_no}: {e}"))?;
        validate_entry(&entry).map_err(|e| format!("invalid replay trace line {line_no}: {e}"))?;
        if entry.action.seq <= last_seq {
            return Err(format!(
                "invalid replay trace line {line_no}: action seq must increase"
            ));
        }
        last_seq = entry.action.seq;
        if let Some(suite_id) = &entry.suite_id {
            suite_ids.insert(suite_id.clone());
        }
        entries.push(entry);
    }

    if entries.is_empty() {
        return Err("replay trace has no actions".to_owned());
    }

    Ok(ValidatedTrace {
        entries,
        suite_ids: suite_ids.into_iter().collect(),
    })
}

pub fn validate_replay_selection(
    trace: &ValidatedTrace,
    selected_suite_ids: &[String],
) -> Result<(), String> {
    for suite_id in selected_suite_ids {
        if suite_id != "edge-resize-stability" {
            return Err(format!(
                "logical replay currently supports edge-resize-stability only, got '{suite_id}'"
            ));
        }
    }
    if trace.suite_ids.is_empty() {
        return Ok(());
    }
    for suite_id in selected_suite_ids {
        if !trace.suite_ids.iter().any(|known| known == suite_id) {
            return Err(format!(
                "replay trace does not contain actions for selected suite '{suite_id}'"
            ));
        }
    }
    Ok(())
}

fn validate_entry(entry: &ActionTraceEntry) -> Result<(), String> {
    is_supported_runner_action_schema_version(&entry.schema_version).map_err(|e| e.to_string())?;
    if let Some(suite_id) = &entry.suite_id {
        if suite_id.trim().is_empty() {
            return Err("suite_id cannot be empty".to_owned());
        }
    }
    validate_action_kind(&entry.action.kind)
}

fn validate_action_kind(kind: &RunnerActionKind) -> Result<(), String> {
    match kind {
        RunnerActionKind::Screenshot { path } => validate_safe_relative_path(path),
        RunnerActionKind::MarkStep { .. }
        | RunnerActionKind::MoveWindow { .. }
        | RunnerActionKind::ResizeWindow { .. }
        | RunnerActionKind::SendKeys { .. }
        | RunnerActionKind::Mouse { .. }
        | RunnerActionKind::MouseDrag { .. }
        | RunnerActionKind::Wait { .. }
        | RunnerActionKind::Note { .. } => Ok(()),
    }
}

fn validate_safe_relative_path(raw: &str) -> Result<(), String> {
    if raw.trim().is_empty() {
        return Err("screenshot path cannot be empty".to_owned());
    }
    let path = Path::new(raw);
    if path.is_absolute() {
        return Err(format!("unsafe absolute screenshot path '{raw}'"));
    }
    for component in path.components() {
        if matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        ) {
            return Err(format!("unsafe screenshot path '{raw}'"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use terminal_manager_diagnostics::{Rect, RunnerAction};

    #[test]
    fn action_trace_jsonl_round_trip_preserves_version_and_action_fields() {
        let dir =
            std::env::temp_dir().join(format!("xtask-dr-action-trace-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(ACTION_TRACE_FILE);
        let recorder = ActionRecorder::create(&path).unwrap();

        recorder
            .record(
                Some("edge-resize-stability"),
                Some("resize-inward"),
                RunnerActionTarget::Window {
                    title: Some("Terminal Manager".to_owned()),
                    process_id: Some(1234),
                },
                RunnerActionKind::MouseDrag {
                    from_x: 4,
                    from_y: 300,
                    to_x: 224,
                    to_y: 300,
                    button: Some("left".to_owned()),
                },
            )
            .unwrap();
        recorder
            .record(
                Some("edge-resize-stability"),
                Some("resize-inward"),
                RunnerActionTarget::Window {
                    title: Some("Terminal Manager".to_owned()),
                    process_id: Some(1234),
                },
                RunnerActionKind::ResizeWindow {
                    bounds: Rect {
                        x: 220,
                        y: 0,
                        width: 740,
                        height: 500,
                    },
                },
            )
            .unwrap();

        let trace = validate_trace_file(&path).unwrap();

        assert_eq!(trace.entries.len(), 2);
        assert_eq!(trace.suite_ids, vec!["edge-resize-stability"]);
        assert_eq!(
            trace.entries[0].schema_version,
            RUNNER_ACTION_SCHEMA_VERSION
        );
        assert_eq!(
            recorder.actions_for_suite("edge-resize-stability"),
            trace
                .entries
                .iter()
                .map(|entry| entry.action.clone())
                .collect::<Vec<_>>()
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn validation_rejects_unknown_schema_version_before_replay() {
        let dir =
            std::env::temp_dir().join(format!("xtask-dr-bad-action-schema-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(ACTION_TRACE_FILE);
        std::fs::write(
            &path,
            r#"{"schema_version":"desktop-regression.runner-action/v99","seq":1,"timestamp_utc":"2026-05-10T17:30:12Z","monotonic_ms":1,"target":{"type":"none"},"kind":{"type":"note","message":"x"}}"#,
        )
        .unwrap();

        let err = validate_trace_file(&path).unwrap_err();

        assert!(err.contains("unsupported required protocol/schema version"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn validation_rejects_unsafe_screenshot_paths() {
        let dir =
            std::env::temp_dir().join(format!("xtask-dr-unsafe-trace-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(ACTION_TRACE_FILE);
        std::fs::write(
            &path,
            r#"{"schema_version":"desktop-regression.runner-action/v1","suite_id":"edge-resize-stability","seq":1,"timestamp_utc":"2026-05-10T17:30:12Z","monotonic_ms":1,"target":{"type":"desktop"},"kind":{"type":"screenshot","path":"../outside.png"}}"#,
        )
        .unwrap();

        let err = validate_trace_file(&path).unwrap_err();

        assert!(err.contains("unsafe screenshot path"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn validation_rejects_unknown_action_kinds() {
        let dir =
            std::env::temp_dir().join(format!("xtask-dr-unknown-action-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(ACTION_TRACE_FILE);
        std::fs::write(
            &path,
            r#"{"schema_version":"desktop-regression.runner-action/v1","suite_id":"edge-resize-stability","seq":1,"timestamp_utc":"2026-05-10T17:30:12Z","monotonic_ms":1,"target":{"type":"none"},"kind":{"type":"delete_file","path":"target/out"}}"#,
        )
        .unwrap();

        let err = validate_trace_file(&path).unwrap_err();

        assert!(err.contains("unknown variant"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn actions_for_suite_filters_tagged_trace_entries() {
        let trace = ValidatedTrace {
            suite_ids: vec!["edge-resize-stability".to_owned()],
            entries: vec![
                ActionTraceEntry::new(
                    Some("edge-resize-stability"),
                    RunnerAction {
                        seq: 1,
                        kind: RunnerActionKind::Note {
                            message: "edge".to_owned(),
                        },
                        ..Default::default()
                    },
                ),
                ActionTraceEntry::new(
                    Some("post-resize-glitches"),
                    RunnerAction {
                        seq: 2,
                        kind: RunnerActionKind::Note {
                            message: "post".to_owned(),
                        },
                        ..Default::default()
                    },
                ),
            ],
        };

        let actions = trace.actions_for_suite("edge-resize-stability");

        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].seq, 1);
    }

    #[test]
    fn actions_for_suite_allows_untagged_single_suite_traces() {
        let trace = ValidatedTrace {
            suite_ids: Vec::new(),
            entries: vec![ActionTraceEntry::new(
                None,
                RunnerAction {
                    seq: 1,
                    kind: RunnerActionKind::Note {
                        message: "legacy".to_owned(),
                    },
                    ..Default::default()
                },
            )],
        };

        let actions = trace.actions_for_suite("edge-resize-stability");

        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].seq, 1);
    }
}
