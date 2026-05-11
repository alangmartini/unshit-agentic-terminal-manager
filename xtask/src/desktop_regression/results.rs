use std::path::Path;
use std::time::SystemTime;

use terminal_manager_diagnostics::{
    ObserveMode, ResultStatus, ResultSummary, RunInfo, SuiteResult, TestRunResult,
    RESULTS_SCHEMA_VERSION,
};

use crate::desktop_regression::artifacts::format_utc_timestamp;
use crate::desktop_regression::registry::SuiteMetadata;

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
        summary: ResultSummary {
            total: suites.len() as u32,
            passed: 0,
            failed: 0,
            skipped: suites.len() as u32,
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
    use terminal_manager_diagnostics::{ResultStatus, RESULTS_SCHEMA_VERSION};

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
}
