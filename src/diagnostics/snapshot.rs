use std::collections::BTreeSet;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Map, Value};
use terminal_manager_diagnostics::{
    DiagnosticLogRecord, InvariantEvaluation, InvariantOutcome, InvariantScope, LayoutNodeSnapshot,
    LayoutSnapshot, PtySessionSnapshot, PtySnapshot, Rect, RendererSnapshot, Size,
    SnapshotAppIdentity, SnapshotOptions, TerminalBufferWindowSnapshot, TerminalCursorSnapshot,
    TerminalGridSnapshot, TerminalManagerSnapshot, TerminalSnapshot, WindowSnapshot,
    SNAPSHOT_SCHEMA_VERSION,
};

use crate::state::{MutexExt, PaneId, SessionSnapshot, SharedState, SharedTerminal, UiSnapshot};

const TERMINAL_BUFFER_MAX_ROWS: usize = 50;
const TERMINAL_BUFFER_MAX_COLS: usize = 200;

#[derive(Default)]
struct ActiveTerminalDiagnostics {
    scrollback_len: Option<u64>,
    cursor: Option<TerminalCursorSnapshot>,
    dirty_regions: Vec<Rect>,
    buffer_window: Option<TerminalBufferWindowSnapshot>,
}

pub fn collect_snapshot(
    shared: &SharedState,
    reason: impl Into<String>,
    diagnostic_endpoint: Option<String>,
    options: &SnapshotOptions,
) -> Result<TerminalManagerSnapshot, String> {
    let captured_at_utc = now_utc_string();
    let (
        ui,
        active_terminal_handle,
        active_session_id,
        pty_session_mappings,
        pty_recent_events,
        renderer_frame_counter,
        renderer_last_present_unix_ms,
    ) = {
        let guard = shared.lock_recover();
        let ui = guard.ui_snapshot();
        let active_pane_id = guard.active_pane.0;
        (
            ui,
            guard.terminals.get(&active_pane_id).cloned(),
            guard.pty_manager.session_id(active_pane_id),
            guard.pty_manager.sessions_iter().collect::<Vec<_>>(),
            guard
                .diagnostic_pty_recent_events
                .iter()
                .cloned()
                .collect::<Vec<_>>(),
            guard.diagnostic_frame_counter,
            guard.diagnostic_last_present_unix_ms,
        )
    };

    let active_terminal_present = active_terminal_handle.is_some();
    let active_terminal =
        collect_active_terminal(active_terminal_handle, options.include_terminal_buffer);

    let active_terminal_grid = active_terminal_present.then_some(TerminalGridSnapshot {
        rows: u32::from(ui.active_terminal_rows),
        cols: u32::from(ui.active_terminal_cols),
    });
    let visible_rows = active_terminal_grid.as_ref().map(|grid| grid.rows);
    let active_session_id = active_session_id
        .map(|id| id.to_string())
        .or_else(|| active_session_id_from_cached_sessions(&ui));

    Ok(TerminalManagerSnapshot {
        schema_version: SNAPSHOT_SCHEMA_VERSION.to_owned(),
        captured_at_utc: captured_at_utc.clone(),
        reason: reason.into(),
        app: SnapshotAppIdentity {
            pid: Some(std::process::id()),
            build: build_identity(),
            commit: option_env!("GIT_COMMIT").map(str::to_owned),
            diagnostic_endpoint,
        },
        window: WindowSnapshot {
            outer_bounds: None,
            client_bounds: None,
            scale_factor: finite_positive(ui.scale_factor).map(f64::from),
            focused: true,
            resize_generation: None,
        },
        layout: collect_layout(&ui),
        terminal: TerminalSnapshot {
            grid: active_terminal_grid,
            visible_rows,
            scrollback_len: active_terminal.scrollback_len,
            cursor: active_terminal.cursor,
            selection_active: false,
            active_session_id,
            buffer_window: active_terminal.buffer_window,
        },
        renderer: RendererSnapshot {
            surface_size: renderer_surface_size(ui.last_grid_width, ui.last_grid_height),
            frame_counter: (renderer_frame_counter > 0).then_some(renderer_frame_counter),
            last_present_time_utc: renderer_last_present_unix_ms.map(format_unix_millis_utc),
            dirty_regions: active_terminal.dirty_regions,
            cached_layers: Vec::new(),
            glyph_atlas: None,
            last_render_error: None,
        },
        pty: collect_pty(&ui, &pty_session_mappings, pty_recent_events),
        input: terminal_manager_diagnostics::InputSnapshot {
            focused_element: Some(format!("pane:{}", ui.active_pane.0)),
            pressed_modifiers: Vec::new(),
            pointer_capture: None,
            hover_target: None,
            drag_state: ui.drag.is_active().then(|| format!("{:?}", ui.drag)),
            resize_handle: None,
        },
        config: collect_config(&ui, options.include_terminal_buffer),
        recent_warnings: Vec::new(),
        recent_errors: collect_recent_errors(&ui, &captured_at_utc),
    })
}

pub fn evaluate_invariants(
    shared: &SharedState,
    scope: InvariantScope,
) -> Result<Vec<InvariantEvaluation>, String> {
    let (ui, active_terminal_present) = {
        let guard = shared.lock_recover();
        let active_terminal_present = guard.terminals.contains_key(&guard.active_pane.0);
        (guard.ui_snapshot(), active_terminal_present)
    };

    let active_pane_exists = pane_exists(&ui.panes, ui.active_pane);
    let has_any_terminal = ui.terminal_count > 0;
    let grid_dimensions_valid = ui.last_grid_width.is_finite()
        && ui.last_grid_height.is_finite()
        && ui.last_grid_width >= 0.0
        && ui.last_grid_height >= 0.0;
    let scale_factor_valid = ui.scale_factor.is_finite() && ui.scale_factor > 0.0;

    let mut results = vec![
        invariant(
            "app.active_pane.exists",
            if active_pane_exists {
                InvariantOutcome::Passed
            } else {
                InvariantOutcome::Failed
            },
            if active_pane_exists {
                "active pane exists in the active layout"
            } else {
                "active pane id is not present in the active layout"
            },
            json!({ "active_pane": ui.active_pane.0 }),
        ),
        invariant(
            "app.active_pane.has_terminal",
            if !has_any_terminal {
                InvariantOutcome::Skipped
            } else if active_terminal_present {
                InvariantOutcome::Passed
            } else {
                InvariantOutcome::Failed
            },
            if !has_any_terminal {
                "terminal handles are not initialized yet"
            } else if active_terminal_present {
                "active pane has an attached terminal handle"
            } else {
                "active pane is missing an attached terminal handle"
            },
            json!({
                "active_pane": ui.active_pane.0,
                "terminal_count": ui.terminal_count
            }),
        ),
        invariant(
            "renderer.grid_dimensions_non_negative",
            if grid_dimensions_valid {
                InvariantOutcome::Passed
            } else {
                InvariantOutcome::Failed
            },
            if grid_dimensions_valid {
                "renderer grid dimensions are finite and non-negative"
            } else {
                "renderer grid dimensions must be finite and non-negative"
            },
            json!({
                "last_grid_width": ui.last_grid_width,
                "last_grid_height": ui.last_grid_height
            }),
        ),
        invariant(
            "window.scale_factor_valid",
            if scale_factor_valid {
                InvariantOutcome::Passed
            } else {
                InvariantOutcome::Failed
            },
            if scale_factor_valid {
                "window scale factor is finite and positive"
            } else {
                "window scale factor must be finite and positive"
            },
            json!({ "scale_factor": ui.scale_factor }),
        ),
    ];

    results.retain(|result| scope_includes(&scope, &result.id));
    Ok(results)
}

fn collect_layout(ui: &UiSnapshot) -> LayoutSnapshot {
    let mut nodes = Vec::new();
    if let Some(workspace) = ui.workspaces.get(ui.active_workspace) {
        nodes.push(LayoutNodeSnapshot {
            id: format!("workspace:{}", ui.active_workspace),
            label: Some(format!("active workspace: {}", workspace.name)),
            bounds: None,
            visible: true,
            z_order: 0,
            dirty: false,
        });
    }
    if let Some(tab) = ui.tabs.get(ui.active_tab) {
        nodes.push(LayoutNodeSnapshot {
            id: format!("tab:{}", tab.id),
            label: Some(format!("active tab: {}", tab.name)),
            bounds: None,
            visible: true,
            z_order: 1,
            dirty: false,
        });
    }
    for (z_order, pane) in ui.panes.iter().flatten().enumerate() {
        nodes.push(LayoutNodeSnapshot {
            id: format!("pane:{}", pane.id.0),
            label: Some(pane.title.clone()),
            bounds: None,
            visible: true,
            z_order: z_order as i32 + 2,
            dirty: false,
        });
    }

    LayoutSnapshot {
        nodes,
        dirty: ui.drag.is_active(),
    }
}

fn collect_active_terminal(
    handle: Option<SharedTerminal>,
    include_buffer: bool,
) -> ActiveTerminalDiagnostics {
    let Some(handle) = handle else {
        return ActiveTerminalDiagnostics::default();
    };

    let terminal = handle.lock_recover();
    let grid = terminal.grid();
    ActiveTerminalDiagnostics {
        scrollback_len: Some(terminal.scrollback_len().min(u64::MAX as usize) as u64),
        cursor: Some(TerminalCursorSnapshot {
            row: grid.cursor_row().min(u32::MAX as usize) as u32,
            col: grid.cursor_col().min(u32::MAX as usize) as u32,
            visible: grid.cursor_visible(),
        }),
        dirty_regions: dirty_regions_from_terminal_grid(grid),
        buffer_window: include_buffer.then(|| terminal_buffer_window(&terminal)),
    }
}

fn dirty_regions_from_terminal_grid(grid: &unshit::core::cell_grid::CellGrid) -> Vec<Rect> {
    grid.line_damage()
        .iter()
        .enumerate()
        .filter_map(|(row, damage)| {
            if damage.is_clean() {
                return None;
            }
            let start = u32::from(damage.first_dirty_col);
            let end = u32::from(damage.last_dirty_col).max(start);
            Some(Rect {
                x: start.min(i32::MAX as u32) as i32,
                y: (row as u32).min(i32::MAX as u32) as i32,
                width: end.saturating_sub(start).saturating_add(1),
                height: 1,
            })
        })
        .collect()
}

fn terminal_buffer_window(terminal: &crate::terminal::Terminal) -> TerminalBufferWindowSnapshot {
    let grid = terminal.display_grid();
    let total_rows = grid.rows();
    let total_cols = grid.cols();
    let row_count = total_rows.min(TERMINAL_BUFFER_MAX_ROWS);
    let col_count = total_cols.min(TERMINAL_BUFFER_MAX_COLS);
    let start_row = total_rows.saturating_sub(row_count);
    let rows = (start_row..total_rows)
        .map(|row| grid.debug_row_string(row, 0, col_count))
        .collect::<Vec<_>>();

    TerminalBufferWindowSnapshot {
        start_row: start_row.min(u32::MAX as usize) as u32,
        start_col: 0,
        row_count: row_count.min(u32::MAX as usize) as u32,
        col_count: col_count.min(u32::MAX as usize) as u32,
        rows,
        truncated: total_rows > TERMINAL_BUFFER_MAX_ROWS || total_cols > TERMINAL_BUFFER_MAX_COLS,
    }
}

fn collect_pty(
    ui: &UiSnapshot,
    session_mappings: &[(u32, u64)],
    mut recent_events: Vec<String>,
) -> PtySnapshot {
    let mut seen_sessions = BTreeSet::new();
    let mut sessions = Vec::new();

    for (pane_id, session_id) in session_mappings {
        let cached = cached_session(ui, *pane_id, *session_id);
        seen_sessions.insert(*session_id);
        sessions.push(PtySessionSnapshot {
            id: session_id.to_string(),
            name: cached.and_then(|session| session.name.clone()),
            process_id: cached.and_then(|session| session.pid),
            status: if cached.map(|session| session.alive).unwrap_or(true) {
                "running".to_owned()
            } else {
                "stopped".to_owned()
            },
            reconnecting: false,
        });
    }

    for session in &ui.sessions {
        if !seen_sessions.insert(session.session_id) {
            continue;
        }
        sessions.push(PtySessionSnapshot {
            id: session.session_id.to_string(),
            name: session.name.clone(),
            process_id: session.pid,
            status: if session.alive {
                "running".to_owned()
            } else {
                "stopped".to_owned()
            },
            reconnecting: false,
        });
    }

    if ui.sessions_stale {
        recent_events.push("sessions_cache_stale".to_owned());
    }

    PtySnapshot {
        sessions,
        pending_writes: 0,
        recent_events,
    }
}

fn cached_session(ui: &UiSnapshot, pane_id: u32, session_id: u64) -> Option<&SessionSnapshot> {
    ui.sessions
        .iter()
        .find(|session| session.session_id == session_id || session.pane_id == pane_id)
}

fn collect_config(ui: &UiSnapshot, terminal_buffer_contents_included: bool) -> Value {
    let toggles: Map<String, Value> = ui
        .toggles
        .iter()
        .map(|(key, value)| (key.as_str().to_owned(), Value::Bool(*value)))
        .collect();

    json!({
        "theme": ui.theme,
        "font_size_pt": ui.font_size_pt,
        "settings_open": ui.settings_open,
        "settings_section": ui.settings_section.label(),
        "palette_open": ui.palette_open,
        "sidebar_collapsed": ui.sidebar_collapsed,
        "sidebar_width": ui.sidebar_width,
        "terminal_count": ui.terminal_count,
        "active_workspace": ui.active_workspace,
        "active_tab": ui.active_tab,
        "active_pane": ui.active_pane.0,
        "default_shell": {
            "program": ui.default_shell.program,
            "args": ui.default_shell.args,
        },
        "toggles": toggles,
        "terminal_buffer_contents_included": terminal_buffer_contents_included,
    })
}

fn collect_recent_errors(ui: &UiSnapshot, captured_at_utc: &str) -> Vec<DiagnosticLogRecord> {
    ui.toasts
        .iter()
        .rev()
        .take(10)
        .map(|toast| DiagnosticLogRecord {
            timestamp_utc: captured_at_utc.to_owned(),
            level: format!("{:?}", toast.kind).to_ascii_lowercase(),
            target: "toast".to_owned(),
            message: toast.message.clone(),
            fields: json!({
                "toast_id": toast.id,
                "title": toast.title,
            }),
        })
        .collect()
}

fn active_session_id_from_cached_sessions(ui: &UiSnapshot) -> Option<String> {
    ui.sessions
        .iter()
        .find(|session| session.pane_id == ui.active_pane.0)
        .map(|session| session.session_id.to_string())
}

fn renderer_surface_size(width: f32, height: f32) -> Option<Size> {
    if width.is_finite() && height.is_finite() && width >= 0.0 && height >= 0.0 {
        Some(Size {
            width: width.round().clamp(0.0, u32::MAX as f32) as u32,
            height: height.round().clamp(0.0, u32::MAX as f32) as u32,
        })
    } else {
        None
    }
}

fn finite_positive(value: f32) -> Option<f32> {
    (value.is_finite() && value > 0.0).then_some(value)
}

fn pane_exists(panes: &[Vec<crate::state::Pane>], active_pane: PaneId) -> bool {
    panes.iter().flatten().any(|pane| pane.id == active_pane)
}

fn invariant(
    id: &str,
    outcome: InvariantOutcome,
    message: &str,
    details: Value,
) -> InvariantEvaluation {
    InvariantEvaluation {
        id: id.to_owned(),
        outcome,
        message: Some(message.to_owned()),
        details,
    }
}

fn scope_includes(scope: &InvariantScope, id: &str) -> bool {
    match scope {
        InvariantScope::All => true,
        InvariantScope::Window => id.starts_with("window."),
        InvariantScope::Layout => id.starts_with("app.active_pane.exists"),
        InvariantScope::Renderer => id.starts_with("renderer."),
        InvariantScope::Terminal => id.starts_with("app.active_pane.has_terminal"),
        InvariantScope::Pty => false,
        InvariantScope::Input => false,
        InvariantScope::Custom(custom) => id == custom,
    }
}

fn build_identity() -> String {
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    format!("{profile}-{}", std::env::consts::ARCH)
}

fn now_utc_string() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format_unix_seconds_utc(seconds)
}

fn format_unix_seconds_utc(seconds: u64) -> String {
    let days = (seconds / 86_400) as i64;
    let seconds_of_day = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    let hour = seconds_of_day / 3_600;
    let minute = (seconds_of_day % 3_600) / 60;
    let second = seconds_of_day % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

fn format_unix_millis_utc(milliseconds: u64) -> String {
    let seconds = milliseconds / 1_000;
    let millis = milliseconds % 1_000;
    let days = (seconds / 86_400) as i64;
    let seconds_of_day = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    let hour = seconds_of_day / 3_600;
    let minute = (seconds_of_day % 3_600) / 60;
    let second = seconds_of_day % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z")
}

fn civil_from_days(days_since_unix_epoch: i64) -> (i64, u32, u32) {
    let z = days_since_unix_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    (year, month as u32, day as u32)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use terminal_manager_diagnostics::{
        InvariantOutcome, SnapshotOptions, SNAPSHOT_SCHEMA_VERSION,
    };

    use super::*;
    use crate::state::{
        record_diagnostic_pty_event, record_diagnostic_renderer_frame, seed_state, PaneId,
    };

    #[test]
    fn snapshot_collects_metadata_without_terminal_buffer_contents() {
        let mut state = seed_state();
        state.last_grid_width = 1024.0;
        state.last_grid_height = 768.0;
        state.scale_factor = 1.25;
        state.terminals.insert(
            state.active_pane.0,
            Arc::new(Mutex::new(crate::terminal::Terminal::new(33, 101))),
        );
        let shared = Arc::new(Mutex::new(state));

        let snapshot = collect_snapshot(
            &shared,
            "pre-resize",
            Some(r"\\.\pipe\tm-diagnostics-test".to_owned()),
            &SnapshotOptions::default(),
        )
        .expect("snapshot");

        assert_eq!(snapshot.schema_version, SNAPSHOT_SCHEMA_VERSION);
        assert_eq!(snapshot.reason, "pre-resize");
        assert_eq!(snapshot.app.pid, Some(std::process::id()));
        assert_eq!(
            snapshot.app.diagnostic_endpoint.as_deref(),
            Some(r"\\.\pipe\tm-diagnostics-test")
        );
        assert_eq!(snapshot.window.scale_factor, Some(1.25));
        assert_eq!(snapshot.renderer.surface_size.unwrap().width, 1024);
        assert_eq!(snapshot.renderer.surface_size.unwrap().height, 768);
        assert_eq!(snapshot.terminal.grid.unwrap().rows, 33);
        assert_eq!(snapshot.terminal.visible_rows, Some(33));
        assert!(snapshot.terminal.buffer_window.is_none());
        assert_eq!(snapshot.config["terminal_buffer_contents_included"], false);
        assert!(snapshot
            .layout
            .nodes
            .iter()
            .any(|node| node.id == "pane:1" && node.visible));
    }

    #[test]
    fn snapshot_wires_live_terminal_pty_and_renderer_diagnostic_fields() {
        let mut state = seed_state();
        state.last_grid_width = 640.0;
        state.last_grid_height = 480.0;
        let pane_id = state.active_pane.0;
        let (_daemon_guard, _write_errors) = state
            .pty_manager
            .test_install_slow_daemon_inner(pane_id, 4242);
        record_diagnostic_pty_event(&mut state, "read pane=1 bytes=12 batched=1");
        record_diagnostic_renderer_frame(&mut state, 1_767_996_000_123);

        let mut terminal = crate::terminal::Terminal::new(2, 20);
        terminal.process_bytes(b"one\r\ntwo\r\nprompt> ");
        state
            .terminals
            .insert(pane_id, Arc::new(Mutex::new(terminal)));
        let shared = Arc::new(Mutex::new(state));

        let snapshot =
            collect_snapshot(&shared, "live", None, &SnapshotOptions::default()).expect("snapshot");

        assert_eq!(snapshot.terminal.active_session_id.as_deref(), Some("4242"));
        assert_eq!(snapshot.terminal.scrollback_len, Some(1));
        let cursor = snapshot.terminal.cursor.expect("cursor");
        let grid = snapshot.terminal.grid.expect("grid");
        assert!(cursor.row < grid.rows);
        assert!(cursor.col < grid.cols);
        assert!(!snapshot.pty.sessions.is_empty());
        assert!(snapshot
            .pty
            .sessions
            .iter()
            .any(|session| session.id == "4242" && session.status == "running"));
        assert!(snapshot
            .pty
            .recent_events
            .iter()
            .any(|event| event.contains("bytes=12")));
        assert_eq!(snapshot.renderer.frame_counter, Some(1));
        assert!(snapshot.renderer.last_present_time_utc.is_some());
        assert!(!snapshot.renderer.dirty_regions.is_empty());
        assert!(snapshot.renderer.glyph_atlas.is_none());
        assert!(snapshot.terminal.buffer_window.is_none());
        assert_eq!(snapshot.config["terminal_buffer_contents_included"], false);
    }

    #[test]
    fn snapshot_includes_terminal_buffer_only_when_requested() {
        let mut state = seed_state();
        let pane_id = state.active_pane.0;
        let mut terminal = crate::terminal::Terminal::new(4, 20);
        terminal.process_bytes(b"diagnostic-prompt> ");
        state
            .terminals
            .insert(pane_id, Arc::new(Mutex::new(terminal)));
        let shared = Arc::new(Mutex::new(state));

        let default_snapshot =
            collect_snapshot(&shared, "default", None, &SnapshotOptions::default())
                .expect("default snapshot");
        assert!(default_snapshot.terminal.buffer_window.is_none());
        assert_eq!(
            default_snapshot.config["terminal_buffer_contents_included"],
            false
        );

        let opt_in_snapshot = collect_snapshot(
            &shared,
            "opt-in",
            None,
            &SnapshotOptions {
                include_terminal_buffer: true,
            },
        )
        .expect("opt-in snapshot");

        let window = opt_in_snapshot
            .terminal
            .buffer_window
            .expect("buffer window");
        assert!(window
            .rows
            .iter()
            .any(|line| line.contains("diagnostic-prompt")));
        assert_eq!(
            opt_in_snapshot.config["terminal_buffer_contents_included"],
            true
        );
    }

    #[test]
    fn invariants_return_stable_pass_fail_and_skipped_records() {
        let mut state = seed_state();
        state.last_grid_width = -1.0;
        state.last_grid_height = 12.0;
        state.scale_factor = 0.0;
        state.active_pane = PaneId(999);
        let shared = Arc::new(Mutex::new(state));

        let results = evaluate_invariants(&shared, InvariantScope::All).expect("invariants");

        assert!(results.iter().any(|result| {
            result.id == "app.active_pane.exists" && result.outcome == InvariantOutcome::Failed
        }));
        assert!(results.iter().any(|result| {
            result.id == "app.active_pane.has_terminal"
                && result.outcome == InvariantOutcome::Skipped
        }));
        assert!(results.iter().any(|result| {
            result.id == "renderer.grid_dimensions_non_negative"
                && result.outcome == InvariantOutcome::Failed
        }));
        assert!(results.iter().any(|result| {
            result.id == "window.scale_factor_valid" && result.outcome == InvariantOutcome::Failed
        }));
        assert!(results.iter().all(|result| result.message.is_some()));
    }
}
