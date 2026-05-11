use std::path::Path;
use std::time::SystemTime;

use terminal_manager_diagnostics::{
    FailureClassification, ObserveMode, ResultAppInfo, ResultStatus, ResultSummary, RunInfo,
    SuiteFailure, SuiteResult, TestRunResult, RESULTS_SCHEMA_VERSION,
};

use crate::desktop_regression::artifacts::format_utc_timestamp;
use crate::desktop_regression::interactive::InteractiveDecision;
use crate::desktop_regression::registry::SuiteMetadata;

#[cfg(test)]
pub fn skipped_skeleton(
    run_id: String,
    observe: ObserveMode,
    selected: &[&SuiteMetadata],
) -> TestRunResult {
    let timestamp = format_utc_timestamp(SystemTime::now());
    let selected_suites = selected.iter().map(|suite| suite.id.to_owned()).collect();
    let suites = selected
        .iter()
        .map(|suite| SuiteResult {
            id: suite.id.to_owned(),
            status: ResultStatus::Skipped,
            failure: None,
            artifacts: Vec::new(),
            actions: Vec::new(),
        })
        .collect::<Vec<_>>();

    TestRunResult {
        schema_version: RESULTS_SCHEMA_VERSION.to_owned(),
        run: RunInfo {
            id: run_id,
            status: ResultStatus::Skipped,
            observe,
            started_at_utc: timestamp.clone(),
            finished_at_utc: Some(timestamp),
            selected_suites,
        },
        app: None,
        replay: None,
        summary: ResultSummary {
            total: suites.len() as u32,
            passed: 0,
            failed: 0,
            skipped: suites.len() as u32,
        },
        suites,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuiteExecutionRecord {
    pub id: String,
    pub status: ResultStatus,
    pub failure: Option<SuiteFailure>,
    pub artifacts: Vec<String>,
    pub actions: Vec<terminal_manager_diagnostics::RunnerAction>,
    pub interactive_decision: Option<InteractiveDecision>,
}

impl SuiteExecutionRecord {
    pub fn passed(id: impl Into<String>, artifacts: Vec<String>) -> Self {
        Self {
            id: id.into(),
            status: ResultStatus::Passed,
            failure: None,
            artifacts,
            actions: Vec::new(),
            interactive_decision: None,
        }
    }

    pub fn skipped(id: impl Into<String>, artifacts: Vec<String>) -> Self {
        Self {
            id: id.into(),
            status: ResultStatus::Skipped,
            failure: None,
            artifacts,
            actions: Vec::new(),
            interactive_decision: None,
        }
    }

    pub fn failed(
        id: impl Into<String>,
        kind: FailureClassification,
        message: impl Into<String>,
        first_bad_signal: Option<String>,
        artifacts: Vec<String>,
    ) -> Self {
        Self {
            id: id.into(),
            status: ResultStatus::Failed,
            failure: Some(SuiteFailure {
                kind,
                message: message.into(),
                first_bad_signal,
            }),
            artifacts,
            actions: Vec::new(),
            interactive_decision: None,
        }
    }

    pub fn set_interactive_decision(&mut self, decision: Option<InteractiveDecision>) {
        self.interactive_decision = decision;
    }

    pub fn should_abort_run_after_interactive_failure(&self) -> bool {
        matches!(
            self.interactive_decision,
            Some(InteractiveDecision::Abort | InteractiveDecision::Close)
        )
    }
}

#[cfg(test)]
pub fn completed_result(
    run_id: String,
    observe: ObserveMode,
    selected: &[&SuiteMetadata],
    app: Option<ResultAppInfo>,
    outcomes: Vec<SuiteExecutionRecord>,
) -> TestRunResult {
    let now = SystemTime::now();
    completed_result_at(run_id, observe, selected, app, outcomes, now, now)
}

pub fn completed_result_at(
    run_id: String,
    observe: ObserveMode,
    selected: &[&SuiteMetadata],
    app: Option<ResultAppInfo>,
    outcomes: Vec<SuiteExecutionRecord>,
    started_at: SystemTime,
    finished_at: SystemTime,
) -> TestRunResult {
    let started_at_utc = format_utc_timestamp(started_at);
    let finished_at_utc = format_utc_timestamp(finished_at);
    let selected_suites = selected.iter().map(|suite| suite.id.to_owned()).collect();
    let suites = outcomes
        .into_iter()
        .map(|outcome| SuiteResult {
            id: outcome.id,
            status: outcome.status,
            failure: outcome.failure,
            artifacts: outcome.artifacts,
            actions: outcome.actions,
        })
        .collect::<Vec<_>>();
    let passed = suites
        .iter()
        .filter(|suite| suite.status == ResultStatus::Passed)
        .count() as u32;
    let failed = suites
        .iter()
        .filter(|suite| suite.status == ResultStatus::Failed)
        .count() as u32;
    let skipped = suites
        .iter()
        .filter(|suite| suite.status == ResultStatus::Skipped)
        .count() as u32;
    let status = if failed > 0 {
        ResultStatus::Failed
    } else if skipped == suites.len() as u32 {
        ResultStatus::Skipped
    } else {
        ResultStatus::Passed
    };

    TestRunResult {
        schema_version: RESULTS_SCHEMA_VERSION.to_owned(),
        run: RunInfo {
            id: run_id,
            status,
            observe,
            started_at_utc,
            finished_at_utc: Some(finished_at_utc),
            selected_suites,
        },
        app,
        replay: None,
        summary: ResultSummary {
            total: suites.len() as u32,
            passed,
            failed,
            skipped,
        },
        suites,
    }
}

pub fn write_results(path: &Path, result: &TestRunResult) -> Result<(), String> {
    let json = serde_json::to_string_pretty(result)
        .map_err(|e| format!("failed to serialize results.json: {e}"))?;
    std::fs::write(path, json)
        .map_err(|e| format!("failed to write results.json at {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use terminal_manager_diagnostics::{
        FailureClassification, ResultAppInfo, ResultStatus, RESULTS_SCHEMA_VERSION,
    };

    #[test]
    fn skeleton_uses_v2_schema_and_marks_suites_skipped() {
        let suites = crate::desktop_regression::registry::resolve_suites(&[
            "edge-resize-stability".to_owned(),
        ])
        .unwrap();

        let result = skipped_skeleton("run-1".to_owned(), ObserveMode::Full, &suites);

        assert_eq!(result.schema_version, RESULTS_SCHEMA_VERSION);
        assert_eq!(result.run.id, "run-1");
        assert_eq!(result.run.observe, ObserveMode::Full);
        assert_eq!(result.run.status, ResultStatus::Skipped);
        assert_eq!(result.summary.total, 1);
        assert_eq!(result.summary.skipped, 1);
        assert_eq!(result.suites[0].id, "edge-resize-stability");
        assert_eq!(result.suites[0].status, ResultStatus::Skipped);
    }

    #[test]
    fn writes_pretty_json_results() {
        let dir = std::env::temp_dir().join(format!("xtask-dr-results-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("results.json");
        let result = TestRunResult::default();

        write_results(&path, &result).unwrap();

        let written = std::fs::read_to_string(&path).unwrap();
        assert!(written.contains("\"schema_version\""));
        let decoded: TestRunResult = serde_json::from_str(&written).unwrap();
        assert_eq!(decoded.schema_version, result.schema_version);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn write_results_surfaces_write_errors() {
        let dir =
            std::env::temp_dir().join(format!("xtask-dr-results-error-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        let err = write_results(&dir, &TestRunResult::default()).unwrap_err();

        assert!(err.contains("failed to write results.json"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn completed_result_summarizes_suite_outcomes() {
        let suites = crate::desktop_regression::registry::resolve_suites(&[
            "edge-resize-stability".to_owned(),
            "post-resize-glitches".to_owned(),
        ])
        .unwrap();
        let app = ResultAppInfo {
            binary: "target/debug/terminal-manager.exe".to_owned(),
            pid: Some(4242),
            ..ResultAppInfo::default()
        };
        let outcomes = vec![
            SuiteExecutionRecord::passed(
                "edge-resize-stability",
                vec!["edge-resize-stability-start.png".to_owned()],
            ),
            SuiteExecutionRecord::failed(
                "post-resize-glitches",
                FailureClassification::Setup,
                "suite is not implemented yet",
                None,
                Vec::new(),
            ),
        ];

        let result = completed_result(
            "run-1".to_owned(),
            ObserveMode::Off,
            &suites,
            Some(app),
            outcomes,
        );

        assert_eq!(result.run.status, ResultStatus::Failed);
        assert_eq!(result.summary.total, 2);
        assert_eq!(result.summary.passed, 1);
        assert_eq!(result.summary.failed, 1);
        assert_eq!(result.summary.skipped, 0);
        assert_eq!(result.app.as_ref().unwrap().pid, Some(4242));
        assert_eq!(result.suites[0].status, ResultStatus::Passed);
        assert_eq!(
            result.suites[0].artifacts,
            vec!["edge-resize-stability-start.png"]
        );
        assert_eq!(result.suites[1].status, ResultStatus::Failed);
        assert_eq!(
            result.suites[1].failure.as_ref().unwrap().kind,
            FailureClassification::Setup
        );
    }

    #[test]
    fn completed_result_at_keeps_distinct_run_timestamps() {
        let suites = crate::desktop_regression::registry::resolve_suites(&[
            "edge-resize-stability".to_owned(),
        ])
        .unwrap();
        let started = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_778_434_212);
        let finished = started + std::time::Duration::from_secs(5);

        let result = completed_result_at(
            "run-1".to_owned(),
            ObserveMode::Off,
            &suites,
            None,
            vec![SuiteExecutionRecord::passed(
                "edge-resize-stability",
                Vec::new(),
            )],
            started,
            finished,
        );

        assert_eq!(result.run.started_at_utc, "2026-05-10T17:30:12Z");
        assert_eq!(
            result.run.finished_at_utc.as_deref(),
            Some("2026-05-10T17:30:17Z")
        );
    }
}
