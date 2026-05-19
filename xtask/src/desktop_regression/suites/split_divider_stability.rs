use std::fs;
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
    artifacts_with_common, assert_launched_process_snapshot, assert_renderer_surface_sane,
    assert_terminal_snapshot_sane, capture_step_snapshot, finalize_diagnostics, format_rect,
    mark_full_step, maybe_prompt_on_failure, record_diagnostic_error, start_diagnostics,
    ObservedDiagnostics,
};
use crate::desktop_regression::suites::{forced_failure_for_suite, SuiteContext};
use crate::desktop_regression::win32::{self, DesktopRect};
use serde_json::json;
use terminal_manager_diagnostics::{
    Rect, RunnerActionKind, RunnerActionTarget, TerminalManagerSnapshot,
};

const SUITE_ID: &str = "split-divider-stability";
const INITIAL_X: i32 = 110;
const INITIAL_Y: i32 = 88;
const TARGET_WINDOW_WIDTH: i32 = 2143;
const TARGET_WINDOW_HEIGHT: i32 = 878;
const SPLIT_WAIT_MS: u64 = 700;
const STABILIZE_MS: u64 = 140;
const SPLIT_DRAG_DELTA: i32 = 333;
const SAMPLE_COUNT: usize = 9;
const TITLEBAR_HEIGHT: i32 = 34;
const TABBAR_HEIGHT: i32 = 38;
const STATUSBAR_HEIGHT: i32 = 24;
const DEFAULT_SIDEBAR_WIDTH: i32 = 252;
const SIDEBAR_RESIZER_WIDTH: i32 = 6;
const DIVIDER_HIT_OFFSETS: [i32; 7] = [0, 6, -6, 12, -12, 18, -18];

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
        run_divider_scenario(context, artifacts, &session, diagnostics.as_ref())
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

fn run_divider_scenario(
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
            SUITE_ID,
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

    thread::sleep(Duration::from_millis(SPLIT_WAIT_MS));
    context
        .record_action(
            SUITE_ID,
            None,
            RunnerActionTarget::None,
            RunnerActionKind::Wait {
                mode: "fixed_sleep".to_owned(),
                reason: "after deterministic placement".to_owned(),
                timeout_ms: SPLIT_WAIT_MS,
            },
        )
        .map_err(SuiteError::setup)?;
    win32::set_window_rect(hwnd, INITIAL_X, INITIAL_Y, width, height).map_err(SuiteError::setup)?;
    thread::sleep(Duration::from_millis(STABILIZE_MS));
    win32::focus_window(hwnd).map_err(SuiteError::setup)?;
    let focus_x = INITIAL_X + width / 2;
    let focus_y = INITIAL_Y + height / 2;
    win32::mouse_click(focus_x, focus_y, Some("left")).map_err(SuiteError::setup)?;
    context
        .record_action(
            SUITE_ID,
            None,
            window_target(session),
            RunnerActionKind::Mouse {
                x: focus_x,
                y: focus_y,
                button: Some("left".to_owned()),
            },
        )
        .map_err(SuiteError::setup)?;

    let start_rect = win32::get_window_rect(hwnd).map_err(SuiteError::setup)?;
    let start_snapshot_path = screenshot_path(context, "start");
    mark_full_step(context, diagnostics, "start", "baseline")?;
    capture_step_snapshot(
        context,
        artifacts,
        SUITE_ID,
        diagnostics,
        "start-snapshot",
        "baseline",
    )?;
    capture_screen(&start_snapshot_path).map_err(SuiteError::setup)?;
    artifacts.push(suite_artifact_name(SUITE_ID, "start", "png"));
    context
        .record_action(
            SUITE_ID,
            Some("start"),
            RunnerActionTarget::Desktop,
            RunnerActionKind::Screenshot {
                path: suite_artifact_name(SUITE_ID, "start", "png"),
            },
        )
        .map_err(SuiteError::setup)?;

    context
        .record_action(
            SUITE_ID,
            Some("split"),
            RunnerActionTarget::None,
            RunnerActionKind::MarkStep {
                id: "split".to_owned(),
            },
        )
        .map_err(SuiteError::setup)?;
    win32::send_ctrl_d().map_err(SuiteError::setup)?;
    context
        .record_action(
            SUITE_ID,
            Some("split"),
            window_target(session),
            RunnerActionKind::SendKeys {
                keys: vec!["ctrl".to_owned(), "d".to_owned()],
            },
        )
        .map_err(SuiteError::setup)?;
    thread::sleep(Duration::from_millis(SPLIT_WAIT_MS));

    mark_full_step(context, diagnostics, "after-split", "after split")?;
    let after_split_snapshot = capture_step_snapshot(
        context,
        artifacts,
        SUITE_ID,
        diagnostics,
        "after-split-snapshot",
        "after split",
    )?;
    let after_split_path = screenshot_path(context, "after-split");
    capture_screen(&after_split_path).map_err(SuiteError::setup)?;
    artifacts.push(suite_artifact_name(SUITE_ID, "after-split", "png"));
    context
        .record_action(
            SUITE_ID,
            Some("split"),
            RunnerActionTarget::Desktop,
            RunnerActionKind::Screenshot {
                path: suite_artifact_name(SUITE_ID, "after-split", "png"),
            },
        )
        .map_err(SuiteError::setup)?;

    let split_snapshot = after_split_snapshot.as_ref().ok_or_else(|| {
        SuiteError::setup(
            "split-divider-stability requires diagnostic snapshots; omit --observe off".to_owned(),
        )
    })?;
    let split_dims = terminal_grid_dims(Some(split_snapshot))
        .ok_or_else(|| SuiteError::setup("after-split snapshot missing terminal grid"))?;
    assert_true(
        pane_count(split_snapshot) >= 2,
        "Ctrl+D did not leave the active tab with multiple panes",
        "split-pane-count",
    )?;

    let client = win32::get_client_rect(hwnd).map_err(SuiteError::setup)?;
    let divider_point = divider_drag_point(client, split_snapshot)?;

    context
        .record_action(
            SUITE_ID,
            Some("drag"),
            RunnerActionTarget::None,
            RunnerActionKind::MarkStep {
                id: "drag".to_owned(),
            },
        )
        .map_err(SuiteError::setup)?;
    mark_full_step(context, diagnostics, "drag", "drag divider")?;
    let (divider_x, observed_drag_dims, drag_attempt_dims) = drag_divider_until_changed(
        hwnd,
        client,
        divider_point,
        split_dims,
        context,
        artifacts,
        diagnostics,
    )?;

    let after_drag_path = screenshot_path(context, "after-drag");
    capture_screen(&after_drag_path).map_err(SuiteError::setup)?;
    artifacts.push(suite_artifact_name(SUITE_ID, "after-drag", "png"));
    context
        .record_action(
            SUITE_ID,
            Some("drag"),
            RunnerActionTarget::Desktop,
            RunnerActionKind::Screenshot {
                path: suite_artifact_name(SUITE_ID, "after-drag", "png"),
            },
        )
        .map_err(SuiteError::setup)?;

    let samples = collect_stable_grid_samples(context, diagnostics, SUITE_ID, "stability")?;
    assert_true(
        samples.len() >= SAMPLE_COUNT,
        "split-divider-stability did not collect enough post-drag samples",
        "divider-sample-count",
    )?;
    write_geometry_summary(context, artifacts, split_dims, &drag_attempt_dims, &samples)?;
    let sample_dims = stability_sample_dims(&samples);

    let final_dims = sample_dims
        .last()
        .copied()
        .ok_or_else(|| SuiteError::setup("missing sample for final dimensions"))?;
    assert_true(
        sample_dims.iter().any(|dims| *dims != split_dims),
        "dragging the divider did not visibly change terminal grid size",
        "divider-no-change",
    )?;
    assert_stable_after_drag(&sample_dims)?;

    println!(
        "splitter_start_rect={} divider_x={divider_x} divider_y={} split_dims={:?} observed_drag_dims={:?} final_dims={:?} samples={:?}",
        format_rect(start_rect),
        divider_point.y,
        split_dims,
        observed_drag_dims,
        final_dims,
        sample_dims
    );

    if let Some(snapshot) = after_split_snapshot.as_ref() {
        if let Some(diagnostics) = diagnostics {
            assert_launched_process_snapshot(snapshot, diagnostics, session.process_id())?;
            assert_terminal_snapshot_sane(snapshot)?;
            assert_renderer_surface_sane(snapshot)?;
        }
    }

    Ok(())
}

fn drag_divider_until_changed(
    hwnd: win32::WindowHandle,
    client: DesktopRect,
    divider_point: DividerDragPoint,
    split_dims: (u32, u32),
    context: &SuiteContext<'_>,
    artifacts: &mut Vec<String>,
    diagnostics: Option<&ObservedDiagnostics>,
) -> SuiteResult<(i32, (u32, u32), Vec<(u32, u32)>)> {
    let mut attempts = Vec::new();
    for (idx, offset) in DIVIDER_HIT_OFFSETS.iter().enumerate() {
        let from_x = clamp_between(
            divider_point.x + offset,
            client.left + 20,
            client.right - 20,
        );
        let to_x = drag_target_for(client, from_x);
        win32::left_edge_drag(hwnd, from_x, divider_point.y, to_x).map_err(SuiteError::setup)?;
        thread::sleep(Duration::from_millis(SPLIT_WAIT_MS));

        let snapshot = capture_step_snapshot(
            context,
            artifacts,
            SUITE_ID,
            diagnostics,
            &format!("drag-attempt-{idx}"),
            "divider drag attempt",
        )?
        .ok_or_else(|| SuiteError::setup("diagnostic snapshot unavailable after divider drag"))?;
        let dims = terminal_grid_dims(Some(&snapshot))
            .ok_or_else(|| SuiteError::setup("divider drag snapshot missing terminal grid"))?;
        attempts.push(dims);
        if dims != split_dims {
            return Ok((from_x, dims, attempts));
        }
    }

    Err(SuiteError::assertion(
        "dragging the divider did not visibly change terminal grid size".to_owned(),
        "divider-no-change",
    ))
}

fn write_geometry_summary(
    context: &SuiteContext<'_>,
    artifacts: &mut Vec<String>,
    before_drag: (u32, u32),
    after_drag_attempts: &[(u32, u32)],
    stability_samples: &[GridSample],
) -> SuiteResult<()> {
    let artifact = suite_artifact_name(SUITE_ID, "geometry-summary", "json");
    let path = context.artifact_layout.run_dir.join(&artifact);
    let stability_dims = stability_sample_dims(stability_samples);
    let summary = json!({
        "schema_version": "desktop-regression.split-divider-stability.geometry/v1",
        "before_drag": grid_dims_json(before_drag),
        "after_drag_attempts": grid_dims_list_json(after_drag_attempts),
        "stability_samples": grid_sample_list_json(stability_samples),
        "unique_states": grid_dims_list_json(&unique_grid_states(&stability_dims)),
        "transition_count": transition_count(&stability_dims),
    });
    let bytes = serde_json::to_vec_pretty(&summary)
        .map_err(|e| SuiteError::setup(format!("failed to serialize geometry summary: {e}")))?;
    fs::write(&path, bytes).map_err(|e| {
        SuiteError::setup(format!(
            "failed to write geometry summary {}: {e}",
            path.display()
        ))
    })?;
    artifacts.push(artifact);
    Ok(())
}

fn grid_dims_json(dims: (u32, u32)) -> serde_json::Value {
    json!({
        "cols": dims.0,
        "rows": dims.1,
    })
}

fn grid_dims_list_json(dims: &[(u32, u32)]) -> Vec<serde_json::Value> {
    dims.iter().copied().map(grid_dims_json).collect()
}

fn grid_sample_list_json(samples: &[GridSample]) -> Vec<serde_json::Value> {
    samples
        .iter()
        .enumerate()
        .map(|(index, sample)| {
            json!({
                "index": index,
                "cols": sample.dims.0,
                "rows": sample.dims.1,
                "captured_at_utc": sample.captured_at_utc.as_deref(),
                "elapsed_ms": sample.elapsed_ms,
            })
        })
        .collect()
}

fn stability_sample_dims(samples: &[GridSample]) -> Vec<(u32, u32)> {
    samples.iter().map(|sample| sample.dims).collect()
}

fn unique_grid_states(samples: &[(u32, u32)]) -> Vec<(u32, u32)> {
    let mut states = Vec::new();
    for sample in samples {
        if !states.contains(sample) {
            states.push(*sample);
        }
    }
    states
}

fn transition_count(samples: &[(u32, u32)]) -> usize {
    samples
        .windows(2)
        .filter(|window| window[0] != window[1])
        .count()
}

fn drag_target_for(client: DesktopRect, from_x: i32) -> i32 {
    if from_x + SPLIT_DRAG_DELTA < client.right {
        from_x + SPLIT_DRAG_DELTA
    } else {
        from_x - SPLIT_DRAG_DELTA
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DividerDragPoint {
    x: i32,
    y: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GridSample {
    dims: (u32, u32),
    captured_at_utc: Option<String>,
    elapsed_ms: u64,
}

fn divider_drag_point(
    client: DesktopRect,
    snapshot: &TerminalManagerSnapshot,
) -> SuiteResult<DividerDragPoint> {
    let scale_factor = snapshot.window.scale_factor.unwrap_or(1.0).max(0.1);
    let sidebar_width = snapshot_config_f64(snapshot, "sidebar_width")
        .filter(|width| *width >= 0.0)
        .unwrap_or(DEFAULT_SIDEBAR_WIDTH as f64);
    let content_x = client.left
        + scaled_px(sidebar_width, scale_factor)
        + scaled_px(SIDEBAR_RESIZER_WIDTH as f64, scale_factor);

    let grid_size = snapshot.renderer.surface_size.as_ref();
    let grid_width = grid_size
        .map(|size| size.width as i32)
        .filter(|width| *width > 0)
        .unwrap_or_else(|| (client.right - content_x).max(1));
    let grid_height = grid_size
        .map(|size| size.height as i32)
        .filter(|height| *height > 0)
        .unwrap_or_else(|| {
            (client.height()
                - scaled_px(TITLEBAR_HEIGHT as f64, scale_factor)
                - scaled_px(TABBAR_HEIGHT as f64, scale_factor)
                - scaled_px(STATUSBAR_HEIGHT as f64, scale_factor))
            .max(1)
        });

    let terminal_top = client.top
        + scaled_px(TITLEBAR_HEIGHT as f64, scale_factor)
        + scaled_px(TABBAR_HEIGHT as f64, scale_factor);
    let terminal_bottom = client.bottom - scaled_px(STATUSBAR_HEIGHT as f64, scale_factor);
    let x = clamp_between(
        content_x + grid_width / 2,
        client.left + 8,
        client.right - 8,
    );
    let y = clamp_between(
        terminal_top + grid_height / 2,
        terminal_top + 8,
        terminal_bottom - 8,
    );

    assert_true(
        x > client.left && x < client.right && y > client.top && y < client.bottom,
        &format!(
            "computed divider drag point outside client rect: point=({x},{y}) client={}",
            format_rect(client)
        ),
        "divider-point-out-of-bounds",
    )?;

    Ok(DividerDragPoint { x, y })
}

fn snapshot_config_f64(snapshot: &TerminalManagerSnapshot, key: &str) -> Option<f64> {
    snapshot
        .config
        .get(key)
        .and_then(|value| value.as_f64())
        .filter(|value| value.is_finite())
}

fn scaled_px(value: f64, scale_factor: f64) -> i32 {
    (value * scale_factor).round() as i32
}

fn pane_count(snapshot: &TerminalManagerSnapshot) -> usize {
    snapshot
        .layout
        .nodes
        .iter()
        .filter(|node| node.id.starts_with("pane:") && node.visible)
        .count()
}

fn clamp_between(value: i32, min: i32, max: i32) -> i32 {
    if min <= max {
        value.clamp(min, max)
    } else {
        value
    }
}

fn collect_stable_grid_samples(
    context: &SuiteContext<'_>,
    diagnostics: Option<&ObservedDiagnostics>,
    suite_id: &str,
    stem: &str,
) -> SuiteResult<Vec<GridSample>> {
    let mut samples = Vec::with_capacity(SAMPLE_COUNT);
    let started_at = Instant::now();
    for idx in 0..SAMPLE_COUNT {
        thread::sleep(Duration::from_millis(STABILIZE_MS));
        if let Some(snapshot) = capture_step_snapshot(
            context,
            &mut Vec::new(),
            suite_id,
            diagnostics,
            &format!("{stem}-{idx}"),
            "divider stability sample",
        )? {
            if let Some(dims) = terminal_grid_dims(Some(&snapshot)) {
                samples.push(GridSample {
                    dims,
                    captured_at_utc: snapshot_captured_at_utc(&snapshot),
                    elapsed_ms: started_at.elapsed().as_millis() as u64,
                });
            }
        }
    }
    Ok(samples)
}

fn snapshot_captured_at_utc(snapshot: &TerminalManagerSnapshot) -> Option<String> {
    serde_json::to_value(snapshot).ok().and_then(|value| {
        value
            .get("captured_at_utc")
            .and_then(|captured_at_utc| captured_at_utc.as_str().map(str::to_owned))
    })
}

fn assert_stable_after_drag(samples: &[(u32, u32)]) -> SuiteResult<()> {
    let transitions = samples
        .windows(2)
        .filter(|window| window[0] != window[1])
        .count();
    assert_true(
        transitions <= 2,
        &format!(
            "divider drag flickered across samples: transition_count={transitions} samples={samples:?}"
        ),
        "divider-flicker-transition",
    )?;

    let last = samples
        .last()
        .copied()
        .ok_or_else(|| SuiteError::setup("missing post-drag samples"))?;
    let tail_start = samples.len().saturating_sub(4);
    let stable_tail = samples[tail_start..].iter().all(|value| *value == last);
    assert_true(
        stable_tail,
        "divider drag should settle; terminal grid dimensions did not stabilize",
        "divider-not-stable",
    )
}

fn terminal_grid_dims(snapshot: Option<&TerminalManagerSnapshot>) -> Option<(u32, u32)> {
    snapshot
        .and_then(|snapshot| snapshot.terminal.grid.as_ref())
        .map(|grid| (grid.cols, grid.rows))
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

#[cfg(test)]
mod tests {
    use super::*;
    use terminal_manager_diagnostics::{LayoutNodeSnapshot, Size};

    #[test]
    fn assert_stable_after_drag_accepts_flatline() {
        assert!(
            assert_stable_after_drag(&[(80, 24), (80, 24), (80, 24), (80, 24)].as_slice()).is_ok()
        );
    }

    #[test]
    fn assert_stable_after_drag_rejects_flicker() {
        let err = assert_stable_after_drag(&[
            (47, 27),
            (108, 27),
            (47, 27),
            (108, 27),
            (47, 27),
            (108, 27),
        ])
        .unwrap_err();

        assert_eq!(
            err.first_bad_signal.as_deref(),
            Some("divider-flicker-transition")
        );
    }

    #[test]
    fn divider_drag_point_uses_sidebar_and_terminal_surface() {
        let mut snapshot = TerminalManagerSnapshot::default();
        snapshot.config = serde_json::json!({ "sidebar_width": 252.0 });
        snapshot.renderer.surface_size = Some(Size {
            width: 1000,
            height: 600,
        });

        let point = divider_drag_point(
            DesktopRect {
                left: 0,
                top: 0,
                right: 1300,
                bottom: 800,
            },
            &snapshot,
        )
        .unwrap();

        assert_eq!(point, DividerDragPoint { x: 758, y: 372 });
    }

    #[test]
    fn divider_drag_point_scales_logical_chrome_to_physical_pixels() {
        let mut snapshot = TerminalManagerSnapshot::default();
        snapshot.window.scale_factor = Some(1.5);
        snapshot.config = serde_json::json!({ "sidebar_width": 252.0 });
        snapshot.renderer.surface_size = Some(Size {
            width: 1943,
            height: 1128,
        });

        let point = divider_drag_point(
            DesktopRect {
                left: 0,
                top: 0,
                right: 2320,
                bottom: 1280,
            },
            &snapshot,
        )
        .unwrap();

        assert_eq!(point, DividerDragPoint { x: 1358, y: 672 });
    }

    #[test]
    fn pane_count_reads_visible_pane_nodes() {
        let mut snapshot = TerminalManagerSnapshot::default();
        snapshot.layout.nodes = vec![
            LayoutNodeSnapshot {
                id: "pane:1".to_owned(),
                visible: true,
                ..LayoutNodeSnapshot::default()
            },
            LayoutNodeSnapshot {
                id: "pane:2".to_owned(),
                visible: true,
                ..LayoutNodeSnapshot::default()
            },
            LayoutNodeSnapshot {
                id: "workspace:0".to_owned(),
                visible: true,
                ..LayoutNodeSnapshot::default()
            },
        ];

        assert_eq!(pane_count(&snapshot), 2);
    }
}
