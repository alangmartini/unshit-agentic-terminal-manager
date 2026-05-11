use std::path::PathBuf;

use crate::desktop_regression::artifacts::create_run_layout;
use crate::desktop_regression::environment::{
    collect_environment_metadata, write_environment_metadata, ENVIRONMENT_METADATA_FILE,
};
use crate::desktop_regression::failure::collect_basic_failure_bundle;
use crate::desktop_regression::launcher::prepare_app_binary;
use crate::desktop_regression::logging::{RunnerEventLogger, RUNNER_EVENTS_FILE};
use crate::desktop_regression::options::{validate_options, DesktopRegressionOpts};
use crate::desktop_regression::registry::{all_suites, resolve_suites, SuiteMetadata};
use crate::desktop_regression::results::{completed_result, write_results, SuiteExecutionRecord};
use crate::desktop_regression::suites::{execute_suite, SuiteContext};
use serde_json::json;
use terminal_manager_diagnostics::{
    FailureClassification, ResultAppInfo, ResultStatus, SuiteFailure,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunOutcome {
    Success,
    Failed,
}

pub fn run(opts: &DesktopRegressionOpts) -> Result<RunOutcome, String> {
    validate_options(opts)?;

    if opts.list {
        print_suite_list(all_suites());
        return Ok(RunOutcome::Success);
    }

    let selected = resolve_suites(&opts.suite_ids)?;
    let workspace_root = workspace_root()?;
    let layout = create_run_layout(&workspace_root, &opts.artifact_root)?;
    let mut logger = RunnerEventLogger::create(&layout.run_dir.join(RUNNER_EVENTS_FILE))?;
    let mut common_artifacts = vec![RUNNER_EVENTS_FILE.to_owned()];

    logger.log(
        "run.start",
        None,
        json!({
            "run_id": layout.run_id,
            "observe": opts.observe,
            "selected_suites": selected.iter().map(|suite| suite.id).collect::<Vec<_>>(),
        }),
    )?;
    logger.log(
        "artifact.write",
        None,
        json!({ "kind": "runner_events", "path": RUNNER_EVENTS_FILE }),
    )?;

    let exe_path =
        match prepare_app_binary(&workspace_root, opts.skip_build, opts.exe_path.as_deref()) {
            Ok(path) => path,
            Err(err) => {
                logger.log(
                    "run.failure",
                    None,
                    json!({
                        "classification": "setup",
                        "message": format!("failed to prepare app binary: {err}"),
                    }),
                )?;
                let outcomes = selected
                    .iter()
                    .map(|suite| {
                        let failure = SuiteFailure {
                            kind: FailureClassification::Setup,
                            message: format!("failed to prepare app binary: {err}"),
                            first_bad_signal: Some("app-binary-setup".to_owned()),
                        };
                        let mut artifacts = common_artifacts.clone();
                        artifacts.extend(collect_basic_failure_bundle(
                            &layout.run_dir,
                            &layout.run_id,
                            suite.id,
                            &failure,
                            &artifacts,
                        ));
                        SuiteExecutionRecord::failed(
                            suite.id,
                            failure.kind,
                            failure.message,
                            failure.first_bad_signal,
                            artifacts,
                        )
                    })
                    .collect::<Vec<_>>();
                let result = completed_result(
                    layout.run_id.clone(),
                    opts.observe,
                    &selected,
                    None,
                    outcomes,
                );
                logger.log("run.end", None, json!({ "status": "failed" }))?;
                write_results(&layout.results_path, &result)?;
                logger.log(
                    "artifact.write",
                    None,
                    json!({ "kind": "results", "path": "results.json" }),
                )?;
                print_run_summary(
                    &layout.run_id,
                    &layout.run_dir,
                    &layout.results_path,
                    &result,
                );
                return Ok(RunOutcome::Failed);
            }
        };

    let environment = collect_environment_metadata(&workspace_root, &exe_path);
    let app_sha256 = environment.binary.sha256.clone();
    write_environment_metadata(
        &layout.run_dir.join(ENVIRONMENT_METADATA_FILE),
        &environment,
    )?;
    common_artifacts.push(ENVIRONMENT_METADATA_FILE.to_owned());
    logger.log(
        "artifact.write",
        None,
        json!({ "kind": "environment", "path": ENVIRONMENT_METADATA_FILE }),
    )?;

    let context = SuiteContext {
        workspace_root: &workspace_root,
        artifact_layout: &layout,
        exe_path: &exe_path,
        common_artifacts: &common_artifacts,
    };
    let mut outcomes = Vec::new();
    for suite in &selected {
        logger.log("suite.start", Some(suite.id), json!({}))?;
        let mut outcome = execute_suite(suite.id, &context);
        append_common_artifacts(&mut outcome, &common_artifacts);
        for artifact in &outcome.artifacts {
            logger.log(
                "artifact.write",
                Some(suite.id),
                json!({ "path": artifact }),
            )?;
        }
        if let Some(failure) = &outcome.failure {
            logger.log(
                "suite.failure",
                Some(suite.id),
                json!({
                    "classification": failure.kind,
                    "message": failure.message,
                    "first_bad_signal": failure.first_bad_signal,
                }),
            )?;
        }
        logger.log(
            "suite.end",
            Some(suite.id),
            json!({ "status": outcome.status }),
        )?;
        logger.log(
            "cleanup.complete",
            Some(suite.id),
            json!({ "scope": "suite" }),
        )?;
        outcomes.push(outcome);
    }
    let run_status = if outcomes
        .iter()
        .any(|outcome| outcome.status == ResultStatus::Failed)
    {
        ResultStatus::Failed
    } else {
        ResultStatus::Passed
    };
    logger.log("run.end", None, json!({ "status": run_status }))?;
    let result = completed_result(
        layout.run_id.clone(),
        opts.observe,
        &selected,
        Some(ResultAppInfo {
            binary: exe_path.display().to_string(),
            sha256: app_sha256,
            diagnostics: None,
            ..ResultAppInfo::default()
        }),
        outcomes,
    );
    let outcome = if result.run.status == ResultStatus::Failed {
        RunOutcome::Failed
    } else {
        RunOutcome::Success
    };
    write_results(&layout.results_path, &result)?;
    logger.log(
        "artifact.write",
        None,
        json!({ "kind": "results", "path": "results.json" }),
    )?;

    print_run_summary(
        &layout.run_id,
        &layout.run_dir,
        &layout.results_path,
        &result,
    );

    Ok(outcome)
}

fn append_common_artifacts(outcome: &mut SuiteExecutionRecord, common_artifacts: &[String]) {
    for artifact in common_artifacts {
        if !outcome.artifacts.contains(artifact) {
            outcome.artifacts.push(artifact.clone());
        }
    }
}

fn print_run_summary(
    run_id: &str,
    run_dir: &std::path::Path,
    results_path: &std::path::Path,
    result: &terminal_manager_diagnostics::TestRunResult,
) {
    println!(
        "desktop-regression: {} passed, {} failed, {} skipped",
        result.summary.passed, result.summary.failed, result.summary.skipped
    );
    println!("  run id: {}", run_id);
    println!("  artifacts: {}", run_dir.display());
    println!("  results: {}", results_path.display());
}

fn print_suite_list(suites: &[SuiteMetadata]) {
    for suite in suites {
        println!("{} - {}", suite.id, suite.title);
        println!("  tags: {}", suite.tags.join(", "));
        println!("  coverage: {}", suite.coverage);
        println!("  observability: {}", suite.observability_needs.join(", "));
        println!("  platforms: {}", suite.supported_platforms.join(", "));
    }
}

fn workspace_root() -> Result<PathBuf, String> {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let mut dir = PathBuf::from(manifest_dir);
    loop {
        let candidate = dir.join("Cargo.toml");
        if candidate.is_file() {
            if let Ok(contents) = std::fs::read_to_string(&candidate) {
                if contents.contains("[workspace]") {
                    return Ok(dir);
                }
            }
        }
        if !dir.pop() {
            return Err(format!(
                "could not find workspace root starting from {manifest_dir}"
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop_regression::options::DesktopRegressionOpts;

    #[test]
    fn unknown_suite_fails_before_artifact_creation() {
        let artifact_root = PathBuf::from(format!(
            "target/xtask-dr-missing-suite-{}",
            std::process::id()
        ));
        let absolute_artifact_root = workspace_root().unwrap().join(&artifact_root);
        let _ = std::fs::remove_dir_all(&absolute_artifact_root);

        let opts = DesktopRegressionOpts {
            suite_ids: vec!["missing".to_owned()],
            artifact_root,
            ..DesktopRegressionOpts::default()
        };

        let err = run(&opts).unwrap_err();

        assert!(err.contains("missing"));
        assert!(!absolute_artifact_root.exists());
    }

    #[test]
    fn common_artifacts_are_linked_without_duplicates() {
        let mut outcome = SuiteExecutionRecord::passed(
            "edge-resize-stability",
            vec!["runner.events.jsonl".to_owned()],
        );
        let common = vec![
            "runner.events.jsonl".to_owned(),
            "environment.json".to_owned(),
        ];

        append_common_artifacts(&mut outcome, &common);

        assert_eq!(
            outcome.artifacts,
            vec![
                "runner.events.jsonl".to_owned(),
                "environment.json".to_owned()
            ]
        );
    }
}
