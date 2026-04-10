use unshit::core::element::*;

use crate::state::{mutate_with, SharedState, Subtab, UiSnapshot, Workspace};
use crate::ui::icons::*;

pub fn build_sidebar(state: &UiSnapshot, shared: &SharedState) -> ElementDef {
    let mut scroll = ElementDef::new(Tag::Div).with_class("sidebar-scroll");
    for (w_idx, workspace) in state.workspaces.iter().enumerate() {
        scroll = scroll.with_child(build_workspace(w_idx, workspace, shared));
    }

    let mut sidebar =
        ElementDef::new(Tag::Div).with_class("sidebar").with_class("role-aside").with_id("sidebar");
    if state.sidebar_collapsed {
        sidebar = sidebar.with_class("collapsed");
    }
    sidebar
        .with_child(build_sidebar_head())
        .with_child(scroll)
        .with_child(build_sidebar_footer())
        .with_child(build_sidebar_hints())
}

fn build_sidebar_head() -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("sidebar-head")
        .with_child(ElementDef::new(Tag::Span).with_class("sidebar-title").with_text("workspaces"))
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("sidebar-head-actions")
                .with_child(
                    ElementDef::new(Tag::Button)
                        .with_class("icon-btn")
                        .with_class("tight")
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
        .with_child(ElementDef::new(Tag::Span).with_class("chevron").with_text("\u{25BE}"))
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("workspace-num")
                .with_text(workspace.num.to_string()),
        )
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("workspace-name")
                .with_text(workspace.name.clone()),
        )
        .with_child(
            ElementDef::new(Tag::Span).with_class("workspace-meta").with_child({
                let mut branch_tag = ElementDef::new(Tag::Span)
                    .with_class("branch-tag")
                    .with_text(workspace.branch.clone());
                if workspace.branch_muted {
                    branch_tag = branch_tag.with_class("muted");
                }
                branch_tag
            }),
        );

    let mut body = ElementDef::new(Tag::Div).with_class("workspace-body");
    for (s_idx, subtab) in workspace.subtabs.iter().enumerate() {
        body = body.with_child(build_subtab(workspace_index, s_idx, subtab, shared));
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
    shared: &SharedState,
) -> ElementDef {
    let mut btn = ElementDef::new(Tag::Button).with_class("subtab");
    if subtab.active {
        btn = btn.with_class("active");
    }

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

    btn = btn.with_child(
        ElementDef::new(Tag::Span).with_class("tree-glyph").with_text(subtab.tree_glyph),
    );

    if let Some(icon) = subtab.icon {
        btn = btn.with_child(
            ElementDef::new(Tag::Span)
                .with_class("subtab-icon")
                .with_child(svg_icon(subtab_icon_for(icon))),
        );
    }

    btn = btn.with_child(
        ElementDef::new(Tag::Span).with_class("subtab-label").with_text(subtab.label.clone()),
    );

    if let Some(count) = subtab.count {
        let mut count_el =
            ElementDef::new(Tag::Span).with_class("subtab-count").with_text(count.to_string());
        if subtab.pulse {
            count_el = count_el.with_class("pulse");
        }
        btn = btn.with_child(count_el);
    }

    btn
}

fn build_sidebar_footer() -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("sidebar-footer")
        .with_child(ElementDef::new(Tag::Div).with_class("footer-title").with_text("activity"))
        .with_child(activity_item("running", "claude", "running", "refactor split pane logic"))
        .with_child(activity_item("stopped", "amp", "stopped", "verify readme docs"))
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
            ElementDef::new(Tag::Div).with_class("activity-desc").with_text(desc.to_string()),
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
        .with_child(ElementDef::new(Tag::Span).with_class("kbd").with_text(key.to_string()))
        .with_child(ElementDef::new(Tag::Span).with_text(label.to_string()))
}
