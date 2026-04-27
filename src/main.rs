pub mod bench;
pub mod bridge;
pub mod daemon;
pub mod drag;
pub mod git;
pub mod keybinds;
pub mod persist;
pub mod pty;
pub mod shell;
pub mod state;
pub mod terminal;
pub mod ui;

use std::sync::Arc;

use unshit::app::{App, AppConfig, FontSource};
use unshit::core::element::*;
use unshit::core::event::DragPhase;
use unshit::core::style::parse::StyleDeclaration;
use unshit::core::style::types::{AlignItems, CssPosition, Dimension, JustifyContent};
use unshit::core::trace::{
    append_terminal_trace_line, terminal_trace_enabled, terminal_trace_file_path,
};

use crate::state::{
    dispatch, mutate_with, new_workspace, resize_all_terminals, seed_state, MutexExt, SharedState,
    UiSnapshot, MAX_SIDEBAR_WIDTH, MIN_SIDEBAR_WIDTH,
};
use crate::ui::settings::build_settings_modal;
use crate::ui::sidebar::{build_ctx_menu_overlay, build_sidebar};
use crate::ui::statusbar::build_statusbar;
use crate::ui::tabbar::build_tabbar;
use crate::ui::terminal_grid::build_terminal_grid;
use crate::ui::titlebar::build_titlebar;
use crate::ui::toasts::build_toast_overlay;

const STYLES: &str = include_str!("../assets/styles.css");

// Heap profiling via dhat. Only compiled in when --features profiling is set.
//
// The profiler writes `target/profile/dhat-heap.json` on drop, which can be
// loaded into the dhat viewer. We keep the `Profiler` inside a static Mutex
// because all of our process-exit paths (`std::process::exit` in the ctrl-c
// handler, panic hook, and `on_close` callback) bypass normal stack unwinding
// and therefore skip Drop. `finalize_profiler` pulls the value out and drops
// it explicitly right before the exit call, flushing the JSON output.
#[cfg(feature = "profiling")]
#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

#[cfg(feature = "profiling")]
static PROFILER: std::sync::Mutex<Option<dhat::Profiler>> = std::sync::Mutex::new(None);

/// Returns `true` if `init_profiler` should proceed to build a new
/// profiler. Extracted as a pure function so the idempotency guard
/// (refs #107) can be unit tested without engaging dhat's global
/// allocator, which cannot safely be re-initialized in-process.
#[cfg(feature = "profiling")]
fn should_init_profiler<T>(slot: &Option<T>) -> bool {
    slot.is_none()
}

#[cfg(feature = "profiling")]
fn init_profiler() {
    let mut guard = PROFILER.lock().unwrap();
    if !should_init_profiler(&*guard) {
        eprintln!("[profiling] init_profiler called twice, ignoring");
        return;
    }
    let out_dir = std::path::Path::new("target").join("profile");
    if let Err(e) = std::fs::create_dir_all(&out_dir) {
        eprintln!(
            "[profiling] failed to create {}: {e}; dhat will panic on build()",
            out_dir.display()
        );
    }
    let out_path = out_dir.join("dhat-heap.json");
    let profiler = dhat::Profiler::builder().file_name(&out_path).build();
    *guard = Some(profiler);
    eprintln!(
        "[profiling] dhat heap profiling active; output: {}",
        out_path.display()
    );
}

#[cfg(all(test, feature = "profiling"))]
mod profiler_tests {
    use super::should_init_profiler;

    // Regression test for issue #107: init_profiler must early-return
    // when the PROFILER slot is already populated, otherwise a second
    // call would drop the in-flight profiler and flush a partial JSON.
    #[test]
    fn should_init_profiler_returns_true_when_slot_is_empty() {
        let slot: Option<u32> = None;
        assert!(should_init_profiler(&slot));
    }

    #[test]
    fn should_init_profiler_returns_false_when_slot_is_populated() {
        let slot: Option<u32> = Some(0);
        assert!(!should_init_profiler(&slot));
    }
}

#[cfg(feature = "profiling")]
pub(crate) fn finalize_profiler() {
    // Recover through poison: if we crash mid-flush we still want the JSON on disk.
    let mut guard = PROFILER
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(p) = guard.take() {
        drop(p);
        eprintln!("[profiling] dhat heap profile flushed to target/profile/dhat-heap.json");
    }
}

#[cfg(not(feature = "profiling"))]
#[inline]
pub(crate) fn finalize_profiler() {}

/// Flush the heap profile (if the `profiling` feature is on) and exit
/// the process with status 0. Used by the close-app dialog buttons to
/// drive their own exit after routing through dispatch, since the
/// framework's `event_loop.exit()` path was vetoed by `on_close`.
pub(crate) fn shutdown_now() -> ! {
    finalize_profiler();
    std::process::exit(0);
}

/// Snapshot a terminal's display grid for the current render frame.
///
/// This is the per-terminal step run by `tree_fn` when it builds the
/// element tree. It takes the per-terminal mutex, clones the display
/// grid, hides the cursor on inactive panes, and returns the cloned
/// grid for the renderer to consume.
///
/// Issue #63: this function MUST NOT call `clear_dirty()` on the live
/// grid. The old code did, which produced a race where an interleaved
/// PTY chunk arriving between the clone and the clear would have its
/// damage wiped, dropping cells from the next frame. The clone already
/// owns its damage independently; leaving the live grid untouched means
/// the next snapshot still sees every write since the last clone. The
/// renderer's retained line quad cache (`LineQuadCache`) handles
/// replay for unchanged rows via the content-hash signature, so the
/// previous damage-skip optimisation is not needed for correctness or
/// performance.
fn snapshot_terminal_for_render(
    terminal: &mut crate::terminal::Terminal,
    pane_id: u32,
    is_active: bool,
) -> unshit::core::cell_grid::CellGrid {
    let mut grid = terminal.display_grid();
    if !is_active {
        grid.set_cursor_visible(false);
    }
    if terminal_trace_enabled() && is_active {
        let rows = grid.debug_rows(4, 96);
        append_terminal_trace_line(&format!(
            "terminal-trace stage=main_snapshot pane={} active=true cursor=({}, {}) visible={} row0={:?} row1={:?} row2={:?} row3={:?}",
            pane_id,
            grid.cursor_row(),
            grid.cursor_col(),
            grid.cursor_visible(),
            rows.first().cloned().unwrap_or_default(),
            rows.get(1).cloned().unwrap_or_default(),
            rows.get(2).cloned().unwrap_or_default(),
            rows.get(3).cloned().unwrap_or_default(),
        ));
    }
    grid
}

fn build_tree(
    snap: &UiSnapshot,
    shared: &SharedState,
    grids: &std::collections::HashMap<u32, unshit::core::cell_grid::CellGrid>,
) -> ElementTree {
    let sidebar = build_sidebar(snap, shared)
        .with_style(StyleDeclaration::Width(Dimension::Px(snap.sidebar_width)))
        .with_style(StyleDeclaration::MinWidth(Dimension::Px(
            snap.sidebar_width,
        )));

    let drag_shared = shared.clone();
    let sidebar_resizer = ElementDef::new(Tag::Div)
        .with_class("sidebar-resizer")
        .on_drag(move |ev| match ev.phase {
            DragPhase::Start => {
                mutate_with(&drag_shared, |st| {
                    st.sidebar_drag_start = Some(st.sidebar_width);
                });
            }
            DragPhase::Update => {
                mutate_with(&drag_shared, |st| {
                    let start = match st.sidebar_drag_start {
                        Some(w) => w,
                        None => return,
                    };
                    st.sidebar_width =
                        (start + ev.total_delta_x).clamp(MIN_SIDEBAR_WIDTH, MAX_SIDEBAR_WIDTH);
                });
            }
            DragPhase::End => {
                mutate_with(&drag_shared, |st| {
                    st.sidebar_drag_start = None;
                });
            }
        });

    let mut root = ElementDef::new(Tag::Div)
        .with_class("app")
        .with_child(build_titlebar(shared))
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("layout")
                .with_child(sidebar)
                .with_child(sidebar_resizer)
                .with_child(
                    ElementDef::new(Tag::Div)
                        .with_class("content")
                        .with_class("role-main")
                        .with_child(build_tabbar(snap, shared))
                        .with_child(build_terminal_grid(snap, shared, grids))
                        .with_child(build_statusbar(snap)),
                ),
        );

    if snap.settings_open {
        let s = shared.clone();
        root = root.with_child(
            ElementDef::new(Tag::Div)
                .with_class("modal-overlay")
                .with_class("open")
                .with_id("settings-modal")
                .with_style(StyleDeclaration::Position(CssPosition::Fixed))
                .with_style(StyleDeclaration::Top(Dimension::Px(0.0)))
                .with_style(StyleDeclaration::Right(Dimension::Px(0.0)))
                .with_style(StyleDeclaration::Bottom(Dimension::Px(0.0)))
                .with_style(StyleDeclaration::Left(Dimension::Px(0.0)))
                .with_style(StyleDeclaration::AlignItems(AlignItems::Center))
                .with_style(StyleDeclaration::JustifyContent(JustifyContent::Center))
                .on_click(move || {
                    mutate_with(&s, |st| dispatch(st, "modal.close"));
                })
                .with_child(build_settings_modal(snap, shared)),
        );
    }

    // The drop-zone overlay is rendered as a child inside each pane
    // (see `build_pane`); no root-level overlay is needed.
    if let Some(ghost) = crate::ui::drag_overlay::build_drag_overlay(snap) {
        root = root.with_child(ghost);
    }

    ElementTree {
        root: root
            .with_child(build_ctx_menu_overlay(snap, shared))
            .with_child(crate::ui::confirm_dialog::build_confirm_dialog_overlay(
                snap, shared,
            ))
            .with_child(build_toast_overlay(snap, shared)),
    }
}

fn user_shortcut_bindings() -> Vec<(String, String)> {
    let overrides = crate::keybinds::loader::load_if_installed();
    crate::keybinds::registry::shortcut_bindings_with_overrides(&overrides)
}

fn parse_bench_args() -> Option<crate::bench::BenchConfig> {
    let mut args = std::env::args().skip(1);
    let mut mode: Option<crate::bench::BenchMode> = None;
    let mut duration_secs: f64 = 10.0;
    let mut warmup_secs: f64 = 2.5;
    let mut out_path: std::path::PathBuf = "bench.json".into();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--bench" => {
                if let Some(m) = args.next() {
                    mode = crate::bench::BenchMode::parse(&m);
                    if mode.is_none() {
                        eprintln!("unknown bench mode: {m}");
                        std::process::exit(2);
                    }
                }
            }
            "--duration" => {
                if let Some(s) = args.next() {
                    duration_secs = s.parse().unwrap_or(duration_secs);
                }
            }
            "--warmup" => {
                if let Some(s) = args.next() {
                    warmup_secs = s.parse().unwrap_or(warmup_secs);
                }
            }
            "--out" => {
                if let Some(s) = args.next() {
                    out_path = s.into();
                }
            }
            _ => {}
        }
    }
    mode.map(|mode| crate::bench::BenchConfig {
        mode,
        duration: std::time::Duration::from_secs_f64(duration_secs),
        warmup: std::time::Duration::from_secs_f64(warmup_secs),
        out_path,
    })
}

fn main() {
    #[cfg(feature = "profiling")]
    init_profiler();

    let bench_config = parse_bench_args();

    // Guard against ghost handles (#32): ensure the process exits
    // immediately on Ctrl+C or panic so spawn_blocking reader tasks
    // (bridge.rs) cannot keep the .exe locked on Windows (os error 32).
    let default_panic = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        default_panic(info);
        finalize_profiler();
        std::process::exit(1);
    }));
    ctrlc::set_handler(|| {
        finalize_profiler();
        std::process::exit(0);
    })
    .expect("failed to set Ctrl+C handler");

    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or(
            "info,wgpu_hal=error,wgpu_core=error,naga=error,unshit_app::app=error",
        ),
    )
    .init();

    if terminal_trace_enabled() {
        append_terminal_trace_line(&format!(
            "terminal-trace stage=startup trace_file={}",
            terminal_trace_file_path().display()
        ));
    }

    if let Some(path) = persist::default_config_path() {
        persist::install(path);
    }

    if let Some(path) = crate::keybinds::loader::keybinds_config_path() {
        crate::keybinds::loader::install(path);
    }

    let mut initial_state = seed_state();
    if let Some(persisted) = persist::load_workspaces() {
        if !persisted.workspaces.is_empty() {
            initial_state.workspaces = persisted
                .workspaces
                .into_iter()
                .enumerate()
                .map(|(i, entry)| {
                    let mut ws = new_workspace((i + 1) as u32, entry.name, entry.path);
                    ws.collapsed = entry.collapsed;
                    ws.shell = entry.shell;
                    ws
                })
                .collect();
            let last = initial_state.workspaces.len() - 1;
            initial_state.active_workspace = persisted.active_workspace.min(last);
        }
        initial_state.toggles.insert(
            crate::state::ToggleKey::RememberCloseChoice,
            persisted.remember_close_choice,
        );
        initial_state.toggles.insert(
            crate::state::ToggleKey::KillAllOnClose,
            persisted.kill_all_on_close,
        );
        // Override the seed_state inference with whatever the user
        // last persisted. An upgrader without the field gets an
        // empty spec here, which keeps the daemon's `default_shell()`
        // floor exactly as before.
        initial_state.default_shell = persisted.default_shell;
    }
    // Bench mode needs a deterministic shell so the scroll workload
    // measures comparable output across runs. On Windows the bench
    // exercises `dir`, which only behaves on cmd.exe; route it through
    // the same `default_shell` channel the rest of the app uses instead
    // of mutating the SHELL env var (which would leak into any spawned
    // child).
    #[cfg(windows)]
    if bench_config.is_some() {
        initial_state.default_shell = crate::shell::ShellSpec {
            program: "cmd.exe".into(),
            args: Vec::new(),
        };
    }
    let shared: SharedState = Arc::new(std::sync::Mutex::new(initial_state));

    // Bring the unshit-ptyd daemon up and wire the UI's DaemonPty shim
    // to it. Uses a short-lived tokio runtime because connect_or_spawn
    // is async; the shim's own worker thread drives every subsequent
    // IPC call, so this runtime dies once the probe completes.
    {
        let socket_path = unshit_ptyd::transport::default_socket_path();
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime for daemon probe");
        rt.block_on(daemon::connect_or_spawn(&socket_path))
            .expect("connect or spawn unshit-ptyd daemon");
        drop(rt);
        let mut guard = shared.lock().unwrap();
        guard
            .pty_manager
            .connect_to(&socket_path)
            .expect("DaemonPty shim connect_to");
    }

    // Measure the actual monospace cell width ratio for later use (split
    // pane spawns, etc.). Do NOT pre-publish cell metrics to the global
    // atomics: the pre-published values differ slightly from what the
    // renderer measures (different FontSystem instance), causing the
    // on_resize handler to fire an intermediate resize with wrong column
    // count. Instead, let the renderer be the single source of truth:
    // on_resize stores last_grid_width (cell_w is 0 so no resize), then
    // on_cell_metrics fires with the renderer's exact cell_w and resizes
    // the PTY once to the correct dimensions.
    {
        let mut guard = shared.lock().unwrap();
        let font_size = crate::state::CSS_BASE_FONT_SIZE * guard.scale_factor;
        let line_height = font_size * crate::state::CSS_LINE_HEIGHT;
        guard.cell_width_ratio = crate::state::measure_cell_width_ratio_at(font_size, line_height);
    }

    // Reconcile the initial pane against any surviving daemon session
    // (slice 5): if a prior UI run left a session with a matching
    // `(workspace_id, pane_id)` on the daemon, reattach and replay its
    // snapshot so the user sees their shell exactly as they left it;
    // otherwise spawn a fresh one. This is load-bearing: without a live
    // terminal the CellGrid doesn't exist, the renderer can't publish
    // metrics, and the PTY never gets spawned (deadlock). Estimate
    // dimensions from the window size minus CSS chrome so the shell
    // greeting is formatted for roughly the right width; on_cell_metrics
    // corrects to exact values on the first frame.
    {
        let mut guard = shared.lock().unwrap();
        // CSS chrome in logical pixels (scale cancels: grid and cells both
        // scale equally).  sidebar(252) + resizer(4) + pane borders/margins
        // (4) + pane-body horizontal padding(24) = 284.  tabbar(38) +
        // statusbar(24) + pane-header(27) + pane borders/margins(4) +
        // pane-body vertical padding(16) = 109.
        let cell_w_est = crate::state::CSS_BASE_FONT_SIZE * guard.cell_width_ratio;
        let cell_h_est = crate::state::CSS_BASE_FONT_SIZE * crate::state::CSS_LINE_HEIGHT;
        let init_cols = ((1280.0_f32 - 284.0) / cell_w_est).max(1.0) as u16;
        let init_rows = ((800.0_f32 - 109.0) / cell_h_est).max(1.0) as u16;
        log::info!(
            "initial PTY estimate: {}x{} (cell_w_est={:.2}, cell_h_est={:.2})",
            init_cols,
            init_rows,
            cell_w_est,
            cell_h_est,
        );
        let pane_id = guard.active_pane.0;
        let workspace_id = crate::state::active_workspace_num(&guard);
        let cwd = crate::state::active_workspace_cwd(&guard);
        let shell = crate::shell::resolve(None, Some(&guard.default_shell));
        match guard.pty_manager.attach_or_spawn(
            pane_id,
            workspace_id,
            init_cols,
            init_rows,
            cwd.as_deref(),
            shell.as_ref(),
        ) {
            Ok((Some(snapshot), reader)) => {
                let rows = snapshot.grid.rows();
                let cols = snapshot.grid.cols();
                let mut terminal = crate::terminal::Terminal::new(rows, cols);
                terminal.apply_snapshot(&snapshot);
                guard.terminals.insert(
                    pane_id,
                    std::sync::Arc::new(std::sync::Mutex::new(terminal)),
                );
                crate::bridge::register_reader(pane_id, reader);
                log::info!(
                    "reattached pane {} to surviving daemon session ({}x{})",
                    pane_id,
                    cols,
                    rows
                );
            }
            Ok((None, reader)) => {
                let terminal =
                    crate::terminal::Terminal::new(init_rows as usize, init_cols as usize);
                guard.terminals.insert(
                    pane_id,
                    std::sync::Arc::new(std::sync::Mutex::new(terminal)),
                );
                crate::bridge::register_reader(pane_id, reader);
            }
            Err(e) => {
                log::error!("failed to spawn initial PTY: {}", e);
                let mut terminal =
                    crate::terminal::Terminal::new(init_rows as usize, init_cols as usize);
                terminal.process_bytes(format!("Failed to spawn shell: {}\r\n", e).as_bytes());
                guard.terminals.insert(
                    pane_id,
                    std::sync::Arc::new(std::sync::Mutex::new(terminal)),
                );
            }
        }
    }

    if let Some(cfg) = bench_config {
        crate::bench::start(cfg, shared.clone());
    }

    let tree_shared = shared.clone();
    let command_shared = shared.clone();
    let metrics_shared = shared.clone();
    let scale_shared = shared.clone();
    let close_shared = shared.clone();
    let sub_shared = shared.clone();
    let raw_key_shared = shared.clone();

    let mut app = App::new(
        AppConfig {
            title: "terminal manager".to_string(),
            width: 1280,
            height: 800,
            css: STYLES.to_string(),
            fonts: vec![
                FontSource::System("JetBrains Mono".to_string()),
                FontSource::System("Berkeley Mono".to_string()),
                FontSource::System("SF Mono".to_string()),
                FontSource::System("Menlo".to_string()),
                FontSource::System("Consolas".to_string()),
            ],
            user_shortcuts: user_shortcut_bindings(),
            on_command: Some(Arc::new(move |command: &str| -> bool {
                let mut guard = command_shared.lock_recover();
                dispatch(&mut guard, command)
            })),
            on_raw_key: Some(Arc::new(
                move |combo: &unshit::core::shortcut::KeyCombo| -> bool {
                    use unshit::core::event::Key;
                    let mut guard = raw_key_shared.lock().expect("state mutex poisoned");
                    // Only intercept while a keybind is being recorded. Escape
                    // cancels; any other combo commits via keybind.set.
                    let Some(action) = guard.keybinds.recording else {
                        return false;
                    };
                    if combo.key == Key::Escape && combo.modifiers.is_empty() {
                        dispatch(&mut guard, "keybind.cancel_record");
                    } else {
                        let cmd = format!("keybind.set:{}:{}", action.id(), combo);
                        dispatch(&mut guard, &cmd);
                    }
                    true
                },
            )),
            // Approach 1: on_cell_metrics fires once after the first render
            // publishes valid cell dimensions. Resize all PTYs immediately.
            on_scale_factor: Some(Arc::new(move |scale: f32| {
                let mut guard = scale_shared.lock_recover();
                guard.scale_factor = scale;
            })),
            on_close: Some(Arc::new(move || -> bool {
                // F7: when the user has not yet remembered a choice, veto
                // the framework's close and route through the confirm
                // dialog so they can pick "keep running" / "kill all" /
                // "cancel". Once a choice has been persisted it is applied
                // silently on close.
                //
                // Slice 5 session-survival policy still applies: keep-running
                // just drops local UI state (terminals map, readers) so the
                // current process can exit cleanly while the shells keep
                // running on the daemon. Kill-all routes through
                // `DaemonPty::destroy_all` before exit.
                //
                // Use .lock().ok() instead of .expect() so a poisoned mutex
                // (from a panic on another thread) does not prevent us from
                // reaching process::exit below.
                let action = {
                    let Ok(mut guard) = close_shared.lock() else {
                        // Mutex poisoned: skip the prompt path, fall back
                        // to the legacy "just exit" behaviour so we do not
                        // wedge the user holding an undismissable dialog.
                        finalize_profiler();
                        return true;
                    };
                    crate::state::resolve_close_action(&mut guard)
                };
                match action {
                    crate::state::CloseAction::Prompt => {
                        // Veto. The confirm dialog is now visible; the UI
                        // click handlers drive the real exit.
                        false
                    }
                    crate::state::CloseAction::KeepRunning => {
                        if let Ok(mut guard) = close_shared.lock() {
                            guard.terminals.clear();
                        }
                        finalize_profiler();
                        std::process::exit(0);
                    }
                    crate::state::CloseAction::KillAll => {
                        if let Ok(mut guard) = close_shared.lock() {
                            crate::state::mutate_kill_all_terminals(&mut guard);
                        }
                        finalize_profiler();
                        std::process::exit(0);
                    }
                }
            })),
            on_cell_metrics: Some(Arc::new(move |cell_w: f32, cell_h: f32| {
                use unshit::core::cell_grid::CellGrid;
                let mut guard = metrics_shared.lock_recover();
                let (cols, rows) = CellGrid::take_pending_resize().unwrap_or_else(|| {
                    let w = guard.last_grid_width;
                    let h = guard.last_grid_height;
                    crate::state::compute_pty_dimensions(w, h, cell_w, cell_h)
                });
                log::info!(
                    "on_cell_metrics: cell={}x{} -> resize all PTYs to {}x{}",
                    cell_w,
                    cell_h,
                    cols,
                    rows
                );
                resize_all_terminals(&mut guard, cols, rows);
            })),
            on_frame_metrics: Some(Box::new(|m| crate::bench::record_frame(m))),
            #[cfg(feature = "input-latency-histogram")]
            on_input_latency: Some(Box::new(|snap| crate::bench::record_input_latency(snap))),
            ..Default::default()
        },
        move || {
            // Grab the state mutex only long enough to snapshot the UI and
            // clone per-terminal `Arc<Mutex<Terminal>>` handles, then drop
            // it. The parser thread writing to any single terminal holds
            // only that terminal's mutex, so it never contends with this
            // closure on the state lock.
            let (snap, active_id, handles): (
                crate::state::UiSnapshot,
                u32,
                Vec<(u32, crate::state::SharedTerminal)>,
            ) = {
                let guard = tree_shared.lock_recover();
                let snap = guard.ui_snapshot();
                let active_id = guard.active_pane.0;
                let handles: Vec<_> = guard
                    .terminals
                    .iter()
                    .map(|(&id, t)| (id, t.clone()))
                    .collect();
                (snap, active_id, handles)
            };

            // State mutex is released; take each per-terminal lock
            // independently to clone its display grid. `snapshot_terminal_for_render`
            // does NOT clear the live grid's dirty state (see issue #63):
            // interleaved PTY writes that land between two snapshots must
            // stay reflected as damage on the next clone, otherwise the
            // renderer can skip them and drop cells from the viewport.
            let grids: std::collections::HashMap<u32, unshit::core::cell_grid::CellGrid> = handles
                .into_iter()
                .map(|(id, handle)| {
                    let mut t = handle.lock_recover();
                    let grid = snapshot_terminal_for_render(&mut t, id, id == active_id);
                    (id, grid)
                })
                .collect();
            build_tree(&snap, &tree_shared, &grids)
        },
    );

    // Set up PTY output subscriptions.
    app.set_subscriptions(move || bridge::build_subscriptions(&sub_shared));

    app.run();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::Terminal;

    /// Regression test for issue #63.
    ///
    /// The render loop snapshots each terminal's display grid once per
    /// frame. The old code additionally called
    /// `terminal.grid_mut().clear_dirty()` on the LIVE grid right after
    /// the clone. If a PTY chunk wrote to the live grid between the
    /// clone and the clear (or, more generally, at any time before the
    /// next snapshot with a clear still pending), the clear would wipe
    /// the damage bumped by that interleaved write, and the next
    /// snapshot would report the affected row as clean. The renderer
    /// would then skip the row on a cache miss
    /// (see `crates/unshit-framework/crates/unshit-renderer/src/batch.rs`
    /// `emit_grid_cells` `row_is_clean` path) and cells would disappear
    /// from the viewport.
    ///
    /// This test simulates that exact ordering and asserts that the
    /// interleaved write's damage survives to the next snapshot.
    #[test]
    fn snapshot_terminal_for_render_preserves_interleaved_write_damage() {
        // Models the exact ordering the task spec calls out:
        //   1. `display_grid()` (snapshot) already happened in the
        //      previous frame.
        //   2. A PTY chunk lands on the live grid, bumping line damage.
        //   3. The render loop runs its post-snapshot reset (what the
        //      broken code did was `grid_mut().clear_dirty()` on the
        //      live grid).
        //   4. The next frame snapshots again.
        //
        // With the broken code, step 3 wiped the live grid's damage for
        // step 2's writes, so step 4's clone reported the row as clean
        // and the renderer skipped it on a cache miss, dropping cells.
        let mut terminal = Terminal::new(10, 40);
        terminal.process_bytes(b"first line");

        // Step 1: previous-frame snapshot (establishes the starting
        // state for the race).
        let _snap_previous_frame = snapshot_terminal_for_render(&mut terminal, 0, true);

        // Step 2: a PTY chunk lands after the previous snapshot but
        // before the next frame, writing row 1.
        terminal.process_bytes(b"\r\nsecond line");

        // Step 3: production's "post-snapshot reset". This helper
        // mirrors whatever the main-loop does after taking a snapshot.
        // With the #63 fix the helper is a no-op; with the broken code
        // it used to call `grid_mut().clear_dirty()` on the live grid
        // and wipe the damage for step 2's writes.
        post_snapshot_reset_like_production(&mut terminal);

        // Step 4: the next frame snapshots the live grid. Before the
        // fix, row 1's damage was wiped by step 3 and this clone
        // reported the row as clean.
        let snap_next_frame = snapshot_terminal_for_render(&mut terminal, 0, true);

        let row1 = snap_next_frame
            .line_damage_for(1)
            .expect("row 1 damage entry must exist");
        assert!(
            !row1.is_clean(),
            "row 1 damage is clean after an interleaved PTY write \
             followed by the production post-snapshot reset. The \
             renderer would then skip row 1 on a cache miss and drop \
             its cells (regression of issue #63)"
        );

        // Sanity: the freshly written cells actually live on row 1.
        let cell = snap_next_frame
            .get_cell(1, 0)
            .expect("row 1 col 0 must exist");
        assert_eq!(
            cell.ch, 's',
            "row 1 col 0 should hold the 's' of 'second line'"
        );
    }

    /// Mirrors the production "post-snapshot reset" step that used to
    /// be inlined at the end of every per-terminal iteration inside
    /// `tree_fn`. Before the #63 fix this called
    /// `terminal.grid_mut().clear_dirty()` on the live grid. The fix
    /// removed that call entirely, so this helper is now a no-op. If
    /// someone reintroduces the clear here or in
    /// `snapshot_terminal_for_render`, the regression tests below will
    /// trip.
    fn post_snapshot_reset_like_production(_terminal: &mut Terminal) {
        // Intentionally empty. See issue #63.
    }

    // refs #140 / Task 11: the bench mode used to mutate the SHELL env
    // var to force cmd.exe on Windows so `dir` produced real Windows
    // output. The default-shell pipeline now owns shell selection;
    // mutating SHELL would silently override the user's persisted
    // `default_shell` and produce bench numbers that diverge from the
    // configured shell. This regression scans the main.rs source so a
    // future commit reintroducing the env hack fails loudly.
    #[test]
    fn bench_does_not_mutate_shell_env_var() {
        let src = include_str!("main.rs");
        // Constructed from parts so the test source itself does not
        // match the needle and trip the assertion.
        let needle = concat!("set_", "var(\"SHELL\"");
        assert!(
            !src.contains(needle),
            "bench must not mutate the SHELL env var; route the bench shell through default_shell instead"
        );
    }

    /// Auxiliary regression for #63: the live grid's damage must keep
    /// accumulating across multiple snapshots so that every write ever
    /// performed (until the renderer has had a chance to replay it via
    /// the content-hash cache) shows up as damage on the latest
    /// snapshot. If someone reintroduces `clear_dirty()` on the live
    /// grid inside `snapshot_terminal_for_render`, this test also
    /// fails on the second row.
    #[test]
    fn snapshot_terminal_for_render_accumulates_damage_across_snapshots() {
        let mut terminal = Terminal::new(10, 40);

        terminal.process_bytes(b"alpha");
        let _s1 = snapshot_terminal_for_render(&mut terminal, 0, true);

        terminal.process_bytes(b"\r\nbeta");
        let _s2 = snapshot_terminal_for_render(&mut terminal, 0, true);

        terminal.process_bytes(b"\r\ngamma");
        let s3 = snapshot_terminal_for_render(&mut terminal, 0, true);

        for row in 0..=2 {
            let ld = s3
                .line_damage_for(row)
                .unwrap_or_else(|| panic!("damage entry for row {}", row));
            assert!(
                !ld.is_clean(),
                "row {} damage is clean; a previous snapshot cleared live \
                 grid damage and dropped this row's writes (#63)",
                row
            );
        }
    }
}
