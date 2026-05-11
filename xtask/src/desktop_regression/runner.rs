use std::path::PathBuf;

use crate::desktop_regression::artifacts::create_run_layout;
use crate::desktop_regression::options::{validate_options, DesktopRegressionOpts};
use crate::desktop_regression::registry::{all_suites, resolve_suites, SuiteMetadata};
use crate::desktop_regression::results::{skipped_skeleton, write_results};

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
    let result = skipped_skeleton(layout.run_id.clone(), opts.observe, &selected);
    write_results(&layout.results_path, &result)?;

    println!(
        "desktop-regression: wrote skipped Task 5 result skeleton for {} suite(s)",
        selected.len()
    );
    println!("  run id: {}", layout.run_id);
    println!("  artifacts: {}", layout.run_dir.display());
    println!("  results: {}", layout.results_path.display());
    println!("  suite execution is not implemented until Task 6");

    Ok(RunOutcome::Failed)
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
