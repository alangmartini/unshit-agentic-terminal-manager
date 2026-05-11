use std::path::Path;

use terminal_manager_diagnostics::FailureClassification;

use crate::desktop_regression::artifacts::ArtifactLayout;
use crate::desktop_regression::results::SuiteExecutionRecord;

pub mod edge_resize_stability;

pub struct SuiteContext<'a> {
    pub workspace_root: &'a Path,
    pub artifact_layout: &'a ArtifactLayout,
    pub exe_path: &'a Path,
    pub common_artifacts: &'a [String],
}

pub fn execute_suite(suite_id: &str, context: &SuiteContext<'_>) -> SuiteExecutionRecord {
    match suite_id {
        "edge-resize-stability" => edge_resize_stability::run(context),
        other => SuiteExecutionRecord::failed(
            other,
            FailureClassification::Setup,
            format!("desktop-regression suite '{other}' is not implemented in Rust yet"),
            Some("suite-implementation-missing".to_owned()),
            Vec::new(),
        ),
    }
}
