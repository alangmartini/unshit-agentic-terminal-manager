pub mod bridge;
pub mod pty;
pub mod state;
pub mod terminal;
pub mod ui;

use std::sync::Arc;

use unshit::app::{App, AppConfig, FontSource};
use unshit::core::element::*;
use unshit::core::event::DragPhase;
use unshit::core::style::parse::StyleDeclaration;
use unshit::core::style::types::Dimension;

use crate::state::{
    dispatch, mutate_with, resize_all_terminals, seed_state, SharedState, UiSnapshot,
    MAX_SIDEBAR_WIDTH, MIN_SIDEBAR_WIDTH,
};
use crate::ui::settings::build_settings_modal;
use crate::ui::sidebar::{build_ctx_menu_overlay, build_sidebar};
use crate::ui::statusbar::build_statusbar;
use crate::ui::tabbar::build_tabbar;
use crate::ui::terminal_grid::build_terminal_grid;
use crate::ui::titlebar::build_titlebar;

const STYLES: &str = include_str!("../assets/styles.css");

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
                .on_click(move || {
                    mutate_with(&s, |st| dispatch(st, "modal.close"));
                })
                .with_child(build_settings_modal(snap, shared)),
        );
    }

    ElementTree {
        root: root.with_child(build_ctx_menu_overlay(snap, shared)),
    }
}

fn user_shortcut_bindings() -> Vec<(String, String)> {
    vec![
        ("Ctrl+T".to_string(), "tab.new".to_string()),
        ("Ctrl+W".to_string(), "pane.close".to_string()),
        ("Ctrl+D".to_string(), "pane.split_right".to_string()),
        ("Ctrl+Shift+D".to_string(), "pane.split_down".to_string()),
        ("Ctrl+B".to_string(), "sidebar.toggle".to_string()),
        ("Ctrl+,".to_string(), "modal.open".to_string()),
        ("Ctrl+K".to_string(), "palette.toggle".to_string()),
        ("Ctrl+Shift+P".to_string(), "palette.toggle".to_string()),
        ("Escape".to_string(), "modal.close".to_string()),
        ("Ctrl+1".to_string(), "tab.switch:0".to_string()),
        ("Ctrl+2".to_string(), "tab.switch:1".to_string()),
        ("Ctrl+3".to_string(), "tab.switch:2".to_string()),
        ("Ctrl+4".to_string(), "tab.switch:3".to_string()),
        ("Ctrl+5".to_string(), "tab.switch:4".to_string()),
        ("Ctrl+6".to_string(), "tab.switch:5".to_string()),
        ("Ctrl+7".to_string(), "tab.switch:6".to_string()),
        ("Ctrl+8".to_string(), "tab.switch:7".to_string()),
        ("Ctrl+9".to_string(), "tab.switch:8".to_string()),
        ("Ctrl+=".to_string(), "font.inc".to_string()),
        ("Ctrl+Shift+=".to_string(), "font.inc".to_string()),
        ("Ctrl+-".to_string(), "font.dec".to_string()),
        ("Ctrl+Tab".to_string(), "tab.next".to_string()),
        ("Ctrl+Shift+Tab".to_string(), "tab.prev".to_string()),
    ]
}

fn main() {
    // Guard against ghost handles (#32): ensure the process exits
    // immediately on Ctrl+C or panic so spawn_blocking reader tasks
    // (bridge.rs) cannot keep the .exe locked on Windows (os error 32).
    let default_panic = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        default_panic(info);
        std::process::exit(1);
    }));
    ctrlc::set_handler(|| {
        std::process::exit(0);
    })
    .expect("failed to set Ctrl+C handler");

    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or(
            "info,wgpu_hal=error,wgpu_core=error,naga=error,unshit_app::app=error",
        ),
    )
    .init();

    let shared: SharedState = Arc::new(std::sync::Mutex::new(seed_state()));

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

    // Spawn initial PTY eagerly. This is load-bearing: without a terminal
    // the CellGrid doesn't exist, the renderer can't publish metrics, and
    // the PTY never gets spawned (deadlock). Estimate dimensions from the
    // window size minus CSS chrome so the shell greeting is formatted for
    // roughly the right width. on_cell_metrics corrects to exact values on
    // the first frame.
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
        let cwd = crate::state::active_workspace_cwd(&guard);
        let terminal = crate::terminal::Terminal::new(init_rows as usize, init_cols as usize);
        guard.terminals.insert(pane_id, terminal);
        match guard
            .pty_manager
            .spawn_in(pane_id, init_cols, init_rows, cwd.as_deref())
        {
            Ok(reader) => {
                crate::bridge::register_reader(pane_id, reader);
            }
            Err(e) => {
                log::error!("failed to spawn initial PTY: {}", e);
                if let Some(t) = guard.terminals.get_mut(&pane_id) {
                    t.process_bytes(format!("Failed to spawn shell: {}\r\n", e).as_bytes());
                }
            }
        }
    }

    let tree_shared = shared.clone();
    let command_shared = shared.clone();
    let metrics_shared = shared.clone();
    let scale_shared = shared.clone();
    let close_shared = shared.clone();
    let sub_shared = shared.clone();

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
                let mut guard = command_shared.lock().expect("state mutex poisoned");
                dispatch(&mut guard, command)
            })),
            // Approach 1: on_cell_metrics fires once after the first render
            // publishes valid cell dimensions. Resize all PTYs immediately.
            on_scale_factor: Some(Arc::new(move |scale: f32| {
                let mut guard = scale_shared.lock().expect("state mutex poisoned");
                guard.scale_factor = scale;
            })),
            on_close: Some(Arc::new(move || {
                // Use .lock().ok() instead of .expect() so a poisoned mutex
                // (from a panic on another thread) does not prevent us from
                // reaching process::exit below.
                if let Ok(mut guard) = close_shared.lock() {
                    guard.pty_manager.destroy_all();
                    guard.terminals.clear();
                }
                // Force-exit the process. Without this, tokio's Runtime::drop
                // blocks indefinitely waiting for spawn_blocking reader tasks
                // (bridge.rs) that are stuck on pipe reads. The readers hold
                // cloned PTY pipe handles that keep the .exe locked on Windows
                // (os error 32), preventing cargo from rebuilding.
                std::process::exit(0);
            })),
            on_cell_metrics: Some(Arc::new(move |cell_w: f32, cell_h: f32| {
                use unshit::core::cell_grid::CellGrid;
                let mut guard = metrics_shared.lock().expect("state mutex poisoned");
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
            ..Default::default()
        },
        move || {
            let guard = tree_shared.lock().expect("state mutex poisoned");
            let snap = guard.ui_snapshot();
            let active_id = guard.active_pane.0;
            let grids: std::collections::HashMap<u32, unshit::core::cell_grid::CellGrid> = guard
                .terminals
                .iter()
                .map(|(&id, t)| {
                    let mut grid = t.display_grid();
                    if id != active_id {
                        grid.set_cursor_visible(false);
                    }
                    (id, grid)
                })
                .collect();
            drop(guard);
            build_tree(&snap, &tree_shared, &grids)
        },
    );

    // Set up PTY output subscriptions.
    app.set_subscriptions(move || bridge::build_subscriptions(&sub_shared));

    app.run();
}
