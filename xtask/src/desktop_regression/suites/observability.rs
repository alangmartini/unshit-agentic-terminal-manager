use crate::desktop_regression::artifacts::suite_artifact_name;
use crate::desktop_regression::assertions::{SuiteError, SuiteResult};
use crate::desktop_regression::diagnostics::{
    write_diagnostic_events, write_json_artifact, DiagnosticClient, DiagnosticHello,
    DiagnosticLaunchConfig,
};
use crate::desktop_regression::interactive::{
    prompt_interactive_failure, InteractiveDecision, SuiteInteractiveRuntime,
};
use crate::desktop_regression::launcher::AppSession;
use crate::desktop_regression::suites::SuiteContext;
use crate::desktop_regression::win32::DesktopRect;
use terminal_manager_diagnostics::{
    DiagnosticEventFamily, InvariantEvaluation, InvariantOutcome, ObserveMode, SnapshotOptions,
    TerminalManagerSnapshot,
};

pub(crate) struct ObservedDiagnostics {
    pub(crate) client: DiagnosticClient,
    pub(crate) hello: DiagnosticHello,
}

pub(crate) fn start_diagnostics(
    context: &SuiteContext<'_>,
    artifacts: &mut Vec<String>,
    suite_id: &str,
    launch: Option<&DiagnosticLaunchConfig>,
) -> SuiteResult<Option<ObservedDiagnostics>> {
    let Some(launch) = launch else {
        return Ok(None);
    };

    let client = DiagnosticClient::new(launch);
    let hello = client.wait_for_hello(context.observe).map_err(|e| {
        SuiteError::protocol(format!("diagnostic hello failed: {e}"), "diagnostic-hello")
    })?;
    let hello_artifact = write_json_artifact(
        &context.artifact_layout.run_dir,
        suite_id,
        "diagnostics-hello",
        &hello,
    )
    .map_err(|e| SuiteError::protocol(e, "diagnostic-hello-artifact"))?;
    artifacts.push(hello_artifact);

    if context.observe == ObserveMode::Full {
        client.prepare_deterministic_mode().map_err(|e| {
            SuiteError::protocol(
                format!("diagnostic deterministic mode failed: {e}"),
                "diagnostic-deterministic-mode",
            )
        })?;
    }

    Ok(Some(ObservedDiagnostics { client, hello }))
}

pub(crate) fn mark_full_step(
    context: &SuiteContext<'_>,
    diagnostics: Option<&ObservedDiagnostics>,
    id: &str,
    label: &str,
) -> SuiteResult<()> {
    if context.observe != ObserveMode::Full {
        return Ok(());
    }
    if let Some(diagnostics) = diagnostics {
        diagnostics.client.mark_step(id, label).map_err(|e| {
            SuiteError::protocol(
                format!("diagnostic step marker failed for {id}: {e}"),
                "diagnostic-step-marker",
            )
        })?;
    }
    Ok(())
}

pub(crate) fn capture_step_snapshot(
    context: &SuiteContext<'_>,
    artifacts: &mut Vec<String>,
    suite_id: &str,
    diagnostics: Option<&ObservedDiagnostics>,
    artifact_stem: &str,
    reason: &str,
) -> SuiteResult<Option<TerminalManagerSnapshot>> {
    capture_step_snapshot_with_options(
        context,
        artifacts,
        suite_id,
        diagnostics,
        artifact_stem,
        reason,
        SnapshotOptions::default(),
    )
}

pub(crate) fn capture_step_snapshot_with_options(
    context: &SuiteContext<'_>,
    artifacts: &mut Vec<String>,
    suite_id: &str,
    diagnostics: Option<&ObservedDiagnostics>,
    artifact_stem: &str,
    reason: &str,
    options: SnapshotOptions,
) -> SuiteResult<Option<TerminalManagerSnapshot>> {
    if !captures_step_snapshots(context.observe) {
        return Ok(None);
    }
    let Some(diagnostics) = diagnostics else {
        return Ok(None);
    };

    let snapshot = diagnostics
        .client
        .snapshot_with_options(reason, options)
        .map_err(|e| {
            SuiteError::protocol(
                format!("diagnostic snapshot failed for {reason}: {e}"),
                "diagnostic-step-snapshot",
            )
        })?;
    let artifact = write_json_artifact(
        &context.artifact_layout.run_dir,
        suite_id,
        artifact_stem,
        &snapshot,
    )
    .map_err(|e| SuiteError::protocol(e, "diagnostic-step-snapshot-artifact"))?;
    artifacts.push(artifact);
    Ok(Some(snapshot))
}

fn captures_step_snapshots(observe: ObserveMode) -> bool {
    matches!(observe, ObserveMode::Basic | ObserveMode::Full)
}

pub(crate) fn finalize_diagnostics(
    context: &SuiteContext<'_>,
    artifacts: &mut Vec<String>,
    suite_id: &str,
    diagnostics: Option<&ObservedDiagnostics>,
    failed: bool,
) -> SuiteResult<()> {
    let Some(diagnostics) = diagnostics else {
        return Ok(());
    };

    let snapshot_reason = if failed { "failure" } else { "final" };
    let snapshot = diagnostics.client.snapshot(snapshot_reason).map_err(|e| {
        SuiteError::protocol(
            format!("diagnostic {snapshot_reason} snapshot failed: {e}"),
            "diagnostic-final-snapshot",
        )
    })?;
    let snapshot_artifact = write_json_artifact(
        &context.artifact_layout.run_dir,
        suite_id,
        &format!("diagnostics-{snapshot_reason}-snapshot"),
        &snapshot,
    )
    .map_err(|e| SuiteError::protocol(e, "diagnostic-final-snapshot-artifact"))?;
    artifacts.push(snapshot_artifact);

    let mut deferred_error = None;
    if context.observe == ObserveMode::Full {
        let invariants = diagnostics.client.evaluate_invariants().map_err(|e| {
            SuiteError::protocol(
                format!("diagnostic invariant evaluation failed: {e}"),
                "diagnostic-invariants",
            )
        })?;
        let invariant_artifact = write_json_artifact(
            &context.artifact_layout.run_dir,
            suite_id,
            "diagnostics-invariants",
            &invariants,
        )
        .map_err(|e| SuiteError::protocol(e, "diagnostic-invariants-artifact"))?;
        artifacts.push(invariant_artifact);
        deferred_error = assert_invariants_passed(&invariants).err();
        if let Err(err) = diagnostics.client.clear_step("suite complete") {
            deferred_error.get_or_insert_with(|| {
                SuiteError::protocol(
                    format!("diagnostic clear step failed: {err}"),
                    "diagnostic-clear-step",
                )
            });
        }
    }

    let (events_flushed, flush_dropped) = diagnostics.client.flush().map_err(|e| {
        SuiteError::protocol(format!("diagnostic flush failed: {e}"), "diagnostic-flush")
    })?;
    let (events, drain_dropped) = diagnostics.client.drain_events().map_err(|e| {
        SuiteError::protocol(
            format!("diagnostic event drain failed: {e}"),
            "diagnostic-drain-events",
        )
    })?;
    let event_artifacts =
        write_diagnostic_events(&context.artifact_layout.run_dir, suite_id, &events)
            .map_err(|e| SuiteError::protocol(e, "diagnostic-events-artifact"))?;
    artifacts.extend(event_artifacts);

    let summary = serde_json::json!({
        "events_flushed": events_flushed,
        "dropped_events": flush_dropped + drain_dropped,
        "events_drained": events.len(),
        "log_events_drained": events.iter().filter(|event| event.payload.family == DiagnosticEventFamily::Log).count(),
    });
    let summary_artifact = write_json_artifact(
        &context.artifact_layout.run_dir,
        suite_id,
        "diagnostics-summary",
        &summary,
    )
    .map_err(|e| SuiteError::protocol(e, "diagnostic-summary-artifact"))?;
    artifacts.push(summary_artifact);

    if flush_dropped + drain_dropped > 0 {
        deferred_error.get_or_insert_with(|| {
            SuiteError::protocol(
                format!(
                    "diagnostic event stream dropped {} events",
                    flush_dropped + drain_dropped
                ),
                "diagnostic-dropped-events",
            )
        });
    }

    if let Some(err) = deferred_error {
        Err(err)
    } else {
        Ok(())
    }
}

pub(crate) fn assert_launched_process_snapshot(
    snapshot: &TerminalManagerSnapshot,
    diagnostics: &ObservedDiagnostics,
    process_id: u32,
) -> SuiteResult<()> {
    if diagnostics.hello.app.process_id != Some(process_id) {
        return Err(SuiteError::cross_layer(
            format!(
                "diagnostic hello pid {:?} did not match launched process id {process_id}",
                diagnostics.hello.app.process_id
            ),
            "diagnostic-pid-mismatch",
        ));
    }
    if snapshot.app.pid != Some(process_id) {
        return Err(SuiteError::cross_layer(
            format!(
                "diagnostic snapshot pid {:?} did not match launched process id {process_id}",
                snapshot.app.pid
            ),
            "diagnostic-snapshot-pid-mismatch",
        ));
    }
    Ok(())
}

pub(crate) fn assert_terminal_snapshot_sane(snapshot: &TerminalManagerSnapshot) -> SuiteResult<()> {
    if let Some(grid) = &snapshot.terminal.grid {
        if grid.rows == 0 || grid.cols == 0 {
            return Err(SuiteError::cross_layer(
                format!(
                    "terminal grid dimensions must be non-zero, got {}x{}",
                    grid.cols, grid.rows
                ),
                "diagnostic-terminal-grid-empty",
            ));
        }
        if let Some(visible_rows) = snapshot.terminal.visible_rows {
            if visible_rows > grid.rows {
                return Err(SuiteError::cross_layer(
                    format!(
                        "visible terminal rows {visible_rows} exceed grid rows {}",
                        grid.rows
                    ),
                    "diagnostic-terminal-visible-rows",
                ));
            }
        }
    }
    Ok(())
}

pub(crate) fn assert_renderer_surface_sane(snapshot: &TerminalManagerSnapshot) -> SuiteResult<()> {
    if let Some(surface) = &snapshot.renderer.surface_size {
        if surface.width == 0 || surface.height == 0 {
            return Err(SuiteError::cross_layer(
                "renderer surface dimensions must be non-zero when reported",
                "diagnostic-renderer-surface-empty",
            ));
        }
    }
    Ok(())
}

pub(crate) fn assert_invariants_passed(invariants: &[InvariantEvaluation]) -> SuiteResult<()> {
    if let Some(failed) = invariants
        .iter()
        .find(|result| result.outcome == InvariantOutcome::Failed)
    {
        return Err(SuiteError::cross_layer(
            format!(
                "diagnostic invariant {} failed: {}",
                failed.id,
                failed.message.as_deref().unwrap_or("no message")
            ),
            failed.id.clone(),
        ));
    }
    Ok(())
}

pub(crate) fn record_diagnostic_error(
    context: &SuiteContext<'_>,
    artifacts: &mut Vec<String>,
    suite_id: &str,
    message: &str,
) {
    let artifact = suite_artifact_name(suite_id, "diagnostics-errors", "log");
    let path = context.artifact_layout.run_dir.join(&artifact);
    if std::fs::write(&path, message).is_ok() && !artifacts.contains(&artifact) {
        artifacts.push(artifact);
    }
}

pub(crate) fn maybe_prompt_on_failure(
    context: &SuiteContext<'_>,
    artifacts: &mut Vec<String>,
    suite_id: &str,
    session: &mut AppSession,
    diagnostics: Option<&ObservedDiagnostics>,
) -> Option<InteractiveDecision> {
    if !context.should_pause_on_failure() {
        return None;
    }

    let mut runtime = SuiteInteractiveRuntime::new(
        &context.artifact_layout.run_dir,
        suite_id,
        diagnostics.map(|diagnostics| &diagnostics.client),
        session,
    );
    match prompt_interactive_failure(&context.artifact_layout.run_dir, suite_id, &mut runtime) {
        Ok(result) => {
            artifacts.extend(result.artifacts);
            Some(result.decision)
        }
        Err(err) => {
            record_diagnostic_error(
                context,
                artifacts,
                suite_id,
                &format!("interactive failure workflow failed: {err}"),
            );
            None
        }
    }
}

pub(crate) fn artifacts_with_common(
    common_artifacts: &[String],
    suite_artifacts: &[String],
) -> Vec<String> {
    let mut artifacts = common_artifacts.to_vec();
    artifacts.extend(suite_artifacts.iter().cloned());
    artifacts
}

pub(crate) fn format_rect(rect: DesktopRect) -> String {
    format!(
        "L{} T{} R{} B{} W{} H{}",
        rect.left,
        rect.top,
        rect.right,
        rect.bottom,
        rect.width(),
        rect.height()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn step_snapshots_are_captured_for_observed_modes() {
        assert!(!captures_step_snapshots(ObserveMode::Off));
        assert!(captures_step_snapshots(ObserveMode::Basic));
        assert!(captures_step_snapshots(ObserveMode::Full));
    }

    #[test]
    fn basic_step_snapshot_returns_none_without_diagnostics() {
        let workspace_root = std::env::temp_dir();
        let run_dir = workspace_root.join(format!(
            "xtask-observability-no-diagnostics-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&run_dir).unwrap();
        let artifact_layout = crate::desktop_regression::artifacts::ArtifactLayout {
            run_id: "run-test".to_owned(),
            results_path: run_dir.join("results.json"),
            run_dir,
        };
        let exe_path = workspace_root.join("terminal-manager.exe");
        let common_artifacts = Vec::new();
        let context = SuiteContext {
            workspace_root: &workspace_root,
            artifact_layout: &artifact_layout,
            exe_path: &exe_path,
            common_artifacts: &common_artifacts,
            observe: ObserveMode::Basic,
            interactive: false,
            keep_open_on_failure: false,
            action_recorder: None,
        };
        let mut artifacts = Vec::new();

        let snapshot = capture_step_snapshot(
            &context,
            &mut artifacts,
            "suite",
            None,
            "pre-snap-snapshot",
            "pre-snap",
        )
        .unwrap();

        assert!(snapshot.is_none());
        assert!(artifacts.is_empty());
        let _ = std::fs::remove_dir_all(&artifact_layout.run_dir);
    }

    #[test]
    fn formats_rect_like_legacy_runner() {
        let rect = DesktopRect {
            left: 1,
            top: 2,
            right: 101,
            bottom: 52,
        };

        assert_eq!(format_rect(rect), "L1 T2 R101 B52 W100 H50");
    }
}
