use std::thread;
use std::time::Duration;

use crate::desktop_regression::artifacts::suite_artifact_name;
use crate::desktop_regression::assertions::{assert_close, assert_true, SuiteError, SuiteResult};
use crate::desktop_regression::diagnostics::diagnostic_launch_for_mode;
use crate::desktop_regression::failure::collect_basic_failure_bundle;
use crate::desktop_regression::interactive::InteractiveDecision;
use crate::desktop_regression::launcher::{AppLogFiles, AppSession};
use crate::desktop_regression::replay::ValidatedTrace;
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
use terminal_manager_diagnostics::{Rect, RunnerAction, RunnerActionKind, RunnerActionTarget};

const SUITE_ID: &str = "edge-resize-stability";
const DRAG_DELTA: i32 = 220;
const TOLERANCE: i32 = 2;
const INITIAL_X: i32 = 160;
const INITIAL_Y: i32 = 120;
const MIN_RESIZED_WIDTH: i32 = 360;

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

pub fn run_replay(context: &SuiteContext<'_>, trace: &ValidatedTrace) -> SuiteExecutionRecord {
    let replay_actions = trace.actions_for_suite(SUITE_ID);
    let mut artifacts = Vec::new();
    let mut interactive_decision = None;
    let mut record = match run_replay_inner(
        context,
        &replay_actions,
        &mut artifacts,
        &mut interactive_decision,
    ) {
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
    };
    record.actions = replay_actions;
    record
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

fn run_replay_inner(
    context: &SuiteContext<'_>,
    replay_actions: &[RunnerAction],
    artifacts: &mut Vec<String>,
    interactive_decision: &mut Option<InteractiveDecision>,
) -> SuiteResult<()> {
    if replay_actions.is_empty() {
        return Err(SuiteError::setup(format!(
            "replay trace contains no actions for {SUITE_ID}"
        )));
    }

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
    .map_err(|e| SuiteError::setup(format!("failed to start app for replay: {e}")))?;
    context
        .record_action(
            SUITE_ID,
            None,
            window_target(&session),
            RunnerActionKind::Note {
                message: "app.launch.replay".to_owned(),
            },
        )
        .map_err(SuiteError::setup)?;
    let diagnostics = start_diagnostics(context, artifacts, SUITE_ID, diagnostic_launch.as_ref())?;

    let scenario_result = if let Some(forced) = forced_failure_for_suite(SUITE_ID) {
        Err(forced)
    } else {
        run_replay_actions(
            context,
            artifacts,
            &session,
            diagnostics.as_ref(),
            replay_actions,
        )
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

    let available_width = (screen.width - INITIAL_X - 120).max(480);
    let available_height = (screen.height - INITIAL_Y - 80).max(320);
    let target_width = ((screen.width as f64 * 0.55).round() as i32).clamp(480, available_width);
    let target_height = ((screen.height as f64 * 0.65).round() as i32).clamp(320, available_height);
    win32::set_window_rect(hwnd, INITIAL_X, INITIAL_Y, target_width, target_height)
        .map_err(SuiteError::setup)?;
    context
        .record_action(
            SUITE_ID,
            None,
            window_target(session),
            RunnerActionKind::MoveWindow {
                bounds: Rect {
                    x: INITIAL_X,
                    y: INITIAL_Y,
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
                x: INITIAL_X + target_width / 2,
                y: INITIAL_Y + 8,
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

    let resized_left = (r0.left + DRAG_DELTA).min(r0.right - MIN_RESIZED_WIDTH);
    let resized_width = r0.right - resized_left;
    win32::set_window_rect(hwnd, resized_left, r0.top, resized_width, r0.height())
        .map_err(SuiteError::setup)?;
    thread::sleep(Duration::from_millis(700));
    context
        .record_action(
            SUITE_ID,
            Some("resize-inward"),
            window_target(session),
            RunnerActionKind::ResizeWindow {
                bounds: Rect {
                    x: resized_left,
                    y: r0.top,
                    width: resized_width.max(0) as u32,
                    height: r0.height().max(0) as u32,
                },
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
    win32::set_window_rect(hwnd, r0.left, r0.top, r0.width(), r0.height())
        .map_err(SuiteError::setup)?;
    thread::sleep(Duration::from_millis(700));
    context
        .record_action(
            SUITE_ID,
            Some("resize-restore"),
            window_target(session),
            RunnerActionKind::ResizeWindow {
                bounds: schema_rect(r0),
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

fn run_replay_actions(
    context: &SuiteContext<'_>,
    artifacts: &mut Vec<String>,
    session: &AppSession,
    diagnostics: Option<&ObservedDiagnostics>,
    replay_actions: &[RunnerAction],
) -> SuiteResult<()> {
    let hwnd = session.window();
    let mut resize_checks = 0;

    for action in replay_actions {
        match &action.kind {
            RunnerActionKind::Note { .. } => {}
            RunnerActionKind::MarkStep { id } => {
                mark_full_step(context, diagnostics, id, &format!("Replay {id}"))?;
            }
            RunnerActionKind::MoveWindow { bounds } => {
                win32::set_window_rect(
                    hwnd,
                    bounds.x,
                    bounds.y,
                    bounds.width as i32,
                    bounds.height as i32,
                )
                .map_err(SuiteError::setup)?;
                thread::sleep(Duration::from_millis(250));
            }
            RunnerActionKind::Mouse { x, y, button } => {
                win32::mouse_click(*x, *y, button.as_deref()).map_err(SuiteError::setup)?;
            }
            RunnerActionKind::Wait { timeout_ms, .. } => {
                thread::sleep(Duration::from_millis((*timeout_ms).min(5_000)));
            }
            RunnerActionKind::MouseDrag {
                from_x,
                from_y,
                to_x,
                to_y,
                button,
            } => {
                validate_left_button(button.as_deref())?;
                if from_y != to_y {
                    return Err(SuiteError::setup(format!(
                        "logical replay only supports horizontal edge drags, got y {from_y}->{to_y}"
                    )));
                }
                if from_x != to_x {
                    win32::left_edge_drag(hwnd, *from_x, *from_y, *to_x)
                        .map_err(SuiteError::setup)?;
                }
            }
            RunnerActionKind::ResizeWindow { bounds } => {
                let actual = win32::get_window_rect(hwnd).map_err(SuiteError::setup)?;
                assert_rect_matches_action(actual, *bounds, action.step_id.as_deref())?;
                resize_checks += 1;
            }
            RunnerActionKind::Screenshot { path } => {
                capture_screen(&context.artifact_layout.run_dir.join(path))
                    .map_err(SuiteError::setup)?;
                if !artifacts.contains(path) {
                    artifacts.push(path.clone());
                }
            }
            RunnerActionKind::SendKeys { keys } => {
                replay_send_keys(keys)?;
            }
        }
    }

    assert_true(
        resize_checks > 0,
        "replay trace did not contain any resize assertions",
        "replay-no-resize-checks",
    )
}

fn validate_left_button(button: Option<&str>) -> SuiteResult<()> {
    if button
        .map(|value| value.eq_ignore_ascii_case("left"))
        .unwrap_or(true)
    {
        Ok(())
    } else {
        Err(SuiteError::setup(format!(
            "logical replay only supports left-button drags, got '{}'",
            button.unwrap_or_default()
        )))
    }
}

fn replay_send_keys(keys: &[String]) -> SuiteResult<()> {
    match keys {
        [] => Ok(()),
        [text] => win32::send_text_enter(text).map_err(SuiteError::setup),
        _ => Err(SuiteError::setup(
            "logical replay only supports single text-entry send_keys actions",
        )),
    }
}

fn assert_rect_matches_action(
    actual: DesktopRect,
    expected: Rect,
    step_id: Option<&str>,
) -> SuiteResult<()> {
    let label = step_id.unwrap_or("replay-resize-window");
    assert_close(actual.left, expected.x, TOLERANCE, &format!("{label}-left"))?;
    assert_close(actual.top, expected.y, TOLERANCE, &format!("{label}-top"))?;
    assert_close(
        actual.width(),
        expected.width as i32,
        TOLERANCE,
        &format!("{label}-width"),
    )?;
    assert_close(
        actual.height(),
        expected.height as i32,
        TOLERANCE,
        &format!("{label}-height"),
    )
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

#[cfg(test)]
fn restore_drag_points(original: DesktopRect, after_inward: DesktopRect) -> (i32, i32) {
    let restore_x = 0.max(original.left + 4);
    let restore_from_x = after_inward.left + 4;
    (restore_from_x, restore_x)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(left: i32, right: i32) -> DesktopRect {
        DesktopRect {
            left,
            top: 0,
            right,
            bottom: 100,
        }
    }

    #[test]
    fn restore_drag_points_return_distinct_values_after_inward_resize() {
        assert_eq!(restore_drag_points(rect(0, 500), rect(220, 500)), (224, 4));
    }

    #[test]
    fn restore_drag_points_allow_noop_when_inward_resize_did_not_move() {
        assert_eq!(restore_drag_points(rect(0, 500), rect(0, 500)), (4, 4));
    }

    #[test]
    fn replay_rect_assertion_accepts_expected_bounds() {
        assert!(assert_rect_matches_action(
            DesktopRect {
                left: 2,
                top: 0,
                right: 102,
                bottom: 80,
            },
            Rect {
                x: 0,
                y: 0,
                width: 100,
                height: 80,
            },
            Some("resize")
        )
        .is_ok());
    }

    #[test]
    fn replay_rect_assertion_rejects_unexpected_bounds() {
        let err = assert_rect_matches_action(
            DesktopRect {
                left: 10,
                top: 0,
                right: 110,
                bottom: 80,
            },
            Rect {
                x: 0,
                y: 0,
                width: 100,
                height: 80,
            },
            Some("resize"),
        )
        .unwrap_err();

        assert_eq!(err.first_bad_signal.as_deref(), Some("resize-left"));
    }

    #[test]
    fn replay_rejects_non_left_button_drags() {
        let err = validate_left_button(Some("right")).unwrap_err();

        assert!(err.message.contains("left-button"));
    }
}
