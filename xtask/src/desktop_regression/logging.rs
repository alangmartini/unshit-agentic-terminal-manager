use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::time::{Instant, SystemTime};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::desktop_regression::artifacts::format_utc_timestamp;

pub const RUNNER_EVENTS_FILE: &str = "runner.events.jsonl";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunnerEvent {
    pub schema_version: String,
    pub seq: u64,
    pub timestamp_utc: String,
    pub monotonic_ms: u64,
    pub kind: String,
    pub suite_id: Option<String>,
    pub fields: Value,
}

pub struct RunnerEventLogger {
    file: File,
    seq: u64,
    started: Instant,
}

impl RunnerEventLogger {
    pub fn create(path: &Path) -> Result<Self, String> {
        let file = File::create(path)
            .map_err(|e| format!("failed to create runner event log {}: {e}", path.display()))?;
        Ok(Self {
            file,
            seq: 0,
            started: Instant::now(),
        })
    }

    pub fn log(
        &mut self,
        kind: impl Into<String>,
        suite_id: Option<&str>,
        fields: Value,
    ) -> Result<(), String> {
        let event = RunnerEvent {
            schema_version: "desktop-regression.runner-event/v1".to_owned(),
            seq: self.seq,
            timestamp_utc: format_utc_timestamp(SystemTime::now()),
            monotonic_ms: self.started.elapsed().as_millis() as u64,
            kind: kind.into(),
            suite_id: suite_id.map(ToOwned::to_owned),
            fields,
        };
        self.seq += 1;
        serde_json::to_writer(&mut self.file, &event)
            .map_err(|e| format!("failed to serialize runner event: {e}"))?;
        self.file
            .write_all(b"\n")
            .map_err(|e| format!("failed to write runner event: {e}"))?;
        self.file
            .flush()
            .map_err(|e| format!("failed to flush runner event log: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn writes_runner_events_as_json_lines() {
        let dir = std::env::temp_dir().join(format!("xtask-dr-events-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("runner.events.jsonl");
        let mut logger = RunnerEventLogger::create(&path).unwrap();

        logger
            .log("run.start", None, serde_json::json!({ "run_id": "run-1" }))
            .unwrap();
        logger
            .log(
                "suite.end",
                Some("edge-resize-stability"),
                serde_json::json!({ "status": "failed" }),
            )
            .unwrap();

        let written = std::fs::read_to_string(&path).unwrap();
        let lines = written.lines().collect::<Vec<_>>();
        assert_eq!(lines.len(), 2);
        let first: Value = serde_json::from_str(lines[0]).unwrap();
        let second: Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(first["kind"], "run.start");
        assert_eq!(second["suite_id"], "edge-resize-stability");
        let _ = std::fs::remove_dir_all(dir);
    }
}
