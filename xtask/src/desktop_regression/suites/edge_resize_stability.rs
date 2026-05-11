use std::thread;
use std::time::Duration;

use crate::desktop_regression::artifacts::suite_artifact_name;
use crate::desktop_regression::assertions::{assert_close, assert_true, SuiteError, SuiteResult};
use crate::desktop_regression::diagnostics::diagnostic_launch_for_mode;
use crate::desktop_regression::failure::collect_basic_failure_bundle;
use crate::desktop_regression::interactive::InteractiveDecision;
use crate::desktop_regression::launcher::{AppLogFiles, AppSession};
use crate::desktop_regression::results::SuiteExecutionRecord;
use crate::desktop_regression::screenshots::capture_screen;
use crate::desktop_regression::suites::observability::{
    artifacts_with_common, assert_launched_process_snapshot, assert_renderer_surface_sane,
    assert_terminal_snapshot_sane, capture_step_snapshot, finalize_diagnostics, format_rect,
    mark_full_step, maybe_prompt_on_failure, record_diagnostic_error, start_diagnostics,
    ObservedDiagnostics,
};
use crate::desktop_regression::suites::{forced_failure_for_suite, SuiteContext};
use crate::desktop_regression::win32::{self, DesktopRect};
use terminal_manager_diagnostics::{Rect, RunnerActionKind, RunnerActionTarget};

const SUITE_ID: &str = "edge-resize-stability";
const DRAG_DELTA: i32 = 220;
const TOLERANCE: i32 = 2;

pub fn run(context: &SuiteContext<'_>) -> SuiteExecutionRecord {
    let mut artifacts = Vec::new();
    let mut interactive_decision = None;
    match run_inner(context, &mut artifacts, &mut interactive_decision) {
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
            let mut record = SuiteExecutionRecord::failed(
                SUITE_ID,
                failure.kind,
                failure.message,
                failure.first_bad_signal,
                artifacts,
            );
            record.set_interactive_decision(interactive_decision);
            record
        }
    }
}

fn run_inner(
    context: &SuiteContext<'_>,
    artifacts: &mut Vec<String>,
    interactive_decision: &mut Option<InteractiveDecision>,
) -> SuiteResult<()> {
    let app_logs = AppLogFiles::create(&context.artifact_layout.run_dir, SUITE_ID)
        .map_err(|e| SuiteError::setup(format!("failed to create app log files: {e}")))?;
    artifacts.extend(app_logs.artifact_names());

    let diagnostic_launch =
        diagnostic_launch_for_mode(context.observe, &context.artifact_layout.run_id, SUITE_ID);
    let mut session = AppSession::launch_with_logs(
        context.exe_path,
        context.workspace_root,
        Some(&app_logs),
        diagnostic_launch.as_ref(),
    )
    .map_err(|e| SuiteError::setup(format!("failed to start app: {e}")))?;
    context
        .record_action(
            SUITE_ID,
            None,
            window_target(&session),
            RunnerActionKind::Note {
                message: "app.launch".to_owned(),
            },
        )
        .map_err(SuiteError::setup)?;
    let diagnostics = start_diagnostics(context, artifacts, SUITE_ID, diagnostic_launch.as_ref())?;

    let scenario_result = if let Some(forced) = forced_failure_for_suite(SUITE_ID) {
        Err(forced)
    } else {
        run_resize_scenario(context, artifacts, &session, diagnostics.as_ref())
    };
    let diagnostics_result = finalize_diagnostics(
        context,
        artifacts,
        SUITE_ID,
        diagnostics.as_ref(),
        scenario_result.is_err(),
    );

    let result = match (scenario_result, diagnostics_result) {
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
    };

    if result.is_err() {
        *interactive_decision = maybe_prompt_on_failure(
            context,
            artifacts,
            SUITE_ID,
            &mut session,
            diagnostics.as_ref(),
        );
    }

    result
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
    context
        .record_action(
            SUITE_ID,
            None,
            window_target(session),
            RunnerActionKind::MoveWindow {
                bounds: Rect {
                    x: 0,
                    y: 0,
                    width: target_width as u32,
                    height: target_height as u32,
                },
            },
        )
        .map_err(SuiteError::setup)?;
    thread::sleep(Duration::from_millis(700));
    context
        .record_action(
            SUITE_ID,
            None,
            RunnerActionTarget::None,
            RunnerActionKind::Wait {
                mode: "fixed_sleep".to_owned(),
                reason: "after initial window placement".to_owned(),
                timeout_ms: 700,
            },
        )
        .map_err(SuiteError::setup)?;

    win32::focus_window(hwnd).map_err(SuiteError::setup)?;
    context
        .record_action(
            SUITE_ID,
            None,
            window_target(session),
            RunnerActionKind::Mouse {
                x: target_width / 2,
                y: 8,
                button: Some("left".to_owned()),
            },
        )
        .map_err(SuiteError::setup)?;

    let start = screenshot_path(context, "start");
    let after = screenshot_path(context, "after");
    let restore = screenshot_path(context, "restore");

    mark_full_step(
        context,
        diagnostics,
        "resize-inward",
        "Drag left edge inward",
    )?;
    context
        .record_action(
            SUITE_ID,
            Some("resize-inward"),
            RunnerActionTarget::None,
            RunnerActionKind::MarkStep {
                id: "resize-inward".to_owned(),
            },
        )
        .map_err(SuiteError::setup)?;
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
    context
        .record_action(
            SUITE_ID,
            Some("resize-inward"),
            RunnerActionTarget::Desktop,
            RunnerActionKind::Screenshot {
                path: suite_artifact_name(SUITE_ID, "start", "png"),
            },
        )
        .map_err(SuiteError::setup)?;
    println!("initial_rect={}", format_rect(r0));

    let center_y = ((r0.top + r0.bottom) as f64 / 2.0).round() as i32;
    let left_x = r0.left + 4;
    let drag_to_x = (r0.right - 20).min(left_x + DRAG_DELTA);

    win32::left_edge_drag(hwnd, left_x, center_y, drag_to_x).map_err(SuiteError::setup)?;
    context
        .record_action(
            SUITE_ID,
            Some("resize-inward"),
            window_target(session),
            RunnerActionKind::MouseDrag {
                from_x: left_x,
                from_y: center_y,
                to_x: drag_to_x,
                to_y: center_y,
                button: Some("left".to_owned()),
            },
        )
        .map_err(SuiteError::setup)?;
    let r1 = win32::get_window_rect(hwnd).map_err(SuiteError::setup)?;
    context
        .record_action(
            SUITE_ID,
            Some("resize-inward"),
            window_target(session),
            RunnerActionKind::ResizeWindow {
                bounds: schema_rect(r1),
            },
        )
        .map_err(SuiteError::setup)?;
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
    context
        .record_action(
            SUITE_ID,
            Some("resize-inward"),
            RunnerActionTarget::Desktop,
            RunnerActionKind::Screenshot {
                path: suite_artifact_name(SUITE_ID, "after", "png"),
            },
        )
        .map_err(SuiteError::setup)?;

    mark_full_step(
        context,
        diagnostics,
        "resize-restore",
        "Drag left edge back to origin",
    )?;
    context
        .record_action(
            SUITE_ID,
            Some("resize-restore"),
            RunnerActionTarget::None,
            RunnerActionKind::MarkStep {
                id: "resize-restore".to_owned(),
            },
        )
        .map_err(SuiteError::setup)?;
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
    context
        .record_action(
            SUITE_ID,
            Some("resize-restore"),
            window_target(session),
            RunnerActionKind::MouseDrag {
                from_x: restore_from_x,
                from_y: center_y,
                to_x: restore_x,
                to_y: center_y,
                button: Some("left".to_owned()),
            },
        )
        .map_err(SuiteError::setup)?;
    let r2 = win32::get_window_rect(hwnd).map_err(SuiteError::setup)?;
    context
        .record_action(
            SUITE_ID,
            Some("resize-restore"),
            window_target(session),
            RunnerActionKind::ResizeWindow {
                bounds: schema_rect(r2),
            },
        )
        .map_err(SuiteError::setup)?;
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
    context
        .record_action(
            SUITE_ID,
            Some("resize-restore"),
            RunnerActionTarget::Desktop,
            RunnerActionKind::Screenshot {
                path: suite_artifact_name(SUITE_ID, "restore", "png"),
            },
        )
        .map_err(SuiteError::setup)?;

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

fn window_target(session: &AppSession) -> RunnerActionTarget {
    RunnerActionTarget::Window {
        title: Some("Terminal Manager".to_owned()),
        process_id: Some(session.process_id()),
    }
}

fn schema_rect(rect: DesktopRect) -> Rect {
    Rect {
        x: rect.left,
        y: rect.top,
        width: rect.width().max(0) as u32,
        height: rect.height().max(0) as u32,
    }
}
