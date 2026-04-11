pub mod bridge;
pub mod pty;
pub mod state;
pub mod terminal;
pub mod ui;

use std::sync::Arc;

use unshit::app::{App, AppConfig, FontSource};
use unshit::core::element::*;

use crate::state::{
    dispatch, mutate_with, seed_state, SharedState, UiSnapshot,
};
use crate::ui::settings::build_settings_modal;
use crate::ui::sidebar::build_sidebar;
use crate::ui::statusbar::build_statusbar;
use crate::ui::tabbar::build_tabbar;
use crate::ui::terminal_grid::build_terminal_grid;
use crate::ui::titlebar::build_titlebar;

const STYLES: &str = include_str!("../assets/styles.css");

fn build_tree(snap: &UiSnapshot, shared: &SharedState, grids: &std::collections::HashMap<u32, unshit::core::cell_grid::CellGrid>) -> ElementTree {
    let mut modal_overlay =
        ElementDef::new(Tag::Div).with_class("modal-overlay").with_id("settings-modal");
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

    // Pre-publish cell metrics with defaults (scale=1.0, ratio=0.6) so that
    // on_resize has non-zero values even before the renderer runs. The
    // on_scale_factor callback will re-publish with the real DPI and a
    // font-measured ratio once the window reports its scale factor. Issue #5.
    {
        let guard = shared.lock().unwrap();
        crate::state::pre_publish_cell_metrics(guard.scale_factor, guard.cell_width_ratio);
    }

    let tree_shared = shared.clone();
    let command_shared = shared.clone();
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
            on_scale_factor: Some(Arc::new(move |scale: f32| {
                let mut guard = scale_shared.lock().expect("state mutex poisoned");
                guard.scale_factor = scale;
                guard.cell_width_ratio =
                    crate::state::measure_cell_width_ratio_at(12.0 * scale);
                crate::state::pre_publish_cell_metrics(
                    guard.scale_factor,
                    guard.cell_width_ratio,
                );
            })),
            on_close: Some(Arc::new(move || {
                let mut guard = close_shared.lock().expect("state mutex poisoned");
                guard.pty_manager.destroy_all();
            })),
            ..Default::default()
        },
        move || {
            let guard = tree_shared.lock().expect("state mutex poisoned");
            let snap = guard.ui_snapshot();
            let active_id = guard.active_pane.0;
            let win_focused = unshit::core::cell_grid::CellGrid::is_window_focused();
            // Clone grids for rendering. This gives us an immutable snapshot.
            // Enforce cursor visibility: only the active pane shows a cursor,
            // and only when the OS window has focus.
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
    app.set_subscriptions(move || {
        bridge::build_subscriptions(&sub_shared)
    });

    app.run();
}
