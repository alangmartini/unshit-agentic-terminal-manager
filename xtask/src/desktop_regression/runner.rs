use std::path::PathBuf;
use std::time::SystemTime;

use crate::desktop_regression::artifacts::create_run_layout;
use crate::desktop_regression::environment::{
    collect_environment_metadata, write_environment_metadata, ENVIRONMENT_METADATA_FILE,
};
use crate::desktop_regression::failure::collect_basic_failure_bundle;
use crate::desktop_regression::launcher::prepare_app_binary;
use crate::desktop_regression::logging::{RunnerEventLogger, RUNNER_EVENTS_FILE};
use crate::desktop_regression::options::{validate_options, DesktopRegressionOpts};
use crate::desktop_regression::registry::{all_suites, resolve_suites, SuiteMetadata};
use crate::desktop_regression::replay::{
    validate_replay_selection, validate_trace_file, ActionRecorder, ACTION_TRACE_FILE,
};
use crate::desktop_regression::results::{
    completed_result_at, write_results, SuiteExecutionRecord,
};
use crate::desktop_regression::suites::{execute_suite, execute_suite_replay, SuiteContext};
use serde_json::json;
use terminal_manager_diagnostics::{
    FailureClassification, ObserveMode, ReplayMode, ResultAppInfo, ResultDiagnosticInfo,
    ResultReplayInfo, ResultStatus, RunnerActionKind, RunnerActionTarget, SuiteFailure,
    DIAGNOSTIC_PROTOCOL_VERSION,
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

    let run_started_at = SystemTime::now();
    let workspace_root = workspace_root()?;
    let replay_trace = match opts.replay.as_deref() {
        Some(path) => Some(validate_trace_file(path)?),
        None => None,
    };
    let selected_suite_ids = selected_suite_ids(opts, replay_trace.as_ref())?;
    if let Some(trace) = replay_trace.as_ref() {
        validate_replay_selection(trace, &selected_suite_ids)?;
    }
    let selected = resolve_suites(&selected_suite_ids)?;
    let layout = create_run_layout(&workspace_root, &opts.artifact_root)?;
    let mut logger = RunnerEventLogger::create(&layout.run_dir.join(RUNNER_EVENTS_FILE))?;
    let mut common_artifacts = vec![RUNNER_EVENTS_FILE.to_owned()];
    let action_trace_path = layout.run_dir.join(ACTION_TRACE_FILE);
    let action_recorder = if opts.record {
        let recorder = ActionRecorder::create(&action_trace_path)?;
        common_artifacts.push(ACTION_TRACE_FILE.to_owned());
        Some(recorder)
    } else {
        None
    };

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
    if let Some(recorder) = action_recorder.as_ref() {
        recorder.record(
            None,
            None,
            RunnerActionTarget::None,
            RunnerActionKind::Note {
                message: "run.start".to_owned(),
            },
        )?;
        logger.log(
            "artifact.write",
            None,
            json!({ "kind": "action_trace", "path": ACTION_TRACE_FILE }),
        )?;
    }

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
                let result = completed_result_at(
                    layout.run_id.clone(),
                    opts.observe,
                    &selected,
                    None,
                    outcomes,
                    run_started_at,
                    SystemTime::now(),
                );
                logger.log("run.end", None, json!({ "status": "failed" }))?;
                let mut result = result;
                attach_replay_info(&mut result, opts, replay_trace.as_ref());
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
    if let Some(recorder) = action_recorder.as_ref() {
        recorder.record(
            None,
            None,
            RunnerActionTarget::None,
            RunnerActionKind::Note {
                message: format!("app.binary.prepared:{}", exe_path.display()),
            },
        )?;
        recorder.record(
            None,
            None,
            RunnerActionTarget::None,
            RunnerActionKind::Note {
                message: format!("environment.metadata.write:{ENVIRONMENT_METADATA_FILE}"),
            },
        )?;
    }

    let context = SuiteContext {
        workspace_root: &workspace_root,
        artifact_layout: &layout,
        exe_path: &exe_path,
        common_artifacts: &common_artifacts,
        observe: opts.observe,
        interactive: opts.interactive,
        keep_open_on_failure: opts.keep_open_on_failure,
        action_recorder: action_recorder.as_ref(),
    };
    let mut outcomes = Vec::new();
    for (index, suite) in selected.iter().enumerate() {
        logger.log("suite.start", Some(suite.id), json!({}))?;
        if let Some(recorder) = action_recorder.as_ref() {
            recorder.record(
                Some(suite.id),
                None,
                RunnerActionTarget::None,
                RunnerActionKind::Note {
                    message: "suite.start".to_owned(),
                },
            )?;
        }
        let mut outcome = if let Some(trace) = replay_trace.as_ref() {
            execute_suite_replay(suite.id, &context, trace)
        } else {
            execute_suite(suite.id, &context)
        };
        if let Some(recorder) = action_recorder.as_ref() {
            outcome.actions = recorder.actions_for_suite(suite.id);
        }
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
        let should_abort = outcome.should_abort_run_after_interactive_failure();
        outcomes.push(outcome);
        if should_abort {
            for skipped in selected.iter().skip(index + 1) {
                logger.log(
                    "suite.skipped",
                    Some(skipped.id),
                    json!({ "reason": "interactive failure workflow aborted the run" }),
                )?;
                outcomes.push(SuiteExecutionRecord::skipped(
                    skipped.id,
                    common_artifacts.clone(),
                ));
            }
            break;
        }
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
    let mut result = completed_result_at(
        layout.run_id.clone(),
        opts.observe,
        &selected,
        Some(ResultAppInfo {
            binary: exe_path.display().to_string(),
            sha256: app_sha256,
            diagnostics: result_diagnostics(opts.observe),
            ..ResultAppInfo::default()
        }),
        outcomes,
        run_started_at,
        SystemTime::now(),
    );
    attach_replay_info(&mut result, opts, replay_trace.as_ref());
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

fn selected_suite_ids(
    opts: &DesktopRegressionOpts,
    replay_trace: Option<&crate::desktop_regression::replay::ValidatedTrace>,
) -> Result<Vec<String>, String> {
    if !opts.suite_ids.is_empty() {
        return Ok(opts.suite_ids.clone());
    }
    if let Some(trace) = replay_trace {
        if !trace.suite_ids.is_empty() {
            return Ok(trace.suite_ids.clone());
        }
        return Err("--replay traces without suite ids require --suite".to_owned());
    }
    Ok(Vec::new())
}

fn attach_replay_info(
    result: &mut terminal_manager_diagnostics::TestRunResult,
    opts: &DesktopRegressionOpts,
    replay_trace: Option<&crate::desktop_regression::replay::ValidatedTrace>,
) {
    if let (Some(path), Some(trace)) = (opts.replay.as_ref(), replay_trace) {
        result.replay = Some(ResultReplayInfo {
            mode: ReplayMode::Logical,
            trace: path.display().to_string(),
            validated_actions: trace.action_count(),
        });
    }
}

fn result_diagnostics(observe: ObserveMode) -> Option<ResultDiagnosticInfo> {
    if observe == ObserveMode::Off {
        None
    } else {
        Some(ResultDiagnosticInfo {
            enabled: true,
            protocol_version: Some(DIAGNOSTIC_PROTOCOL_VERSION.to_owned()),
            transport: Some("named_pipe".to_owned()),
        })
    }
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

    #[test]
    fn invalid_replay_trace_fails_before_artifact_creation() {
        let trace_dir =
            std::env::temp_dir().join(format!("xtask-dr-invalid-replay-{}", std::process::id()));
        std::fs::create_dir_all(&trace_dir).unwrap();
        let trace_path = trace_dir.join("bad.actions.jsonl");
        std::fs::write(
            &trace_path,
            r#"{"schema_version":"desktop-regression.runner-action/v99","seq":1,"timestamp_utc":"2026-05-10T17:30:12Z","monotonic_ms":1,"target":{"type":"none"},"kind":{"type":"note","message":"x"}}"#,
        )
        .unwrap();
        let artifact_root = PathBuf::from(format!(
            "target/xtask-dr-invalid-replay-{}",
            std::process::id()
        ));
        let absolute_artifact_root = workspace_root().unwrap().join(&artifact_root);
        let _ = std::fs::remove_dir_all(&absolute_artifact_root);
        let opts = DesktopRegressionOpts {
            replay: Some(trace_path.clone()),
            suite_ids: vec!["edge-resize-stability".to_owned()],
            artifact_root,
            ..DesktopRegressionOpts::default()
        };

        let err = run(&opts).unwrap_err();

        assert!(err.contains("unsupported required protocol/schema version"));
        assert!(!absolute_artifact_root.exists());
        let _ = std::fs::remove_dir_all(trace_dir);
    }
}
