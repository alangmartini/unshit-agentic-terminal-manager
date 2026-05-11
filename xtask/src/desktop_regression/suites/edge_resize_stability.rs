use std::thread;
use std::time::Duration;

use crate::desktop_regression::artifacts::suite_artifact_name;
use crate::desktop_regression::assertions::{assert_close, assert_true, SuiteError, SuiteResult};
use crate::desktop_regression::diagnostics::{
    diagnostic_launch_for_mode, write_diagnostic_events, write_json_artifact, DiagnosticClient,
    DiagnosticHello,
};
use crate::desktop_regression::failure::collect_basic_failure_bundle;
use crate::desktop_regression::launcher::{AppLogFiles, AppSession};
use crate::desktop_regression::results::SuiteExecutionRecord;
use crate::desktop_regression::screenshots::capture_screen;
use crate::desktop_regression::suites::SuiteContext;
use crate::desktop_regression::win32::{self, DesktopRect};
use terminal_manager_diagnostics::{
    DiagnosticEventFamily, InvariantEvaluation, InvariantOutcome, ObserveMode,
    TerminalManagerSnapshot,
};

const SUITE_ID: &str = "edge-resize-stability";
const DRAG_DELTA: i32 = 220;
const TOLERANCE: i32 = 2;

pub fn run(context: &SuiteContext<'_>) -> SuiteExecutionRecord {
    let mut artifacts = Vec::new();
    match run_inner(context, &mut artifacts) {
        Ok(()) => SuiteExecutionRecord::passed(SUITE_ID, artifacts),
        Err(err) => {
            let failure = err.to_suite_failure();
            let added = collect_basic_failure_bundle(
                &context.artifact_layout.run_dir,
                &context.artifact_layout.run_id,
                SUITE_ID,
                &failure,
                &artifacts_with_common(context.common_artifacts, &artifacts),
            );
            artifacts.extend(added);
            SuiteExecutionRecord::failed(
                SUITE_ID,
                failure.kind,
                failure.message,
                failure.first_bad_signal,
                artifacts,
            )
        }
    }
}

fn run_inner(context: &SuiteContext<'_>, artifacts: &mut Vec<String>) -> SuiteResult<()> {
    let app_logs = AppLogFiles::create(&context.artifact_layout.run_dir, SUITE_ID)
        .map_err(|e| SuiteError::setup(format!("failed to create app log files: {e}")))?;
    artifacts.extend(app_logs.artifact_names());

    let diagnostic_launch =
        diagnostic_launch_for_mode(context.observe, &context.artifact_layout.run_id, SUITE_ID);
    let session = AppSession::launch_with_logs(
        context.exe_path,
        context.workspace_root,
        Some(&app_logs),
        diagnostic_launch.as_ref(),
    )
    .map_err(|e| SuiteError::setup(format!("failed to start app: {e}")))?;
    let diagnostics = start_diagnostics(context, artifacts, diagnostic_launch.as_ref())?;

    let scenario_result = run_resize_scenario(context, artifacts, &session, diagnostics.as_ref());
    let diagnostics_result = finalize_diagnostics(
        context,
        artifacts,
        diagnostics.as_ref(),
        scenario_result.is_err(),
    );

    match (scenario_result, diagnostics_result) {
        (Err(primary), Err(diagnostic_error)) => {
            record_diagnostic_error(context, artifacts, &diagnostic_error.message);
            Err(primary)
        }
        (Err(primary), Ok(())) => Err(primary),
        (Ok(()), Err(diagnostic_error)) => {
            record_diagnostic_error(context, artifacts, &diagnostic_error.message);
            Err(diagnostic_error)
        }
        (Ok(()), Ok(())) => Ok(()),
    }
}

fn run_resize_scenario(
    context: &SuiteContext<'_>,
    artifacts: &mut Vec<String>,
    session: &AppSession,
    diagnostics: Option<&ObservedDiagnostics>,
) -> SuiteResult<()> {
    let hwnd = session.window();
    let screen = win32::screen_size().map_err(SuiteError::setup)?;

    let target_width = (screen.width as f64 / 2.0).round() as i32;
    let target_height = 500.max((screen.height as f64 * 0.88).round() as i32);
    win32::set_window_rect(hwnd, 0, 0, target_width, target_height).map_err(SuiteError::setup)?;
    thread::sleep(Duration::from_millis(700));

    win32::focus_window(hwnd).map_err(SuiteError::setup)?;

    let start = screenshot_path(context, "start");
    let after = screenshot_path(context, "after");
    let restore = screenshot_path(context, "restore");

    mark_full_step(
        context,
        diagnostics,
        "resize-inward",
        "Drag left edge inward",
    )?;
    let r0 = win32::get_window_rect(hwnd).map_err(SuiteError::setup)?;
    capture_step_snapshot(
        context,
        artifacts,
        diagnostics,
        "resize-inward-before-snapshot",
        "before resize inward",
    )?;
    capture_screen(&start).map_err(SuiteError::setup)?;
    artifacts.push(suite_artifact_name(SUITE_ID, "start", "png"));
    println!("initial_rect={}", format_rect(r0));

    let center_y = ((r0.top + r0.bottom) as f64 / 2.0).round() as i32;
    let left_x = r0.left + 4;
    let drag_to_x = (r0.right - 20).min(left_x + DRAG_DELTA);

    win32::left_edge_drag(hwnd, left_x, center_y, drag_to_x).map_err(SuiteError::setup)?;
    let r1 = win32::get_window_rect(hwnd).map_err(SuiteError::setup)?;
    capture_step_snapshot(
        context,
        artifacts,
        diagnostics,
        "resize-inward-after-snapshot",
        "after resize inward",
    )?;
    capture_screen(&after).map_err(SuiteError::setup)?;
    artifacts.push(suite_artifact_name(SUITE_ID, "after", "png"));

    mark_full_step(
        context,
        diagnostics,
        "resize-restore",
        "Drag left edge back to origin",
    )?;
    capture_step_snapshot(
        context,
        artifacts,
        diagnostics,
        "resize-restore-before-snapshot",
        "before resize restore",
    )?;
    let restore_x = 0.max(r0.left + 4);
    let restore_from_x = r1.left + 4;
    win32::left_edge_drag(hwnd, restore_from_x, center_y, restore_x).map_err(SuiteError::setup)?;
    let r2 = win32::get_window_rect(hwnd).map_err(SuiteError::setup)?;
    let restore_snapshot = capture_step_snapshot(
        context,
        artifacts,
        diagnostics,
        "resize-restore-after-snapshot",
        "after resize restore",
    )?;
    capture_screen(&restore).map_err(SuiteError::setup)?;
    artifacts.push(suite_artifact_name(SUITE_ID, "restore", "png"));

    println!("after_rect={}", format_rect(r1));
    println!("restore_rect={}", format_rect(r2));

    assert_close(r1.right, r0.right, TOLERANCE, "after-right-edge")?;
    assert_close(r2.right, r0.right, TOLERANCE, "restore-right-edge")?;
    assert_true(
        r1.left > r0.left,
        "left edge did not move right during inward resize",
        "left-edge-inward-resize",
    )?;
    assert_true(
        (r2.left - r0.left).abs() <= TOLERANCE,
        "left edge did not return near origin after outward resize",
        "left-edge-restore",
    )?;
    if let (Some(diagnostics), Some(snapshot)) = (diagnostics, restore_snapshot.as_ref()) {
        assert_full_snapshot_sane(snapshot, &diagnostics.hello, session.process_id())?;
    }

    Ok(())
}

struct ObservedDiagnostics {
    client: DiagnosticClient,
    hello: DiagnosticHello,
}

fn start_diagnostics(
    context: &SuiteContext<'_>,
    artifacts: &mut Vec<String>,
    launch: Option<&crate::desktop_regression::diagnostics::DiagnosticLaunchConfig>,
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
        SUITE_ID,
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

fn mark_full_step(
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

fn capture_step_snapshot(
    context: &SuiteContext<'_>,
    artifacts: &mut Vec<String>,
    diagnostics: Option<&ObservedDiagnostics>,
    artifact_stem: &str,
    reason: &str,
) -> SuiteResult<Option<TerminalManagerSnapshot>> {
    if context.observe != ObserveMode::Full {
        return Ok(None);
    }
    let Some(diagnostics) = diagnostics else {
        return Ok(None);
    };

    let snapshot = diagnostics.client.snapshot(reason).map_err(|e| {
        SuiteError::protocol(
            format!("diagnostic snapshot failed for {reason}: {e}"),
            "diagnostic-step-snapshot",
        )
    })?;
    let artifact = write_json_artifact(
        &context.artifact_layout.run_dir,
        SUITE_ID,
        artifact_stem,
        &snapshot,
    )
    .map_err(|e| SuiteError::protocol(e, "diagnostic-step-snapshot-artifact"))?;
    artifacts.push(artifact);
    Ok(Some(snapshot))
}

fn finalize_diagnostics(
    context: &SuiteContext<'_>,
    artifacts: &mut Vec<String>,
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
        SUITE_ID,
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
            SUITE_ID,
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
        write_diagnostic_events(&context.artifact_layout.run_dir, SUITE_ID, &events)
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
        SUITE_ID,
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

fn assert_full_snapshot_sane(
    snapshot: &TerminalManagerSnapshot,
    hello: &DiagnosticHello,
    process_id: u32,
) -> SuiteResult<()> {
    if hello.app.process_id != Some(process_id) {
        return Err(SuiteError::cross_layer(
            format!(
                "diagnostic hello pid {:?} did not match launched process id {process_id}",
                hello.app.process_id
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

fn assert_invariants_passed(invariants: &[InvariantEvaluation]) -> SuiteResult<()> {
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

fn record_diagnostic_error(context: &SuiteContext<'_>, artifacts: &mut Vec<String>, message: &str) {
    let artifact = suite_artifact_name(SUITE_ID, "diagnostics-errors", "log");
    let path = context.artifact_layout.run_dir.join(&artifact);
    if std::fs::write(&path, message).is_ok() && !artifacts.contains(&artifact) {
        artifacts.push(artifact);
    }
}

fn artifacts_with_common(common_artifacts: &[String], suite_artifacts: &[String]) -> Vec<String> {
    let mut artifacts = common_artifacts.to_vec();
    artifacts.extend(suite_artifacts.iter().cloned());
    artifacts
}

fn screenshot_path(context: &SuiteContext<'_>, name: &str) -> std::path::PathBuf {
    let file_name = suite_artifact_name(SUITE_ID, name, "png");
    context.artifact_layout.run_dir.join(file_name)
}

fn format_rect(rect: DesktopRect) -> String {
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
