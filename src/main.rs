pub mod bridge;
pub mod pty;
pub mod state;
pub mod terminal;
pub mod ui;

use std::sync::Arc;
use unshit::app::{App, AppConfig, FontSource};
use unshit::core::element::*;
use crate::state::{dispatch, mutate_with, seed_state, SharedState, UiSnapshot};
use crate::ui::settings::build_settings_modal;
use crate::ui::sidebar::build_sidebar;
use crate::ui::statusbar::build_statusbar;
use crate::ui::tabbar::build_tabbar;
use crate::ui::terminal_grid::build_terminal_grid;
use crate::ui::titlebar::build_titlebar;

const STYLES: &str = include_str!("../assets/styles.css");

fn build_tree(snap: &UiSnapshot, shared: &SharedState, grids: &std::collections::HashMap<u32, unshit::core::cell_grid::CellGrid>) -> ElementTree {
    let mut modal_overlay = ElementDef::new(Tag::Div).with_class("modal-overlay").with_id("settings-modal");
    if snap.settings_open { modal_overlay = modal_overlay.with_class("open"); }
    { let s = shared.clone(); modal_overlay = modal_overlay.on_click(move || { mutate_with(&s, |st| dispatch(st, "modal.close")); }); }
    modal_overlay = modal_overlay.with_child(build_settings_modal(snap, shared));
    ElementTree {
        root: ElementDef::new(Tag::Div).with_class("app")
            .with_child(build_titlebar(shared))
            .with_child(ElementDef::new(Tag::Div).with_class("layout")
                .with_child(build_sidebar(snap, shared))
                .with_child(ElementDef::new(Tag::Div).with_class("content").with_class("role-main")
                    .with_child(build_tabbar(snap, shared))
                    .with_child(build_terminal_grid(snap, shared, grids))
                    .with_child(build_statusbar(snap))))
            .with_child(modal_overlay),
    }
}

fn user_shortcut_bindings() -> Vec<(String, String)> {
    vec![
        ("Ctrl+T".into(), "tab.new".into()), ("Ctrl+W".into(), "pane.close".into()),
        ("Ctrl+D".into(), "pane.split_right".into()), ("Ctrl+Shift+D".into(), "pane.split_down".into()),
        ("Ctrl+B".into(), "sidebar.toggle".into()), ("Ctrl+,".into(), "modal.open".into()),
        ("Ctrl+K".into(), "palette.toggle".into()), ("Ctrl+Shift+P".into(), "palette.toggle".into()),
        ("Escape".into(), "modal.close".into()),
        ("Ctrl+1".into(), "tab.switch:0".into()), ("Ctrl+2".into(), "tab.switch:1".into()),
        ("Ctrl+3".into(), "tab.switch:2".into()), ("Ctrl+4".into(), "tab.switch:3".into()),
        ("Ctrl+5".into(), "tab.switch:4".into()), ("Ctrl+6".into(), "tab.switch:5".into()),
        ("Ctrl+7".into(), "tab.switch:6".into()), ("Ctrl+8".into(), "tab.switch:7".into()),
        ("Ctrl+9".into(), "tab.switch:8".into()),
        ("Ctrl+=".into(), "font.inc".into()), ("Ctrl+Shift+=".into(), "font.inc".into()),
        ("Ctrl+-".into(), "font.dec".into()),
        ("Ctrl+Tab".into(), "tab.next".into()), ("Ctrl+Shift+Tab".into(), "tab.prev".into()),
    ]
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info,wgpu_hal=error,wgpu_core=error,naga=error")).init();
    let shared: SharedState = Arc::new(std::sync::Mutex::new(seed_state()));
    // PTY spawn deferred to blink subscription (issue #5)
    let tree_shared = shared.clone();
    let command_shared = shared.clone();
    let sub_shared = shared.clone();
    let mut app = App::new(
        AppConfig {
            title: "terminal manager".into(), width: 1280, height: 800, css: STYLES.into(),
            fonts: vec![FontSource::System("JetBrains Mono".into()), FontSource::System("Berkeley Mono".into()),
                FontSource::System("SF Mono".into()), FontSource::System("Menlo".into()), FontSource::System("Consolas".into())],
            user_shortcuts: user_shortcut_bindings(),
            on_command: Some(Arc::new(move |cmd: &str| -> bool { let mut g = command_shared.lock().expect("poisoned"); dispatch(&mut g, cmd) })),
            ..Default::default()
        },
        move || {
            let guard = tree_shared.lock().expect("poisoned");
            let snap = guard.ui_snapshot();
            let active_id = guard.active_pane.0;
            let focused = unshit::core::cell_grid::CellGrid::is_window_focused();
            let grids: std::collections::HashMap<u32, unshit::core::cell_grid::CellGrid> = guard.terminals.iter()
                .map(|(&id, t)| { let mut g = t.grid().clone(); if id != active_id || !focused { g.set_cursor_visible(false); } (id, g) }).collect();
            drop(guard);
            build_tree(&snap, &tree_shared, &grids)
        },
    );
    app.set_subscriptions(move || bridge::build_subscriptions(&sub_shared));
    app.run();
}
