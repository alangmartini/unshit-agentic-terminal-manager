use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Map, Value};
use terminal_manager_diagnostics::{
    DiagnosticLogRecord, InvariantEvaluation, InvariantOutcome, InvariantScope, LayoutNodeSnapshot,
    LayoutSnapshot, PtySessionSnapshot, PtySnapshot, RendererSnapshot, Size, SnapshotAppIdentity,
    TerminalGridSnapshot, TerminalManagerSnapshot, TerminalSnapshot, WindowSnapshot,
    SNAPSHOT_SCHEMA_VERSION,
};

use crate::state::{MutexExt, PaneId, SharedState, UiSnapshot};

pub fn collect_snapshot(
    shared: &SharedState,
    reason: impl Into<String>,
    diagnostic_endpoint: Option<String>,
) -> Result<TerminalManagerSnapshot, String> {
    let captured_at_utc = now_utc_string();
    let (ui, active_terminal_present) = {
        let guard = shared.lock_recover();
        let active_terminal_present = guard.terminals.contains_key(&guard.active_pane.0);
        (guard.ui_snapshot(), active_terminal_present)
    };

    let active_terminal_grid = active_terminal_present.then_some(TerminalGridSnapshot {
        rows: u32::from(ui.active_terminal_rows),
        cols: u32::from(ui.active_terminal_cols),
    });
    let visible_rows = active_terminal_grid.as_ref().map(|grid| grid.rows);

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
            scrollback_len: None,
            cursor: None,
            selection_active: false,
            active_session_id: active_session_id(&ui),
        },
        renderer: RendererSnapshot {
            surface_size: renderer_surface_size(ui.last_grid_width, ui.last_grid_height),
            frame_counter: None,
            last_present_time_utc: None,
            dirty_regions: Vec::new(),
            cached_layers: Vec::new(),
            glyph_atlas: None,
            last_render_error: None,
        },
        pty: collect_pty(&ui),
        input: terminal_manager_diagnostics::InputSnapshot {
            focused_element: Some(format!("pane:{}", ui.active_pane.0)),
            pressed_modifiers: Vec::new(),
            pointer_capture: None,
            hover_target: None,
            drag_state: ui.drag.is_active().then(|| format!("{:?}", ui.drag)),
            resize_handle: None,
        },
        config: collect_config(&ui),
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

fn collect_pty(ui: &UiSnapshot) -> PtySnapshot {
    PtySnapshot {
        sessions: ui
            .sessions
            .iter()
            .map(|session| PtySessionSnapshot {
                id: session.session_id.to_string(),
                name: session.name.clone(),
                process_id: session.pid,
                status: if session.alive {
                    "running".to_owned()
                } else {
                    "stopped".to_owned()
                },
                reconnecting: false,
            })
            .collect(),
        pending_writes: 0,
        recent_events: ui
            .sessions_stale
            .then(|| "sessions_cache_stale".to_owned())
            .into_iter()
            .collect(),
    }
}

fn collect_config(ui: &UiSnapshot) -> Value {
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
        "terminal_buffer_contents_included": false,
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

fn active_session_id(ui: &UiSnapshot) -> Option<String> {
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

    use terminal_manager_diagnostics::{InvariantOutcome, SNAPSHOT_SCHEMA_VERSION};

    use super::*;
    use crate::state::{seed_state, PaneId};

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
        assert_eq!(snapshot.config["terminal_buffer_contents_included"], false);
        assert!(snapshot
            .layout
            .nodes
            .iter()
            .any(|node| node.id == "pane:1" && node.visible));
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
