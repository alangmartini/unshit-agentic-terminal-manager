use std::path::Path;

use terminal_manager_diagnostics::ObserveMode;
use terminal_manager_diagnostics::{FailureClassification, RunnerActionKind, RunnerActionTarget};

use crate::desktop_regression::artifacts::ArtifactLayout;
use crate::desktop_regression::assertions::SuiteError;
use crate::desktop_regression::replay::ActionRecorder;
use crate::desktop_regression::results::SuiteExecutionRecord;

pub const ENV_FORCE_FAILURE: &str = "TM_DESKTOP_REGRESSION_FORCE_FAILURE";

pub mod edge_resize_stability;
pub(crate) mod observability;
pub mod post_resize_glitches;

pub struct SuiteContext<'a> {
    pub workspace_root: &'a Path,
    pub artifact_layout: &'a ArtifactLayout,
    pub exe_path: &'a Path,
    pub common_artifacts: &'a [String],
    pub observe: ObserveMode,
    pub interactive: bool,
    pub keep_open_on_failure: bool,
    pub action_recorder: Option<&'a ActionRecorder>,
}

impl SuiteContext<'_> {
    pub fn should_pause_on_failure(&self) -> bool {
        self.interactive && self.keep_open_on_failure
    }

    pub fn record_action(
        &self,
        suite_id: &str,
        step_id: Option<&str>,
        target: RunnerActionTarget,
        kind: RunnerActionKind,
    ) -> Result<(), String> {
        if let Some(recorder) = self.action_recorder {
            recorder.record(Some(suite_id), step_id, target, kind)?;
        }
        Ok(())
    }
}

pub fn execute_suite(suite_id: &str, context: &SuiteContext<'_>) -> SuiteExecutionRecord {
    match suite_id {
        "edge-resize-stability" => edge_resize_stability::run(context),
        "post-resize-glitches" => post_resize_glitches::run(context),
        other => SuiteExecutionRecord::failed(
            other,
            FailureClassification::Setup,
            format!("desktop-regression suite '{other}' is not implemented in Rust yet"),
            Some("suite-implementation-missing".to_owned()),
            Vec::new(),
        ),
    }
}

pub(crate) fn forced_failure_for_suite(suite_id: &str) -> Option<SuiteError> {
    let requested = std::env::var(ENV_FORCE_FAILURE).ok()?;
    if requested == "1" || requested.eq_ignore_ascii_case("all") || requested == suite_id {
        Some(SuiteError::assertion(
            format!("forced desktop regression failure for {suite_id}"),
            "forced-interactive-failure",
        ))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forced_failure_env_matches_all_or_suite_id() {
        std::env::set_var(ENV_FORCE_FAILURE, "all");
        assert!(forced_failure_for_suite("edge-resize-stability").is_some());

        std::env::set_var(ENV_FORCE_FAILURE, "post-resize-glitches");
        assert!(forced_failure_for_suite("post-resize-glitches").is_some());
        assert!(forced_failure_for_suite("edge-resize-stability").is_none());

        std::env::remove_var(ENV_FORCE_FAILURE);
    }
}
