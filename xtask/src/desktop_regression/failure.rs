use std::path::Path;

use terminal_manager_diagnostics::{
    FailureBundleArtifact, FailureManifest, SuiteFailure, FAILURE_MANIFEST_SCHEMA_VERSION,
};

use crate::desktop_regression::artifacts::suite_artifact_name;
use crate::desktop_regression::screenshots::capture_screen;

pub fn failure_artifact(kind: impl Into<String>, path: impl Into<String>) -> FailureBundleArtifact {
    FailureBundleArtifact {
        kind: kind.into(),
        path: path.into(),
    }
}

pub fn build_failure_manifest(
    run_id: &str,
    suite_id: &str,
    failure: &SuiteFailure,
    artifacts: Vec<FailureBundleArtifact>,
    secondary_errors: Vec<String>,
) -> FailureManifest {
    FailureManifest {
        schema_version: FAILURE_MANIFEST_SCHEMA_VERSION.to_owned(),
        run_id: run_id.to_owned(),
        suite_id: suite_id.to_owned(),
        classification: failure.kind,
        message: failure.message.clone(),
        first_bad_signal: failure.first_bad_signal.clone(),
        artifacts,
        invariant_results: Vec::new(),
        secondary_errors,
    }
}

pub fn write_failure_manifest(
    run_dir: &Path,
    run_id: &str,
    suite_id: &str,
    failure: &SuiteFailure,
    artifacts: Vec<FailureBundleArtifact>,
    secondary_errors: Vec<String>,
) -> Result<String, String> {
    let manifest_name = suite_artifact_name(suite_id, "failure-manifest", "json");
    let manifest = build_failure_manifest(run_id, suite_id, failure, artifacts, secondary_errors);
    let json = serde_json::to_string_pretty(&manifest)
        .map_err(|e| format!("failed to serialize failure manifest: {e}"))?;
    let manifest_path = run_dir.join(&manifest_name);
    std::fs::write(&manifest_path, json).map_err(|e| {
        format!(
            "failed to write failure manifest at {}: {e}",
            manifest_path.display()
        )
    })?;
    Ok(manifest_name)
}

pub fn collect_basic_failure_bundle(
    run_dir: &Path,
    run_id: &str,
    suite_id: &str,
    failure: &SuiteFailure,
    linked_artifacts: &[String],
) -> Vec<String> {
    let mut added_artifacts = Vec::new();
    let mut secondary_errors = Vec::new();
    let final_screenshot = suite_artifact_name(suite_id, "final", "png");
    let final_screenshot_path = run_dir.join(&final_screenshot);

    if let Err(err) = capture_screen(&final_screenshot_path) {
        secondary_errors.push(format!("final screenshot capture failed: {err}"));
    } else {
        added_artifacts.push(final_screenshot.clone());
    }

    let mut manifest_artifacts = linked_artifacts
        .iter()
        .map(|path| failure_artifact(infer_artifact_kind(path), path.clone()))
        .collect::<Vec<_>>();
    manifest_artifacts.extend(
        added_artifacts
            .iter()
            .map(|path| failure_artifact(infer_artifact_kind(path), path.clone())),
    );

    match write_failure_manifest(
        run_dir,
        run_id,
        suite_id,
        failure,
        manifest_artifacts,
        secondary_errors,
    ) {
        Ok(manifest_name) => added_artifacts.push(manifest_name),
        Err(err) => eprintln!("desktop-regression: failed to write failure manifest: {err}"),
    }

    added_artifacts
}

fn infer_artifact_kind(path: &str) -> &'static str {
    if path.ends_with(".png") {
        "screenshot"
    } else if path.ends_with(".jsonl") {
        "events"
    } else if path.ends_with(".log") {
        "log"
    } else if path.contains("manifest") {
        "failure_manifest"
    } else if path.contains("environment") {
        "environment"
    } else {
        "artifact"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use terminal_manager_diagnostics::{FailureClassification, SuiteFailure};

    #[test]
    fn manifest_preserves_primary_error_and_secondary_capture_errors() {
        let failure = SuiteFailure {
            kind: FailureClassification::Assertion,
            message: "left edge did not move".to_owned(),
            first_bad_signal: Some("left-edge-inward-resize".to_owned()),
        };

        let manifest = build_failure_manifest(
            "run-1",
            "edge-resize-stability",
            &failure,
            vec![failure_artifact(
                "screenshot",
                "edge-resize-stability-final.png",
            )],
            vec!["screenshot capture failed: denied".to_owned()],
        );

        assert_eq!(manifest.run_id, "run-1");
        assert_eq!(manifest.suite_id, "edge-resize-stability");
        assert_eq!(manifest.classification, FailureClassification::Assertion);
        assert_eq!(manifest.message, "left edge did not move");
        assert_eq!(
            manifest.first_bad_signal.as_deref(),
            Some("left-edge-inward-resize")
        );
        assert_eq!(manifest.artifacts[0].kind, "screenshot");
        assert_eq!(
            manifest.secondary_errors,
            vec!["screenshot capture failed: denied"]
        );
    }

    #[test]
    fn infers_bundle_artifact_kinds_for_manifest_entries() {
        assert_eq!(infer_artifact_kind("runner.events.jsonl"), "events");
        assert_eq!(infer_artifact_kind("app.stderr.log"), "log");
        assert_eq!(
            infer_artifact_kind("edge-resize-stability-final.png"),
            "screenshot"
        );
        assert_eq!(infer_artifact_kind("environment.json"), "environment");
    }
}
