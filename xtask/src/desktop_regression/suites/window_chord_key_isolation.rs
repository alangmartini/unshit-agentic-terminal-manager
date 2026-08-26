//! Window-manager chords must never reach the terminal.
//!
//! Two ways a key the user never typed can land in the PTY, both covered here:
//!
//! 1. A chord the window manager owns (`Alt+Space`, `Win+D`, `Alt+Tab`) is
//!    delivered to the app instead of being swallowed by the shell, and the
//!    encoder turns the naked key into bytes.
//! 2. The window loses and regains focus while a key is physically held. On
//!    Windows, winit reports the held key as a *synthetic* press when focus
//!    returns, with the modifier flags already cleared; routing that as real
//!    input types the tail of the chord into whatever the shell is running.
//!
//! Neither is visible to a unit test: the second one bypasses the key encoder
//! entirely, so only a real window losing real focus reproduces it. The suite
//! leaves an unterminated command on the prompt and proves nothing gets
//! appended to it -- an agent CLI reading a leaked byte can submit a
//! half-written prompt, which is the damage this protects against.
//!
//! The focus round trip runs first because it is the deterministic half; the
//! chords depend on what the window manager decides to swallow, and firing
//! them can leave the desktop in a state the next leg would have to clean up.

use std::thread;
use std::time::Duration;

use crate::desktop_regression::artifacts::suite_artifact_name;
use crate::desktop_regression::assertions::{assert_true, SuiteError, SuiteResult};
use crate::desktop_regression::diagnostics::{diagnostic_launch_for_mode, write_json_artifact};
use crate::desktop_regression::failure::collect_basic_failure_bundle;
use crate::desktop_regression::interactive::InteractiveDecision;
use crate::desktop_regression::launcher::{AppLogFiles, AppSession};
use crate::desktop_regression::results::SuiteExecutionRecord;
use crate::desktop_regression::screenshots::capture_screen;
use crate::desktop_regression::suites::observability::{
    artifacts_with_common, assert_launched_process_snapshot, capture_step_snapshot_with_options,
    finalize_diagnostics, mark_full_step, maybe_prompt_on_failure, record_diagnostic_error,
    start_diagnostics, ObservedDiagnostics,
};
use crate::desktop_regression::suites::{forced_failure_for_suite, SuiteContext};
use crate::desktop_regression::win32::{self, DesktopRect, DesktopSize, WindowManagerChord};
use terminal_manager_diagnostics::{
    Rect, RunnerActionKind, RunnerActionTarget, SnapshotOptions, TerminalManagerSnapshot,
};

const SUITE_ID: &str = "window-chord-key-isolation";

/// Left on the prompt without a trailing Enter. Letters and hyphens only: on a
/// US-International layout `'`, `"`, `~`, `^` and `` ` `` are dead keys and
/// would not type as themselves. Not a real command, so even a leaked Enter
/// only produces a harmless "not recognized" line.
const SENTINEL: &str = "tm-chord-leak-sentinel";
/// Held down across the focus round trip in the first leg.
const HELD_LETTER: char = 'd';

const SETTLE_MS: u64 = 700;
const CHORD_SETTLE_MS: u64 = 1200;
const SHELL_READY_MS: u64 = 6000;
const TITLEBAR_HEIGHT: i32 = 34;
const TABBAR_HEIGHT: i32 = 38;
const STATUSBAR_HEIGHT: i32 = 24;
const DEFAULT_SIDEBAR_WIDTH: i32 = 252;
const SIDEBAR_RESIZER_WIDTH: i32 = 6;

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
        run_chord_isolation_scenario(context, artifacts, &session, diagnostics.as_ref())
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

fn run_chord_isolation_scenario(
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
    record_wait(
        context,
        None,
        "after initial window placement",
        SHELL_READY_MS,
    )?;

    // A modifier left down by whatever ran before this suite -- an earlier
    // chord suite, or a run of this one that failed mid-flight -- would turn
    // the sentinel below into a stream of `Win+<letter>` chords instead of
    // text, so start from a known-clean keyboard.
    clear_held_modifiers(context, None)?;
    focus_terminal_pane(context, session, None)?;

    mark_full_step(
        context,
        diagnostics,
        "sentinel",
        "Leave an unterminated command on the prompt",
    )?;
    win32::send_physical_text_to_window(hwnd, SENTINEL).map_err(SuiteError::setup)?;
    context
        .record_action(
            SUITE_ID,
            Some("sentinel"),
            window_target(session),
            RunnerActionKind::SendKeys {
                keys: SENTINEL.chars().map(|ch| ch.to_string()).collect(),
            },
        )
        .map_err(SuiteError::setup)?;
    record_wait(
        context,
        Some("sentinel"),
        "after typing sentinel",
        SETTLE_MS,
    )?;

    let sentinel_snapshot = require_snapshot(
        context,
        artifacts,
        diagnostics,
        "sentinel-snapshot",
        "sentinel typed",
    )?;
    if let Some(diagnostics) = diagnostics {
        assert_launched_process_snapshot(&sentinel_snapshot, diagnostics, session.process_id())?;
    }
    let sentinel_state = TerminalState::capture(&sentinel_snapshot, SENTINEL)?;
    capture_named_screenshot(context, artifacts, "sentinel")?;

    // --- Leg 1: focus lost and regained with a key held -------------------
    //
    // The deterministic half, and the one that runs first: the window manager
    // may swallow the chords below before the app ever sees them, but
    // minimizing the window with a key down always produces the focus-loss /
    // focus-gain pair that triggers winit's synthetic key replay.
    mark_full_step(
        context,
        diagnostics,
        "focus-replay",
        "Round-trip focus with a key physically held",
    )?;
    hold_key_across_focus_round_trip(context, session)?;

    let replay_snapshot = require_snapshot(
        context,
        artifacts,
        diagnostics,
        "after-focus-replay-snapshot",
        "after focus round trip with a held key",
    )?;
    let replay_state = TerminalState::capture(&replay_snapshot, SENTINEL)?;
    capture_named_screenshot(context, artifacts, "after-focus-replay")?;
    let replay_events = new_pty_events(
        &sentinel_snapshot.pty.recent_events,
        &replay_snapshot.pty.recent_events,
    );
    assert_typed_letters(
        &sentinel_state,
        &replay_state,
        &replay_events,
        &HELD_LETTER.to_string(),
        "a key held across a focus change was replayed into the terminal",
        "focus-replay-keystroke-leak",
    )?;

    // --- Leg 2: chords the window manager owns ---------------------------
    mark_full_step(
        context,
        diagnostics,
        "chords",
        "Fire window-manager chords at the focused pane",
    )?;
    for chord in [
        WindowManagerChord::AltSpace,
        WindowManagerChord::WinD,
        WindowManagerChord::AltTab,
    ] {
        fire_chord(context, session, chord)?;
    }

    let chord_snapshot = require_snapshot(
        context,
        artifacts,
        diagnostics,
        "after-chords-snapshot",
        "after window-manager chords",
    )?;
    let chord_state = TerminalState::capture(&chord_snapshot, SENTINEL)?;
    capture_named_screenshot(context, artifacts, "after-chords")?;
    let chord_events = new_pty_events(
        &replay_snapshot.pty.recent_events,
        &chord_snapshot.pty.recent_events,
    );
    assert_typed_letters(
        &replay_state,
        &chord_state,
        &chord_events,
        "",
        "window-manager chords typed into the terminal",
        "chord-keystroke-leak",
    )?;

    let summary = summary_json(
        &sentinel_state,
        &chord_state,
        &replay_state,
        &chord_events,
        &replay_events,
    );
    let summary_artifact = write_json_artifact(
        &context.artifact_layout.run_dir,
        SUITE_ID,
        "chord-isolation-summary",
        &summary,
    )
    .map_err(|e| SuiteError::setup(format!("failed to write chord isolation summary: {e}")))?;
    artifacts.push(summary_artifact);

    // The suppression events prove the chord actually reached the app rather
    // than being swallowed by the shell, so their absence is information, not
    // a failure: on a healthy desktop Windows eats `Alt+Tab` before the app
    // sees it. Print either way so a run is self-explaining.
    let suppressed = suppressed_chord_events(&chord_events);
    println!(
        "chord_isolation_suppressed_events={} ({})",
        suppressed.len(),
        if suppressed.is_empty() {
            "chords were swallowed by the window manager before reaching the app".to_owned()
        } else {
            suppressed.join(" | ")
        }
    );
    println!("chord_isolation_sentinel_row={:?}", sentinel_state.row);
    println!(
        "chord_isolation_after_focus_replay_row={:?}",
        replay_state.row
    );
    println!("chord_isolation_after_chords_row={:?}", chord_state.row);

    Ok(())
}

/// The state a leaked keystroke would disturb, read straight from the app.
#[derive(Debug, Clone, PartialEq, Eq)]
struct TerminalState {
    row: String,
    cursor: Option<(u32, u32)>,
    scrollback_len: Option<u64>,
}

impl TerminalState {
    fn capture(snapshot: &TerminalManagerSnapshot, sentinel: &str) -> SuiteResult<Self> {
        let buffer = snapshot.terminal.buffer_window.as_ref().ok_or_else(|| {
            SuiteError::protocol(
                "diagnostic snapshot carried no terminal buffer window".to_owned(),
                "terminal-buffer-window-missing",
            )
        })?;
        let row = sentinel_row(&buffer.rows, sentinel).ok_or_else(|| {
            SuiteError::setup(format!(
                "sentinel {sentinel:?} is not on the screen; the pane did not have keyboard                  focus, or the shell was still starting when it was typed"
            ))
        })?;

        Ok(Self {
            row,
            cursor: snapshot
                .terminal
                .cursor
                .as_ref()
                .map(|cursor| (cursor.row, cursor.col)),
            scrollback_len: snapshot.terminal.scrollback_len,
        })
    }
}

/// Assert the terminal advanced by exactly `expected_typed` and nothing more.
///
/// `expected_typed` is empty for keys nobody typed and one character for the
/// leg that deliberately presses a key, so an extra replayed press shows up as
/// both a longer row and a second PTY write.
fn assert_typed_letters(
    before: &TerminalState,
    after: &TerminalState,
    new_events: &[String],
    expected_typed: &str,
    message: &str,
    first_bad_signal: &str,
) -> SuiteResult<()> {
    let expected_row = format!("{}{expected_typed}", before.row);
    assert_true(
        after.row == expected_row,
        &format!(
            "{message}: prompt line was {:?}, expected {:?}",
            after.row, expected_row
        ),
        first_bad_signal,
    )?;

    let expected_cursor = before
        .cursor
        .map(|(row, col)| (row, col + expected_typed.chars().count() as u32));
    assert_true(
        after.cursor == expected_cursor,
        &format!(
            "{message}: cursor moved to {:?}, expected {:?}",
            after.cursor, expected_cursor
        ),
        &format!("{first_bad_signal}-cursor"),
    )?;

    assert_true(
        after.scrollback_len == before.scrollback_len,
        &format!(
            "{message}: scrollback grew from {:?} to {:?}, so a leaked Enter ran the pending command",
            before.scrollback_len, after.scrollback_len
        ),
        &format!("{first_bad_signal}-scrollback"),
    )?;

    let writes = keyboard_write_count(new_events);
    let expected_writes = expected_typed.chars().count();
    assert_true(
        writes == expected_writes,
        &format!(
            "{message}: {writes} keyboard write(s) reached the PTY, expected {expected_writes} ({})",
            new_events.join(" | ")
        ),
        &format!("{first_bad_signal}-pty-write"),
    )
}

/// Events appended to the PTY diagnostic ring since `before` was captured.
///
/// The ring is bounded, so `after` is `before` with some prefix evicted plus
/// whatever arrived in between. Find the smallest eviction count that makes
/// the two overlap and return the tail past it; with no overlap at all the
/// whole of `after` is new, which over-reports rather than hiding a leak.
fn new_pty_events(before: &[String], after: &[String]) -> Vec<String> {
    for evicted in 0..=before.len() {
        let retained = before.len() - evicted;
        if retained > after.len() {
            continue;
        }
        if before[evicted..] == after[..retained] {
            return after[retained..].to_vec();
        }
    }

    after.to_vec()
}

fn keyboard_write_count(events: &[String]) -> usize {
    events
        .iter()
        .filter(|event| event.starts_with("write ") && event.contains("source=keyboard"))
        .count()
}

fn suppressed_chord_events(events: &[String]) -> Vec<String> {
    events
        .iter()
        .filter(|event| event.starts_with("key_suppressed "))
        .cloned()
        .collect()
}

fn sentinel_row(rows: &[String], sentinel: &str) -> Option<String> {
    rows.iter()
        .rev()
        .find(|row| row.contains(sentinel))
        .map(|row| row.trim_end().to_owned())
}

fn summary_json(
    sentinel: &TerminalState,
    after_chords: &TerminalState,
    after_replay: &TerminalState,
    chord_events: &[String],
    replay_events: &[String],
) -> serde_json::Value {
    serde_json::json!({
        "sentinel": SENTINEL,
        "held_letter": HELD_LETTER.to_string(),
        "rows": {
            "sentinel": sentinel.row,
            "after_chords": after_chords.row,
            "after_focus_replay": after_replay.row,
        },
        "chords": {
            "fired": [
                WindowManagerChord::AltSpace.label(),
                WindowManagerChord::WinD.label(),
                WindowManagerChord::AltTab.label(),
            ],
            "new_pty_events": chord_events,
            "suppressed_events": suppressed_chord_events(chord_events),
            "keyboard_writes": keyboard_write_count(chord_events),
        },
        "focus_replay": {
            "new_pty_events": replay_events,
            "keyboard_writes": keyboard_write_count(replay_events),
        },
    })
}

/// Fire one chord and put focus back on the pane afterwards.
///
/// `Win+D` and `Alt+Tab` are toggles: the second press is what brings the
/// window back, and the explicit re-focus covers the desktop where the first
/// press moved focus somewhere the second press did not undo.
fn fire_chord(
    context: &SuiteContext<'_>,
    session: &AppSession,
    chord: WindowManagerChord,
) -> SuiteResult<()> {
    let step_id = "chords";
    let repeats = match chord {
        WindowManagerChord::AltSpace => 1,
        WindowManagerChord::WinD | WindowManagerChord::AltTab => 2,
    };

    for _ in 0..repeats {
        win32::send_window_manager_chord(chord).map_err(SuiteError::setup)?;
        context
            .record_action(
                SUITE_ID,
                Some(step_id),
                window_target(session),
                RunnerActionKind::SendKeys {
                    keys: vec![chord.label().to_owned()],
                },
            )
            .map_err(SuiteError::setup)?;
        record_wait(
            context,
            Some(step_id),
            &format!("after {}", chord.label()),
            CHORD_SETTLE_MS,
        )?;
    }

    if chord == WindowManagerChord::AltSpace {
        // Dismiss the window menu before it eats the next chord.
        win32::send_escape().map_err(SuiteError::setup)?;
        record_wait(
            context,
            Some(step_id),
            "after dismissing the window menu",
            SETTLE_MS,
        )?;
    }

    clear_held_modifiers(context, Some(step_id))?;
    focus_terminal_pane(context, session, Some(step_id))
}

/// Press a key, take focus away and give it back, then release.
///
/// Windows reports the still-held key when focus returns; the app must treat
/// that as a state notification rather than a keystroke.
fn hold_key_across_focus_round_trip(
    context: &SuiteContext<'_>,
    session: &AppSession,
) -> SuiteResult<()> {
    let step_id = "focus-replay";
    let hwnd = session.window();

    // A modifier still held here would turn the plain letter below into a
    // chord, and the leg would pass for the wrong reason.
    clear_held_modifiers(context, Some(step_id))?;

    win32::set_letter_key_pressed(HELD_LETTER, true).map_err(SuiteError::setup)?;
    context
        .record_action(
            SUITE_ID,
            Some(step_id),
            window_target(session),
            RunnerActionKind::SendKeys {
                keys: vec![format!("{HELD_LETTER}-down")],
            },
        )
        .map_err(SuiteError::setup)?;
    record_wait(
        context,
        Some(step_id),
        "after holding a key down",
        SETTLE_MS,
    )?;

    // Release the key even if the window refuses to minimize or restore --
    // a stuck key would poison every later suite in the run.
    let round_trip = (|| -> SuiteResult<()> {
        win32::minimize_window(hwnd).map_err(SuiteError::setup)?;
        record_wait(
            context,
            Some(step_id),
            "after minimizing with a key held",
            CHORD_SETTLE_MS,
        )?;
        win32::restore_window(hwnd).map_err(SuiteError::setup)?;
        record_wait(
            context,
            Some(step_id),
            "after restoring with a key held",
            CHORD_SETTLE_MS,
        )?;
        // Restoring a window does not necessarily hand focus back, and the
        // replay only happens on the focus *gain* -- so take focus while the
        // key is still down. Releasing first would make this leg pass without
        // ever exercising what it protects.
        focus_terminal_pane(context, session, Some(step_id))?;
        record_wait(
            context,
            Some(step_id),
            "after regaining focus with a key held",
            CHORD_SETTLE_MS,
        )
    })();

    let release = win32::set_letter_key_pressed(HELD_LETTER, false).map_err(SuiteError::setup);
    round_trip?;
    release?;
    context
        .record_action(
            SUITE_ID,
            Some(step_id),
            window_target(session),
            RunnerActionKind::SendKeys {
                keys: vec![format!("{HELD_LETTER}-up")],
            },
        )
        .map_err(SuiteError::setup)?;
    record_wait(
        context,
        Some(step_id),
        "after releasing the held key",
        SETTLE_MS,
    )
}

/// Force every modifier back up and refuse to continue if one stays down.
///
/// Injected chords sometimes lose a key-up: the window manager consumes the
/// chord and the release never reaches the desktop, leaving `Alt` or `Win`
/// logically held. The next plain letter then arrives as a chord, which both
/// hides a real leak and produces a baffling assertion failure, so treat a
/// modifier that survives the release as a broken environment rather than a
/// product bug.
fn clear_held_modifiers(context: &SuiteContext<'_>, step_id: Option<&str>) -> SuiteResult<()> {
    win32::release_modifier_keys().map_err(SuiteError::setup)?;
    thread::sleep(Duration::from_millis(SETTLE_MS));

    if let Some(stuck) = win32::stuck_modifier_name().map_err(SuiteError::setup)? {
        return Err(SuiteError::setup(format!(
            "{stuck} stayed held after releasing every modifier; the desktop cannot deliver a              plain keystroke"
        )));
    }

    context
        .record_action(
            SUITE_ID,
            step_id,
            RunnerActionTarget::None,
            RunnerActionKind::Note {
                message: "input.modifiers_released".to_owned(),
            },
        )
        .map_err(SuiteError::setup)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PanePoint {
    x: i32,
    y: i32,
}

/// Click the middle of the terminal pane.
///
/// This both raises the window and hands keyboard capture back to the grid.
/// Activating the window by its titlebar instead would leave the app focused
/// but the pane not, and every later assertion would pass for the wrong
/// reason.
fn focus_terminal_pane(
    context: &SuiteContext<'_>,
    session: &AppSession,
    step_id: Option<&str>,
) -> SuiteResult<()> {
    let hwnd = session.window();
    let client = win32::get_client_rect(hwnd).map_err(SuiteError::setup)?;
    let scale_factor = win32::window_scale_factor(hwnd).map_err(SuiteError::setup)?;
    let pane = terminal_pane_point(client, scale_factor);

    win32::focus_window_at(hwnd, pane.x, pane.y).map_err(SuiteError::setup)?;
    context
        .record_action(
            SUITE_ID,
            step_id,
            window_target(session),
            RunnerActionKind::Mouse {
                x: pane.x,
                y: pane.y,
                button: Some("left".to_owned()),
            },
        )
        .map_err(SuiteError::setup)?;
    thread::sleep(Duration::from_millis(SETTLE_MS));

    assert_true(
        win32::is_window_foreground(hwnd).map_err(SuiteError::setup)?,
        "terminal window did not come back to the foreground after a window-manager chord",
        "chord-focus-not-restored",
    )
}

fn require_snapshot(
    context: &SuiteContext<'_>,
    artifacts: &mut Vec<String>,
    diagnostics: Option<&ObservedDiagnostics>,
    artifact_stem: &str,
    reason: &str,
) -> SuiteResult<TerminalManagerSnapshot> {
    capture_step_snapshot_with_options(
        context,
        artifacts,
        SUITE_ID,
        diagnostics,
        artifact_stem,
        reason,
        SnapshotOptions {
            include_terminal_buffer: true,
        },
    )?
    .ok_or_else(|| {
        SuiteError::setup(
            "window-chord-key-isolation reads the terminal buffer to spot a single leaked \
             character; omit --observe off"
                .to_owned(),
        )
    })
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

fn screenshot_path(context: &SuiteContext<'_>, name: &str) -> std::path::PathBuf {
    context
        .artifact_layout
        .run_dir
        .join(suite_artifact_name(SUITE_ID, name, "png"))
}

fn terminal_pane_point(client: DesktopRect, scale_factor: f64) -> PanePoint {
    let content_left = client.left
        + scaled_px(DEFAULT_SIDEBAR_WIDTH, scale_factor)
        + scaled_px(SIDEBAR_RESIZER_WIDTH, scale_factor);
    let terminal_top = client.top
        + scaled_px(TITLEBAR_HEIGHT, scale_factor)
        + scaled_px(TABBAR_HEIGHT, scale_factor);
    let terminal_bottom = client.bottom - scaled_px(STATUSBAR_HEIGHT, scale_factor);

    PanePoint {
        x: clamp_between(
            (content_left + client.right) / 2,
            client.left + 8,
            client.right - 8,
        ),
        y: clamp_between(
            (terminal_top + terminal_bottom) / 2,
            terminal_top + 8,
            terminal_bottom - 8,
        ),
    }
}

fn scaled_px(value: i32, scale_factor: f64) -> i32 {
    (value as f64 * scale_factor).round() as i32
}

fn clamp_between(value: i32, min: i32, max: i32) -> i32 {
    if min <= max {
        value.clamp(min, max)
    } else {
        value
    }
}

fn initial_window_rect(screen: DesktopSize) -> DesktopRect {
    let width = (screen.width * 62 / 100).clamp(900, screen.width.saturating_sub(120));
    let height = (screen.height * 62 / 100).clamp(560, screen.height.saturating_sub(120));
    let left = ((screen.width - width) / 2).max(0);
    let top = ((screen.height - height) / 2).max(0);

    DesktopRect {
        left,
        top,
        right: left + width,
        bottom: top + height,
    }
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

    fn events(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_owned()).collect()
    }

    fn state(row: &str, cursor: Option<(u32, u32)>, scrollback: Option<u64>) -> TerminalState {
        TerminalState {
            row: row.to_owned(),
            cursor,
            scrollback_len: scrollback,
        }
    }

    #[test]
    fn sentinel_is_free_of_dead_key_characters() {
        // A US-International layout turns these into dead keys, so typing the
        // sentinel would compose accents instead of the literal characters.
        assert!(!SENTINEL.contains(['\'', '"', '~', '^', '`']));
        assert!(SENTINEL
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch == '-'));
    }

    #[test]
    fn new_pty_events_returns_the_tail_when_nothing_was_evicted() {
        let before = events(&["read pane=1", "read pane=1 bytes=4"]);
        let after = events(&["read pane=1", "read pane=1 bytes=4", "write pane=1"]);

        assert_eq!(new_pty_events(&before, &after), events(&["write pane=1"]));
    }

    #[test]
    fn new_pty_events_handles_an_evicted_prefix() {
        let before = events(&["a", "b", "c"]);
        let after = events(&["b", "c", "d", "e"]);

        assert_eq!(new_pty_events(&before, &after), events(&["d", "e"]));
    }

    #[test]
    fn new_pty_events_reports_everything_when_the_ring_fully_rolled_over() {
        let before = events(&["a", "b"]);
        let after = events(&["x", "y", "z"]);

        assert_eq!(new_pty_events(&before, &after), after);
    }

    #[test]
    fn new_pty_events_survives_repeated_identical_lines() {
        let write = "write pane=1 bytes=1 source=keyboard";
        let before = events(&[write, write, write]);
        let after = events(&[write, write, write, write]);

        assert_eq!(new_pty_events(&before, &after), events(&[write]));
    }

    #[test]
    fn new_pty_events_is_empty_when_nothing_happened() {
        let before = events(&["read pane=1", "read pane=1 bytes=4"]);

        assert!(new_pty_events(&before, &before).is_empty());
    }

    #[test]
    fn keyboard_write_count_ignores_reads_and_other_sources() {
        let sample = events(&[
            "write pane=1 bytes=1 source=keyboard",
            "write pane=1 bytes=3 source=wheel",
            "read pane=1 bytes=9 batched=1",
            "write pane=1 bytes=1 source=keyboard",
            "key_suppressed pane=1 key=Space mods=Modifiers(ALT) reason=os_reserved_chord",
        ]);

        assert_eq!(keyboard_write_count(&sample), 2);
    }

    #[test]
    fn suppressed_chord_events_are_collected_for_the_summary() {
        let sample = events(&[
            "read pane=1 bytes=9 batched=1",
            "key_suppressed pane=1 key=Space mods=Modifiers(ALT) reason=os_reserved_chord",
        ]);

        assert_eq!(suppressed_chord_events(&sample).len(), 1);
    }

    #[test]
    fn sentinel_row_picks_the_last_matching_row_and_trims_padding() {
        let rows = events(&[
            "PS C:\\> tm-chord-leak-sentinel    ",
            "",
            "PS C:\\> tm-chord-leak-sentinel      ",
            "                                   ",
        ]);

        assert_eq!(
            sentinel_row(&rows, SENTINEL).as_deref(),
            Some("PS C:\\> tm-chord-leak-sentinel")
        );
    }

    #[test]
    fn sentinel_row_is_absent_when_the_pane_never_received_the_text() {
        assert!(sentinel_row(&events(&["PS C:\\>"]), SENTINEL).is_none());
    }

    #[test]
    fn untouched_terminal_passes_the_no_typing_assertion() {
        let before = state("PS C:\\> tm-chord-leak-sentinel", Some((4, 30)), Some(120));

        assert!(assert_typed_letters(&before, &before, &[], "", "leak", "signal").is_ok());
    }

    #[test]
    fn a_single_leaked_character_fails_the_no_typing_assertion() {
        let before = state("PS C:\\> tm-chord-leak-sentinel", Some((4, 30)), Some(120));
        let after = state("PS C:\\> tm-chord-leak-sentineld", Some((4, 31)), Some(120));
        let leaked = events(&["write pane=1 bytes=1 source=keyboard"]);

        let err = assert_typed_letters(&before, &after, &leaked, "", "leak", "signal").unwrap_err();

        assert_eq!(err.first_bad_signal.as_deref(), Some("signal"));
        assert!(err.message.contains("tm-chord-leak-sentineld"));
    }

    #[test]
    fn a_replayed_press_fails_the_single_keystroke_assertion() {
        let before = state("PS C:\\> tm-chord-leak-sentinel", Some((4, 30)), Some(120));
        let replayed = state(
            "PS C:\\> tm-chord-leak-sentineldd",
            Some((4, 32)),
            Some(120),
        );
        let writes = events(&[
            "write pane=1 bytes=1 source=keyboard",
            "write pane=1 bytes=1 source=keyboard",
        ]);

        let err =
            assert_typed_letters(&before, &replayed, &writes, "d", "replay", "signal").unwrap_err();

        assert_eq!(err.first_bad_signal.as_deref(), Some("signal"));
    }

    #[test]
    fn one_deliberate_press_passes_the_single_keystroke_assertion() {
        let before = state("PS C:\\> tm-chord-leak-sentinel", Some((4, 30)), Some(120));
        let after = state("PS C:\\> tm-chord-leak-sentineld", Some((4, 31)), Some(120));
        let writes = events(&["write pane=1 bytes=1 source=keyboard"]);

        assert!(assert_typed_letters(&before, &after, &writes, "d", "replay", "signal").is_ok());
    }

    #[test]
    fn a_leaked_enter_is_caught_by_scrollback_growth() {
        let before = state("PS C:\\> tm-chord-leak-sentinel", Some((4, 30)), Some(120));
        let after = state("PS C:\\> tm-chord-leak-sentinel", Some((4, 30)), Some(121));

        let err = assert_typed_letters(&before, &after, &[], "", "leak", "signal").unwrap_err();

        assert_eq!(err.first_bad_signal.as_deref(), Some("signal-scrollback"));
    }

    #[test]
    fn a_pty_write_with_no_visible_change_still_fails() {
        let before = state("PS C:\\> tm-chord-leak-sentinel", Some((4, 30)), Some(120));
        let writes = events(&["write pane=1 bytes=1 source=keyboard"]);

        let err =
            assert_typed_letters(&before, &before, &writes, "", "leak", "signal").unwrap_err();

        assert_eq!(err.first_bad_signal.as_deref(), Some("signal-pty-write"));
    }

    #[test]
    fn terminal_pane_point_lands_right_of_the_sidebar_and_below_the_tabbar() {
        let client = DesktopRect {
            left: 100,
            top: 50,
            right: 1300,
            bottom: 850,
        };

        let pane = terminal_pane_point(client, 1.0);

        assert!(pane.x > client.left + DEFAULT_SIDEBAR_WIDTH);
        assert!(pane.x < client.right);
        assert!(pane.y > client.top + TITLEBAR_HEIGHT + TABBAR_HEIGHT);
        assert!(pane.y < client.bottom - STATUSBAR_HEIGHT);
    }

    #[test]
    fn terminal_pane_point_scales_chrome_offsets() {
        let client = DesktopRect {
            left: 0,
            top: 0,
            right: 2400,
            bottom: 1600,
        };

        let scaled = terminal_pane_point(client, 2.0);

        assert!(scaled.x > terminal_pane_point(client, 1.0).x);
        assert!(scaled.y > terminal_pane_point(client, 1.0).y);
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
            screenshot_path(&context, "after-chords"),
            root.join("window-chord-key-isolation-after-chords.png")
        );
    }
}
