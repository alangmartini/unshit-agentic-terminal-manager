pub mod bridge;
pub mod pty;
pub mod state;
pub mod terminal;
pub mod ui;

use std::sync::Arc;

use unshit::app::{App, AppConfig, FontSource};
use unshit::core::element::*;

use crate::state::{
    dispatch, mutate_with, resize_all_terminals, seed_state, SharedState, UiSnapshot,
};
use crate::ui::settings::build_settings_modal;
use crate::ui::sidebar::build_sidebar;
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
    let mut modal_overlay = ElementDef::new(Tag::Div)
        .with_class("modal-overlay")
        .with_id("settings-modal");
    if snap.settings_open {
        modal_overlay = modal_overlay.with_class("open");
    }
    {
        let s = shared.clone();
        modal_overlay = modal_overlay.on_click(move || {
            mutate_with(&s, |st| dispatch(st, "modal.close"));
        });
    }
    modal_overlay = modal_overlay.with_child(build_settings_modal(snap, shared));

    ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("app")
            .with_child(build_titlebar(shared))
            .with_child(
                ElementDef::new(Tag::Div)
                    .with_class("layout")
                    .with_child(build_sidebar(snap, shared))
                    .with_child(
                        ElementDef::new(Tag::Div)
                            .with_class("content")
                            .with_class("role-main")
                            .with_child(build_tabbar(snap, shared))
                            .with_child(build_terminal_grid(snap, shared, grids))
                            .with_child(build_statusbar(snap)),
                    ),
            )
            .with_child(modal_overlay),
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
    env_logger::Builder::from_env(
        env_logger::Env::default()
            .default_filter_or("info,wgpu_hal=error,wgpu_core=error,naga=error"),
    )
    .init();

    let shared: SharedState = Arc::new(std::sync::Mutex::new(seed_state()));

    // Spawn initial PTY for the default pane eagerly at 80x24.
    // The blink subscription and on_cell_metrics callback will correct
    // the dimensions once the renderer publishes real cell metrics.
    {
        let mut guard = shared.lock().unwrap();
        let pane_id = guard.active_pane.0;
        let (cols, rows) = (80u16, 24u16);
        let terminal = crate::terminal::Terminal::new(rows as usize, cols as usize);
        guard.terminals.insert(pane_id, terminal);
        match guard.pty_manager.spawn(pane_id, cols, rows) {
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
                let mut guard = close_shared.lock().expect("state mutex poisoned");
                let ids: Vec<u32> = guard.terminals.keys().copied().collect();
                for id in ids {
                    guard.pty_manager.destroy(id);
                }
                guard.terminals.clear();
            })),
            on_cell_metrics: Some(Arc::new(move |cell_w: f32, cell_h: f32| {
                use unshit::core::cell_grid::CellGrid;
                let (cols, rows) = CellGrid::take_pending_resize().unwrap_or_else(|| {
                    let cols = (1280.0 / cell_w).max(1.0) as u16;
                    let rows = (800.0 / cell_h).max(1.0) as u16;
                    (cols, rows)
                });
                log::info!(
                    "on_cell_metrics: cell={}x{} -> resize all PTYs to {}x{}",
                    cell_w, cell_h, cols, rows
                );
                let mut guard = metrics_shared.lock().expect("state mutex poisoned");
                resize_all_terminals(&mut guard, cols, rows);
            })),
            ..Default::default()
        },
        move || {
            let guard = tree_shared.lock().expect("state mutex poisoned");
            let snap = guard.ui_snapshot();
            let active_id = guard.active_pane.0;
            let win_focused = unshit::core::cell_grid::CellGrid::is_window_focused();
            let grids: std::collections::HashMap<u32, unshit::core::cell_grid::CellGrid> = guard
                .terminals
                .iter()
                .map(|(&id, t)| {
                    let mut grid = t.grid().clone();
                    if id != active_id || !win_focused {
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
