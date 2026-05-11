use std::thread;
use std::time::Duration;

use crate::desktop_regression::artifacts::suite_artifact_name;
use crate::desktop_regression::assertions::{assert_close, assert_true, SuiteError, SuiteResult};
use crate::desktop_regression::diagnostics::diagnostic_launch_for_mode;
use crate::desktop_regression::failure::collect_basic_failure_bundle;
use crate::desktop_regression::launcher::{AppLogFiles, AppSession};
use crate::desktop_regression::results::SuiteExecutionRecord;
use crate::desktop_regression::screenshots::capture_screen;
use crate::desktop_regression::suites::observability::{
    artifacts_with_common, assert_launched_process_snapshot, assert_renderer_surface_sane,
    assert_terminal_snapshot_sane, capture_step_snapshot, finalize_diagnostics, format_rect,
    mark_full_step, record_diagnostic_error, start_diagnostics, ObservedDiagnostics,
};
use crate::desktop_regression::suites::SuiteContext;
use crate::desktop_regression::win32;

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
    let diagnostics = start_diagnostics(context, artifacts, SUITE_ID, diagnostic_launch.as_ref())?;

    let scenario_result = run_resize_scenario(context, artifacts, &session, diagnostics.as_ref());
    let diagnostics_result = finalize_diagnostics(
        context,
        artifacts,
        SUITE_ID,
        diagnostics.as_ref(),
        scenario_result.is_err(),
    );

    match (scenario_result, diagnostics_result) {
        (Err(primary), Err(diagnostic_error)) => {
            record_diagnostic_error(context, artifacts, SUITE_ID, &diagnostic_error.message);
            Err(primary)
        }
        (Err(primary), Ok(())) => Err(primary),
        (Ok(()), Err(diagnostic_error)) => {
            record_diagnostic_error(context, artifacts, SUITE_ID, &diagnostic_error.message);
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
        SUITE_ID,
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
        SUITE_ID,
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
        SUITE_ID,
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
        SUITE_ID,
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
        assert_launched_process_snapshot(snapshot, diagnostics, session.process_id())?;
        assert_terminal_snapshot_sane(snapshot)?;
        assert_renderer_surface_sane(snapshot)?;
    }

    Ok(())
}

fn screenshot_path(context: &SuiteContext<'_>, name: &str) -> std::path::PathBuf {
    let file_name = suite_artifact_name(SUITE_ID, name, "png");
    context.artifact_layout.run_dir.join(file_name)
}
