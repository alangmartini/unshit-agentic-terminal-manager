use std::thread;
use std::time::Duration;

use crate::desktop_regression::artifacts::suite_artifact_name;
use crate::desktop_regression::assertions::{assert_close, assert_true, SuiteError, SuiteResult};
use crate::desktop_regression::failure::collect_basic_failure_bundle;
use crate::desktop_regression::launcher::{AppLogFiles, AppSession};
use crate::desktop_regression::results::SuiteExecutionRecord;
use crate::desktop_regression::screenshots::capture_screen;
use crate::desktop_regression::suites::SuiteContext;
use crate::desktop_regression::win32::{self, DesktopRect};

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

    let session =
        AppSession::launch_with_logs(context.exe_path, context.workspace_root, Some(&app_logs))
            .map_err(|e| SuiteError::setup(format!("failed to start app: {e}")))?;
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

    let r0 = win32::get_window_rect(hwnd).map_err(SuiteError::setup)?;
    capture_screen(&start).map_err(SuiteError::setup)?;
    artifacts.push(suite_artifact_name(SUITE_ID, "start", "png"));
    println!("initial_rect={}", format_rect(r0));

    let center_y = ((r0.top + r0.bottom) as f64 / 2.0).round() as i32;
    let left_x = r0.left + 4;
    let drag_to_x = (r0.right - 20).min(left_x + DRAG_DELTA);

    win32::left_edge_drag(hwnd, left_x, center_y, drag_to_x).map_err(SuiteError::setup)?;
    let r1 = win32::get_window_rect(hwnd).map_err(SuiteError::setup)?;
    capture_screen(&after).map_err(SuiteError::setup)?;
    artifacts.push(suite_artifact_name(SUITE_ID, "after", "png"));

    let restore_x = 0.max(r0.left + 4);
    let restore_from_x = r1.left + 4;
    win32::left_edge_drag(hwnd, restore_from_x, center_y, restore_x).map_err(SuiteError::setup)?;
    let r2 = win32::get_window_rect(hwnd).map_err(SuiteError::setup)?;
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

    Ok(())
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
