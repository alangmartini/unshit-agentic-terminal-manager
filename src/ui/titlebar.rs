use unshit::app::EventSink;
use unshit::core::element::*;

use crate::state::{
    dispatch, mutate_kill_all_terminals, mutate_with, resolve_close_action, CloseAction, MutexExt,
    SharedState, UiSnapshot,
};
use crate::ui::icons::*;

pub fn build_titlebar(
    state: &UiSnapshot,
    shared: &SharedState,
    window_events: Option<EventSink>,
) -> ElementDef {
    let search_state = shared.clone();
    let sidebar_state = shared.clone();
    let settings_state = shared.clone();
    let close_state = shared.clone();
    let minimize_events = window_events.clone();
    let maximize_events = window_events;
    let workspace = state
        .workspaces
        .get(state.active_workspace)
        .or_else(|| state.workspaces.first());
    let workspace_name = workspace
        .map(|ws| ws.name.as_str())
        .unwrap_or("workspace")
        .to_string();
    let branch = workspace
        .and_then(|ws| ws.git_branch.as_deref())
        .unwrap_or("no branch")
        .to_string();

    ElementDef::new(Tag::Div)
        .with_class("titlebar")
        .with_class("role-header")
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("titlebar-left")
                .with_child(
                    ElementDef::new(Tag::Div)
                        .with_class("brand")
                        .with_child(
                            ElementDef::new(Tag::Span)
                                .with_class("brand-mark")
                                .with_child(svg_icon(icon_brand_chevron())),
                        )
                        .with_child(
                            ElementDef::new(Tag::Span)
                                .with_class("brand-name")
                                .with_text("terminal.mgr"),
                        ),
                )
                .with_child(
                    ElementDef::new(Tag::Div)
                        .with_class("titlebar-breadcrumb")
                        .with_child(
                            ElementDef::new(Tag::Span)
                                .with_class("crumb")
                                .with_text("workspaces"),
                        )
                        .with_child(
                            ElementDef::new(Tag::Span)
                                .with_class("sep")
                                .with_class("crumb-sep")
                                .with_text("/"),
                        )
                        .with_child(
                            ElementDef::new(Tag::Span)
                                .with_class("crumb")
                                .with_class("amber")
                                .with_text(workspace_name),
                        )
                        .with_child(
                            ElementDef::new(Tag::Span)
                                .with_class("sep")
                                .with_text("\u{00B7}"),
                        )
                        .with_child(
                            ElementDef::new(Tag::Span)
                                .with_class("crumb")
                                .with_class("sage")
                                .with_text(format!("({branch})")),
                        ),
                ),
        )
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("titlebar-right")
                .with_child(
                    ElementDef::new(Tag::Button)
                        .with_class("pill-btn")
                        .with_class("tm-search")
                        .on_click(move || {
                            mutate_with(&search_state, |st| dispatch(st, "palette.toggle"));
                        })
                        .with_child(svg_icon(icon_search()))
                        .with_child(ElementDef::new(Tag::Span).with_text("find session, command"))
                        .with_child(
                            ElementDef::new(Tag::Span)
                                .with_class("kbd")
                                .with_text("Ctrl K"),
                        ),
                )
                .with_child(
                    ElementDef::new(Tag::Div)
                        .with_class("tm-tb-right")
                        .with_child(
                            ElementDef::new(Tag::Button)
                                .with_class("icon-btn")
                                .with_class("tight")
                                .on_click(move || {
                                    mutate_with(&sidebar_state, |st| {
                                        dispatch(st, "sidebar.toggle")
                                    });
                                })
                                .with_child(svg_icon(icon_sidebar_toggle())),
                        )
                        .with_child(
                            ElementDef::new(Tag::Button)
                                .with_class("icon-btn")
                                .with_class("tight")
                                .on_click(move || {
                                    mutate_with(&settings_state, |st| dispatch(st, "modal.open"));
                                })
                                .with_child(svg_icon(icon_settings())),
                        ),
                )
                .with_child(
                    ElementDef::new(Tag::Div)
                        .with_class("tm-win-controls")
                        .with_child(
                            ElementDef::new(Tag::Button)
                                .with_class("win-btn")
                                .on_click(move || {
                                    if let Some(sink) = &minimize_events {
                                        let _ = sink.minimize_window();
                                    }
                                })
                                .with_child(svg_icon(icon_window_minimize())),
                        )
                        .with_child(
                            ElementDef::new(Tag::Button)
                                .with_class("win-btn")
                                .on_click(move || {
                                    if let Some(sink) = &maximize_events {
                                        let _ = sink.toggle_maximize_window();
                                    }
                                })
                                .with_child(svg_icon(icon_window_maximize())),
                        )
                        .with_child(
                            ElementDef::new(Tag::Button)
                                .with_class("win-btn")
                                .with_class("win-close")
                                .on_click(move || {
                                    let action = {
                                        let mut guard = close_state.lock_recover();
                                        resolve_close_action(&mut guard)
                                    };
                                    match action {
                                        CloseAction::Prompt => {}
                                        CloseAction::KeepRunning => {
                                            let mut guard = close_state.lock_recover();
                                            guard.terminals.clear();
                                            crate::shutdown_now();
                                        }
                                        CloseAction::KillAll => {
                                            let mut guard = close_state.lock_recover();
                                            mutate_kill_all_terminals(&mut guard);
                                            crate::shutdown_now();
                                        }
                                    }
                                })
                                .with_child(svg_icon(icon_close())),
                        ),
                ),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{seed_state, SharedState};
    use std::sync::{Arc, Mutex};
    use unshit_test::TestHarness;

    fn test_shared() -> SharedState {
        Arc::new(Mutex::new(seed_state()))
    }

    fn test_snapshot() -> UiSnapshot {
        seed_state().ui_snapshot()
    }

    #[test]
    fn build_titlebar_does_not_panic() {
        let shared = test_shared();
        let snap = test_snapshot();
        let _elem = build_titlebar(&snap, &shared, None);
    }

    #[test]
    fn build_titlebar_returns_div() {
        let shared = test_shared();
        let snap = test_snapshot();
        let elem = build_titlebar(&snap, &shared, None);
        assert!(matches!(elem.tag, Tag::Div));
    }

    #[test]
    fn build_titlebar_has_children() {
        let shared = test_shared();
        let snap = test_snapshot();
        let elem = build_titlebar(&snap, &shared, None);
        assert_eq!(elem.children.len(), 2);
    }

    #[test]
    fn titlebar_has_correct_classes() {
        let shared = test_shared();
        let snap = test_snapshot();
        let el = build_titlebar(&snap, &shared, None);
        assert!(el.classes.contains(&"titlebar".to_string()));
        assert!(el.classes.contains(&"role-header".to_string()));
    }

    #[test]
    fn titlebar_left_has_brand_and_breadcrumb() {
        let shared = test_shared();
        let snap = test_snapshot();
        let el = build_titlebar(&snap, &shared, None);
        let left = &el.children[0];
        assert!(left.classes.contains(&"titlebar-left".to_string()));
        assert_eq!(left.children.len(), 2);
        assert!(left.children[0].classes.contains(&"brand".to_string()));
        assert!(left.children[1]
            .classes
            .contains(&"titlebar-breadcrumb".to_string()));
    }

    #[test]
    fn titlebar_right_has_search_actions_and_window_controls() {
        let shared = test_shared();
        let snap = test_snapshot();
        let el = build_titlebar(&snap, &shared, None);
        let right = &el.children[1];
        assert!(right.classes.contains(&"titlebar-right".to_string()));
        assert_eq!(right.children.len(), 3);
        assert!(right.children[0].classes.contains(&"tm-search".to_string()));
        assert!(right.children[1]
            .classes
            .contains(&"tm-tb-right".to_string()));
        assert!(right.children[2]
            .classes
            .contains(&"tm-win-controls".to_string()));
    }

    #[test]
    fn search_button_click_toggles_palette() {
        let shared = test_shared();
        let snap = test_snapshot();
        let el = build_titlebar(&snap, &shared, None);
        let right = &el.children[1];
        let search_btn = &right.children[0];
        assert!(search_btn.classes.contains(&"pill-btn".to_string()));
        assert!(search_btn.on_click.is_some());
        (search_btn.on_click.as_ref().unwrap())();
        assert!(shared.lock().unwrap().palette_open);
    }

    #[test]
    fn sidebar_toggle_click_toggles_sidebar() {
        let shared = test_shared();
        let initial = shared.lock().unwrap().sidebar_collapsed;
        let snap = test_snapshot();
        let el = build_titlebar(&snap, &shared, None);
        let actions = &el.children[1].children[1];
        let sidebar_btn = &actions.children[0];
        assert!(sidebar_btn.on_click.is_some());
        (sidebar_btn.on_click.as_ref().unwrap())();
        let after = shared.lock().unwrap().sidebar_collapsed;
        assert_ne!(initial, after);
    }

    #[test]
    fn settings_button_opens_settings() {
        let shared = test_shared();
        let snap = test_snapshot();
        let el = build_titlebar(&snap, &shared, None);
        let actions = &el.children[1].children[1];
        let settings_btn = &actions.children[1];
        assert!(settings_btn.on_click.is_some());
        (settings_btn.on_click.as_ref().unwrap())();
        assert!(shared.lock().unwrap().settings_open);
    }

    #[test]
    fn brand_has_mark_and_name() {
        let shared = test_shared();
        let snap = test_snapshot();
        let el = build_titlebar(&snap, &shared, None);
        let brand = &el.children[0].children[0];
        assert_eq!(brand.children.len(), 2);
        assert!(brand.children[0]
            .classes
            .contains(&"brand-mark".to_string()));
        assert!(brand.children[1]
            .classes
            .contains(&"brand-name".to_string()));
    }

    #[test]
    fn breadcrumb_uses_design_system_segments() {
        let shared = test_shared();
        let snap = test_snapshot();
        let el = build_titlebar(&snap, &shared, None);
        let breadcrumb = &el.children[0].children[1];
        assert_eq!(breadcrumb.children.len(), 5);
        assert!(breadcrumb.children[0]
            .classes
            .contains(&"crumb".to_string()));
        assert!(breadcrumb.children[1].classes.contains(&"sep".to_string()));
        assert!(breadcrumb.children[2]
            .classes
            .contains(&"amber".to_string()));
        assert!(breadcrumb.children[4].classes.contains(&"sage".to_string()));
    }

    #[test]
    fn window_controls_are_present() {
        let shared = test_shared();
        let snap = test_snapshot();
        let el = build_titlebar(&snap, &shared, None);
        let controls = &el.children[1].children[2];
        assert!(controls.classes.contains(&"tm-win-controls".to_string()));
        assert_eq!(controls.children.len(), 3);
        assert!(
            controls.children[0].on_click.is_some(),
            "minimize button should be wired"
        );
        assert!(
            controls.children[1].on_click.is_some(),
            "maximize button should be wired"
        );
        assert!(controls.children[2]
            .classes
            .contains(&"win-close".to_string()));
    }

    #[test]
    fn window_controls_anchor_to_right_edge_with_stylesheet() {
        let shared = test_shared();
        let snap = test_snapshot();
        let tree_shared = shared.clone();
        let tree_snap = snap.clone();
        let mut harness = TestHarness::new(
            include_str!("../../assets/styles.css"),
            move || ElementTree {
                root: ElementDef::new(Tag::Div)
                    .with_class("app")
                    .with_child(build_titlebar(&tree_snap, &tree_shared, None)),
            },
            1280.0,
            720.0,
        );
        harness.step();

        let controls = harness
            .query(".tm-win-controls")
            .expect("window controls should render");
        let right_edge = controls.layout_rect.x + controls.layout_rect.width;
        assert!(
            right_edge >= 1279.0,
            "window controls should reach the titlebar right edge, got {:?}",
            controls.layout_rect
        );

        let close = harness.query(".win-close").expect("close button");
        assert_eq!(close.layout_rect.height, 34.0);
    }
}
