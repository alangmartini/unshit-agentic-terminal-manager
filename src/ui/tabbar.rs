use unshit::core::element::*;
use unshit::core::event::DragPhase;

use crate::state::{
    dispatch, mutate_close_tab, mutate_with, SharedState, TabStatus, TerminalTab, UiSnapshot,
};
use crate::ui::icons::*;

pub fn build_tabbar(state: &UiSnapshot, shared: &SharedState) -> ElementDef {
    let mut tabs = ElementDef::new(Tag::Div).with_class("tabs").with_id("tabs");
    let placeholder_index = pane_drag_insertion_index(state);
    let dragging_source_id = state.drag.dragged_tab().map(|s| s.to_string());
    for (index, tab) in state.tabs.iter().enumerate() {
        if Some(index) == placeholder_index {
            tabs = tabs.with_child(build_tab_drop_placeholder());
        }
        let is_dragging = dragging_source_id.as_deref() == Some(tab.id.as_str());
        tabs = tabs.with_child(build_tab(
            index,
            tab,
            index == state.active_tab,
            is_dragging,
            shared,
        ));
    }
    if placeholder_index == Some(state.tabs.len()) {
        tabs = tabs.with_child(build_tab_drop_placeholder());
    }
    let add_state = shared.clone();
    tabs = tabs.with_child(
        ElementDef::new(Tag::Button)
            .with_class("tab-add")
            .with_key("tab-add")
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

    let resize_state = shared.clone();
    ElementDef::new(Tag::Div)
        .with_class("tabbar")
        .with_child(tabs)
        .with_child(actions)
        .on_resize(move |w, h| {
            mutate_with(&resize_state, |st| {
                // The tabbar lives immediately to the right of the
                // sidebar and its 6px resizer, below the titlebar. The
                // framework's on_resize reports w/h in *physical*
                // pixels, but sidebar_width and the CSS constants are
                // in logical pixels, so divide w/h to make the rect compose
                // with cursor coords that are also normalised to CSS.
                const TITLEBAR_HEIGHT: f32 = 34.0;
                const SIDEBAR_RESIZER_WIDTH: f32 = 6.0;
                let sf = st.scale_factor.max(1e-3);
                st.tabbar_rect = crate::drag::Rect {
                    x: st.sidebar_width + SIDEBAR_RESIZER_WIDTH,
                    y: TITLEBAR_HEIGHT,
                    width: w / sf,
                    height: h / sf,
                };
            });
        })
}

/// When a pane drag is active and the cursor is over the tab bar,
/// returns the slot at which a dropped tab would be inserted so the
/// renderer can place a visual placeholder there. `None` otherwise.
fn pane_drag_insertion_index(state: &UiSnapshot) -> Option<usize> {
    let (cursor_x, cursor_y) = state.drag.cursor()?;
    crate::drag::resolve_tabbar_drop(cursor_x, cursor_y, state.tabbar_rect, state.tabs.len())
}

fn build_tab_drop_placeholder() -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("tab-drop-placeholder")
        .with_key("tab-drop-placeholder")
}

fn build_tab(
    index: usize,
    tab: &TerminalTab,
    is_active: bool,
    is_dragging_source: bool,
    shared: &SharedState,
) -> ElementDef {
    let status_class = match tab.status {
        TabStatus::Running => "running",
        TabStatus::Idle => "idle",
        TabStatus::Stopped => "stopped",
    };

    // Stable reconcile key derived from the tab's persistent id. Without
    // this the framework's positional match shuffles every tab + the
    // add button when a tab is inserted or removed mid-strip.
    let mut btn = ElementDef::new(Tag::Button)
        .with_class("tab")
        .with_key(format!("tab:{}", tab.id));
    if is_active {
        btn = btn.with_class("active");
    }
    if is_dragging_source {
        btn = btn.with_class("dragging");
    }
    let activate_state = shared.clone();
    btn = btn.on_click(move || {
        mutate_with(&activate_state, |st| {
            dispatch(st, &format!("tab.switch:{}", index));
        });
    });

    // Drag source for F1: the user grabs a tab label and drops it on
    // a pane zone (split) or the tab bar (reorder). The framework's
    // 4px threshold keeps a regular click firing on_click instead.
    let drag_state = shared.clone();
    let drag_tab_id = tab.id.clone();
    btn = btn.on_drag(move |ev| match ev.phase {
        DragPhase::Start => {
            mutate_with(&drag_state, |st| {
                dispatch(
                    st,
                    &format!("drag.start_tab:{}:{}:{}", drag_tab_id, ev.x, ev.y),
                );
            });
        }
        DragPhase::Update => {
            mutate_with(&drag_state, |st| {
                dispatch(st, &format!("drag.update:{}:{}", ev.x, ev.y));
            });
        }
        DragPhase::End => {
            mutate_with(&drag_state, |st| {
                dispatch(st, &format!("drag.update:{}:{}", ev.x, ev.y));
                dispatch(st, "drag.end");
            });
        }
    });

    let close_state = shared.clone();
    btn.with_child(
        ElementDef::new(Tag::Span)
            .with_class("tab-status")
            .with_class(status_class.to_string()),
    )
    .with_child(
        ElementDef::new(Tag::Span)
            .with_class("tab-name")
            .with_text(tab.name.clone()),
    )
    .with_child(
        ElementDef::new(Tag::Span)
            .with_class("tab-subtitle")
            .with_text(tab.subtitle.clone()),
    )
    .with_child(
        ElementDef::new(Tag::Span)
            .with_class("tab-close")
            .with_text("\u{00D7}")
            .on_click(move || {
                mutate_with(&close_state, |st| mutate_close_tab(st, index));
            }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{seed_state, Pane, PaneId, SharedState, TabStatus, TerminalTab};
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
        let pane = Pane {
            id: PaneId(1),
            title: name.to_string(),
            subtitle: "bash".to_string(),
            pid: 0,
            cpu: 0.0,
        };
        TerminalTab {
            id: format!("t-{}", name),
            name: name.to_string(),
            subtitle: "bash".to_string(),
            status,
            panes: vec![vec![pane]],
            active_pane: PaneId(1),
            row_ratios: vec![1.0],
            col_ratios: vec![vec![1.0]],
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
        let el = build_tab(0, &tab, true, false, &shared);

        assert_eq!(el.tag, Tag::Button);
        assert!(has_class(&el, "tab"));
        assert!(has_class(&el, "active"));
    }

    #[test]
    fn tab_inactive() {
        let shared = make_shared();
        let tab = make_tab("shell", TabStatus::Running);
        let el = build_tab(0, &tab, false, false, &shared);

        assert!(has_class(&el, "tab"));
        assert!(!has_class(&el, "active"));
    }

    #[test]
    fn tab_status_running() {
        let shared = make_shared();
        let tab = make_tab("shell", TabStatus::Running);
        let el = build_tab(0, &tab, false, false, &shared);
        let status = find_by_class(&el, "tab-status").unwrap();
        assert!(has_class(status, "running"));
    }

    #[test]
    fn tab_status_idle() {
        let shared = make_shared();
        let tab = make_tab("vim", TabStatus::Idle);
        let el = build_tab(0, &tab, false, false, &shared);
        let status = find_by_class(&el, "tab-status").unwrap();
        assert!(has_class(status, "idle"));
    }

    #[test]
    fn tab_status_stopped() {
        let shared = make_shared();
        let tab = make_tab("done", TabStatus::Stopped);
        let el = build_tab(0, &tab, false, false, &shared);
        let status = find_by_class(&el, "tab-status").unwrap();
        assert!(has_class(status, "stopped"));
    }

    #[test]
    fn tab_shows_name_and_subtitle() {
        let shared = make_shared();
        let tab = make_tab("myshell", TabStatus::Running);
        let el = build_tab(0, &tab, false, false, &shared);

        let name_el = find_by_class(&el, "tab-name").unwrap();
        assert_eq!(text_of(name_el), Some("myshell"));

        let subtitle_el = find_by_class(&el, "tab-subtitle").unwrap();
        assert_eq!(text_of(subtitle_el), Some("bash"));
    }

    #[test]
    fn tab_has_close_button() {
        let shared = make_shared();
        let tab = make_tab("shell", TabStatus::Running);
        let el = build_tab(0, &tab, false, false, &shared);

        let close = find_by_class(&el, "tab-close").unwrap();
        assert_eq!(text_of(close), Some("\u{00D7}"));
    }

    #[test]
    fn tab_children_order() {
        let shared = make_shared();
        let tab = make_tab("shell", TabStatus::Running);
        let el = build_tab(0, &tab, false, false, &shared);

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
        let el = build_tab(0, &tab, false, false, &shared);
        assert!(el.on_click.is_some());
    }

    #[test]
    fn tab_close_has_click_handler() {
        let shared = make_shared();
        let tab = make_tab("shell", TabStatus::Running);
        let el = build_tab(0, &tab, false, false, &shared);
        let close = find_by_class(&el, "tab-close").unwrap();
        assert!(close.on_click.is_some());
    }

    #[test]
    fn tab_has_dragging_class_when_source_of_drag() {
        let shared = make_shared();
        let tab = make_tab("shell", TabStatus::Running);
        let el = build_tab(0, &tab, false, true, &shared);
        assert!(has_class(&el, "dragging"));
    }

    #[test]
    fn tabbar_marks_only_source_tab_as_dragging() {
        // Prepare a snapshot with two tabs and a DraggingTab state
        // pointing at the second one; only that tab should carry the
        // .dragging class so CSS can fade the source while the ghost
        // follows the cursor.
        use crate::state::{AppState, Pane, PaneId, TabStatus, TerminalTab};
        let shared: SharedState = Arc::new(Mutex::new(seed_state()));
        let mut state = seed_state();
        let second_pane = Pane {
            id: PaneId(999),
            title: "second".into(),
            subtitle: "bash".into(),
            pid: 0,
            cpu: 0.0,
        };
        let second_tab = TerminalTab {
            id: "t-other".into(),
            name: "other".into(),
            subtitle: "bash".into(),
            status: TabStatus::Running,
            panes: vec![vec![second_pane]],
            active_pane: PaneId(999),
            row_ratios: vec![1.0],
            col_ratios: vec![vec![1.0]],
        };
        state.tabs.push(second_tab);
        state.drag = crate::drag::DragState::DraggingTab {
            source_tab: "t-other".into(),
            cursor_x: 0.0,
            cursor_y: 0.0,
        };
        let snap = state.ui_snapshot();
        let bar = build_tabbar(&snap, &shared);
        let tabs_row = find_by_id(&bar, "tabs").unwrap();
        let tab_buttons: Vec<&ElementDef> = tabs_row
            .children
            .iter()
            .filter(|c| has_class(c, "tab"))
            .collect();
        assert_eq!(tab_buttons.len(), 2);
        assert!(!has_class(tab_buttons[0], "dragging"));
        assert!(has_class(tab_buttons[1], "dragging"));
        // Silence unused warning on AppState alias import.
        let _: fn() -> AppState = seed_state;
    }

    // -- tabbar drop-target geometry tracking --------------------------------

    /// The tabbar root must carry an `on_resize` handler that keeps
    /// `tabbar_rect.width` / `tabbar_rect.height` in sync with its real
    /// size. Without this, the pane-drag drop hit-test has no target.
    #[test]
    fn tabbar_on_resize_updates_tabbar_rect() {
        let shared = make_shared();
        let snap = shared.lock().unwrap().ui_snapshot();
        let el = build_tabbar(&snap, &shared);

        let resize = el
            .on_resize
            .as_ref()
            .expect("tabbar must have on_resize for drop-zone tracking")
            .clone();
        resize(720.0, 38.0);

        let guard = shared.lock().unwrap();
        assert_eq!(guard.tabbar_rect.width, 720.0);
        assert_eq!(guard.tabbar_rect.height, 38.0);
    }

    /// The tabbar lives to the right of the sidebar + its resizer. When
    /// on_resize fires, the stored rect must reflect that absolute x/y
    /// offset so cursor_x (in window coordinates) hit-tests correctly.
    #[test]
    fn tabbar_on_resize_records_absolute_origin() {
        let shared = make_shared();
        {
            let mut guard = shared.lock().unwrap();
            guard.sidebar_width = 252.0;
        }
        let snap = shared.lock().unwrap().ui_snapshot();
        let el = build_tabbar(&snap, &shared);
        let resize = el.on_resize.as_ref().unwrap().clone();
        resize(548.0, 38.0);

        let guard = shared.lock().unwrap();
        assert!(
            (guard.tabbar_rect.x - 258.0).abs() < 0.5,
            "tabbar_rect.x should sit after sidebar + resizer (~258), got {}",
            guard.tabbar_rect.x
        );
        assert!(
            (guard.tabbar_rect.y - 34.0).abs() < 0.5,
            "tabbar_rect.y should be titlebar height (~34), got {}",
            guard.tabbar_rect.y
        );
    }

    /// While a pane drag is in progress and the cursor is within the
    /// tabbar, an insertion placeholder element must be rendered at the
    /// computed slot so the user sees where the new tab will land.
    #[test]
    fn tabbar_renders_insertion_placeholder_during_pane_drag() {
        let shared = make_shared();
        {
            let mut guard = shared.lock().unwrap();
            guard.tabs = vec![
                make_tab("a", TabStatus::Running),
                make_tab("b", TabStatus::Running),
                make_tab("c", TabStatus::Running),
            ];
            guard.active_tab = 0;
            guard.tabbar_rect = crate::drag::Rect {
                x: 0.0,
                y: 34.0,
                width: 600.0,
                height: 38.0,
            };
            guard.drag = crate::drag::DragState::DraggingPane {
                pane: PaneId(1),
                cursor_x: 400.0,
                cursor_y: 50.0,
            };
        }
        let snap = shared.lock().unwrap().ui_snapshot();
        let el = build_tabbar(&snap, &shared);
        let tabs = &el.children[0];
        assert!(
            find_by_class(tabs, "tab-drop-placeholder").is_some(),
            "insertion placeholder must appear during a pane drag over the tab bar"
        );
    }

    /// When no drag is in progress, the placeholder is absent so it
    /// doesn't take up space or interfere with click targets.
    #[test]
    fn tabbar_has_no_placeholder_when_idle() {
        let shared = make_shared();
        let snap = shared.lock().unwrap().ui_snapshot();
        let el = build_tabbar(&snap, &shared);
        assert!(
            find_by_class(&el, "tab-drop-placeholder").is_none(),
            "placeholder must only appear during an active pane drag"
        );
    }

    /// Without stable keys the framework's reconciler matches tab-bar
    /// children by position. Inserting a new tab then shuffles every
    /// subsequent element (tabs, placeholder, add button) into the
    /// wrong slot on the next render, causing the squished/misaligned
    /// rendering we saw after pane extraction. Each tab needs a key
    /// tied to its stable `id`, and the add button needs its own.
    #[test]
    fn tabs_have_unique_keys_for_reconcile_stability() {
        let shared = make_shared();
        {
            let mut guard = shared.lock().unwrap();
            guard.tabs = vec![
                make_tab("shell", TabStatus::Running),
                make_tab("vim", TabStatus::Idle),
                make_tab("build", TabStatus::Stopped),
            ];
        }
        // Override the tab ids so collisions aren't hidden by
        // coincidence; make_tab uses a formatted label so "t-shell"
        // etc. are naturally unique, but we want the assertion explicit.
        let snap = shared.lock().unwrap().ui_snapshot();
        let el = build_tabbar(&snap, &shared);
        let tabs_el = &el.children[0];

        let mut seen = std::collections::HashSet::new();
        for (i, child) in tabs_el.children.iter().enumerate() {
            let key = child
                .key
                .clone()
                .unwrap_or_else(|| panic!("child {i} missing reconcile key"));
            assert!(seen.insert(key.clone()), "duplicate reconcile key: {}", key);
        }
    }

    /// A pane drag whose cursor sits outside the tab bar (e.g. still
    /// hovering the pane body) should not render the placeholder.
    #[test]
    fn tabbar_has_no_placeholder_when_drag_outside_tabbar() {
        let shared = make_shared();
        {
            let mut guard = shared.lock().unwrap();
            guard.tabbar_rect = crate::drag::Rect {
                x: 0.0,
                y: 34.0,
                width: 600.0,
                height: 38.0,
            };
            guard.drag = crate::drag::DragState::DraggingPane {
                pane: PaneId(1),
                cursor_x: 300.0,
                cursor_y: 400.0,
            };
        }
        let snap = shared.lock().unwrap().ui_snapshot();
        let el = build_tabbar(&snap, &shared);
        assert!(
            find_by_class(&el, "tab-drop-placeholder").is_none(),
            "placeholder must not render when cursor is outside the tab bar"
        );
    }
}
