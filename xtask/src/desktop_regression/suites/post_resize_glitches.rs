use std::thread;
use std::time::Duration;

use crate::desktop_regression::artifacts::suite_artifact_name;
use crate::desktop_regression::assertions::{assert_true, SuiteError, SuiteResult};
use crate::desktop_regression::diagnostics::diagnostic_launch_for_mode;
use crate::desktop_regression::failure::collect_basic_failure_bundle;
use crate::desktop_regression::interactive::InteractiveDecision;
use crate::desktop_regression::launcher::{AppLogFiles, AppSession};
use crate::desktop_regression::results::SuiteExecutionRecord;
use crate::desktop_regression::screenshots::{
    capture_screen, sample_png_lit_ratios, PixelSampleRatios, SampleRect,
};
use crate::desktop_regression::suites::observability::{
    artifacts_with_common, assert_launched_process_snapshot, assert_renderer_surface_sane,
    assert_terminal_snapshot_sane, capture_step_snapshot, capture_step_snapshot_with_options,
    finalize_diagnostics, format_rect, mark_full_step, maybe_prompt_on_failure,
    record_diagnostic_error, start_diagnostics, ObservedDiagnostics,
};
use crate::desktop_regression::suites::{forced_failure_for_suite, SuiteContext};
use crate::desktop_regression::win32::{self, DesktopRect};
use terminal_manager_diagnostics::{
    Rect, RunnerActionKind, RunnerActionTarget, SnapshotOptions, TerminalBufferWindowSnapshot,
    TerminalManagerSnapshot,
};

const SUITE_ID: &str = "post-resize-glitches";
const SNAP_LIT_RATIO_THRESHOLD: f64 = 0.01;
const SNAP_MID_LIT_RATIO_THRESHOLD: f64 = 0.005;
// Conservative lower bound for "some terminal content is present" after clear.
// Keep this below the stale-row threshold and re-baseline when the terminal
// theme, foreground palette, antialiasing, or sample geometry changes.
const SNAP_MID_LIT_PRESENCE_THRESHOLD: f64 = 0.0005;
const SNAP_TABBAR_PX: i32 = 88;
const SNAP_STATUSBAR_PX: i32 = 32;
const SNAP_SIDEBAR_PX: i32 = 252;
const SNAP_STRIPE_HEIGHT_PX: i32 = 12;
const SNAP_CONTENT_SAMPLE_Y_OFFSET_PX: i32 = 34;
const SNAP_REFOCUS_TITLEBAR_Y_OFFSET_PX: i32 = 8;
const SNAP_REFOCUS_DELAY_MS: u64 = 250;
const SNAP_SETTLE_MS: u64 = 1500;
const SNAP_BUFFER_EXCERPT_CHARS: usize = 96;

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
        run_snap_scenario(context, artifacts, &session, diagnostics.as_ref())
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
    context
        .record_action(
            SUITE_ID,
            None,
            window_target(session),
            RunnerActionKind::MoveWindow {
                bounds: Rect {
                    x: 200,
                    y: 200,
                    width: pre_width as u32,
                    height: pre_height as u32,
                },
            },
        )
        .map_err(SuiteError::setup)?;
    thread::sleep(Duration::from_millis(800));
    context
        .record_action(
            SUITE_ID,
            None,
            RunnerActionTarget::None,
            RunnerActionKind::Wait {
                mode: "fixed_sleep".to_owned(),
                reason: "after initial window placement".to_owned(),
                timeout_ms: 800,
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
                x: 200 + pre_width / 2,
                y: 208,
                button: Some("left".to_owned()),
            },
        )
        .map_err(SuiteError::setup)?;

    mark_full_step(
        context,
        diagnostics,
        "pre-clear",
        "Clear terminal before snap",
    )?;
    context
        .record_action(
            SUITE_ID,
            Some("pre-clear"),
            RunnerActionTarget::None,
            RunnerActionKind::MarkStep {
                id: "pre-clear".to_owned(),
            },
        )
        .map_err(SuiteError::setup)?;
    capture_step_snapshot(
        context,
        artifacts,
        SUITE_ID,
        diagnostics,
        "pre-clear-snapshot",
        "before clear",
    )?;
    win32::send_text_enter("clear").map_err(SuiteError::setup)?;
    context
        .record_action(
            SUITE_ID,
            Some("pre-clear"),
            window_target(session),
            RunnerActionKind::SendKeys {
                keys: vec![
                    "c".to_owned(),
                    "l".to_owned(),
                    "e".to_owned(),
                    "a".to_owned(),
                    "r".to_owned(),
                    "enter".to_owned(),
                ],
            },
        )
        .map_err(SuiteError::setup)?;
    thread::sleep(Duration::from_millis(500));
    context
        .record_action(
            SUITE_ID,
            Some("pre-clear"),
            RunnerActionTarget::None,
            RunnerActionKind::Wait {
                mode: "fixed_sleep".to_owned(),
                reason: "after clear command".to_owned(),
                timeout_ms: 500,
            },
        )
        .map_err(SuiteError::setup)?;
    win32::focus_window(hwnd).map_err(SuiteError::setup)?;

    let pre_rect = win32::get_window_rect(hwnd).map_err(SuiteError::setup)?;
    let pre_path = screenshot_path(context, "pre");
    let post_path = screenshot_path(context, "post");

    mark_full_step(context, diagnostics, "pre-snap", "Before Win+Left snap")?;
    context
        .record_action(
            SUITE_ID,
            Some("pre-snap"),
            RunnerActionTarget::None,
            RunnerActionKind::MarkStep {
                id: "pre-snap".to_owned(),
            },
        )
        .map_err(SuiteError::setup)?;
    let pre_snapshot = capture_step_snapshot_with_options(
        context,
        artifacts,
        SUITE_ID,
        diagnostics,
        "pre-snap-snapshot",
        "before snap",
        SnapshotOptions {
            include_terminal_buffer: true,
        },
    )?;
    capture_screen(&pre_path).map_err(SuiteError::setup)?;
    artifacts.push(suite_artifact_name(SUITE_ID, "pre", "png"));
    context
        .record_action(
            SUITE_ID,
            Some("pre-snap"),
            RunnerActionTarget::Desktop,
            RunnerActionKind::Screenshot {
                path: suite_artifact_name(SUITE_ID, "pre", "png"),
            },
        )
        .map_err(SuiteError::setup)?;

    win32::focus_window(hwnd).map_err(SuiteError::setup)?;
    let mut post_rect =
        send_win_left_click_and_wait(context, session, hwnd, "after Win+Left snap", false)?;
    if !snap_height_grew(pre_rect, post_rect) {
        win32::focus_window(hwnd).map_err(SuiteError::setup)?;
        post_rect = send_win_left_click_and_wait(
            context,
            session,
            hwnd,
            "after retry Win+Left snap",
            true,
        )?;
    }

    let grew = snap_height_grew(pre_rect, post_rect);
    let resize_signal = classify_snap_failure(grew, 1.0, 1.0, 0.0, None);
    assert_true(
        grew,
        &format!(
            "Win+Left did not grow window height: pre={} post={}",
            pre_rect.height(),
            post_rect.height()
        ),
        &resize_signal,
    )?;

    assert_snap_capture_ready(hwnd, post_rect)?;

    mark_full_step(context, diagnostics, "post-snap", "After Win+Left snap")?;
    context
        .record_action(
            SUITE_ID,
            Some("post-snap"),
            RunnerActionTarget::None,
            RunnerActionKind::MarkStep {
                id: "post-snap".to_owned(),
            },
        )
        .map_err(SuiteError::setup)?;
    let post_snapshot = capture_step_snapshot_with_options(
        context,
        artifacts,
        SUITE_ID,
        diagnostics,
        "post-snap-snapshot",
        "after snap",
        SnapshotOptions {
            include_terminal_buffer: true,
        },
    )?;
    capture_screen(&post_path).map_err(SuiteError::setup)?;
    artifacts.push(suite_artifact_name(SUITE_ID, "post", "png"));
    context
        .record_action(
            SUITE_ID,
            Some("post-snap"),
            RunnerActionTarget::Desktop,
            RunnerActionKind::Screenshot {
                path: suite_artifact_name(SUITE_ID, "post", "png"),
            },
        )
        .map_err(SuiteError::setup)?;

    println!("snap_pre_rect={}", format_rect(pre_rect));
    println!("snap_post_rect={}", format_rect(post_rect));

    let samples = snap_samples(post_rect);
    let ratios = sample_png_lit_ratios(
        &post_path,
        samples.bottom,
        samples.content,
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
        "snap_content_lit_ratio={:.4} presence_threshold={:.4} sample=({},{} {}x{})",
        ratios.content_lit_ratio,
        SNAP_MID_LIT_PRESENCE_THRESHOLD,
        samples.content.x,
        samples.content.y,
        samples.content.width,
        samples.content.height
    );
    println!(
        "snap_mid_max_lit_ratio={:.4} presence_threshold={:.4} stale_threshold={:.4} sample=({},{} {}x{})",
        ratios.mid_max_lit_ratio,
        SNAP_MID_LIT_PRESENCE_THRESHOLD,
        SNAP_MID_LIT_RATIO_THRESHOLD,
        samples.mid.x,
        samples.mid.y,
        samples.mid.width,
        samples.mid.height
    );
    println!("screenshots:{};{}", pre_path.display(), post_path.display());

    let blank_mid_pane_diagnosis =
        diagnose_blank_mid_pane(pre_snapshot.as_ref(), post_snapshot.as_ref(), ratios);
    assert_visual_ratios(grew, ratios, blank_mid_pane_diagnosis.as_ref())?;
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
    content: SampleRect,
    mid: SampleRect,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BlankMidPaneDiagnosis {
    first_bad_signal: String,
    message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TerminalBufferEvidence {
    visible_rows: usize,
    visible_chars: usize,
    first_visible_row: Option<u32>,
    first_visible_excerpt: Option<String>,
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
        content: SampleRect {
            x: mid_x,
            y: pane_top + SNAP_CONTENT_SAMPLE_Y_OFFSET_PX,
            width: mid_width,
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

fn assert_visual_ratios(
    grew: bool,
    ratios: PixelSampleRatios,
    blank_mid_pane_diagnosis: Option<&BlankMidPaneDiagnosis>,
) -> SuiteResult<()> {
    let bottom_ok = ratios.bottom_lit_ratio >= SNAP_LIT_RATIO_THRESHOLD;
    let bottom_signal = classify_snap_failure(
        grew,
        ratios.bottom_lit_ratio,
        ratios.content_lit_ratio,
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

    let terminal_content_present = ratios.content_lit_ratio >= SNAP_MID_LIT_PRESENCE_THRESHOLD
        || ratios.mid_max_lit_ratio >= SNAP_MID_LIT_PRESENCE_THRESHOLD;
    let mid_present_signal = classify_snap_failure(
        grew,
        ratios.bottom_lit_ratio,
        ratios.content_lit_ratio,
        ratios.mid_max_lit_ratio,
        blank_mid_pane_diagnosis.map(|diagnosis| diagnosis.first_bad_signal.as_str()),
    );
    let mid_present_message = if let Some(diagnosis) = blank_mid_pane_diagnosis {
        format!(
            "snap-resize regression: content lit ratio {:.4} and mid-pane lit ratio {:.4} are below {:.4}; terminal pane appears blank after snap; {}",
            ratios.content_lit_ratio, ratios.mid_max_lit_ratio, SNAP_MID_LIT_PRESENCE_THRESHOLD, diagnosis.message
        )
    } else {
        format!(
            "snap-resize regression: content lit ratio {:.4} and mid-pane lit ratio {:.4} are below {:.4}; terminal pane appears blank after snap",
            ratios.content_lit_ratio, ratios.mid_max_lit_ratio, SNAP_MID_LIT_PRESENCE_THRESHOLD
        )
    };
    assert_true(
        terminal_content_present,
        &mid_present_message,
        &mid_present_signal,
    )?;

    let mid_ok = ratios.mid_max_lit_ratio <= SNAP_MID_LIT_RATIO_THRESHOLD;
    let mid_signal = classify_snap_failure(
        grew,
        ratios.bottom_lit_ratio,
        ratios.content_lit_ratio,
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

fn diagnose_blank_mid_pane(
    pre_snapshot: Option<&TerminalManagerSnapshot>,
    post_snapshot: Option<&TerminalManagerSnapshot>,
    ratios: PixelSampleRatios,
) -> Option<BlankMidPaneDiagnosis> {
    if ratios.content_lit_ratio >= SNAP_MID_LIT_PRESENCE_THRESHOLD
        || ratios.mid_max_lit_ratio >= SNAP_MID_LIT_PRESENCE_THRESHOLD
    {
        return None;
    }

    let pre_evidence = pre_snapshot.and_then(snapshot_buffer_evidence);
    let post_evidence = post_snapshot.and_then(snapshot_buffer_evidence);
    match (pre_evidence, post_evidence) {
        (_, Some(post)) if post.visible_chars > 0 => Some(BlankMidPaneDiagnosis {
            first_bad_signal: "snap-renderer-blank-with-buffer-content".to_owned(),
            message: format!(
                "terminal buffer still has visible content after snap ({}) so the buffer was not erased; renderer/layout did not paint it",
                format_post_buffer_evidence(&post)
            ),
        }),
        (Some(pre), Some(post)) if pre.visible_chars > 0 && post.visible_chars == 0 => {
            Some(BlankMidPaneDiagnosis {
                first_bad_signal: "snap-terminal-buffer-erased".to_owned(),
                message: format!(
                    "terminal buffer had visible content before snap ({}) but none after snap ({})",
                    format_pre_buffer_evidence(&pre),
                    format_post_buffer_evidence(&post)
                ),
            })
        }
        _ => None,
    }
}

fn snapshot_buffer_evidence(snapshot: &TerminalManagerSnapshot) -> Option<TerminalBufferEvidence> {
    snapshot
        .terminal
        .buffer_window
        .as_ref()
        .map(terminal_buffer_evidence)
}

fn terminal_buffer_evidence(buffer: &TerminalBufferWindowSnapshot) -> TerminalBufferEvidence {
    let mut visible_rows = 0;
    let mut visible_chars = 0;
    let mut first_visible_row = None;
    let mut first_visible_excerpt = None;

    for (row_offset, row) in buffer.rows.iter().enumerate() {
        let trimmed = row.trim();
        if trimmed.is_empty() {
            continue;
        }

        visible_rows += 1;
        visible_chars += trimmed.chars().count();
        if first_visible_row.is_none() {
            first_visible_row = Some(buffer.start_row.saturating_add(row_offset as u32));
            first_visible_excerpt = Some(truncated_excerpt(trimmed, SNAP_BUFFER_EXCERPT_CHARS));
        }
    }

    TerminalBufferEvidence {
        visible_rows,
        visible_chars,
        first_visible_row,
        first_visible_excerpt,
    }
}

fn format_pre_buffer_evidence(evidence: &TerminalBufferEvidence) -> String {
    format_buffer_evidence("pre", evidence)
}

fn format_post_buffer_evidence(evidence: &TerminalBufferEvidence) -> String {
    format_buffer_evidence("post", evidence)
}

fn format_buffer_evidence(label: &str, evidence: &TerminalBufferEvidence) -> String {
    let mut parts = vec![
        format!("{label}_buffer_visible_rows={}", evidence.visible_rows),
        format!("{label}_buffer_visible_chars={}", evidence.visible_chars),
    ];
    if let Some(row) = evidence.first_visible_row {
        parts.push(format!("{label}_first_visible_row={row}"));
    }
    if let Some(excerpt) = &evidence.first_visible_excerpt {
        parts.push(format!("{label}_first_visible_excerpt={excerpt:?}"));
    }
    parts.join(" ")
}

fn truncated_excerpt(value: &str, max_chars: usize) -> String {
    let mut excerpt = value.chars().take(max_chars).collect::<String>();
    if value.chars().count() > max_chars {
        excerpt.push_str("...");
    }
    excerpt
}

fn assert_snap_capture_ready(hwnd: win32::WindowHandle, post_rect: DesktopRect) -> SuiteResult<()> {
    win32::verify_snap_capture_ready(hwnd, post_rect)
        .map_err(suite_error_for_snap_capture_readiness)
}

fn wait_after_snap(context: &SuiteContext<'_>, reason: &str) -> SuiteResult<()> {
    thread::sleep(Duration::from_millis(SNAP_SETTLE_MS));
    context
        .record_action(
            SUITE_ID,
            Some("pre-snap"),
            RunnerActionTarget::None,
            RunnerActionKind::Wait {
                mode: "fixed_sleep".to_owned(),
                reason: reason.to_owned(),
                timeout_ms: SNAP_SETTLE_MS,
            },
        )
        .map_err(SuiteError::setup)
}

fn send_win_left_click_and_wait(
    context: &SuiteContext<'_>,
    session: &AppSession,
    hwnd: win32::WindowHandle,
    settle_reason: &str,
    retry: bool,
) -> SuiteResult<DesktopRect> {
    win32::send_win_left().map_err(SuiteError::setup)?;
    context
        .record_action(
            SUITE_ID,
            Some("pre-snap"),
            window_target(session),
            RunnerActionKind::SendKeys {
                keys: snap_keys_for_action_trace(retry),
            },
        )
        .map_err(SuiteError::setup)?;

    thread::sleep(Duration::from_millis(SNAP_REFOCUS_DELAY_MS));
    let refocus_rect = win32::get_window_rect(hwnd).map_err(SuiteError::setup)?;
    click_snapped_window_after_snap(context, session, refocus_rect)?;
    wait_after_snap(context, settle_reason)?;
    win32::get_window_rect(hwnd).map_err(SuiteError::setup)
}

fn snap_keys_for_action_trace(retry: bool) -> Vec<String> {
    let mut keys = vec!["win".to_owned(), "left".to_owned()];
    if retry {
        keys.push("retry-after-refocus".to_owned());
    }
    keys
}

fn snap_height_grew(pre_rect: DesktopRect, post_rect: DesktopRect) -> bool {
    post_rect.height() > pre_rect.height()
}

fn click_snapped_window_after_snap(
    context: &SuiteContext<'_>,
    session: &AppSession,
    post_rect: DesktopRect,
) -> SuiteResult<()> {
    let (x, y) = post_snap_refocus_click_point(post_rect);
    win32::mouse_click(x, y, Some("left")).map_err(SuiteError::setup)?;
    context
        .record_action(
            SUITE_ID,
            Some("pre-snap"),
            window_target(session),
            RunnerActionKind::Mouse {
                x,
                y,
                button: Some("left".to_owned()),
            },
        )
        .map_err(SuiteError::setup)
}

fn post_snap_refocus_click_point(post_rect: DesktopRect) -> (i32, i32) {
    (
        (post_rect.left + post_rect.right) / 2,
        post_rect.top + SNAP_REFOCUS_TITLEBAR_Y_OFFSET_PX,
    )
}

fn suite_error_for_snap_capture_readiness(err: win32::SnapCaptureReadinessError) -> SuiteError {
    SuiteError::assertion(
        format!("post-snap capture readiness failed: {}", err.message()),
        err.first_bad_signal(),
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
    content_lit_ratio: f64,
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
    if content_lit_ratio < SNAP_MID_LIT_PRESENCE_THRESHOLD
        && mid_max_lit_ratio < SNAP_MID_LIT_PRESENCE_THRESHOLD
    {
        return "snap-mid-pane-blank".to_owned();
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
    fn classify_snap_failure_prefers_resize_before_pixel_failures() {
        let classification = classify_snap_failure(false, 0.0, 0.0, 1.0, None);

        assert_eq!(classification, "snap-window-height-not-grown");
    }

    #[test]
    fn classify_snap_failure_reports_pixel_only_visual_failures() {
        let classification = classify_snap_failure(true, 0.0, 0.0, 0.0, None);

        assert_eq!(classification, "snap-bottom-stripe-missing");
    }

    #[test]
    fn classify_snap_failure_reports_blank_mid_pane_when_bottom_is_present() {
        let classification = classify_snap_failure(true, 0.02, 0.0, 0.0, None);

        assert_eq!(classification, "snap-mid-pane-blank");
    }

    #[test]
    fn classify_snap_failure_reports_stale_mid_pane_rows() {
        let classification =
            classify_snap_failure(true, 0.02, 0.02, SNAP_MID_LIT_RATIO_THRESHOLD + 0.001, None);

        assert_eq!(classification, "snap-mid-pane-stale-rows");
    }

    #[test]
    fn assert_visual_ratios_rejects_blank_mid_pane() {
        let err = assert_visual_ratios(
            true,
            PixelSampleRatios {
                bottom_lit_ratio: 0.02,
                content_lit_ratio: 0.0,
                mid_max_lit_ratio: 0.0,
            },
            None,
        )
        .unwrap_err();

        assert_eq!(err.first_bad_signal.as_deref(), Some("snap-mid-pane-blank"));
    }

    #[test]
    fn assert_visual_ratios_rejects_stale_mid_pane_rows() {
        let err = assert_visual_ratios(
            true,
            PixelSampleRatios {
                bottom_lit_ratio: 0.02,
                content_lit_ratio: 0.02,
                mid_max_lit_ratio: SNAP_MID_LIT_RATIO_THRESHOLD + 0.001,
            },
            None,
        )
        .unwrap_err();

        assert_eq!(
            err.first_bad_signal.as_deref(),
            Some("snap-mid-pane-stale-rows")
        );
    }

    #[test]
    fn assert_visual_ratios_keeps_bottom_stripe_precedence() {
        let err = assert_visual_ratios(
            true,
            PixelSampleRatios {
                bottom_lit_ratio: 0.0,
                content_lit_ratio: 0.0,
                mid_max_lit_ratio: 0.0,
            },
            None,
        )
        .unwrap_err();

        assert_eq!(
            err.first_bad_signal.as_deref(),
            Some("snap-bottom-stripe-missing")
        );
    }

    #[test]
    fn assert_visual_ratios_accepts_mid_pane_between_presence_and_stale_bounds() {
        assert!(assert_visual_ratios(
            true,
            PixelSampleRatios {
                bottom_lit_ratio: 0.02,
                content_lit_ratio: 0.0,
                mid_max_lit_ratio: 0.001,
            },
            None,
        )
        .is_ok());
    }

    #[test]
    fn assert_visual_ratios_accepts_top_content_with_blank_mid_pane() {
        assert!(assert_visual_ratios(
            true,
            PixelSampleRatios {
                bottom_lit_ratio: 0.02,
                content_lit_ratio: SNAP_MID_LIT_PRESENCE_THRESHOLD,
                mid_max_lit_ratio: 0.0,
            },
            None,
        )
        .is_ok());
    }

    #[test]
    fn blank_mid_pane_diagnosis_reports_renderer_blank_when_buffer_still_has_text() {
        let pre = snapshot_with_buffer_rows(&["prompt> before"]);
        let post = snapshot_with_buffer_rows(&["prompt> after"]);

        let diagnosis = diagnose_blank_mid_pane(
            Some(&pre),
            Some(&post),
            PixelSampleRatios {
                bottom_lit_ratio: 0.02,
                content_lit_ratio: 0.0,
                mid_max_lit_ratio: 0.0,
            },
        )
        .expect("diagnosis");

        assert_eq!(
            diagnosis.first_bad_signal,
            "snap-renderer-blank-with-buffer-content"
        );
        assert!(diagnosis.message.contains("post_buffer_visible_chars=13"));
    }

    #[test]
    fn blank_mid_pane_diagnosis_reports_terminal_buffer_erased() {
        let pre = snapshot_with_buffer_rows(&["prompt> before"]);
        let post = snapshot_with_buffer_rows(&["   "]);

        let diagnosis = diagnose_blank_mid_pane(
            Some(&pre),
            Some(&post),
            PixelSampleRatios {
                bottom_lit_ratio: 0.02,
                content_lit_ratio: 0.0,
                mid_max_lit_ratio: 0.0,
            },
        )
        .expect("diagnosis");

        assert_eq!(diagnosis.first_bad_signal, "snap-terminal-buffer-erased");
    }

    #[test]
    fn blank_mid_pane_diagnosis_skips_when_pixels_show_terminal_content() {
        let pre = snapshot_with_buffer_rows(&["prompt> before"]);
        let post = snapshot_with_buffer_rows(&["prompt> after"]);

        assert!(diagnose_blank_mid_pane(
            Some(&pre),
            Some(&post),
            PixelSampleRatios {
                bottom_lit_ratio: 0.02,
                content_lit_ratio: SNAP_MID_LIT_PRESENCE_THRESHOLD,
                mid_max_lit_ratio: 0.0,
            },
        )
        .is_none());
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
        assert_eq!(
            samples.content,
            SampleRect {
                x: 492,
                y: 172,
                width: 408,
                height: 12,
            }
        );
    }

    #[test]
    fn post_snap_refocus_click_targets_window_titlebar() {
        let rect = DesktopRect {
            left: 100,
            top: 50,
            right: 900,
            bottom: 650,
        };

        assert_eq!(post_snap_refocus_click_point(rect), (500, 58));
    }

    #[test]
    fn snap_height_grew_requires_post_snap_height_increase() {
        let pre_rect = DesktopRect {
            left: 200,
            top: 200,
            right: 1000,
            bottom: 920,
        };
        let unchanged_rect = DesktopRect {
            left: 0,
            top: 0,
            right: 1280,
            bottom: 720,
        };
        let grown_rect = DesktopRect {
            left: 0,
            top: 0,
            right: 1280,
            bottom: 1040,
        };

        assert!(!snap_height_grew(pre_rect, unchanged_rect));
        assert!(snap_height_grew(pre_rect, grown_rect));
    }

    #[test]
    fn retry_snap_action_trace_marks_retry_after_refocus() {
        assert_eq!(
            snap_keys_for_action_trace(false),
            vec!["win".to_owned(), "left".to_owned()]
        );
        assert_eq!(
            snap_keys_for_action_trace(true),
            vec![
                "win".to_owned(),
                "left".to_owned(),
                "retry-after-refocus".to_owned(),
            ]
        );
    }

    #[test]
    fn snap_capture_readiness_errors_map_to_first_bad_signals() {
        let foreground = suite_error_for_snap_capture_readiness(
            win32::SnapCaptureReadinessError::ForegroundStolen {
                foreground: Some(win32::WindowHandle(11)),
            },
        );
        let modifier = suite_error_for_snap_capture_readiness(
            win32::SnapCaptureReadinessError::StuckModifier { modifier: "win" },
        );
        let occlusion = suite_error_for_snap_capture_readiness(
            win32::SnapCaptureReadinessError::WindowOccluded {
                occluder: win32::WindowOcclusionCandidate {
                    handle: win32::WindowHandle(12),
                    rect: DesktopRect {
                        left: 0,
                        top: 0,
                        right: 10,
                        bottom: 10,
                    },
                    visible: true,
                    owned: false,
                },
            },
        );

        assert_eq!(
            foreground.first_bad_signal.as_deref(),
            Some("snap-foreground-stolen")
        );
        assert_eq!(
            modifier.first_bad_signal.as_deref(),
            Some("snap-stuck-modifier")
        );
        assert_eq!(
            occlusion.first_bad_signal.as_deref(),
            Some("snap-window-occluded")
        );
    }

    fn snapshot_with_buffer_rows(rows: &[&str]) -> TerminalManagerSnapshot {
        TerminalManagerSnapshot {
            terminal: terminal_manager_diagnostics::TerminalSnapshot {
                buffer_window: Some(TerminalBufferWindowSnapshot {
                    start_row: 0,
                    start_col: 0,
                    row_count: rows.len() as u32,
                    col_count: 80,
                    rows: rows.iter().map(|row| (*row).to_owned()).collect(),
                    truncated: false,
                }),
                ..Default::default()
            },
            ..Default::default()
        }
    }
}
