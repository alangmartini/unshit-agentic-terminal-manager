use std::path::PathBuf;

use crate::desktop_regression::artifacts::create_run_layout;
use crate::desktop_regression::launcher::prepare_app_binary;
use crate::desktop_regression::options::{validate_options, DesktopRegressionOpts};
use crate::desktop_regression::registry::{all_suites, resolve_suites, SuiteMetadata};
use crate::desktop_regression::results::{completed_result, write_results, SuiteExecutionRecord};
use crate::desktop_regression::suites::{execute_suite, SuiteContext};
use terminal_manager_diagnostics::{FailureClassification, ResultAppInfo, ResultStatus};

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

    let exe_path =
        match prepare_app_binary(&workspace_root, opts.skip_build, opts.exe_path.as_deref()) {
            Ok(path) => path,
            Err(err) => {
                let outcomes = selected
                    .iter()
                    .map(|suite| {
                        SuiteExecutionRecord::failed(
                            suite.id,
                            FailureClassification::Setup,
                            format!("failed to prepare app binary: {err}"),
                            Some("app-binary-setup".to_owned()),
                            Vec::new(),
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
                write_results(&layout.results_path, &result)?;
                print_run_summary(
                    &layout.run_id,
                    &layout.run_dir,
                    &layout.results_path,
                    &result,
                );
                return Ok(RunOutcome::Failed);
            }
        };

    let context = SuiteContext {
        workspace_root: &workspace_root,
        artifact_layout: &layout,
        exe_path: &exe_path,
    };
    let outcomes = selected
        .iter()
        .map(|suite| execute_suite(suite.id, &context))
        .collect::<Vec<_>>();
    let result = completed_result(
        layout.run_id.clone(),
        opts.observe,
        &selected,
        Some(ResultAppInfo {
            binary: exe_path.display().to_string(),
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

    print_run_summary(
        &layout.run_id,
        &layout.run_dir,
        &layout.results_path,
        &result,
    );

    Ok(outcome)
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
}
