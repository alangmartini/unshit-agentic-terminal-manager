use unshit::core::element::*;

use crate::state::{
    dispatch, mutate_close_tab, mutate_with, SharedState, TabStatus, TerminalTab, UiSnapshot,
};
use crate::ui::icons::*;

pub fn build_tabbar(state: &UiSnapshot, shared: &SharedState) -> ElementDef {
    let mut tabs = ElementDef::new(Tag::Div).with_class("tabs").with_id("tabs");
    for (index, tab) in state.tabs.iter().enumerate() {
        tabs = tabs.with_child(build_tab(index, tab, index == state.active_tab, shared));
    }
    let add_state = shared.clone();
    tabs = tabs.with_child(
        ElementDef::new(Tag::Button)
            .with_class("tab-add")
            .on_click(move || {
                mutate_with(&add_state, |st| dispatch(st, "tab.new"));
            })
            .with_child(svg_icon(icon_plus())),
    );

    let split_h_state = shared.clone();
    let split_v_state = shared.clone();
    let settings_state = shared.clone();
    let actions = ElementDef::new(Tag::Div)
        .with_class("tabbar-actions")
        .with_child(
            ElementDef::new(Tag::Button)
                .with_class("icon-btn")
                .with_id("btn-split-h")
                .on_click(move || {
                    mutate_with(&split_h_state, |st| dispatch(st, "pane.split_right"));
                })
                .with_child(svg_icon(icon_split_h())),
        )
        .with_child(
            ElementDef::new(Tag::Button)
                .with_class("icon-btn")
                .with_id("btn-split-v")
                .on_click(move || {
                    mutate_with(&split_v_state, |st| dispatch(st, "pane.split_down"));
                })
                .with_child(svg_icon(icon_split_v())),
        )
        .with_child(
            ElementDef::new(Tag::Button)
                .with_class("icon-btn")
                .with_id("btn-grid")
                .with_child(svg_icon(icon_grid())),
        )
        .with_child(
            ElementDef::new(Tag::Button)
                .with_class("icon-btn")
                .with_id("btn-balance")
                .with_child(svg_icon(icon_balance())),
        )
        .with_child(ElementDef::new(Tag::Div).with_class("tabbar-divider"))
        .with_child(
            ElementDef::new(Tag::Button)
                .with_class("icon-btn")
                .with_id("btn-settings")
                .on_click(move || {
                    mutate_with(&settings_state, |st| dispatch(st, "modal.open"));
                })
                .with_child(svg_icon(icon_settings())),
        );

    ElementDef::new(Tag::Div).with_class("tabbar").with_child(tabs).with_child(actions)
}

fn build_tab(
    index: usize,
    tab: &TerminalTab,
    is_active: bool,
    shared: &SharedState,
) -> ElementDef {
    let status_class = match tab.status {
        TabStatus::Running => "running",
        TabStatus::Idle => "idle",
        TabStatus::Stopped => "stopped",
    };

    let mut btn = ElementDef::new(Tag::Button).with_class("tab");
    if is_active {
        btn = btn.with_class("active");
    }
    let activate_state = shared.clone();
    btn = btn.on_click(move || {
        mutate_with(&activate_state, |st| {
            if st.active_tab != index {
                st.active_tab = index;
            }
        });
    });

    let close_state = shared.clone();
    btn.with_child(
        ElementDef::new(Tag::Span).with_class("tab-status").with_class(status_class.to_string()),
    )
    .with_child(ElementDef::new(Tag::Span).with_class("tab-name").with_text(tab.name.clone()))
    .with_child(
        ElementDef::new(Tag::Span).with_class("tab-subtitle").with_text(tab.subtitle.clone()),
    )
    .with_child(
        ElementDef::new(Tag::Span).with_class("tab-close").with_text("\u{00D7}").on_click(
            move || {
                mutate_with(&close_state, |st| mutate_close_tab(st, index));
            },
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{seed_state, SharedState, TabStatus, TerminalTab};
    use std::sync::{Arc, Mutex};

    fn has_class(el: &ElementDef, class: &str) -> bool {
        el.classes.iter().any(|c| c == class)
    }

    fn text_of(el: &ElementDef) -> Option<&str> {
        match &el.content {
            ElementContent::Text(t) => Some(t.as_str()),
            _ => None,
        }
    }

    fn find_by_class<'a>(el: &'a ElementDef, class: &str) -> Option<&'a ElementDef> {
        if has_class(el, class) {
            return Some(el);
        }
        for child in &el.children {
            if let Some(found) = find_by_class(child, class) {
                return Some(found);
            }
        }
        None
    }

    fn find_by_id<'a>(el: &'a ElementDef, id: &str) -> Option<&'a ElementDef> {
        if el.id.as_deref() == Some(id) {
            return Some(el);
        }
        for child in &el.children {
            if let Some(found) = find_by_id(child, id) {
                return Some(found);
            }
        }
        None
    }

    fn make_shared() -> SharedState {
        Arc::new(Mutex::new(seed_state()))
    }

    fn make_tab(name: &str, status: TabStatus) -> TerminalTab {
        TerminalTab {
            id: format!("t-{}", name),
            name: name.to_string(),
            subtitle: "bash".to_string(),
            status,
        }
    }

    // -- build_tabbar --

    #[test]
    fn tabbar_structure() {
        let shared = make_shared();
        let state = shared.lock().unwrap().ui_snapshot();
        let el = build_tabbar(&state, &shared);

        assert!(has_class(&el, "tabbar"));
        assert_eq!(el.children.len(), 2); // tabs + actions
    }

    #[test]
    fn tabbar_tabs_section_has_correct_children() {
        let shared = make_shared();
        let state = shared.lock().unwrap().ui_snapshot();
        let el = build_tabbar(&state, &shared);
        let tabs = &el.children[0];
        assert!(has_class(tabs, "tabs"));
        assert_eq!(tabs.id.as_deref(), Some("tabs"));
        // seed_state has 1 tab + the add button
        assert_eq!(tabs.children.len(), state.tabs.len() + 1);
    }

    #[test]
    fn tabbar_has_add_button() {
        let shared = make_shared();
        let state = shared.lock().unwrap().ui_snapshot();
        let el = build_tabbar(&state, &shared);
        let tabs = &el.children[0];
        let add_btn = tabs.children.last().unwrap();
        assert!(has_class(add_btn, "tab-add"));
        assert_eq!(add_btn.tag, Tag::Button);
    }

    #[test]
    fn tabbar_actions_section() {
        let shared = make_shared();
        let state = shared.lock().unwrap().ui_snapshot();
        let el = build_tabbar(&state, &shared);
        let actions = &el.children[1];
        assert!(has_class(actions, "tabbar-actions"));
        // split-h, split-v, grid, balance, divider, settings = 6 children
        assert_eq!(actions.children.len(), 6);
    }

    #[test]
    fn tabbar_action_buttons_have_ids() {
        let shared = make_shared();
        let state = shared.lock().unwrap().ui_snapshot();
        let el = build_tabbar(&state, &shared);
        assert!(find_by_id(&el, "btn-split-h").is_some());
        assert!(find_by_id(&el, "btn-split-v").is_some());
        assert!(find_by_id(&el, "btn-grid").is_some());
        assert!(find_by_id(&el, "btn-balance").is_some());
        assert!(find_by_id(&el, "btn-settings").is_some());
    }

    #[test]
    fn tabbar_with_multiple_tabs() {
        let shared = make_shared();
        {
            let mut guard = shared.lock().unwrap();
            guard.tabs = vec![
                make_tab("shell", TabStatus::Running),
                make_tab("vim", TabStatus::Idle),
                make_tab("build", TabStatus::Stopped),
            ];
            guard.active_tab = 1;
        }
        let state = shared.lock().unwrap().ui_snapshot();
        let el = build_tabbar(&state, &shared);
        let tabs = &el.children[0];
        // 3 tabs + add button
        assert_eq!(tabs.children.len(), 4);
    }

    // -- build_tab --

    #[test]
    fn tab_active() {
        let shared = make_shared();
        let tab = make_tab("shell", TabStatus::Running);
        let el = build_tab(0, &tab, true, &shared);

        assert_eq!(el.tag, Tag::Button);
        assert!(has_class(&el, "tab"));
        assert!(has_class(&el, "active"));
    }

    #[test]
    fn tab_inactive() {
        let shared = make_shared();
        let tab = make_tab("shell", TabStatus::Running);
        let el = build_tab(0, &tab, false, &shared);

        assert!(has_class(&el, "tab"));
        assert!(!has_class(&el, "active"));
    }

    #[test]
    fn tab_status_running() {
        let shared = make_shared();
        let tab = make_tab("shell", TabStatus::Running);
        let el = build_tab(0, &tab, false, &shared);
        let status = find_by_class(&el, "tab-status").unwrap();
        assert!(has_class(status, "running"));
    }

    #[test]
    fn tab_status_idle() {
        let shared = make_shared();
        let tab = make_tab("vim", TabStatus::Idle);
        let el = build_tab(0, &tab, false, &shared);
        let status = find_by_class(&el, "tab-status").unwrap();
        assert!(has_class(status, "idle"));
    }

    #[test]
    fn tab_status_stopped() {
        let shared = make_shared();
        let tab = make_tab("done", TabStatus::Stopped);
        let el = build_tab(0, &tab, false, &shared);
        let status = find_by_class(&el, "tab-status").unwrap();
        assert!(has_class(status, "stopped"));
    }

    #[test]
    fn tab_shows_name_and_subtitle() {
        let shared = make_shared();
        let tab = make_tab("myshell", TabStatus::Running);
        let el = build_tab(0, &tab, false, &shared);

        let name_el = find_by_class(&el, "tab-name").unwrap();
        assert_eq!(text_of(name_el), Some("myshell"));

        let subtitle_el = find_by_class(&el, "tab-subtitle").unwrap();
        assert_eq!(text_of(subtitle_el), Some("bash"));
    }

    #[test]
    fn tab_has_close_button() {
        let shared = make_shared();
        let tab = make_tab("shell", TabStatus::Running);
        let el = build_tab(0, &tab, false, &shared);

        let close = find_by_class(&el, "tab-close").unwrap();
        assert_eq!(text_of(close), Some("\u{00D7}"));
    }

    #[test]
    fn tab_children_order() {
        let shared = make_shared();
        let tab = make_tab("shell", TabStatus::Running);
        let el = build_tab(0, &tab, false, &shared);

        // Expected order: status, name, subtitle, close
        assert_eq!(el.children.len(), 4);
        assert!(has_class(&el.children[0], "tab-status"));
        assert!(has_class(&el.children[1], "tab-name"));
        assert!(has_class(&el.children[2], "tab-subtitle"));
        assert!(has_class(&el.children[3], "tab-close"));
    }

    // -- closure invocation tests (cover on_click bodies) ----------------------

    #[test]
    fn add_button_click_adds_new_tab() {
        let shared = make_shared();
        let initial_count = shared.lock().unwrap().tabs.len();
        let state = shared.lock().unwrap().ui_snapshot();
        let el = build_tabbar(&state, &shared);
        let tabs_el = &el.children[0];
        // Last child in tabs is the add button
        let add_btn = tabs_el.children.last().unwrap();
        assert!(has_class(add_btn, "tab-add"));
        (add_btn.on_click.as_ref().unwrap())();
        assert_eq!(shared.lock().unwrap().tabs.len(), initial_count + 1);
    }

    #[test]
    fn tab_click_activates_tab() {
        let shared = make_shared();
        {
            let mut guard = shared.lock().unwrap();
            guard.tabs = vec![
                make_tab("shell", TabStatus::Running),
                make_tab("vim", TabStatus::Idle),
            ];
            guard.active_tab = 0;
        }
        let state = shared.lock().unwrap().ui_snapshot();
        let el = build_tabbar(&state, &shared);
        let tabs_el = &el.children[0];
        // Click the second tab (index 1)
        let tab_btn = &tabs_el.children[1];
        (tab_btn.on_click.as_ref().unwrap())();
        assert_eq!(shared.lock().unwrap().active_tab, 1);
    }

    #[test]
    fn tab_click_same_index_no_change() {
        let shared = make_shared();
        {
            let mut guard = shared.lock().unwrap();
            guard.tabs = vec![
                make_tab("shell", TabStatus::Running),
                make_tab("vim", TabStatus::Idle),
            ];
            guard.active_tab = 0;
        }
        let state = shared.lock().unwrap().ui_snapshot();
        let el = build_tabbar(&state, &shared);
        let tabs_el = &el.children[0];
        // Click the already active tab (index 0)
        let tab_btn = &tabs_el.children[0];
        (tab_btn.on_click.as_ref().unwrap())();
        assert_eq!(shared.lock().unwrap().active_tab, 0);
    }

    #[test]
    fn tab_close_click_removes_tab() {
        let shared = make_shared();
        {
            let mut guard = shared.lock().unwrap();
            guard.tabs = vec![
                make_tab("shell", TabStatus::Running),
                make_tab("vim", TabStatus::Idle),
            ];
            guard.active_tab = 0;
        }
        let state = shared.lock().unwrap().ui_snapshot();
        let el = build_tabbar(&state, &shared);
        let tabs_el = &el.children[0];
        // The first tab button has children; the close is the 4th child (index 3)
        let first_tab = &tabs_el.children[0];
        let close_span = &first_tab.children[3];
        assert!(has_class(close_span, "tab-close"));
        (close_span.on_click.as_ref().unwrap())();
        assert_eq!(shared.lock().unwrap().tabs.len(), 1);
    }

    #[test]
    fn split_h_button_has_click_handler() {
        let shared = make_shared();
        let state = shared.lock().unwrap().ui_snapshot();
        let el = build_tabbar(&state, &shared);
        let actions = &el.children[1];
        let split_h = &actions.children[0]; // btn-split-h
        assert_eq!(split_h.id.as_deref(), Some("btn-split-h"));
        assert!(split_h.on_click.is_some());
    }

    #[test]
    fn split_v_button_has_click_handler() {
        let shared = make_shared();
        let state = shared.lock().unwrap().ui_snapshot();
        let el = build_tabbar(&state, &shared);
        let actions = &el.children[1];
        let split_v = &actions.children[1]; // btn-split-v
        assert_eq!(split_v.id.as_deref(), Some("btn-split-v"));
        assert!(split_v.on_click.is_some());
    }

    #[test]
    fn settings_button_click_opens_modal() {
        let shared = make_shared();
        assert!(!shared.lock().unwrap().settings_open);
        let state = shared.lock().unwrap().ui_snapshot();
        let el = build_tabbar(&state, &shared);
        let actions = &el.children[1];
        let settings_btn = &actions.children[5]; // btn-settings (index 5)
        assert_eq!(settings_btn.id.as_deref(), Some("btn-settings"));
        (settings_btn.on_click.as_ref().unwrap())();
        assert!(shared.lock().unwrap().settings_open);
    }

    #[test]
    fn tab_add_button_has_click_handler() {
        let shared = make_shared();
        let state = shared.lock().unwrap().ui_snapshot();
        let el = build_tabbar(&state, &shared);
        let tabs_el = &el.children[0];
        let add_btn = tabs_el.children.last().unwrap();
        assert!(add_btn.on_click.is_some());
    }

    #[test]
    fn tab_has_click_handler_for_activation() {
        let shared = make_shared();
        let tab = make_tab("shell", TabStatus::Running);
        let el = build_tab(0, &tab, false, &shared);
        assert!(el.on_click.is_some());
    }

    #[test]
    fn tab_close_has_click_handler() {
        let shared = make_shared();
        let tab = make_tab("shell", TabStatus::Running);
        let el = build_tab(0, &tab, false, &shared);
        let close = find_by_class(&el, "tab-close").unwrap();
        assert!(close.on_click.is_some());
    }
}
