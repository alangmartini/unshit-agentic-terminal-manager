use unshit::core::element::*;

use crate::state::{
    mutate_add_workspace_with_path, mutate_with, CtxMenu, SharedState, Subtab, TerminalEntry,
    UiSnapshot, Workspace,
};
use crate::ui::icons::*;

pub fn build_sidebar(state: &UiSnapshot, shared: &SharedState) -> ElementDef {
    let mut scroll = ElementDef::new(Tag::Div).with_class("sidebar-scroll");
    for (w_idx, workspace) in state.workspaces.iter().enumerate() {
        scroll = scroll.with_child(build_workspace(
            w_idx,
            w_idx == state.active_workspace,
            state.active_pane,
            workspace,
            shared,
        ));
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
        .with_child(build_sidebar_footer(state))
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
    is_active: bool,
    active_pane: crate::state::PaneId,
    workspace: &Workspace,
    shared: &SharedState,
) -> ElementDef {
    let head_state = shared.clone();
    let chevron_state = shared.clone();
    let ctx_state = shared.clone();
    let idx = workspace_index;
    let head = ElementDef::new(Tag::Div)
        .with_class("workspace-head")
        .with_class("sb-ws")
        .with_tab_index(0)
        .on_click(move || {
            mutate_with(&head_state, |st| {
                if let Some(ws) = st.workspaces.get_mut(idx) {
                    ws.collapsed = false;
                }
                let pane = crate::state::workspace_active_pane(st, idx);
                let cmd = match pane {
                    Some(pid) => format!("terminal.focus:{}:{}", idx, pid.0),
                    None => format!("workspace.switch:{}", idx),
                };
                crate::state::dispatch(st, &cmd);
            });
        })
        .on_context_menu(move |x, y| {
            mutate_with(&ctx_state, |st| {
                // Toggle: if the menu is already open for this workspace, close it.
                let same_ws = matches!(
                    st.ctx_menu.as_ref().map(|m| &m.target),
                    Some(crate::state::CtxMenuTarget::Workspace { idx: i }) if *i == idx
                );
                if same_ws {
                    st.ctx_menu = None;
                } else {
                    // Divide by scale_factor: cursor coords are physical pixels,
                    // but Dimension::Px values get multiplied by scale_all_styles.
                    let sf = st.scale_factor;
                    st.ctx_menu = Some(CtxMenu {
                        x: x / sf,
                        y: y / sf,
                        target: crate::state::CtxMenuTarget::Workspace { idx },
                    });
                }
            });
        })
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("chevron")
                .with_text("\u{25BE}")
                .on_click(move || {
                    mutate_with(&chevron_state, |st| {
                        if let Some(ws) = st.workspaces.get_mut(idx) {
                            ws.collapsed = !ws.collapsed;
                        }
                    });
                }),
        )
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("workspace-num")
                .with_text(workspace.num.to_string()),
        )
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("workspace-name")
                .with_class("sb-label")
                .with_text(workspace.name.clone()),
        )
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("workspace-meta")
                .with_class("ws-meta")
                .with_text(workspace.terminal_entries.len().to_string()),
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
                entries = entries.with_child(build_terminal_entry(
                    workspace_index,
                    entry,
                    t_idx == count - 1,
                    entry.pane_id == active_pane,
                    shared,
                ));
            }
            body = body.with_child(entries);
        }
    }

    let mut container = ElementDef::new(Tag::Div).with_class("workspace");
    if workspace.collapsed {
        container = container.with_class("collapsed");
    }
    if is_active {
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
                crate::state::mutate_switch_workspace(st, wi);
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

fn build_terminal_entry(
    workspace_index: usize,
    entry: &TerminalEntry,
    is_last: bool,
    is_active: bool,
    shared: &SharedState,
) -> ElementDef {
    let glyph = if is_last { "\u{2514}" } else { "\u{251C}" };

    let click_shared = shared.clone();
    let ws_idx = workspace_index;
    let pane_id = entry.pane_id;
    let mut row = ElementDef::new(Tag::Div)
        .with_class("terminal-entry")
        .with_class("sb-row")
        .with_tab_index(0)
        .on_click(move || {
            mutate_with(&click_shared, |st| {
                crate::state::dispatch(st, &format!("terminal.focus:{}:{}", ws_idx, pane_id.0));
            });
        })
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("tree-glyph")
                .with_text(glyph),
        )
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("terminal-entry-name")
                .with_class("sb-label")
                .with_text(entry.name.clone()),
        );
    if is_active {
        row = row.with_class("active");
    }

    let mut tag = ElementDef::new(Tag::Span)
        .with_class("branch-tag")
        .with_class("sb-branch")
        .with_text(entry.branch.clone());
    if entry.branch_muted {
        tag = tag.with_class("muted");
    }
    if entry.branch_error {
        tag = tag.with_class("error");
    }
    row = row.with_child(tag);

    row
}

fn build_sidebar_footer(state: &UiSnapshot) -> ElementDef {
    let sessions = if state.terminal_count == 1 {
        "1 sess".to_string()
    } else {
        format!("{} sess", state.terminal_count)
    };
    let panes: usize = state
        .tabs
        .iter()
        .map(|tab| tab.panes.iter().map(|row| row.len()).sum::<usize>())
        .sum();
    ElementDef::new(Tag::Div)
        .with_class("sidebar-footer")
        .with_class("sb-foot")
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("dot")
                .with_class("status-running"),
        )
        .with_child(ElementDef::new(Tag::Span).with_text("ptyd"))
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("dim")
                .with_text(format!("\u{00B7} {sessions} \u{00B7} {panes} panes")),
        )
}

pub fn build_ctx_menu_overlay(snap: &UiSnapshot, shared: &SharedState) -> ElementDef {
    let ctx = match &snap.ctx_menu {
        Some(c) => c,
        None => return ElementDef::new(Tag::Div).with_class("ctx-menu-hidden"),
    };

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

    let menu = match &ctx.target {
        crate::state::CtxMenuTarget::Workspace { idx } => {
            let installed = crate::shell::discover_installed();
            build_workspace_ctx_menu(snap, shared, ctx.x, ctx.y, *idx, &installed)
        }
        crate::state::CtxMenuTarget::Tab { pane_id } => {
            build_tab_ctx_menu(snap, shared, ctx.x, ctx.y, *pane_id)
        }
    };

    backdrop.with_child(menu)
}

fn ctx_menu_item(label: &str, shared: &SharedState, command: String) -> ElementDef {
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

fn ctx_menu_separator() -> ElementDef {
    ElementDef::new(Tag::Div).with_class("ctx-menu-separator")
}

fn ctx_menu_section_header(label: &str) -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("ctx-menu-section-header")
        .with_text(label.to_string())
}

fn ctx_menu_item_active(label: &str, shared: &SharedState, command: String) -> ElementDef {
    ctx_menu_item(label, shared, command).with_class("active")
}

fn workspace_ctx_shell_items(
    ws_idx: usize,
    current: &crate::shell::ShellSpec,
    installed: &[std::path::PathBuf],
    shared: &SharedState,
) -> Vec<ElementDef> {
    let mut items: Vec<ElementDef> = Vec::new();
    items.push(ctx_menu_section_header("Shell"));

    let labels = crate::shell::label_installed_shells(installed);
    for (path, label) in installed.iter().zip(labels.iter()) {
        let program = path.display().to_string();
        let spec = crate::shell::ShellSpec {
            program: program.clone(),
            args: current.args.clone(),
        };
        let json = serde_json::to_string(&spec).unwrap_or_else(|_| "{}".into());
        let command = format!("shell.set_workspace:{ws_idx}:{json}");
        let is_active = !current.program.is_empty() && current.program == program;
        let item = if is_active {
            ctx_menu_item_active(label, shared, command)
        } else {
            ctx_menu_item(label, shared, command)
        };
        items.push(item);
    }

    if !current.is_empty() {
        items.push(ctx_menu_item(
            "Use app default",
            shared,
            format!("shell.clear_workspace:{ws_idx}"),
        ));
    }

    items
}

fn build_workspace_ctx_menu(
    snap: &UiSnapshot,
    shared: &SharedState,
    x: f32,
    y: f32,
    ws_idx: usize,
    installed: &[std::path::PathBuf],
) -> ElementDef {
    use unshit::core::style::parse::StyleDeclaration;
    use unshit::core::style::types::Dimension;

    let ws = snap.workspaces.get(ws_idx);
    let ws_name = ws.map(|w| w.name.clone()).unwrap_or_default();
    let is_collapsed = ws.map(|w| w.collapsed).unwrap_or(false);
    let current_shell = ws.map(|w| w.shell.clone()).unwrap_or_default();
    let can_remove = snap.workspaces.len() > 1;
    let collapse_label = if is_collapsed { "Expand" } else { "Collapse" };

    let mut menu = ElementDef::new(Tag::Div)
        .with_class("ctx-menu")
        .with_style(StyleDeclaration::Left(Dimension::Px(x)))
        .with_style(StyleDeclaration::Top(Dimension::Px(y)))
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("ctx-menu-header")
                .with_text(ws_name),
        )
        .with_child(ctx_menu_separator())
        .with_child(ctx_menu_item(
            "Set active",
            shared,
            format!("workspace.switch:{}", ws_idx),
        ))
        .with_child(ctx_menu_item(
            "New terminal",
            shared,
            format!("workspace.new_terminal:{}", ws_idx),
        ))
        .with_child(ctx_menu_item(
            collapse_label,
            shared,
            format!("workspace.collapse:{}", ws_idx),
        ));

    menu = menu.with_child(ctx_menu_separator());
    for item in workspace_ctx_shell_items(ws_idx, &current_shell, installed, shared) {
        menu = menu.with_child(item);
    }

    menu = menu.with_child(ctx_menu_separator()).with_child(
        ctx_menu_item(
            "Kill all terminals in workspace",
            shared,
            format!("workspace.request_kill_all:{}", ws_idx),
        )
        .with_class("danger"),
    );

    if can_remove {
        menu = menu.with_child(ctx_menu_separator()).with_child(
            ctx_menu_item(
                "Remove workspace",
                shared,
                format!("workspace.remove:{}", ws_idx),
            )
            .with_class("danger"),
        );
    }

    menu
}

fn build_tab_ctx_menu(
    snap: &UiSnapshot,
    shared: &SharedState,
    x: f32,
    y: f32,
    pane_id: u32,
) -> ElementDef {
    use unshit::core::style::parse::StyleDeclaration;
    use unshit::core::style::types::Dimension;

    // Header shows the current pane title so the user can tell which
    // session they are about to rename / kill. Fall back to the pane
    // id if no matching pane is found, which only happens if the menu
    // races a tab close.
    let header = snap
        .panes
        .iter()
        .flat_map(|row| row.iter())
        .find(|p| p.id.0 == pane_id)
        .map(|p| p.title.clone())
        .unwrap_or_else(|| format!("pane {pane_id}"));

    ElementDef::new(Tag::Div)
        .with_class("ctx-menu")
        .with_style(StyleDeclaration::Left(Dimension::Px(x)))
        .with_style(StyleDeclaration::Top(Dimension::Px(y)))
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("ctx-menu-header")
                .with_text(header),
        )
        .with_child(ctx_menu_separator())
        .with_child(ctx_menu_item(
            "Rename session",
            shared,
            format!("tab.request_rename:{}", pane_id),
        ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{seed_state, SharedState, Subtab, SubtabIcon, TerminalEntry, Workspace};
    use std::sync::{Arc, Mutex};
    use unshit_test::TestHarness;

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
            git_branch: None,
            tabs: vec![],
            active_tab: 0,
            shell: crate::shell::ShellSpec::default(),
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
        // head, scroll, real ptyd footer
        assert_eq!(el.children.len(), 3);
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
        let el = build_workspace(0, false, crate::state::PaneId(1), &ws, &shared);
        assert!(has_class(&el, "workspace"));
        assert!(!has_class(&el, "collapsed"));
    }

    #[test]
    fn workspace_collapsed_has_collapsed_class() {
        let shared = make_shared();
        let ws = make_workspace(2, true);
        let el = build_workspace(0, false, crate::state::PaneId(1), &ws, &shared);
        assert!(has_class(&el, "workspace"));
        assert!(has_class(&el, "collapsed"));
    }

    #[test]
    fn workspace_gets_active_class_when_is_active() {
        let shared = make_shared();
        let ws = make_workspace(1, false);
        let el = build_workspace(0, true, crate::state::PaneId(1), &ws, &shared);
        assert!(has_class(&el, "active"));
    }

    #[test]
    fn workspace_gets_no_active_class_when_not_is_active() {
        let shared = make_shared();
        let ws = make_workspace(2, false);
        let el = build_workspace(0, false, crate::state::PaneId(1), &ws, &shared);
        assert!(!has_class(&el, "active"));
    }

    // Regression for issue #104.
    #[test]
    fn workspace_active_class_independent_of_workspace_num() {
        let shared = make_shared();
        let ws_num_5 = make_workspace(5, false);
        let ws_num_1 = make_workspace(1, false);

        let el_active = build_workspace(2, true, crate::state::PaneId(1), &ws_num_5, &shared);
        let el_other = build_workspace(0, false, crate::state::PaneId(1), &ws_num_1, &shared);

        assert!(has_class(&el_active, "active"));
        assert!(!has_class(&el_other, "active"));
    }

    // -- Click behavior: chevron vs name area (issue #98) --

    #[test]
    fn workspace_head_click_switches_active_and_expands() {
        let shared = make_shared();
        {
            let mut guard = shared.lock().unwrap();
            guard.active_workspace = 0;
            guard.workspaces[2].collapsed = true;
        }
        let snapshot = shared.lock().unwrap().ui_snapshot();
        let el = build_workspace(
            2,
            2 == snapshot.active_workspace,
            snapshot.active_pane,
            &snapshot.workspaces[2],
            &shared,
        );
        let head = find_by_class(&el, "workspace-head").expect("workspace-head");
        (head.on_click.as_ref().expect("head on_click"))();

        let guard = shared.lock().unwrap();
        assert_eq!(guard.active_workspace, 2);
        assert!(!guard.workspaces[2].collapsed);
    }

    #[test]
    fn workspace_head_click_on_already_expanded_keeps_expanded() {
        let shared = make_shared();
        {
            let mut guard = shared.lock().unwrap();
            guard.active_workspace = 0;
            guard.workspaces[2].collapsed = false;
        }
        let snapshot = shared.lock().unwrap().ui_snapshot();
        let el = build_workspace(
            2,
            2 == snapshot.active_workspace,
            snapshot.active_pane,
            &snapshot.workspaces[2],
            &shared,
        );
        let head = find_by_class(&el, "workspace-head").expect("workspace-head");
        (head.on_click.as_ref().expect("head on_click"))();

        let guard = shared.lock().unwrap();
        assert_eq!(guard.active_workspace, 2);
        assert!(!guard.workspaces[2].collapsed);
    }

    #[test]
    fn chevron_click_toggles_collapse_and_does_not_change_active() {
        let shared = make_shared();
        {
            let mut guard = shared.lock().unwrap();
            guard.active_workspace = 0;
            guard.workspaces[1].collapsed = false;
        }
        let snapshot = shared.lock().unwrap().ui_snapshot();
        let el = build_workspace(
            1,
            1 == snapshot.active_workspace,
            snapshot.active_pane,
            &snapshot.workspaces[1],
            &shared,
        );
        let head = find_by_class(&el, "workspace-head").expect("workspace-head");
        let chevron = find_by_class(head, "chevron").expect("chevron");
        (chevron.on_click.as_ref().expect("chevron on_click"))();

        let guard = shared.lock().unwrap();
        assert!(
            guard.workspaces[1].collapsed,
            "chevron should toggle collapse"
        );
        assert_eq!(
            guard.active_workspace, 0,
            "chevron should not change active"
        );
    }

    #[test]
    fn chevron_click_on_collapsed_expands() {
        let shared = make_shared();
        {
            let mut guard = shared.lock().unwrap();
            guard.workspaces[1].collapsed = true;
        }
        let snapshot = shared.lock().unwrap().ui_snapshot();
        let el = build_workspace(
            1,
            1 == snapshot.active_workspace,
            snapshot.active_pane,
            &snapshot.workspaces[1],
            &shared,
        );
        let head = find_by_class(&el, "workspace-head").expect("workspace-head");
        let chevron = find_by_class(head, "chevron").expect("chevron");
        (chevron.on_click.as_ref().expect("chevron on_click"))();

        let guard = shared.lock().unwrap();
        assert!(!guard.workspaces[1].collapsed);
    }

    #[test]
    fn terminal_entry_branch_muted() {
        let entry = TerminalEntry {
            name: "zsh".to_string(),
            branch: "main".to_string(),
            branch_muted: true,
            branch_error: false,
            pane_id: crate::state::PaneId(0),
        };
        let el = build_terminal_entry(0, &entry, false, false, &make_shared());
        let branch_tag = find_by_class(&el, "branch-tag").expect("branch-tag not found");
        assert!(has_class(branch_tag, "muted"));
    }

    #[test]
    fn terminal_entry_branch_not_muted() {
        let entry = TerminalEntry {
            name: "zsh".to_string(),
            branch: "main".to_string(),
            branch_muted: false,
            branch_error: false,
            pane_id: crate::state::PaneId(0),
        };
        let el = build_terminal_entry(0, &entry, false, false, &make_shared());
        let branch_tag = find_by_class(&el, "branch-tag").expect("branch-tag not found");
        assert!(!has_class(branch_tag, "muted"));
    }

    #[test]
    fn terminal_entry_branch_error() {
        let entry = TerminalEntry {
            name: "zsh".to_string(),
            branch: "main".to_string(),
            branch_muted: false,
            branch_error: true,
            pane_id: crate::state::PaneId(0),
        };
        let el = build_terminal_entry(0, &entry, false, false, &make_shared());
        let branch_tag = find_by_class(&el, "branch-tag").expect("branch-tag not found");
        assert!(has_class(branch_tag, "error"));
    }

    #[test]
    fn terminal_entry_branch_not_error() {
        let entry = TerminalEntry {
            name: "zsh".to_string(),
            branch: "main".to_string(),
            branch_muted: false,
            branch_error: false,
            pane_id: crate::state::PaneId(0),
        };
        let el = build_terminal_entry(0, &entry, false, false, &make_shared());
        let branch_tag = find_by_class(&el, "branch-tag").expect("branch-tag not found");
        assert!(!has_class(branch_tag, "error"));
    }

    #[test]
    fn workspace_head_shows_name_and_num() {
        let shared = make_shared();
        let ws = make_workspace(3, false);
        let el = build_workspace(0, false, crate::state::PaneId(1), &ws, &shared);
        let head = find_by_class(&el, "workspace-head").unwrap();
        let num_el = find_by_class(head, "workspace-num").unwrap();
        assert_eq!(text_of(num_el), Some("3"));
        let name_el = find_by_class(head, "workspace-name").unwrap();
        assert_eq!(text_of(name_el), Some("ws-3"));
    }

    #[test]
    fn workspace_head_click_switches_active_workspace() {
        let shared = make_shared();
        assert_eq!(shared.lock().unwrap().active_workspace, 0);
        let ws = shared.lock().unwrap().ui_snapshot().workspaces[2].clone();
        let el = build_workspace(2, false, crate::state::PaneId(1), &ws, &shared);
        let head = find_by_class(&el, "workspace-head").unwrap();
        (head.on_click.as_ref().unwrap())();
        assert_eq!(shared.lock().unwrap().active_workspace, 2);
    }

    #[test]
    fn workspace_head_click_expands_collapsed_workspace() {
        let shared = make_shared();
        {
            let mut guard = shared.lock().unwrap();
            guard.workspaces[1].collapsed = true;
        }
        let ws = shared.lock().unwrap().ui_snapshot().workspaces[1].clone();
        let el = build_workspace(1, false, crate::state::PaneId(1), &ws, &shared);
        let head = find_by_class(&el, "workspace-head").unwrap();
        (head.on_click.as_ref().unwrap())();
        assert!(!shared.lock().unwrap().workspaces[1].collapsed);
    }

    #[test]
    fn workspace_head_click_on_populated_workspace_focuses_active_pane() {
        use crate::state::PaneId;
        // Seed state has ws0 active with a live pane id 1, plus ws1 with no tabs.
        let shared = make_shared();
        // Switch to ws1 and create a terminal there so ws1 has a saved pane.
        {
            let mut st = shared.lock().unwrap();
            crate::state::mutate_switch_workspace(&mut st, 1);
            crate::state::mutate_add_tab(&mut st);
        }
        let new_pane_id = shared.lock().unwrap().active_pane;
        // Switch back to ws0 so ws1's state is saved.
        {
            let mut st = shared.lock().unwrap();
            crate::state::mutate_switch_workspace(&mut st, 0);
        }
        assert_eq!(shared.lock().unwrap().active_workspace, 0);
        // Clicking ws1's head must switch workspace and set active_pane to
        // ws1's saved pane, matching a terminal-entry click on that pane.
        let ws = shared.lock().unwrap().ui_snapshot().workspaces[1].clone();
        let el = build_workspace(1, false, crate::state::PaneId(1), &ws, &shared);
        let head = find_by_class(&el, "workspace-head").unwrap();
        (head.on_click.as_ref().unwrap())();
        let st = shared.lock().unwrap();
        assert_eq!(st.active_workspace, 1);
        assert_eq!(st.active_pane, new_pane_id);
        assert_ne!(new_pane_id, PaneId(0));
    }

    #[test]
    fn workspace_head_click_on_empty_workspace_still_switches() {
        // Seed: ws2 and ws3 have no saved tabs. Clicking their head must still
        // switch active_workspace via the workspace.switch fallback.
        let shared = make_shared();
        assert_eq!(shared.lock().unwrap().active_workspace, 0);
        assert!(shared.lock().unwrap().workspaces[2].tabs.is_empty());
        let ws = shared.lock().unwrap().ui_snapshot().workspaces[2].clone();
        let el = build_workspace(2, false, crate::state::PaneId(1), &ws, &shared);
        let head = find_by_class(&el, "workspace-head").unwrap();
        (head.on_click.as_ref().unwrap())();
        assert_eq!(shared.lock().unwrap().active_workspace, 2);
    }

    #[test]
    fn workspace_body_has_subtabs() {
        let shared = make_shared();
        let ws = make_workspace(2, false);
        let el = build_workspace(0, false, crate::state::PaneId(1), &ws, &shared);
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
    fn sidebar_footer_has_ptyd_status() {
        let shared = make_shared();
        let state = shared.lock().unwrap().ui_snapshot();
        let footer = build_sidebar_footer(&state);
        assert!(has_class(&footer, "sidebar-footer"));
        assert!(has_class(&footer, "sb-foot"));
        assert_eq!(footer.children.len(), 3);
        assert_eq!(text_of(&footer.children[1]), Some("ptyd"));
    }

    #[test]
    fn sidebar_footer_stays_statusbar_height_with_stylesheet() {
        let shared = make_shared();
        let state = shared.lock().unwrap().ui_snapshot();
        let tree_shared = shared.clone();
        let tree_state = state.clone();
        let css = format!(
            "{}\n.sidebar-test-root {{ display: flex; width: 252px; height: 720px; }}",
            include_str!("../../assets/styles.css")
        );
        let mut harness = TestHarness::new(
            &css,
            move || ElementTree {
                root: ElementDef::new(Tag::Div)
                    .with_class("sidebar-test-root")
                    .with_child(build_sidebar(&tree_state, &tree_shared)),
            },
            1280.0,
            720.0,
        );
        harness.step();

        let footer = harness
            .query(".sb-foot")
            .expect("sidebar footer should render");
        assert_eq!(footer.layout_rect.height, 24.0);
    }

    // -- build_workspace_ctx_menu shell submenu (Task 10) --

    fn fake_installed() -> Vec<std::path::PathBuf> {
        vec![
            std::path::PathBuf::from("/usr/bin/pwsh"),
            std::path::PathBuf::from("/usr/bin/cmd"),
        ]
    }

    fn collect_text_recursive(root: &ElementDef) -> String {
        let mut acc = String::new();
        if let Some(t) = text_of(root) {
            acc.push_str(t);
            acc.push(' ');
        }
        for child in &root.children {
            acc.push_str(&collect_text_recursive(child));
        }
        acc
    }

    fn collect_with_class<'a>(root: &'a ElementDef, class: &str) -> Vec<&'a ElementDef> {
        let mut out = Vec::new();
        if has_class(root, class) {
            out.push(root);
        }
        for child in &root.children {
            out.extend(collect_with_class(child, class));
        }
        out
    }

    fn item_text_contains(el: &ElementDef, needle: &str) -> bool {
        collect_text_recursive(el).contains(needle)
    }

    #[test]
    fn workspace_ctx_menu_includes_shell_subsection_header() {
        let shared = make_shared();
        let snap = shared.lock().unwrap().ui_snapshot();
        let installed = fake_installed();
        let menu = build_workspace_ctx_menu(&snap, &shared, 0.0, 0.0, 0, &installed);
        let text = collect_text_recursive(&menu);
        assert!(
            text.contains("Shell"),
            "ctx menu must include a Shell subsection header, got text: {text:?}"
        );
    }

    #[test]
    fn workspace_ctx_menu_lists_each_installed_shell_by_stem() {
        let shared = make_shared();
        let snap = shared.lock().unwrap().ui_snapshot();
        let installed = fake_installed();
        let menu = build_workspace_ctx_menu(&snap, &shared, 0.0, 0.0, 0, &installed);
        let items = collect_with_class(&menu, "ctx-menu-item");
        assert!(
            items.iter().any(|el| item_text_contains(el, "pwsh")),
            "menu must list a pwsh item; items text: {:?}",
            items
                .iter()
                .map(|e| collect_text_recursive(e))
                .collect::<Vec<_>>()
        );
        assert!(
            items.iter().any(|el| item_text_contains(el, "cmd")),
            "menu must list a cmd item"
        );
    }

    #[test]
    fn workspace_ctx_menu_marks_current_shell_as_active() {
        let shared = make_shared();
        {
            let mut guard = shared.lock().unwrap();
            guard.workspaces[0].shell = crate::shell::ShellSpec {
                program: "/usr/bin/pwsh".into(),
                args: vec![],
            };
        }
        let snap = shared.lock().unwrap().ui_snapshot();
        let installed = fake_installed();
        let menu = build_workspace_ctx_menu(&snap, &shared, 0.0, 0.0, 0, &installed);
        let active_items: Vec<&ElementDef> = collect_with_class(&menu, "ctx-menu-item")
            .into_iter()
            .filter(|el| has_class(el, "active"))
            .collect();
        assert!(
            active_items.iter().any(|el| item_text_contains(el, "pwsh")),
            "active class must mark the chip whose program matches workspace shell"
        );
    }

    #[test]
    fn workspace_ctx_menu_includes_use_app_default_when_override_set() {
        let shared = make_shared();
        {
            let mut guard = shared.lock().unwrap();
            guard.workspaces[0].shell = crate::shell::ShellSpec {
                program: "/usr/bin/pwsh".into(),
                args: vec![],
            };
        }
        let snap = shared.lock().unwrap().ui_snapshot();
        let installed = fake_installed();
        let menu = build_workspace_ctx_menu(&snap, &shared, 0.0, 0.0, 0, &installed);
        let text = collect_text_recursive(&menu);
        assert!(
            text.contains("Use app default"),
            "menu must include 'Use app default' when override is set, got text: {text:?}"
        );
    }

    #[test]
    fn workspace_ctx_menu_omits_use_app_default_when_no_override() {
        let shared = make_shared();
        let snap = shared.lock().unwrap().ui_snapshot();
        assert!(snap.workspaces[0].shell.is_empty());
        let installed = fake_installed();
        let menu = build_workspace_ctx_menu(&snap, &shared, 0.0, 0.0, 0, &installed);
        let text = collect_text_recursive(&menu);
        assert!(
            !text.contains("Use app default"),
            "menu must NOT include 'Use app default' when no override, got text: {text:?}"
        );
    }

    #[test]
    fn workspace_ctx_menu_clicking_shell_dispatches_set_workspace() {
        let shared = make_shared();
        let snap = shared.lock().unwrap().ui_snapshot();
        let installed = vec![std::path::PathBuf::from("/usr/bin/pwsh")];
        let menu = build_workspace_ctx_menu(&snap, &shared, 0.0, 0.0, 0, &installed);
        let pwsh_item = collect_with_class(&menu, "ctx-menu-item")
            .into_iter()
            .find(|el| item_text_contains(el, "pwsh"))
            .expect("pwsh item must be present");
        (pwsh_item.on_click.as_ref().expect("pwsh item on_click"))();
        let guard = shared.lock().unwrap();
        assert_eq!(guard.workspaces[0].shell.program, "/usr/bin/pwsh");
    }

    #[test]
    fn workspace_ctx_menu_clicking_use_app_default_clears_override() {
        let shared = make_shared();
        {
            let mut guard = shared.lock().unwrap();
            guard.workspaces[0].shell = crate::shell::ShellSpec {
                program: "/usr/bin/pwsh".into(),
                args: vec![],
            };
        }
        let snap = shared.lock().unwrap().ui_snapshot();
        let installed = fake_installed();
        let menu = build_workspace_ctx_menu(&snap, &shared, 0.0, 0.0, 0, &installed);
        let item = collect_with_class(&menu, "ctx-menu-item")
            .into_iter()
            .find(|el| item_text_contains(el, "Use app default"))
            .expect("Use app default item must be present");
        (item.on_click.as_ref().expect("use default on_click"))();
        let guard = shared.lock().unwrap();
        assert!(
            guard.workspaces[0].shell.is_empty(),
            "clicking Use app default must clear the workspace override"
        );
    }
}
