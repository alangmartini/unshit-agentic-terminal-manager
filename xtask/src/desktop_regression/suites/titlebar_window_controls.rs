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
use crate::desktop_regression::win32::{self, DesktopRect, DesktopSize};
use terminal_manager_diagnostics::{Rect, RunnerActionKind, RunnerActionTarget};

const SUITE_ID: &str = "titlebar-window-controls";
const SETTLE_MS: u64 = 800;
const TITLEBAR_CLICK_Y: i32 = 17;
const MAXIMIZE_BUTTON_CENTER_FROM_RIGHT: i32 = 69;
const RESTORE_TOLERANCE: i32 = 24;

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
        run_window_controls_scenario(context, artifacts, &session, diagnostics.as_ref())
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

fn run_window_controls_scenario(
    context: &SuiteContext<'_>,
    artifacts: &mut Vec<String>,
    session: &AppSession,
    diagnostics: Option<&ObservedDiagnostics>,
) -> SuiteResult<()> {
    let hwnd = session.window();
    let placement = initial_window_rect(win32::screen_size().map_err(SuiteError::setup)?);
    win32::set_window_rect(
        hwnd,
        placement.left,
        placement.top,
        placement.width(),
        placement.height(),
    )
    .map_err(SuiteError::setup)?;
    context
        .record_action(
            SUITE_ID,
            None,
            window_target(session),
            RunnerActionKind::MoveWindow {
                bounds: schema_rect(placement),
            },
        )
        .map_err(SuiteError::setup)?;
    record_wait(context, None, "after initial window placement", SETTLE_MS)?;
    win32::focus_window(hwnd).map_err(SuiteError::setup)?;
    let restored_before = win32::get_window_rect(hwnd).map_err(SuiteError::setup)?;
    assert_true(
        !win32::is_window_maximized(hwnd).map_err(SuiteError::setup)?,
        "window started maximized before titlebar control test",
        "titlebar-window-started-maximized",
    )?;

    capture_named_screenshot(context, artifacts, "restored-before")?;

    mark_full_step(
        context,
        diagnostics,
        "maximize",
        "Click custom titlebar maximize button",
    )?;
    context
        .record_action(
            SUITE_ID,
            Some("maximize"),
            RunnerActionTarget::None,
            RunnerActionKind::MarkStep {
                id: "maximize".to_owned(),
            },
        )
        .map_err(SuiteError::setup)?;
    capture_step_snapshot(
        context,
        artifacts,
        SUITE_ID,
        diagnostics,
        "maximize-before-snapshot",
        "before titlebar maximize",
    )?;
    click_maximize_button(context, session, "maximize")?;
    record_wait(
        context,
        Some("maximize"),
        "after titlebar maximize",
        SETTLE_MS,
    )?;
    let maximized = win32::get_window_rect(hwnd).map_err(SuiteError::setup)?;
    context
        .record_action(
            SUITE_ID,
            Some("maximize"),
            window_target(session),
            RunnerActionKind::ResizeWindow {
                bounds: schema_rect(maximized),
            },
        )
        .map_err(SuiteError::setup)?;
    let maximized_snapshot = capture_step_snapshot(
        context,
        artifacts,
        SUITE_ID,
        diagnostics,
        "maximize-after-snapshot",
        "after titlebar maximize",
    )?;
    capture_named_screenshot(context, artifacts, "maximized")?;
    assert_true(
        win32::is_window_maximized(hwnd).map_err(SuiteError::setup)?,
        "custom titlebar maximize button did not maximize the window",
        "titlebar-maximize-state",
    )?;
    assert_true(
        rect_grew(restored_before, maximized),
        &format!(
            "custom titlebar maximize did not grow window: before={} after={}",
            format_rect(restored_before),
            format_rect(maximized)
        ),
        "titlebar-maximize-geometry",
    )?;

    mark_full_step(
        context,
        diagnostics,
        "restore",
        "Click custom titlebar restore button",
    )?;
    context
        .record_action(
            SUITE_ID,
            Some("restore"),
            RunnerActionTarget::None,
            RunnerActionKind::MarkStep {
                id: "restore".to_owned(),
            },
        )
        .map_err(SuiteError::setup)?;
    capture_step_snapshot(
        context,
        artifacts,
        SUITE_ID,
        diagnostics,
        "restore-before-snapshot",
        "before titlebar restore",
    )?;
    click_maximize_button(context, session, "restore")?;
    record_wait(
        context,
        Some("restore"),
        "after titlebar restore",
        SETTLE_MS,
    )?;
    let restored_after = win32::get_window_rect(hwnd).map_err(SuiteError::setup)?;
    context
        .record_action(
            SUITE_ID,
            Some("restore"),
            window_target(session),
            RunnerActionKind::ResizeWindow {
                bounds: schema_rect(restored_after),
            },
        )
        .map_err(SuiteError::setup)?;
    let restored_snapshot = capture_step_snapshot(
        context,
        artifacts,
        SUITE_ID,
        diagnostics,
        "restore-after-snapshot",
        "after titlebar restore",
    )?;
    capture_named_screenshot(context, artifacts, "restored-after")?;
    assert_true(
        !win32::is_window_maximized(hwnd).map_err(SuiteError::setup)?,
        "custom titlebar restore click left the window maximized",
        "titlebar-restore-state",
    )?;
    assert_rect_close(
        restored_after,
        restored_before,
        RESTORE_TOLERANCE,
        "titlebar-restore",
    )?;

    println!("titlebar_restored_before={}", format_rect(restored_before));
    println!("titlebar_maximized={}", format_rect(maximized));
    println!("titlebar_restored_after={}", format_rect(restored_after));

    if let (Some(diagnostics), Some(snapshot)) = (diagnostics, maximized_snapshot.as_ref()) {
        assert_launched_process_snapshot(snapshot, diagnostics, session.process_id())?;
        assert_terminal_snapshot_sane(snapshot)?;
        assert_renderer_surface_sane(snapshot)?;
    }
    if let (Some(diagnostics), Some(snapshot)) = (diagnostics, restored_snapshot.as_ref()) {
        assert_launched_process_snapshot(snapshot, diagnostics, session.process_id())?;
        assert_terminal_snapshot_sane(snapshot)?;
        assert_renderer_surface_sane(snapshot)?;
    }

    Ok(())
}

fn click_maximize_button(
    context: &SuiteContext<'_>,
    session: &AppSession,
    step_id: &str,
) -> SuiteResult<()> {
    let hwnd = session.window();
    let client_rect = win32::get_client_rect(hwnd).map_err(SuiteError::setup)?;
    let scale_factor = win32::window_scale_factor(hwnd).map_err(SuiteError::setup)?;
    let (x, y) = maximize_button_point(client_rect, scale_factor);
    win32::mouse_click(x, y, Some("left")).map_err(SuiteError::setup)?;
    context
        .record_action(
            SUITE_ID,
            Some(step_id),
            window_target(session),
            RunnerActionKind::Mouse {
                x,
                y,
                button: Some("left".to_owned()),
            },
        )
        .map_err(SuiteError::setup)
}

fn record_wait(
    context: &SuiteContext<'_>,
    step_id: Option<&str>,
    reason: &str,
    timeout_ms: u64,
) -> SuiteResult<()> {
    thread::sleep(Duration::from_millis(timeout_ms));
    context
        .record_action(
            SUITE_ID,
            step_id,
            RunnerActionTarget::None,
            RunnerActionKind::Wait {
                mode: "fixed_sleep".to_owned(),
                reason: reason.to_owned(),
                timeout_ms,
            },
        )
        .map_err(SuiteError::setup)
}

fn capture_named_screenshot(
    context: &SuiteContext<'_>,
    artifacts: &mut Vec<String>,
    name: &str,
) -> SuiteResult<()> {
    let artifact = suite_artifact_name(SUITE_ID, name, "png");
    capture_screen(&screenshot_path(context, name)).map_err(SuiteError::setup)?;
    artifacts.push(artifact.clone());
    context
        .record_action(
            SUITE_ID,
            None,
            RunnerActionTarget::Desktop,
            RunnerActionKind::Screenshot { path: artifact },
        )
        .map_err(SuiteError::setup)
}

fn maximize_button_point(client_rect: DesktopRect, scale_factor: f64) -> (i32, i32) {
    (
        client_rect.right - css_px_to_physical(MAXIMIZE_BUTTON_CENTER_FROM_RIGHT, scale_factor),
        client_rect.top + css_px_to_physical(TITLEBAR_CLICK_Y, scale_factor),
    )
}

fn css_px_to_physical(css_px: i32, scale_factor: f64) -> i32 {
    (css_px as f64 * scale_factor).round() as i32
}

fn initial_window_rect(screen: DesktopSize) -> DesktopRect {
    let width = (screen.width * 55 / 100).clamp(720, screen.width.saturating_sub(160));
    let height = (screen.height * 58 / 100).clamp(480, screen.height.saturating_sub(160));
    let left = ((screen.width - width) / 2).max(0);
    let top = ((screen.height - height) / 2).max(0);

    DesktopRect {
        left,
        top,
        right: left + width,
        bottom: top + height,
    }
}

fn rect_grew(before: DesktopRect, after: DesktopRect) -> bool {
    after.width() > before.width() || after.height() > before.height()
}

fn assert_rect_close(
    actual: DesktopRect,
    expected: DesktopRect,
    tolerance: i32,
    label: &str,
) -> SuiteResult<()> {
    assert_close(
        actual.left,
        expected.left,
        tolerance,
        &format!("{label}-left"),
    )?;
    assert_close(actual.top, expected.top, tolerance, &format!("{label}-top"))?;
    assert_close(
        actual.width(),
        expected.width(),
        tolerance,
        &format!("{label}-width"),
    )?;
    assert_close(
        actual.height(),
        expected.height(),
        tolerance,
        &format!("{label}-height"),
    )
}

fn screenshot_path(context: &SuiteContext<'_>, name: &str) -> std::path::PathBuf {
    context
        .artifact_layout
        .run_dir
        .join(suite_artifact_name(SUITE_ID, name, "png"))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(left: i32, top: i32, right: i32, bottom: i32) -> DesktopRect {
        DesktopRect {
            left,
            top,
            right,
            bottom,
        }
    }

    #[test]
    fn maximize_button_point_targets_middle_window_control() {
        assert_eq!(
            maximize_button_point(rect(100, 50, 900, 650), 1.0),
            (831, 67)
        );
    }

    #[test]
    fn maximize_button_point_scales_css_offsets() {
        assert_eq!(
            maximize_button_point(rect(100, 50, 900, 650), 1.5),
            (796, 76)
        );
    }

    #[test]
    fn css_px_to_physical_rounds_scaled_pixels() {
        assert_eq!(css_px_to_physical(69, 1.25), 86);
    }

    #[test]
    fn rect_grew_accepts_width_or_height_growth() {
        assert!(rect_grew(rect(0, 0, 100, 100), rect(0, 0, 200, 100)));
        assert!(rect_grew(rect(0, 0, 100, 100), rect(0, 0, 100, 200)));
        assert!(!rect_grew(rect(0, 0, 100, 100), rect(0, 0, 100, 100)));
    }

    #[test]
    fn assert_rect_close_allows_small_restore_drift() {
        assert!(assert_rect_close(
            rect(104, 96, 904, 646),
            rect(100, 100, 900, 650),
            8,
            "restore",
        )
        .is_ok());
    }

    #[test]
    fn assert_rect_close_rejects_large_restore_drift() {
        let err = assert_rect_close(
            rect(140, 100, 940, 650),
            rect(100, 100, 900, 650),
            8,
            "restore",
        )
        .unwrap_err();

        assert_eq!(err.first_bad_signal.as_deref(), Some("restore-left"));
    }

    #[test]
    fn initial_window_rect_stays_inside_screen() {
        let got = initial_window_rect(DesktopSize {
            width: 1920,
            height: 1080,
        });

        assert!(got.left >= 0);
        assert!(got.top >= 0);
        assert!(got.right <= 1920);
        assert!(got.bottom <= 1080);
        assert!(got.width() >= 720);
        assert!(got.height() >= 480);
    }

    #[test]
    fn screenshot_path_uses_suite_artifact_name() {
        let root = std::path::PathBuf::from("run");
        let layout = crate::desktop_regression::artifacts::ArtifactLayout {
            run_id: "run-id".to_owned(),
            run_dir: root.clone(),
            results_path: root.join("results.json"),
        };
        let context = SuiteContext {
            workspace_root: std::path::Path::new("."),
            artifact_layout: &layout,
            exe_path: std::path::Path::new("app.exe"),
            common_artifacts: &[],
            observe: terminal_manager_diagnostics::ObserveMode::Off,
            interactive: false,
            keep_open_on_failure: false,
            action_recorder: None,
        };

        assert_eq!(
            screenshot_path(&context, "maximized"),
            root.join("titlebar-window-controls-maximized.png")
        );
    }
}
