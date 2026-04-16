use unshit::core::element::*;

use crate::state::{
    mutate_add_workspace_with_path, mutate_with, CtxMenu, SharedState, Subtab, TerminalEntry,
    UiSnapshot, Workspace,
};
use crate::ui::icons::*;

pub fn build_sidebar(state: &UiSnapshot, shared: &SharedState) -> ElementDef {
    let mut scroll = ElementDef::new(Tag::Div).with_class("sidebar-scroll");
    for (w_idx, workspace) in state.workspaces.iter().enumerate() {
        scroll = scroll.with_child(build_workspace(w_idx, workspace, shared));
    }

    let mut sidebar = ElementDef::new(Tag::Div)
        .with_class("sidebar")
        .with_class("role-aside")
        .with_id("sidebar");
    if state.sidebar_collapsed {
        sidebar = sidebar.with_class("collapsed");
    }
    sidebar
        .with_child(build_sidebar_head(shared))
        .with_child(scroll)
        .with_child(build_sidebar_footer())
        .with_child(build_sidebar_hints())
}

fn build_sidebar_head(shared: &SharedState) -> ElementDef {
    let add_state = shared.clone();
    ElementDef::new(Tag::Div)
        .with_class("sidebar-head")
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("sidebar-title")
                .with_text("workspaces"),
        )
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("sidebar-head-actions")
                .with_child(
                    ElementDef::new(Tag::Button)
                        .with_class("icon-btn")
                        .with_class("tight")
                        .on_click(move || {
                            let picked = rfd::FileDialog::new()
                                .set_title("Select workspace folder")
                                .pick_folder();
                            if let Some(folder) = picked {
                                mutate_with(&add_state, |st| {
                                    mutate_add_workspace_with_path(st, Some(folder));
                                    crate::persist::save_workspaces(st);
                                });
                            }
                        })
                        .with_child(svg_icon(icon_plus())),
                )
                .with_child(
                    ElementDef::new(Tag::Button)
                        .with_class("icon-btn")
                        .with_class("tight")
                        .with_child(svg_icon(icon_chevrons())),
                ),
        )
}

fn build_workspace(
    workspace_index: usize,
    workspace: &Workspace,
    shared: &SharedState,
) -> ElementDef {
    let head_state = shared.clone();
    let ctx_state = shared.clone();
    let idx = workspace_index;
    let head = ElementDef::new(Tag::Div)
        .with_class("workspace-head")
        .with_tab_index(0)
        .on_click(move || {
            mutate_with(&head_state, |st| {
                if let Some(ws) = st.workspaces.get_mut(idx) {
                    ws.collapsed = !ws.collapsed;
                }
            });
        })
        .on_context_menu(move |x, y| {
            mutate_with(&ctx_state, |st| {
                // Toggle: if the menu is already open for this workspace, close it.
                if st.ctx_menu.as_ref().is_some_and(|m| m.workspace_idx == idx) {
                    st.ctx_menu = None;
                } else {
                    // Divide by scale_factor: cursor coords are physical pixels,
                    // but Dimension::Px values get multiplied by scale_all_styles.
                    let sf = st.scale_factor;
                    st.ctx_menu = Some(CtxMenu {
                        x: x / sf,
                        y: y / sf,
                        workspace_idx: idx,
                    });
                }
            });
        })
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("chevron")
                .with_text("\u{25BE}"),
        )
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("workspace-num")
                .with_text(workspace.num.to_string()),
        )
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("workspace-name")
                .with_text(workspace.name.clone()),
        );

    let mut body = ElementDef::new(Tag::Div).with_class("workspace-body");
    for (s_idx, subtab) in workspace.subtabs.iter().enumerate() {
        body = body.with_child(build_subtab(
            workspace_index,
            s_idx,
            subtab,
            workspace,
            shared,
        ));
        if subtab.label == "terminals"
            && workspace.terminals_expanded
            && !workspace.terminal_entries.is_empty()
        {
            let mut entries = ElementDef::new(Tag::Div).with_class("terminal-entries");
            let count = workspace.terminal_entries.len();
            for (t_idx, entry) in workspace.terminal_entries.iter().enumerate() {
                entries = entries.with_child(build_terminal_entry(entry, t_idx == count - 1));
            }
            body = body.with_child(entries);
        }
    }

    let mut container = ElementDef::new(Tag::Div).with_class("workspace");
    if workspace.collapsed {
        container = container.with_class("collapsed");
    }
    if workspace.num == 1 {
        container = container.with_class("active");
    }
    container.with_child(head).with_child(body)
}

fn build_subtab(
    workspace_index: usize,
    subtab_index: usize,
    subtab: &Subtab,
    workspace: &Workspace,
    shared: &SharedState,
) -> ElementDef {
    let mut btn = ElementDef::new(Tag::Button).with_class("subtab");
    if subtab.active {
        btn = btn.with_class("active");
    }
    if subtab.disabled {
        btn = btn.with_class("disabled");
    }

    if subtab.label == "terminals" {
        let s = shared.clone();
        let wi = workspace_index;
        btn = btn.on_click(move || {
            mutate_with(&s, |st| {
                if let Some(ws) = st.workspaces.get_mut(wi) {
                    ws.terminals_expanded = !ws.terminals_expanded;
                }
            });
        });
    } else if !subtab.disabled {
        let s = shared.clone();
        let (wi, si) = (workspace_index, subtab_index);
        btn = btn.on_click(move || {
            mutate_with(&s, |st| {
                st.active_workspace = wi;
                if let Some(ws) = st.workspaces.get_mut(wi) {
                    for (i, sub) in ws.subtabs.iter_mut().enumerate() {
                        sub.active = i == si;
                    }
                }
            });
        });
    }

    if subtab.label == "terminals" {
        let chevron = if workspace.terminals_expanded {
            "\u{25BE}"
        } else {
            "\u{25B8}"
        };
        btn = btn.with_child(
            ElementDef::new(Tag::Span)
                .with_class("subtab-chevron")
                .with_text(chevron),
        );
    } else {
        btn = btn.with_child(
            ElementDef::new(Tag::Span)
                .with_class("tree-glyph")
                .with_text(subtab.tree_glyph),
        );
    }

    if let Some(icon) = subtab.icon {
        btn = btn.with_child(
            ElementDef::new(Tag::Span)
                .with_class("subtab-icon")
                .with_child(svg_icon(subtab_icon_for(icon))),
        );
    }

    btn = btn.with_child(
        ElementDef::new(Tag::Span)
            .with_class("subtab-label")
            .with_text(subtab.label.clone()),
    );

    if let Some(count) = subtab.count {
        let mut count_el = ElementDef::new(Tag::Span)
            .with_class("subtab-count")
            .with_text(count.to_string());
        if subtab.pulse {
            count_el = count_el.with_class("pulse");
        }
        btn = btn.with_child(count_el);
    }

    btn
}

fn build_terminal_entry(entry: &TerminalEntry, is_last: bool) -> ElementDef {
    let glyph = if is_last { "\u{2514}" } else { "\u{251C}" };

    let mut row = ElementDef::new(Tag::Div)
        .with_class("terminal-entry")
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("tree-glyph")
                .with_text(glyph),
        )
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("terminal-entry-name")
                .with_text(entry.name.clone()),
        );

    let mut tag = ElementDef::new(Tag::Span)
        .with_class("branch-tag")
        .with_text(entry.branch.clone());
    if entry.branch_muted {
        tag = tag.with_class("muted");
    }
    row = row.with_child(tag);

    row
}

fn build_sidebar_footer() -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("sidebar-footer")
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("footer-title")
                .with_text("activity"),
        )
        .with_child(activity_item(
            "running",
            "claude",
            "running",
            "refactor split pane logic",
        ))
        .with_child(activity_item(
            "stopped",
            "amp",
            "stopped",
            "verify readme docs",
        ))
        .with_child(activity_item("waiting", "codex", "waiting", "needs review"))
}

fn activity_item(state_class: &str, name: &str, state: &str, desc: &str) -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("activity-item")
        .with_class(state_class.to_string())
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("activity-row")
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("activity-name")
                        .with_text(name.to_string()),
                )
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("activity-state")
                        .with_text(state.to_string()),
                ),
        )
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("activity-desc")
                .with_text(desc.to_string()),
        )
}

fn build_sidebar_hints() -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("sidebar-hints")
        .with_child(hint_item("\u{2191}\u{2193}", "cycle"))
        .with_child(hint_item("\u{23CE}", "open"))
        .with_child(hint_item("x", "kill"))
        .with_child(hint_item("t", "theme"))
}

fn hint_item(key: &str, label: &str) -> ElementDef {
    ElementDef::new(Tag::Span)
        .with_class("hint")
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("kbd")
                .with_text(key.to_string()),
        )
        .with_child(ElementDef::new(Tag::Span).with_text(label.to_string()))
}

pub fn build_ctx_menu_overlay(snap: &UiSnapshot, shared: &SharedState) -> ElementDef {
    use unshit::core::style::parse::StyleDeclaration;
    use unshit::core::style::types::Dimension;

    let ctx = match &snap.ctx_menu {
        Some(c) => c,
        None => return ElementDef::new(Tag::Div).with_class("ctx-menu-hidden"),
    };

    let ws_idx = ctx.workspace_idx;
    let ws_name = snap
        .workspaces
        .get(ws_idx)
        .map(|w| w.name.clone())
        .unwrap_or_default();
    let is_collapsed = snap
        .workspaces
        .get(ws_idx)
        .map(|w| w.collapsed)
        .unwrap_or(false);
    let can_remove = snap.workspaces.len() > 1;

    // Backdrop: clicking (left or right) outside the menu closes it.
    let backdrop_shared = shared.clone();
    let backdrop_ctx_shared = shared.clone();
    let backdrop = ElementDef::new(Tag::Div)
        .with_class("ctx-menu-backdrop")
        .on_click(move || {
            mutate_with(&backdrop_shared, |st| {
                st.ctx_menu = None;
            });
        })
        .on_context_menu(move |_x, _y| {
            mutate_with(&backdrop_ctx_shared, |st| {
                st.ctx_menu = None;
            });
        });

    // Menu items.
    fn menu_item(label: &str, shared: &SharedState, command: String) -> ElementDef {
        let s = shared.clone();
        ElementDef::new(Tag::Div)
            .with_class("ctx-menu-item")
            .on_click(move || {
                mutate_with(&s, |st| {
                    crate::state::dispatch(st, &command);
                });
            })
            .with_child(
                ElementDef::new(Tag::Span)
                    .with_class("ctx-menu-item-label")
                    .with_text(label.to_string()),
            )
    }

    fn menu_separator() -> ElementDef {
        ElementDef::new(Tag::Div).with_class("ctx-menu-separator")
    }

    let collapse_label = if is_collapsed { "Expand" } else { "Collapse" };

    let mut menu = ElementDef::new(Tag::Div)
        .with_class("ctx-menu")
        .with_style(StyleDeclaration::Left(Dimension::Px(ctx.x)))
        .with_style(StyleDeclaration::Top(Dimension::Px(ctx.y)))
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("ctx-menu-header")
                .with_text(ws_name),
        )
        .with_child(menu_separator())
        .with_child(menu_item(
            "Set active",
            shared,
            format!("workspace.switch:{}", ws_idx),
        ))
        .with_child(menu_item(
            collapse_label,
            shared,
            format!("workspace.collapse:{}", ws_idx),
        ));

    if can_remove {
        menu = menu.with_child(menu_separator()).with_child(
            menu_item(
                "Remove workspace",
                shared,
                format!("workspace.remove:{}", ws_idx),
            )
            .with_class("danger"),
        );
    }

    backdrop.with_child(menu)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{seed_state, SharedState, Subtab, SubtabIcon, TerminalEntry, Workspace};
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
                return found.into();
            }
        }
        None
    }

    fn make_shared() -> SharedState {
        Arc::new(Mutex::new(seed_state()))
    }

    fn make_workspace(num: u32, collapsed: bool) -> Workspace {
        Workspace {
            num,
            name: format!("ws-{}", num),
            path: None,
            collapsed,
            terminals_expanded: false,
            terminal_entries: vec![],
            subtabs: vec![
                Subtab {
                    label: "terminals".to_string(),
                    count: Some(3),
                    pulse: false,
                    active: true,
                    disabled: false,
                    icon: Some(SubtabIcon::Terminal),
                    tree_glyph: "\u{251C}",
                },
                Subtab {
                    label: "agents".to_string(),
                    count: None,
                    pulse: false,
                    active: false,
                    disabled: false,
                    icon: None,
                    tree_glyph: "\u{2514}",
                },
            ],
        }
    }

    // -- build_sidebar --

    #[test]
    fn build_sidebar_not_collapsed() {
        let shared = make_shared();
        let state = shared.lock().unwrap().ui_snapshot();
        let el = build_sidebar(&state, &shared);

        assert_eq!(el.tag, Tag::Div);
        assert!(has_class(&el, "sidebar"));
        assert!(!has_class(&el, "collapsed"));
        assert_eq!(el.id.as_deref(), Some("sidebar"));
        // Should have 4 children: head, scroll, footer, hints
        assert_eq!(el.children.len(), 4);
    }

    #[test]
    fn build_sidebar_collapsed() {
        let shared = make_shared();
        {
            let mut guard = shared.lock().unwrap();
            guard.sidebar_collapsed = true;
        }
        let state = shared.lock().unwrap().ui_snapshot();
        let el = build_sidebar(&state, &shared);

        assert!(has_class(&el, "sidebar"));
        assert!(has_class(&el, "collapsed"));
    }

    #[test]
    fn build_sidebar_scroll_contains_workspaces() {
        let shared = make_shared();
        let state = shared.lock().unwrap().ui_snapshot();
        let el = build_sidebar(&state, &shared);
        // children[1] is the scroll container
        let scroll = &el.children[1];
        assert!(has_class(scroll, "sidebar-scroll"));
        assert_eq!(scroll.children.len(), state.workspaces.len());
    }

    // -- build_sidebar_head --

    #[test]
    fn sidebar_head_has_title_and_actions() {
        let shared = make_shared();
        let head = build_sidebar_head(&shared);
        assert!(has_class(&head, "sidebar-head"));
        // First child should be the title span
        let title = &head.children[0];
        assert!(has_class(title, "sidebar-title"));
        assert_eq!(text_of(title), Some("workspaces"));
        // Second child is actions div with two buttons
        let actions = &head.children[1];
        assert!(has_class(actions, "sidebar-head-actions"));
        assert_eq!(actions.children.len(), 2);
        for btn in &actions.children {
            assert_eq!(btn.tag, Tag::Button);
            assert!(has_class(btn, "icon-btn"));
        }
    }

    // -- build_workspace --

    #[test]
    fn workspace_not_collapsed_has_no_collapsed_class() {
        let shared = make_shared();
        let ws = make_workspace(2, false);
        let el = build_workspace(0, &ws, &shared);
        assert!(has_class(&el, "workspace"));
        assert!(!has_class(&el, "collapsed"));
    }

    #[test]
    fn workspace_collapsed_has_collapsed_class() {
        let shared = make_shared();
        let ws = make_workspace(2, true);
        let el = build_workspace(0, &ws, &shared);
        assert!(has_class(&el, "workspace"));
        assert!(has_class(&el, "collapsed"));
    }

    #[test]
    fn workspace_num_1_is_active() {
        let shared = make_shared();
        let ws = make_workspace(1, false);
        let el = build_workspace(0, &ws, &shared);
        assert!(has_class(&el, "active"));
    }

    #[test]
    fn workspace_num_2_is_not_active() {
        let shared = make_shared();
        let ws = make_workspace(2, false);
        let el = build_workspace(0, &ws, &shared);
        assert!(!has_class(&el, "active"));
    }

    #[test]
    fn terminal_entry_branch_muted() {
        let entry = TerminalEntry {
            name: "zsh".to_string(),
            branch: "main".to_string(),
            branch_muted: true,
        };
        let el = build_terminal_entry(&entry, false);
        let branch_tag = find_by_class(&el, "branch-tag").expect("branch-tag not found");
        assert!(has_class(branch_tag, "muted"));
    }

    #[test]
    fn terminal_entry_branch_not_muted() {
        let entry = TerminalEntry {
            name: "zsh".to_string(),
            branch: "main".to_string(),
            branch_muted: false,
        };
        let el = build_terminal_entry(&entry, false);
        let branch_tag = find_by_class(&el, "branch-tag").expect("branch-tag not found");
        assert!(!has_class(branch_tag, "muted"));
    }

    #[test]
    fn workspace_head_shows_name_and_num() {
        let shared = make_shared();
        let ws = make_workspace(3, false);
        let el = build_workspace(0, &ws, &shared);
        let head = find_by_class(&el, "workspace-head").unwrap();
        let num_el = find_by_class(head, "workspace-num").unwrap();
        assert_eq!(text_of(num_el), Some("3"));
        let name_el = find_by_class(head, "workspace-name").unwrap();
        assert_eq!(text_of(name_el), Some("ws-3"));
    }

    #[test]
    fn workspace_body_has_subtabs() {
        let shared = make_shared();
        let ws = make_workspace(2, false);
        let el = build_workspace(0, &ws, &shared);
        let body = find_by_class(&el, "workspace-body").unwrap();
        assert_eq!(body.children.len(), 2);
    }

    // -- build_subtab --

    #[test]
    fn subtab_active() {
        let shared = make_shared();
        let ws = make_workspace(1, false);
        let sub = Subtab {
            label: "test".to_string(),
            count: None,
            pulse: false,
            active: true,
            disabled: false,
            icon: None,
            tree_glyph: "\u{251C}",
        };
        let el = build_subtab(0, 0, &sub, &ws, &shared);
        assert_eq!(el.tag, Tag::Button);
        assert!(has_class(&el, "subtab"));
        assert!(has_class(&el, "active"));
    }

    #[test]
    fn subtab_inactive() {
        let shared = make_shared();
        let ws = make_workspace(1, false);
        let sub = Subtab {
            label: "test".to_string(),
            count: None,
            pulse: false,
            active: false,
            disabled: false,
            icon: None,
            tree_glyph: "\u{251C}",
        };
        let el = build_subtab(0, 0, &sub, &ws, &shared);
        assert!(has_class(&el, "subtab"));
        assert!(!has_class(&el, "active"));
    }

    #[test]
    fn subtab_with_icon() {
        let shared = make_shared();
        let ws = make_workspace(1, false);
        let sub = Subtab {
            label: "terminals".to_string(),
            count: None,
            pulse: false,
            active: false,
            disabled: false,
            icon: Some(SubtabIcon::Terminal),
            tree_glyph: "\u{251C}",
        };
        let el = build_subtab(0, 0, &sub, &ws, &shared);
        assert!(find_by_class(&el, "subtab-icon").is_some());
    }

    #[test]
    fn subtab_without_icon() {
        let shared = make_shared();
        let ws = make_workspace(1, false);
        let sub = Subtab {
            label: "plain".to_string(),
            count: None,
            pulse: false,
            active: false,
            disabled: false,
            icon: None,
            tree_glyph: "\u{251C}",
        };
        let el = build_subtab(0, 0, &sub, &ws, &shared);
        assert!(find_by_class(&el, "subtab-icon").is_none());
    }

    #[test]
    fn subtab_with_count() {
        let shared = make_shared();
        let ws = make_workspace(1, false);
        let sub = Subtab {
            label: "stuff".to_string(),
            count: Some(42),
            pulse: false,
            active: false,
            disabled: false,
            icon: None,
            tree_glyph: "\u{251C}",
        };
        let el = build_subtab(0, 0, &sub, &ws, &shared);
        let count_el = find_by_class(&el, "subtab-count").expect("subtab-count not found");
        assert_eq!(text_of(count_el), Some("42"));
        assert!(!has_class(count_el, "pulse"));
    }

    #[test]
    fn subtab_without_count() {
        let shared = make_shared();
        let ws = make_workspace(1, false);
        let sub = Subtab {
            label: "stuff".to_string(),
            count: None,
            pulse: false,
            active: false,
            disabled: false,
            icon: None,
            tree_glyph: "\u{251C}",
        };
        let el = build_subtab(0, 0, &sub, &ws, &shared);
        assert!(find_by_class(&el, "subtab-count").is_none());
    }

    #[test]
    fn subtab_with_pulse() {
        let shared = make_shared();
        let ws = make_workspace(1, false);
        let sub = Subtab {
            label: "agents".to_string(),
            count: Some(5),
            pulse: true,
            active: false,
            disabled: false,
            icon: None,
            tree_glyph: "\u{251C}",
        };
        let el = build_subtab(0, 0, &sub, &ws, &shared);
        let count_el = find_by_class(&el, "subtab-count").unwrap();
        assert!(has_class(count_el, "pulse"));
    }

    #[test]
    fn subtab_has_tree_glyph_and_label() {
        let shared = make_shared();
        let ws = make_workspace(1, false);
        let sub = Subtab {
            label: "sessions".to_string(),
            count: None,
            pulse: false,
            active: false,
            disabled: false,
            icon: None,
            tree_glyph: "\u{2514}",
        };
        let el = build_subtab(0, 0, &sub, &ws, &shared);
        let glyph = find_by_class(&el, "tree-glyph").unwrap();
        assert_eq!(text_of(glyph), Some("\u{2514}"));
        let label = find_by_class(&el, "subtab-label").unwrap();
        assert_eq!(text_of(label), Some("sessions"));
    }

    // -- build_sidebar_footer --

    #[test]
    fn sidebar_footer_has_activity_items() {
        let footer = build_sidebar_footer();
        assert!(has_class(&footer, "sidebar-footer"));
        // 1 title + 3 activity items
        assert_eq!(footer.children.len(), 4);
        let title = &footer.children[0];
        assert!(has_class(title, "footer-title"));
        assert_eq!(text_of(title), Some("activity"));
    }

    // -- activity_item --

    #[test]
    fn activity_item_structure() {
        let item = activity_item("running", "claude", "running", "refactor logic");
        assert!(has_class(&item, "activity-item"));
        assert!(has_class(&item, "running"));
        let row = find_by_class(&item, "activity-row").unwrap();
        let name_el = find_by_class(row, "activity-name").unwrap();
        assert_eq!(text_of(name_el), Some("claude"));
        let state_el = find_by_class(row, "activity-state").unwrap();
        assert_eq!(text_of(state_el), Some("running"));
        let desc_el = find_by_class(&item, "activity-desc").unwrap();
        assert_eq!(text_of(desc_el), Some("refactor logic"));
    }

    // -- build_sidebar_hints --

    #[test]
    fn sidebar_hints_has_four_hints() {
        let hints = build_sidebar_hints();
        assert!(has_class(&hints, "sidebar-hints"));
        assert_eq!(hints.children.len(), 4);
    }

    // -- hint_item --

    #[test]
    fn hint_item_structure() {
        let item = hint_item("x", "kill");
        assert!(has_class(&item, "hint"));
        assert_eq!(item.children.len(), 2);
        let kbd = &item.children[0];
        assert!(has_class(kbd, "kbd"));
        assert_eq!(text_of(kbd), Some("x"));
        let label = &item.children[1];
        assert_eq!(text_of(label), Some("kill"));
    }
}
