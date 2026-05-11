use std::path::Path;

use terminal_manager_diagnostics::ObserveMode;
use terminal_manager_diagnostics::{FailureClassification, RunnerActionKind, RunnerActionTarget};

use crate::desktop_regression::artifacts::ArtifactLayout;
use crate::desktop_regression::replay::ActionRecorder;
use crate::desktop_regression::results::SuiteExecutionRecord;

pub mod edge_resize_stability;
pub(crate) mod observability;
pub mod post_resize_glitches;

pub struct SuiteContext<'a> {
    pub workspace_root: &'a Path,
    pub artifact_layout: &'a ArtifactLayout,
    pub exe_path: &'a Path,
    pub common_artifacts: &'a [String],
    pub observe: ObserveMode,
    pub action_recorder: Option<&'a ActionRecorder>,
}

impl SuiteContext<'_> {
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
