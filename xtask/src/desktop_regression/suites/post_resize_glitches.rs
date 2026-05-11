use std::thread;
use std::time::Duration;

use crate::desktop_regression::artifacts::suite_artifact_name;
use crate::desktop_regression::assertions::{assert_true, SuiteError, SuiteResult};
use crate::desktop_regression::diagnostics::diagnostic_launch_for_mode;
use crate::desktop_regression::failure::collect_basic_failure_bundle;
use crate::desktop_regression::launcher::{AppLogFiles, AppSession};
use crate::desktop_regression::results::SuiteExecutionRecord;
use crate::desktop_regression::screenshots::{
    capture_screen, sample_png_lit_ratios, PixelSampleRatios, SampleRect,
};
use crate::desktop_regression::suites::observability::{
    artifacts_with_common, assert_launched_process_snapshot, assert_renderer_surface_sane,
    assert_terminal_snapshot_sane, capture_step_snapshot, finalize_diagnostics, format_rect,
    mark_full_step, record_diagnostic_error, start_diagnostics, ObservedDiagnostics,
};
use crate::desktop_regression::suites::SuiteContext;
use crate::desktop_regression::win32::{self, DesktopRect};
use terminal_manager_diagnostics::{Rect, TerminalManagerSnapshot};

const SUITE_ID: &str = "post-resize-glitches";
const SNAP_LIT_RATIO_THRESHOLD: f64 = 0.01;
const SNAP_MID_LIT_RATIO_THRESHOLD: f64 = 0.005;
const SNAP_TABBAR_PX: i32 = 88;
const SNAP_STATUSBAR_PX: i32 = 32;
const SNAP_SIDEBAR_PX: i32 = 252;
const SNAP_STRIPE_HEIGHT_PX: i32 = 12;

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

    let scenario_result = run_snap_scenario(context, artifacts, &session, diagnostics.as_ref());
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

fn run_snap_scenario(
    context: &SuiteContext<'_>,
    artifacts: &mut Vec<String>,
    session: &AppSession,
    diagnostics: Option<&ObservedDiagnostics>,
) -> SuiteResult<()> {
    let hwnd = session.window();
    let screen = win32::screen_size().map_err(SuiteError::setup)?;
    let pre_width = (screen.width as f64 / 2.5).round() as i32;
    let pre_height = (screen.height as f64 / 2.0).round() as i32;

    win32::set_window_rect(hwnd, 200, 200, pre_width, pre_height).map_err(SuiteError::setup)?;
    thread::sleep(Duration::from_millis(800));
    win32::focus_window(hwnd).map_err(SuiteError::setup)?;

    mark_full_step(
        context,
        diagnostics,
        "pre-clear",
        "Clear terminal before snap",
    )?;
    capture_step_snapshot(
        context,
        artifacts,
        SUITE_ID,
        diagnostics,
        "pre-clear-snapshot",
        "before clear",
    )?;
    win32::send_text_enter("clear").map_err(SuiteError::setup)?;
    thread::sleep(Duration::from_millis(500));
    win32::focus_window(hwnd).map_err(SuiteError::setup)?;

    let pre_rect = win32::get_window_rect(hwnd).map_err(SuiteError::setup)?;
    let pre_path = screenshot_path(context, "pre");
    let post_path = screenshot_path(context, "post");

    mark_full_step(context, diagnostics, "pre-snap", "Before Win+Left snap")?;
    let pre_snapshot = capture_step_snapshot(
        context,
        artifacts,
        SUITE_ID,
        diagnostics,
        "pre-snap-snapshot",
        "before snap",
    )?;
    capture_screen(&pre_path).map_err(SuiteError::setup)?;
    artifacts.push(suite_artifact_name(SUITE_ID, "pre", "png"));

    win32::focus_window(hwnd).map_err(SuiteError::setup)?;
    win32::send_win_left().map_err(SuiteError::setup)?;
    thread::sleep(Duration::from_millis(1500));

    let post_rect = win32::get_window_rect(hwnd).map_err(SuiteError::setup)?;
    let grew = post_rect.height() > pre_rect.height();
    let resize_signal = classify_snap_failure(grew, 1.0, 0.0, None);
    assert_true(
        grew,
        &format!(
            "Win+Left did not grow window height: pre={} post={}",
            pre_rect.height(),
            post_rect.height()
        ),
        &resize_signal,
    )?;

    mark_full_step(context, diagnostics, "post-snap", "After Win+Left snap")?;
    let post_snapshot = capture_step_snapshot(
        context,
        artifacts,
        SUITE_ID,
        diagnostics,
        "post-snap-snapshot",
        "after snap",
    )?;
    capture_screen(&post_path).map_err(SuiteError::setup)?;
    artifacts.push(suite_artifact_name(SUITE_ID, "post", "png"));

    println!("snap_pre_rect={}", format_rect(pre_rect));
    println!("snap_post_rect={}", format_rect(post_rect));

    let samples = snap_samples(post_rect);
    let ratios = sample_png_lit_ratios(
        &post_path,
        samples.bottom,
        samples.mid,
        SNAP_STRIPE_HEIGHT_PX,
        SNAP_STRIPE_HEIGHT_PX,
    )
    .map_err(SuiteError::setup)?;
    println!(
        "snap_bottom_lit_ratio={:.4} threshold={:.4} sample=({},{} {}x{})",
        ratios.bottom_lit_ratio,
        SNAP_LIT_RATIO_THRESHOLD,
        samples.bottom.x,
        samples.bottom.y,
        samples.bottom.width,
        samples.bottom.height
    );
    println!(
        "snap_mid_max_lit_ratio={:.4} threshold={:.4} sample=({},{} {}x{})",
        ratios.mid_max_lit_ratio,
        SNAP_MID_LIT_RATIO_THRESHOLD,
        samples.mid.x,
        samples.mid.y,
        samples.mid.width,
        samples.mid.height
    );
    println!("screenshots:{};{}", pre_path.display(), post_path.display());

    assert_visual_ratios(grew, ratios)?;
    if let (Some(diagnostics), Some(pre_snapshot), Some(post_snapshot)) =
        (diagnostics, pre_snapshot.as_ref(), post_snapshot.as_ref())
    {
        assert_snap_cross_layer(
            diagnostics,
            session.process_id(),
            pre_rect,
            post_rect,
            pre_snapshot,
            post_snapshot,
        )?;
    }

    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SnapSamples {
    bottom: SampleRect,
    mid: SampleRect,
}

fn snap_samples(post_rect: DesktopRect) -> SnapSamples {
    let bottom_y = (post_rect.bottom - SNAP_STRIPE_HEIGHT_PX - 4).max(post_rect.top);
    let pane_left = post_rect.left + SNAP_SIDEBAR_PX + 140;
    let pane_top = post_rect.top + SNAP_TABBAR_PX;
    let pane_bottom = post_rect.bottom - SNAP_STATUSBAR_PX;
    let pane_height = pane_bottom - pane_top;
    let mid_x = (post_rect.right - 1).min(pane_left);
    let mid_y = pane_top + (pane_height as f64 * 0.22) as i32;
    let mid_width = 480.min(post_rect.right - mid_x);
    let mid_height = SNAP_STRIPE_HEIGHT_PX.max((pane_height as f64 * 0.56) as i32);

    SnapSamples {
        bottom: SampleRect {
            x: post_rect.left,
            y: bottom_y,
            width: post_rect.width(),
            height: SNAP_STRIPE_HEIGHT_PX,
        },
        mid: SampleRect {
            x: mid_x,
            y: mid_y,
            width: mid_width,
            height: mid_height,
        },
    }
}

fn assert_visual_ratios(grew: bool, ratios: PixelSampleRatios) -> SuiteResult<()> {
    let bottom_ok = ratios.bottom_lit_ratio >= SNAP_LIT_RATIO_THRESHOLD;
    let bottom_signal = classify_snap_failure(
        grew,
        ratios.bottom_lit_ratio,
        ratios.mid_max_lit_ratio,
        None,
    );
    assert_true(
        bottom_ok,
        &format!(
            "snap-resize regression: bottom stripe lit ratio {:.4} < {:.4}; statusbar did not reflow to the new window bottom",
            ratios.bottom_lit_ratio, SNAP_LIT_RATIO_THRESHOLD
        ),
        &bottom_signal,
    )?;

    let mid_ok = ratios.mid_max_lit_ratio <= SNAP_MID_LIT_RATIO_THRESHOLD;
    let mid_signal = classify_snap_failure(
        grew,
        ratios.bottom_lit_ratio,
        ratios.mid_max_lit_ratio,
        None,
    );
    assert_true(
        mid_ok,
        &format!(
            "snap-resize regression: mid-pane lit ratio {:.4} > {:.4}; stale terminal rows appeared in the enlarged viewport",
            ratios.mid_max_lit_ratio, SNAP_MID_LIT_RATIO_THRESHOLD
        ),
        &mid_signal,
    )
}

fn assert_snap_cross_layer(
    diagnostics: &ObservedDiagnostics,
    process_id: u32,
    pre_rect: DesktopRect,
    post_rect: DesktopRect,
    pre_snapshot: &TerminalManagerSnapshot,
    post_snapshot: &TerminalManagerSnapshot,
) -> SuiteResult<()> {
    assert_launched_process_snapshot(post_snapshot, diagnostics, process_id)?;
    assert_terminal_snapshot_sane(post_snapshot)?;
    assert_renderer_surface_sane(post_snapshot)?;

    if let Some(signal) =
        cross_layer_failure_signal(pre_rect, post_rect, pre_snapshot, post_snapshot)
    {
        return Err(SuiteError::cross_layer(
            format!("post-snap diagnostic cross-layer assertion failed: {signal}"),
            signal,
        ));
    }
    Ok(())
}

fn cross_layer_failure_signal(
    pre_rect: DesktopRect,
    post_rect: DesktopRect,
    pre_snapshot: &TerminalManagerSnapshot,
    post_snapshot: &TerminalManagerSnapshot,
) -> Option<&'static str> {
    if let (Some(pre_outer), Some(post_outer)) = (
        pre_snapshot.window.outer_bounds.as_ref(),
        post_snapshot.window.outer_bounds.as_ref(),
    ) {
        if !rect_close_to_window(*post_outer, post_rect, 12) {
            return Some("snap-diagnostic-window-bounds-mismatch");
        }
        if post_outer.height <= pre_outer.height && post_rect.height() > pre_rect.height() {
            return Some("snap-diagnostic-window-height-stale");
        }
    }

    if let (Some(pre_surface), Some(post_surface)) = (
        pre_snapshot.renderer.surface_size.as_ref(),
        post_snapshot.renderer.surface_size.as_ref(),
    ) {
        if post_surface.width == 0 || post_surface.height == 0 {
            return Some("snap-diagnostic-render-surface-empty");
        }
        if post_surface.height < pre_surface.height && post_rect.height() > pre_rect.height() {
            return Some("snap-diagnostic-render-surface-shrank");
        }
    }

    if post_snapshot.layout.nodes.is_empty() {
        return Some("snap-diagnostic-layout-empty");
    }

    if let (Some(pre_grid), Some(post_grid)) = (
        pre_snapshot.terminal.grid.as_ref(),
        post_snapshot.terminal.grid.as_ref(),
    ) {
        if post_grid.rows == 0 || post_grid.cols == 0 {
            return Some("snap-diagnostic-terminal-grid-empty");
        }
        if post_grid.rows < pre_grid.rows && post_rect.height() > pre_rect.height() {
            return Some("snap-diagnostic-terminal-rows-shrank");
        }
    }

    None
}

fn rect_close_to_window(rect: Rect, window: DesktopRect, tolerance: i32) -> bool {
    (rect.x - window.left).abs() <= tolerance
        && (rect.y - window.top).abs() <= tolerance
        && (rect.width as i32 - window.width()).abs() <= tolerance
        && (rect.height as i32 - window.height()).abs() <= tolerance
}

fn classify_snap_failure(
    window_height_grew: bool,
    bottom_lit_ratio: f64,
    mid_max_lit_ratio: f64,
    diagnostic_signal: Option<&str>,
) -> String {
    if !window_height_grew {
        return "snap-window-height-not-grown".to_owned();
    }
    if let Some(signal) = diagnostic_signal {
        return signal.to_owned();
    }
    if bottom_lit_ratio < SNAP_LIT_RATIO_THRESHOLD {
        return "snap-bottom-stripe-missing".to_owned();
    }
    if mid_max_lit_ratio > SNAP_MID_LIT_RATIO_THRESHOLD {
        return "snap-mid-pane-stale-rows".to_owned();
    }
    "snap-diagnostic-unavailable".to_owned()
}

fn screenshot_path(context: &SuiteContext<'_>, name: &str) -> std::path::PathBuf {
    let file_name = suite_artifact_name(SUITE_ID, name, "png");
    context.artifact_layout.run_dir.join(file_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_snap_failure_prefers_resize_before_pixel_failures() {
        let classification = classify_snap_failure(false, 0.0, 1.0, None);

        assert_eq!(classification, "snap-window-height-not-grown");
    }

    #[test]
    fn classify_snap_failure_reports_pixel_only_visual_failures() {
        let classification = classify_snap_failure(true, 0.0, 0.0, None);

        assert_eq!(classification, "snap-bottom-stripe-missing");
    }

    #[test]
    fn snap_samples_match_legacy_geometry() {
        let rect = DesktopRect {
            left: 100,
            top: 50,
            right: 900,
            bottom: 650,
        };

        let samples = snap_samples(rect);

        assert_eq!(
            samples.bottom,
            SampleRect {
                x: 100,
                y: 634,
                width: 800,
                height: 12,
            }
        );
        assert_eq!(samples.mid.x, 492);
        assert_eq!(samples.mid.width, 408);
    }
}
