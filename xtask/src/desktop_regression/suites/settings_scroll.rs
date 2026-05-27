use std::collections::BTreeMap;
use std::thread;
use std::time::{Duration, Instant};

use crate::desktop_regression::artifacts::suite_artifact_name;
use crate::desktop_regression::assertions::{assert_true, SuiteError, SuiteResult};
use crate::desktop_regression::diagnostics::diagnostic_launch_for_mode;
use crate::desktop_regression::failure::collect_basic_failure_bundle;
use crate::desktop_regression::interactive::InteractiveDecision;
use crate::desktop_regression::launcher::{AppLogFiles, AppSession};
use crate::desktop_regression::results::SuiteExecutionRecord;
use crate::desktop_regression::screenshots::capture_screen;
use crate::desktop_regression::suites::observability::{
    artifacts_with_common, capture_step_snapshot, finalize_diagnostics, mark_full_step,
    maybe_prompt_on_failure, record_diagnostic_error, start_diagnostics, ObservedDiagnostics,
};
use crate::desktop_regression::suites::{forced_failure_for_suite, SuiteContext};
use crate::desktop_regression::win32::{self, DesktopRect};
use terminal_manager_diagnostics::{
    Rect, RunnerActionKind, RunnerActionTarget, TerminalManagerSnapshot,
};

pub const SMOOTHNESS_SUITE_ID: &str = "settings-scroll-smoothness";
pub const OPTIONS_SUITE_ID: &str = "settings-scroll-options";
pub const FPS_SUITE_ID: &str = "fps-overlay-scroll-updates";

const INITIAL_X: i32 = 120;
const INITIAL_Y: i32 = 90;
const TARGET_WINDOW_WIDTH: i32 = 1280;
const TARGET_WINDOW_HEIGHT: i32 = 800;
const SETTLE_MS: u64 = 650;
const WHEEL_DELTA_DOWN: i32 = -120;
const SINGLE_WHEEL_SAMPLE_MS: u64 = 260;
const WHEEL_BURST_TICKS: usize = 14;
const WHEEL_BURST_INTERVAL_MS: u64 = 15;
const POST_BURST_SAMPLE_MS: u64 = 120;
const MIN_SCROLL_LINE_PX: f64 = 95.0;
const MIN_SMOOTH_DURATION_MS: u64 = 170;
const MAX_SMOOTH_DURATION_MS: u64 = 190;
const MAX_SMOOTH_SAMPLE_GAP_MS: f64 = 18.0;
const MIN_SCROLL_FRAME_DELTA: u64 = 8;

pub fn run_smoothness(context: &SuiteContext<'_>) -> SuiteExecutionRecord {
    let mut artifacts = Vec::new();
    let mut interactive_decision = None;
    match run_smoothness_inner(context, &mut artifacts, &mut interactive_decision) {
        Ok(()) => SuiteExecutionRecord::passed(SMOOTHNESS_SUITE_ID, artifacts),
        Err(err) => {
            let failure = err.to_suite_failure();
            let added = collect_basic_failure_bundle(
                &context.artifact_layout.run_dir,
                &context.artifact_layout.run_id,
                SMOOTHNESS_SUITE_ID,
                &failure,
                &artifacts_with_common(context.common_artifacts, &artifacts),
            );
            artifacts.extend(added);
            let mut record = SuiteExecutionRecord::failed(
                SMOOTHNESS_SUITE_ID,
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

pub fn run_options(context: &SuiteContext<'_>) -> SuiteExecutionRecord {
    let mut artifacts = Vec::new();
    let mut interactive_decision = None;
    match run_options_inner(context, &mut artifacts, &mut interactive_decision) {
        Ok(()) => SuiteExecutionRecord::passed(OPTIONS_SUITE_ID, artifacts),
        Err(err) => {
            let failure = err.to_suite_failure();
            let added = collect_basic_failure_bundle(
                &context.artifact_layout.run_dir,
                &context.artifact_layout.run_id,
                OPTIONS_SUITE_ID,
                &failure,
                &artifacts_with_common(context.common_artifacts, &artifacts),
            );
            artifacts.extend(added);
            let mut record = SuiteExecutionRecord::failed(
                OPTIONS_SUITE_ID,
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

pub fn run_fps_overlay(context: &SuiteContext<'_>) -> SuiteExecutionRecord {
    let mut artifacts = Vec::new();
    let mut interactive_decision = None;
    match run_fps_overlay_inner(context, &mut artifacts, &mut interactive_decision) {
        Ok(()) => SuiteExecutionRecord::passed(FPS_SUITE_ID, artifacts),
        Err(err) => {
            let failure = err.to_suite_failure();
            let added = collect_basic_failure_bundle(
                &context.artifact_layout.run_dir,
                &context.artifact_layout.run_id,
                FPS_SUITE_ID,
                &failure,
                &artifacts_with_common(context.common_artifacts, &artifacts),
            );
            artifacts.extend(added);
            let mut record = SuiteExecutionRecord::failed(
                FPS_SUITE_ID,
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

fn run_fps_overlay_inner(
    context: &SuiteContext<'_>,
    artifacts: &mut Vec<String>,
    interactive_decision: &mut Option<InteractiveDecision>,
) -> SuiteResult<()> {
    let app_logs = AppLogFiles::create(&context.artifact_layout.run_dir, FPS_SUITE_ID)
        .map_err(|e| SuiteError::setup(format!("failed to create app log files: {e}")))?;
    artifacts.extend(app_logs.artifact_names());

    let diagnostic_launch = diagnostic_launch_for_mode(
        context.observe,
        &context.artifact_layout.run_id,
        FPS_SUITE_ID,
    );
    let mut env = BTreeMap::new();
    env.insert("TM_OPEN_SETTINGS", "1".to_owned());
    let mut session = AppSession::launch_with_logs_and_env(
        context.exe_path,
        context.workspace_root,
        Some(&app_logs),
        diagnostic_launch.as_ref(),
        &env,
    )
    .map_err(|e| SuiteError::setup(format!("failed to start app: {e}")))?;
    context
        .record_action(
            FPS_SUITE_ID,
            None,
            window_target(&session),
            RunnerActionKind::Note {
                message: "app.launch.settings".to_owned(),
            },
        )
        .map_err(SuiteError::setup)?;
    let diagnostics =
        start_diagnostics(context, artifacts, FPS_SUITE_ID, diagnostic_launch.as_ref())?;

    let scenario_result = if let Some(forced) = forced_failure_for_suite(FPS_SUITE_ID) {
        Err(forced)
    } else {
        run_fps_overlay_scenario(context, artifacts, &session, diagnostics.as_ref())
    };
    let diagnostics_result = finalize_diagnostics(
        context,
        artifacts,
        FPS_SUITE_ID,
        diagnostics.as_ref(),
        scenario_result.is_err(),
    );

    let result = match (scenario_result, diagnostics_result) {
        (Err(primary), Err(diagnostic_error)) => {
            record_diagnostic_error(context, artifacts, FPS_SUITE_ID, &diagnostic_error.message);
            Err(primary)
        }
        (Err(primary), Ok(())) => Err(primary),
        (Ok(()), Err(diagnostic_error)) => {
            record_diagnostic_error(context, artifacts, FPS_SUITE_ID, &diagnostic_error.message);
            Err(diagnostic_error)
        }
        (Ok(()), Ok(())) => Ok(()),
    };

    if result.is_err() {
        *interactive_decision = maybe_prompt_on_failure(
            context,
            artifacts,
            FPS_SUITE_ID,
            &mut session,
            diagnostics.as_ref(),
        );
    }

    result
}

fn run_fps_overlay_scenario(
    context: &SuiteContext<'_>,
    artifacts: &mut Vec<String>,
    session: &AppSession,
    diagnostics: Option<&ObservedDiagnostics>,
) -> SuiteResult<()> {
    let hwnd = session.window();
    let screen = win32::screen_size().map_err(SuiteError::setup)?;
    let width = TARGET_WINDOW_WIDTH.min(screen.width - INITIAL_X - 20);
    let height = TARGET_WINDOW_HEIGHT.min(screen.height - INITIAL_Y - 20);
    win32::set_window_rect(hwnd, INITIAL_X, INITIAL_Y, width, height).map_err(SuiteError::setup)?;
    context
        .record_action(
            FPS_SUITE_ID,
            None,
            window_target(session),
            RunnerActionKind::MoveWindow {
                bounds: Rect {
                    x: INITIAL_X,
                    y: INITIAL_Y,
                    width: width as u32,
                    height: height as u32,
                },
            },
        )
        .map_err(SuiteError::setup)?;
    thread::sleep(Duration::from_millis(SETTLE_MS));
    win32::focus_window(hwnd).map_err(SuiteError::setup)?;
    win32::send_ctrl_shift_f().map_err(SuiteError::setup)?;
    context
        .record_action(
            FPS_SUITE_ID,
            Some("toggle-fps-overlay"),
            RunnerActionTarget::Desktop,
            RunnerActionKind::SendKeys {
                keys: vec!["Ctrl+Shift+F".to_owned()],
            },
        )
        .map_err(SuiteError::setup)?;
    thread::sleep(Duration::from_millis(250));

    mark_full_step(context, diagnostics, "fps-baseline", "fps overlay baseline")?;
    let baseline = capture_step_snapshot(
        context,
        artifacts,
        FPS_SUITE_ID,
        diagnostics,
        "fps-baseline-snapshot",
        "fps overlay baseline",
    )?
    .ok_or_else(|| {
        SuiteError::setup(
            "fps-overlay-scroll-updates requires diagnostic snapshots; omit --observe off"
                .to_owned(),
        )
    })?;
    assert_fps_overlay_visible(&baseline)?;
    let start_recorded = fps_overlay_u64(&baseline, "/fps_overlay/recorded_generation")?;
    let start_rendered = fps_overlay_u64(&baseline, "/fps_overlay/rendered_generation")?;

    let client = win32::get_client_rect(hwnd).map_err(SuiteError::setup)?;
    let (wheel_x, wheel_y) = settings_scroll_point(client);
    for _ in 0..WHEEL_BURST_TICKS {
        win32::mouse_wheel(wheel_x, wheel_y, WHEEL_DELTA_DOWN).map_err(SuiteError::setup)?;
        thread::sleep(Duration::from_millis(WHEEL_BURST_INTERVAL_MS));
    }
    thread::sleep(Duration::from_millis(450));

    mark_full_step(
        context,
        diagnostics,
        "fps-after-scroll",
        "fps overlay after scroll",
    )?;
    let after = capture_step_snapshot(
        context,
        artifacts,
        FPS_SUITE_ID,
        diagnostics,
        "fps-after-scroll-snapshot",
        "fps overlay after scroll",
    )?
    .ok_or_else(|| SuiteError::setup("diagnostic snapshot unavailable after fps scroll"))?;
    assert_fps_overlay_visible(&after)?;
    let recorded = fps_overlay_u64(&after, "/fps_overlay/recorded_generation")?;
    let rendered = fps_overlay_u64(&after, "/fps_overlay/rendered_generation")?;
    assert_true(
        recorded > start_recorded,
        &format!("fps overlay samples did not advance during scroll: {start_recorded}->{recorded}"),
        "fps-overlay-samples-stale",
    )?;
    assert_true(
        rendered > start_rendered,
        &format!("fps overlay tree did not rebuild during scroll: {start_rendered}->{rendered}"),
        "fps-overlay-render-stale",
    )?;

    let screenshot = context.artifact_layout.run_dir.join(suite_artifact_name(
        FPS_SUITE_ID,
        "fps-after-scroll",
        "png",
    ));
    capture_screen(&screenshot).map_err(SuiteError::setup)?;
    artifacts.push(suite_artifact_name(FPS_SUITE_ID, "fps-after-scroll", "png"));

    Ok(())
}

fn run_options_inner(
    context: &SuiteContext<'_>,
    artifacts: &mut Vec<String>,
    interactive_decision: &mut Option<InteractiveDecision>,
) -> SuiteResult<()> {
    let app_logs = AppLogFiles::create(&context.artifact_layout.run_dir, OPTIONS_SUITE_ID)
        .map_err(|e| SuiteError::setup(format!("failed to create app log files: {e}")))?;
    artifacts.extend(app_logs.artifact_names());

    let diagnostic_launch = diagnostic_launch_for_mode(
        context.observe,
        &context.artifact_layout.run_id,
        OPTIONS_SUITE_ID,
    );
    let mut env = BTreeMap::new();
    env.insert("TM_OPEN_SETTINGS", "1".to_owned());
    let mut session = AppSession::launch_with_logs_and_env(
        context.exe_path,
        context.workspace_root,
        Some(&app_logs),
        diagnostic_launch.as_ref(),
        &env,
    )
    .map_err(|e| SuiteError::setup(format!("failed to start app: {e}")))?;
    context
        .record_action(
            OPTIONS_SUITE_ID,
            None,
            window_target(&session),
            RunnerActionKind::Note {
                message: "app.launch.settings".to_owned(),
            },
        )
        .map_err(SuiteError::setup)?;
    let diagnostics = start_diagnostics(
        context,
        artifacts,
        OPTIONS_SUITE_ID,
        diagnostic_launch.as_ref(),
    )?;

    let scenario_result = if let Some(forced) = forced_failure_for_suite(OPTIONS_SUITE_ID) {
        Err(forced)
    } else {
        run_options_scenario(context, artifacts, &session, diagnostics.as_ref())
    };
    let diagnostics_result = finalize_diagnostics(
        context,
        artifacts,
        OPTIONS_SUITE_ID,
        diagnostics.as_ref(),
        scenario_result.is_err(),
    );

    let result = match (scenario_result, diagnostics_result) {
        (Err(primary), Err(diagnostic_error)) => {
            record_diagnostic_error(
                context,
                artifacts,
                OPTIONS_SUITE_ID,
                &diagnostic_error.message,
            );
            Err(primary)
        }
        (Err(primary), Ok(())) => Err(primary),
        (Ok(()), Err(diagnostic_error)) => {
            record_diagnostic_error(
                context,
                artifacts,
                OPTIONS_SUITE_ID,
                &diagnostic_error.message,
            );
            Err(diagnostic_error)
        }
        (Ok(()), Ok(())) => Ok(()),
    };

    if result.is_err() {
        *interactive_decision = maybe_prompt_on_failure(
            context,
            artifacts,
            OPTIONS_SUITE_ID,
            &mut session,
            diagnostics.as_ref(),
        );
    }

    result
}

fn run_options_scenario(
    context: &SuiteContext<'_>,
    artifacts: &mut Vec<String>,
    session: &AppSession,
    diagnostics: Option<&ObservedDiagnostics>,
) -> SuiteResult<()> {
    let hwnd = session.window();
    let screen = win32::screen_size().map_err(SuiteError::setup)?;
    let width = TARGET_WINDOW_WIDTH.min(screen.width - INITIAL_X - 20);
    let height = TARGET_WINDOW_HEIGHT.min(screen.height - INITIAL_Y - 20);
    win32::set_window_rect(hwnd, INITIAL_X, INITIAL_Y, width, height).map_err(SuiteError::setup)?;
    context
        .record_action(
            OPTIONS_SUITE_ID,
            None,
            window_target(session),
            RunnerActionKind::MoveWindow {
                bounds: Rect {
                    x: INITIAL_X,
                    y: INITIAL_Y,
                    width: width as u32,
                    height: height as u32,
                },
            },
        )
        .map_err(SuiteError::setup)?;
    thread::sleep(Duration::from_millis(SETTLE_MS));
    win32::focus_window(hwnd).map_err(SuiteError::setup)?;

    mark_full_step(context, diagnostics, "options", "settings scroll options")?;
    let snapshot = capture_step_snapshot(
        context,
        artifacts,
        OPTIONS_SUITE_ID,
        diagnostics,
        "options-snapshot",
        "settings scroll options",
    )?
    .ok_or_else(|| {
        SuiteError::setup(
            "settings-scroll-options requires diagnostic snapshots; omit --observe off".to_owned(),
        )
    })?;
    assert_scroll_options_available(&snapshot)?;

    let screenshot = context.artifact_layout.run_dir.join(suite_artifact_name(
        OPTIONS_SUITE_ID,
        "options",
        "png",
    ));
    capture_screen(&screenshot).map_err(SuiteError::setup)?;
    artifacts.push(suite_artifact_name(OPTIONS_SUITE_ID, "options", "png"));

    Ok(())
}

fn run_smoothness_inner(
    context: &SuiteContext<'_>,
    artifacts: &mut Vec<String>,
    interactive_decision: &mut Option<InteractiveDecision>,
) -> SuiteResult<()> {
    let app_logs = AppLogFiles::create(&context.artifact_layout.run_dir, SMOOTHNESS_SUITE_ID)
        .map_err(|e| SuiteError::setup(format!("failed to create app log files: {e}")))?;
    artifacts.extend(app_logs.artifact_names());

    let diagnostic_launch = diagnostic_launch_for_mode(
        context.observe,
        &context.artifact_layout.run_id,
        SMOOTHNESS_SUITE_ID,
    );
    let mut env = BTreeMap::new();
    env.insert("TM_OPEN_SETTINGS", "1".to_owned());
    let mut session = AppSession::launch_with_logs_and_env(
        context.exe_path,
        context.workspace_root,
        Some(&app_logs),
        diagnostic_launch.as_ref(),
        &env,
    )
    .map_err(|e| SuiteError::setup(format!("failed to start app: {e}")))?;
    context
        .record_action(
            SMOOTHNESS_SUITE_ID,
            None,
            window_target(&session),
            RunnerActionKind::Note {
                message: "app.launch.settings".to_owned(),
            },
        )
        .map_err(SuiteError::setup)?;
    let diagnostics = start_diagnostics(
        context,
        artifacts,
        SMOOTHNESS_SUITE_ID,
        diagnostic_launch.as_ref(),
    )?;

    let scenario_result = if let Some(forced) = forced_failure_for_suite(SMOOTHNESS_SUITE_ID) {
        Err(forced)
    } else {
        run_smoothness_scenario(context, artifacts, &session, diagnostics.as_ref())
    };
    let diagnostics_result = finalize_diagnostics(
        context,
        artifacts,
        SMOOTHNESS_SUITE_ID,
        diagnostics.as_ref(),
        scenario_result.is_err(),
    );

    let result = match (scenario_result, diagnostics_result) {
        (Err(primary), Err(diagnostic_error)) => {
            record_diagnostic_error(
                context,
                artifacts,
                SMOOTHNESS_SUITE_ID,
                &diagnostic_error.message,
            );
            Err(primary)
        }
        (Err(primary), Ok(())) => Err(primary),
        (Ok(()), Err(diagnostic_error)) => {
            record_diagnostic_error(
                context,
                artifacts,
                SMOOTHNESS_SUITE_ID,
                &diagnostic_error.message,
            );
            Err(diagnostic_error)
        }
        (Ok(()), Ok(())) => Ok(()),
    };

    if result.is_err() {
        *interactive_decision = maybe_prompt_on_failure(
            context,
            artifacts,
            SMOOTHNESS_SUITE_ID,
            &mut session,
            diagnostics.as_ref(),
        );
    }

    result
}

fn run_smoothness_scenario(
    context: &SuiteContext<'_>,
    artifacts: &mut Vec<String>,
    session: &AppSession,
    diagnostics: Option<&ObservedDiagnostics>,
) -> SuiteResult<()> {
    let hwnd = session.window();
    let screen = win32::screen_size().map_err(SuiteError::setup)?;
    let width = TARGET_WINDOW_WIDTH.min(screen.width - INITIAL_X - 20);
    let height = TARGET_WINDOW_HEIGHT.min(screen.height - INITIAL_Y - 20);
    win32::set_window_rect(hwnd, INITIAL_X, INITIAL_Y, width, height).map_err(SuiteError::setup)?;
    context
        .record_action(
            SMOOTHNESS_SUITE_ID,
            None,
            window_target(session),
            RunnerActionKind::MoveWindow {
                bounds: Rect {
                    x: INITIAL_X,
                    y: INITIAL_Y,
                    width: width as u32,
                    height: height as u32,
                },
            },
        )
        .map_err(SuiteError::setup)?;
    thread::sleep(Duration::from_millis(SETTLE_MS));
    win32::focus_window(hwnd).map_err(SuiteError::setup)?;

    mark_full_step(context, diagnostics, "baseline", "settings scroll baseline")?;
    let baseline = capture_step_snapshot(
        context,
        artifacts,
        SMOOTHNESS_SUITE_ID,
        diagnostics,
        "baseline-snapshot",
        "settings scroll baseline",
    )?
    .ok_or_else(|| {
        SuiteError::setup(
            "settings-scroll-smoothness requires diagnostic snapshots; omit --observe off"
                .to_owned(),
        )
    })?;
    assert_scroll_defaults_are_responsive(&baseline)?;

    let client = win32::get_client_rect(hwnd).map_err(SuiteError::setup)?;
    let (wheel_x, wheel_y) = settings_scroll_point(client);

    context
        .record_action(
            SMOOTHNESS_SUITE_ID,
            Some("single-wheel"),
            RunnerActionTarget::Desktop,
            RunnerActionKind::Mouse {
                x: wheel_x,
                y: wheel_y,
                button: None,
            },
        )
        .map_err(SuiteError::setup)?;
    win32::mouse_wheel(wheel_x, wheel_y, WHEEL_DELTA_DOWN).map_err(SuiteError::setup)?;
    thread::sleep(Duration::from_millis(SINGLE_WHEEL_SAMPLE_MS));
    mark_full_step(
        context,
        diagnostics,
        "after-single-wheel",
        "after single scroll wheel tick",
    )?;
    let single_wheel = capture_step_snapshot(
        context,
        artifacts,
        SMOOTHNESS_SUITE_ID,
        diagnostics,
        "after-single-wheel-snapshot",
        "after single scroll wheel tick",
    )?
    .ok_or_else(|| SuiteError::setup("diagnostic snapshot unavailable after single wheel"))?;
    assert_browser_scroll_metrics(&single_wheel)?;

    let start_frame = renderer_frame_counter(&single_wheel);
    context
        .record_action(
            SMOOTHNESS_SUITE_ID,
            Some("wheel-burst"),
            RunnerActionTarget::Desktop,
            RunnerActionKind::Mouse {
                x: wheel_x,
                y: wheel_y,
                button: None,
            },
        )
        .map_err(SuiteError::setup)?;

    let burst_started = Instant::now();
    for _ in 0..WHEEL_BURST_TICKS {
        win32::mouse_wheel(wheel_x, wheel_y, WHEEL_DELTA_DOWN).map_err(SuiteError::setup)?;
        thread::sleep(Duration::from_millis(WHEEL_BURST_INTERVAL_MS));
    }
    thread::sleep(Duration::from_millis(POST_BURST_SAMPLE_MS));
    let elapsed = burst_started.elapsed();

    mark_full_step(
        context,
        diagnostics,
        "after-wheel-burst",
        "after scroll wheel burst",
    )?;
    let after = capture_step_snapshot(
        context,
        artifacts,
        SMOOTHNESS_SUITE_ID,
        diagnostics,
        "after-wheel-burst-snapshot",
        "after scroll wheel burst",
    )?
    .ok_or_else(|| SuiteError::setup("diagnostic snapshot unavailable after wheel burst"))?;
    let frame_delta = renderer_frame_counter(&after).saturating_sub(start_frame);
    let observed_fps = frame_delta as f64 / elapsed.as_secs_f64();
    write_smoothness_summary(context, artifacts, frame_delta, elapsed, observed_fps)?;
    assert_true(
        frame_delta >= MIN_SCROLL_FRAME_DELTA,
        &format!(
            "settings scroll animation produced only {frame_delta} frames over {:.0}ms ({observed_fps:.1}fps)",
            elapsed.as_secs_f64() * 1000.0
        ),
        "settings-scroll-frame-delta-low",
    )?;

    let screenshot = context.artifact_layout.run_dir.join(suite_artifact_name(
        SMOOTHNESS_SUITE_ID,
        "after-wheel-burst",
        "png",
    ));
    capture_screen(&screenshot).map_err(SuiteError::setup)?;
    artifacts.push(suite_artifact_name(
        SMOOTHNESS_SUITE_ID,
        "after-wheel-burst",
        "png",
    ));

    Ok(())
}

fn assert_scroll_defaults_are_responsive(snapshot: &TerminalManagerSnapshot) -> SuiteResult<()> {
    let line_px = snapshot
        .config
        .pointer("/scroll/line_px")
        .and_then(serde_json::Value::as_f64)
        .ok_or_else(|| {
            SuiteError::assertion(
                "diagnostic config is missing scroll.line_px".to_owned(),
                "scroll-line-px-missing",
            )
        })?;
    let duration_ms = snapshot
        .config
        .pointer("/scroll/smooth_duration_ms")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| {
            SuiteError::assertion(
                "diagnostic config is missing scroll.smooth_duration_ms".to_owned(),
                "scroll-duration-missing",
            )
        })?;
    assert_true(
        line_px >= MIN_SCROLL_LINE_PX,
        &format!("default wheel step {line_px:.1}px is too small for responsive scrolling"),
        "scroll-line-px-too-small",
    )?;
    assert_true(
        (MIN_SMOOTH_DURATION_MS..=MAX_SMOOTH_DURATION_MS).contains(&duration_ms),
        &format!(
            "default smooth scroll duration {duration_ms}ms is outside the smooth browser-like target"
        ),
        "scroll-duration-not-browser-like",
    )
}

fn assert_browser_scroll_metrics(snapshot: &TerminalManagerSnapshot) -> SuiteResult<()> {
    let samples = snapshot
        .config
        .pointer("/scroll/samples")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| {
            SuiteError::assertion(
                "diagnostic config is missing scroll.samples".to_owned(),
                "scroll-samples-missing",
            )
        })?;
    assert_true(
        samples.len() >= 6,
        &format!(
            "single wheel scroll produced only {} diagnostic samples",
            samples.len()
        ),
        "scroll-sample-count-low",
    )?;

    let first = samples.first().expect("sample count checked");
    let duration_ms = sample_f64(first, "duration_ms")?;
    assert_true(
        (MIN_SMOOTH_DURATION_MS as f64..=MAX_SMOOTH_DURATION_MS as f64).contains(&duration_ms),
        &format!(
            "single wheel duration {duration_ms:.1}ms is outside the smooth browser-like target"
        ),
        "scroll-sample-duration-off",
    )?;

    let scale = snapshot.window.scale_factor.unwrap_or(1.0);
    let line_px = snapshot
        .config
        .pointer("/scroll/line_px")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(MIN_SCROLL_LINE_PX);
    let start_y = sample_f64(first, "start_y")?;
    let target_y = sample_f64(first, "target_y")?;
    let distance_y = (target_y - start_y).abs();
    let expected_distance = line_px * scale;
    assert_true(
        (distance_y - expected_distance).abs() <= (8.0 * scale).max(8.0),
        &format!(
            "single wheel distance {distance_y:.1}px does not match browser step {expected_distance:.1}px"
        ),
        "scroll-sample-distance-off",
    )?;

    let max_gap = max_sample_gap_ms(samples)?;
    assert_true(
        max_gap <= MAX_SMOOTH_SAMPLE_GAP_MS,
        &format!(
            "single wheel scroll had a {max_gap:.1}ms diagnostic sample gap; browser-like scroll should keep animation frames flowing"
        ),
        "scroll-sample-gap-too-large",
    )?;

    let completed = samples.iter().any(|sample| {
        let phase = sample.get("phase").and_then(serde_json::Value::as_str);
        let elapsed = sample
            .get("elapsed_ms")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(0.0);
        let progress = sample
            .get("progress_y")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(0.0);
        phase == Some("completed")
            || (elapsed >= duration_ms - MAX_SMOOTH_SAMPLE_GAP_MS && progress >= 0.99)
    });
    assert_true(
        completed,
        "single wheel scroll did not publish a completed-or-settled diagnostic sample",
        "scroll-sample-completion-missing",
    )?;

    let quarter = progress_near_elapsed(samples, duration_ms * 0.25)?;
    let half = progress_near_elapsed(samples, duration_ms * 0.5)?;
    let three_quarter = progress_near_elapsed(samples, duration_ms * 0.75)?;
    assert_true(
        (0.12..=0.25).contains(&quarter),
        &format!("quarter-duration scroll progress {quarter:.3} is outside Edge baseline range"),
        "scroll-curve-quarter-off",
    )?;
    assert_true(
        (0.48..=0.62).contains(&half),
        &format!("half-duration scroll progress {half:.3} is outside Edge baseline range"),
        "scroll-curve-half-off",
    )?;
    assert_true(
        (0.80..=0.95).contains(&three_quarter),
        &format!(
            "three-quarter-duration scroll progress {three_quarter:.3} is outside Edge baseline range"
        ),
        "scroll-curve-three-quarter-off",
    )
}

fn max_sample_gap_ms(samples: &[serde_json::Value]) -> SuiteResult<f64> {
    let mut max_gap = 0.0_f64;
    let mut previous: Option<f64> = None;
    for sample in samples {
        let elapsed = sample_f64(sample, "elapsed_ms")?;
        if let Some(prev) = previous {
            max_gap = max_gap.max((elapsed - prev).abs());
        }
        previous = Some(elapsed);
    }
    Ok(max_gap)
}

fn sample_f64(sample: &serde_json::Value, key: &str) -> SuiteResult<f64> {
    sample
        .get(key)
        .and_then(serde_json::Value::as_f64)
        .ok_or_else(|| {
            SuiteError::assertion(
                format!("scroll diagnostic sample is missing numeric {key}"),
                "scroll-sample-field-missing",
            )
        })
}

fn progress_near_elapsed(samples: &[serde_json::Value], elapsed_ms: f64) -> SuiteResult<f64> {
    let mut previous: Option<(f64, f64)> = None;
    for sample in samples {
        let sample_elapsed = sample_f64(sample, "elapsed_ms")?;
        let progress = sample_f64(sample, "progress_y")?;
        if sample_elapsed >= elapsed_ms {
            if let Some((prev_elapsed, prev_progress)) = previous {
                let span = (sample_elapsed - prev_elapsed).max(0.001);
                let t = ((elapsed_ms - prev_elapsed) / span).clamp(0.0, 1.0);
                return Ok(prev_progress + (progress - prev_progress) * t);
            }
            return Ok(progress);
        }
        previous = Some((sample_elapsed, progress));
    }

    previous.map(|(_, progress)| progress).ok_or_else(|| {
        SuiteError::assertion(
            "scroll diagnostic samples are empty".to_owned(),
            "scroll-samples-empty",
        )
    })
}

fn assert_scroll_options_available(snapshot: &TerminalManagerSnapshot) -> SuiteResult<()> {
    let settings_open = snapshot
        .config
        .pointer("/settings_open")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    assert_true(
        settings_open,
        "settings page did not open for scroll options regression",
        "settings-not-open",
    )?;

    let section = snapshot
        .config
        .pointer("/settings_section")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    assert_true(
        section == "appearance",
        &format!("expected appearance settings section, got {section:?}"),
        "settings-section-not-appearance",
    )?;

    assert_scroll_defaults_are_responsive(snapshot)?;
    let scroll = snapshot.config.pointer("/scroll").ok_or_else(|| {
        SuiteError::assertion(
            "diagnostic config is missing scroll tuning object".to_owned(),
            "scroll-options-missing",
        )
    })?;
    assert_true(
        scroll.get("line_px").is_some() && scroll.get("smooth_duration_ms").is_some(),
        "scroll tuning settings are not exposed together in diagnostics",
        "scroll-options-incomplete",
    )
}

fn assert_fps_overlay_visible(snapshot: &TerminalManagerSnapshot) -> SuiteResult<()> {
    let visible = snapshot
        .config
        .pointer("/fps_overlay/visible")
        .and_then(serde_json::Value::as_bool)
        .ok_or_else(|| {
            SuiteError::assertion(
                "diagnostic config is missing fps_overlay.visible".to_owned(),
                "fps-overlay-visible-missing",
            )
        })?;
    assert_true(
        visible,
        "fps overlay was not visible after Ctrl+Shift+F",
        "fps-overlay-not-visible",
    )
}

fn fps_overlay_u64(snapshot: &TerminalManagerSnapshot, pointer: &str) -> SuiteResult<u64> {
    snapshot
        .config
        .pointer(pointer)
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| {
            SuiteError::assertion(
                format!("diagnostic config is missing {pointer}"),
                "fps-overlay-counter-missing",
            )
        })
}

fn write_smoothness_summary(
    context: &SuiteContext<'_>,
    artifacts: &mut Vec<String>,
    frame_delta: u64,
    elapsed: Duration,
    observed_fps: f64,
) -> SuiteResult<()> {
    let artifact = suite_artifact_name(SMOOTHNESS_SUITE_ID, "smoothness-summary", "json");
    let path = context.artifact_layout.run_dir.join(&artifact);
    let body = serde_json::json!({
        "schema_version": "desktop-regression.settings-scroll-smoothness/v1",
        "frame_delta": frame_delta,
        "elapsed_ms": elapsed.as_secs_f64() * 1000.0,
        "observed_fps": observed_fps,
        "minimum_frame_delta": MIN_SCROLL_FRAME_DELTA,
        "minimum_scroll_line_px": MIN_SCROLL_LINE_PX,
        "minimum_smooth_duration_ms": MIN_SMOOTH_DURATION_MS,
        "maximum_smooth_duration_ms": MAX_SMOOTH_DURATION_MS,
        "maximum_smooth_sample_gap_ms": MAX_SMOOTH_SAMPLE_GAP_MS,
    });
    std::fs::write(
        &path,
        serde_json::to_string_pretty(&body)
            .map_err(|e| SuiteError::setup(format!("summary serialization failed: {e}")))?,
    )
    .map_err(|e| SuiteError::setup(format!("failed to write {}: {e}", path.display())))?;
    artifacts.push(artifact);
    Ok(())
}

fn renderer_frame_counter(snapshot: &TerminalManagerSnapshot) -> u64 {
    snapshot.renderer.frame_counter.unwrap_or(0)
}

fn settings_scroll_point(client: DesktopRect) -> (i32, i32) {
    (
        client.left + (client.width() as f32 * 0.62) as i32,
        client.top + (client.height() as f32 * 0.52) as i32,
    )
}

fn window_target(session: &AppSession) -> RunnerActionTarget {
    RunnerActionTarget::Window {
        title: Some("Terminal Manager".to_owned()),
        process_id: Some(session.process_id()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_scroll_point_targets_content_area() {
        let rect = DesktopRect {
            left: 10,
            top: 20,
            right: 1290,
            bottom: 820,
        };
        let (x, y) = settings_scroll_point(rect);
        assert!(x > rect.left + rect.width() / 2);
        assert!(y > rect.top + rect.height() / 3);
        assert!(x < rect.right);
        assert!(y < rect.bottom);
    }

    #[test]
    fn missing_scroll_config_is_a_smoothness_failure() {
        let snapshot = TerminalManagerSnapshot::default();
        let err = assert_scroll_defaults_are_responsive(&snapshot).unwrap_err();
        assert!(err.message.contains("scroll.line_px"));
    }

    #[test]
    fn scroll_options_require_settings_to_be_open() {
        let mut snapshot = TerminalManagerSnapshot::default();
        snapshot.config = serde_json::json!({
            "settings_open": false,
            "settings_section": "appearance",
            "scroll": {
                "line_px": 56,
                "smooth_duration_ms": 80
            }
        });
        let err = assert_scroll_options_available(&snapshot).unwrap_err();
        assert!(err.message.contains("did not open"));
    }

    #[test]
    fn fps_overlay_visibility_requires_diagnostic_field() {
        let snapshot = TerminalManagerSnapshot::default();
        let err = assert_fps_overlay_visible(&snapshot).unwrap_err();
        assert!(err.message.contains("fps_overlay.visible"));
    }
}
