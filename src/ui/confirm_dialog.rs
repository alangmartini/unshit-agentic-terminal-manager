//! Confirmation dialog overlay for destructive actions.
//!
//! Rendered whenever `AppState.confirm_dialog` is populated. Two
//! buttons: Confirm dispatches the queued action via
//! `dialog.confirm`; Cancel clears the dialog via `dialog.cancel`.
//! The `CloseApp` variant adds a "remember my choice" checkbox and
//! dispatches `app.close.keep_running` / `app.close.kill_and_quit`
//! instead of going through `dialog.confirm`; the close-app UI
//! handlers follow up with `process::exit(0)` to drive the real exit.
//! The `RenameSession` variant shows a text input and commits via
//! `dialog.rename_commit` so the commit handler can pull the typed
//! buffer before clearing the dialog.
//! The modal overlay itself blocks clicks to everything behind it.

use std::collections::BTreeSet;

use unshit::core::element::*;
use unshit::core::style::parse::StyleDeclaration;
use unshit::core::style::types::{AlignItems, CssPosition, Dimension, JustifyContent};

use crate::state::{dispatch, mutate_with, ConfirmDialog, SharedState, UiSnapshot};
use crate::ui::icons::{icon_check, icon_close, svg_icon};

/// Build the confirmation modal overlay. Returns an empty hidden div
/// when no dialog is active so the caller can always include this in
/// the root tree unconditionally.
pub fn build_confirm_dialog_overlay(snap: &UiSnapshot, shared: &SharedState) -> ElementDef {
    let Some(dialog) = snap.confirm_dialog.as_ref() else {
        return ElementDef::new(Tag::Div).with_class("confirm-dialog-hidden");
    };

    let is_close_app = matches!(dialog, ConfirmDialog::CloseApp { .. });
    let card = match dialog {
        ConfirmDialog::KillWorkspace { name, .. } => build_simple_confirm_card(
            "Kill all terminals in workspace",
            &format!(
                "Every shell in workspace \"{}\" will be killed and the workspace will be left empty. This cannot be undone.",
                name
            ),
            "Kill all",
            shared,
        ),
        ConfirmDialog::KillAll { count } => build_simple_confirm_card(
            "Kill all terminals",
            &format!(
                "{} running shell{} across every workspace will be killed. All workspaces will be emptied. This cannot be undone.",
                count,
                if *count == 1 { "" } else { "s" }
            ),
            "Kill everything",
            shared,
        ),
        ConfirmDialog::CloseApp {
            count,
            remember,
            kept_pane_ids,
        } => build_close_app_card(
            *count,
            *remember,
            kept_pane_ids,
            close_dialog_entries(snap),
            shared,
        ),
        ConfirmDialog::RenameSession {
            pane_id,
            buffer,
            error,
        } => build_rename_session_card(*pane_id, buffer, error.as_deref(), shared),
    };

    let backdrop_shared = shared.clone();
    let mut overlay = ElementDef::new(Tag::Div)
        .with_class("confirm-dialog-overlay")
        .with_id("confirm-dialog-overlay")
        .with_style(StyleDeclaration::Position(CssPosition::Fixed))
        .with_style(StyleDeclaration::Top(Dimension::Px(0.0)))
        .with_style(StyleDeclaration::Right(Dimension::Px(0.0)))
        .with_style(StyleDeclaration::Bottom(Dimension::Px(0.0)))
        .with_style(StyleDeclaration::Left(Dimension::Px(0.0)))
        .with_style(StyleDeclaration::AlignItems(AlignItems::Center))
        .with_style(StyleDeclaration::JustifyContent(JustifyContent::Center))
        .on_click(move || {
            mutate_with(&backdrop_shared, |st| {
                dispatch(st, "dialog.cancel");
            });
        })
        .with_child(card);
    if is_close_app {
        overlay = overlay.with_class("cd-scrim");
    }
    overlay
}

fn build_simple_confirm_card(
    title: &str,
    body: &str,
    confirm_label: &str,
    shared: &SharedState,
) -> ElementDef {
    let cancel_shared = shared.clone();
    let cancel = ElementDef::new(Tag::Button)
        .with_class("confirm-dialog-button")
        .with_class("cancel")
        .with_class("ghost")
        .on_click(move || {
            mutate_with(&cancel_shared, |st| {
                dispatch(st, "dialog.cancel");
            });
        })
        .with_child(ElementDef::new(Tag::Span).with_text("Cancel".to_string()));

    let confirm_shared = shared.clone();
    let confirm = ElementDef::new(Tag::Div)
        .with_class("confirm-dialog-button")
        .with_class("danger")
        .on_click(move || {
            mutate_with(&confirm_shared, |st| {
                dispatch(st, "dialog.confirm");
            });
        })
        .with_child(ElementDef::new(Tag::Span).with_text(confirm_label.to_string()));

    ElementDef::new(Tag::Div)
        .with_class("confirm-dialog-card")
        .with_class("confirm-dialog-simple-card")
        .on_click(|| {})
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("confirm-dialog-title")
                .with_text(title.to_string()),
        )
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("confirm-dialog-body")
                .with_text(body.to_string()),
        )
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("confirm-dialog-buttons")
                .with_child(cancel)
                .with_child(confirm),
        )
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CloseDialogEntry {
    pane_id: u32,
    label: String,
    path: String,
    meta: String,
    agent: Option<&'static str>,
}

fn build_close_app_card(
    count: usize,
    remember: bool,
    kept_pane_ids: &BTreeSet<u32>,
    entries: Vec<CloseDialogEntry>,
    shared: &SharedState,
) -> ElementDef {
    let entries: Vec<CloseDialogEntry> = if count == 0 {
        Vec::new()
    } else {
        entries.into_iter().take(count).collect()
    };
    let agent_count = entries.iter().filter(|entry| entry.agent.is_some()).count();
    let kept_count = entries
        .iter()
        .filter(|entry| kept_pane_ids.contains(&entry.pane_id))
        .count();

    let cancel_shared = shared.clone();
    let cancel = ElementDef::new(Tag::Button)
        .with_class("confirm-dialog-button")
        .with_class("cancel")
        .with_class("ghost")
        .on_click(move || {
            mutate_with(&cancel_shared, |st| {
                dispatch(st, "dialog.cancel");
            });
        })
        .with_child(ElementDef::new(Tag::Span).with_text("cancel".to_string()));

    let keep_shared = shared.clone();
    let keep_label = if count == 0 {
        "quit".to_string()
    } else if kept_count == count {
        "keep running".to_string()
    } else {
        "keep selected".to_string()
    };
    let keep_running = ElementDef::new(Tag::Button)
        .with_class("confirm-dialog-button")
        .with_class("secondary")
        .on_click(move || {
            mutate_with(&keep_shared, |st| {
                dispatch(st, "app.close.keep_running");
            });
            crate::shutdown_now();
        })
        .with_child(ElementDef::new(Tag::Span).with_text(keep_label))
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("kbd")
                .with_text("Enter".to_string()),
        );

    let kill_shared = shared.clone();
    let kill_label = if count == 0 {
        "quit".to_string()
    } else {
        format!("kill {count} & quit")
    };
    let kill_and_quit = ElementDef::new(Tag::Button)
        .with_class("confirm-dialog-button")
        .with_class("danger")
        .on_click(move || {
            mutate_with(&kill_shared, |st| {
                dispatch(st, "app.close.kill_and_quit");
            });
            crate::shutdown_now();
        })
        .with_child(ElementDef::new(Tag::Span).with_text(kill_label));

    let remember_shared = shared.clone();
    let checkbox = ElementDef::new(Tag::Div)
        .with_class("cd-check")
        .with_class("confirm-dialog-checkbox")
        .with_class(if remember { "checked" } else { "unchecked" })
        .on_click(move || {
            mutate_with(&remember_shared, |st| {
                dispatch(st, "dialog.toggle_remember");
            });
        })
        .with_child(build_check_box(
            "cd-box confirm-dialog-checkbox-box",
            remember,
        ))
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("confirm-dialog-checkbox-label")
                .with_text("remember choice for this workspace".to_string()),
        );

    let mut body = ElementDef::new(Tag::Div)
        .with_class("cd-body")
        .with_child(build_close_blurb(count, agent_count, kept_count));
    body = body.with_child(build_close_session_list(entries, kept_pane_ids, shared));
    body = body.with_child(checkbox);

    ElementDef::new(Tag::Div)
        .with_class("cd-panel")
        .with_class("confirm-dialog-close-card")
        .on_click(|| {})
        .with_child(build_close_header(shared))
        .with_child(body)
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("cd-foot")
                .with_class("confirm-dialog-buttons")
                .with_child(cancel)
                .with_child(ElementDef::new(Tag::Span).with_class("cd-spacer"))
                .with_child(keep_running)
                .with_child(kill_and_quit),
        )
}

fn build_close_header(shared: &SharedState) -> ElementDef {
    let close_shared = shared.clone();
    ElementDef::new(Tag::Div)
        .with_class("cd-head")
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("cd-mark")
                .with_text("\u{25C6}".to_string()),
        )
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("cd-title")
                .with_class("confirm-dialog-title")
                .with_text("close terminal.mgr?".to_string()),
        )
        .with_child(ElementDef::new(Tag::Span).with_class("cd-spacer"))
        .with_child(
            ElementDef::new(Tag::Button)
                .with_class("icon-btn")
                .with_class("cd-close")
                .on_click(move || {
                    mutate_with(&close_shared, |st| {
                        dispatch(st, "dialog.cancel");
                    });
                })
                .with_child(svg_icon(icon_close())),
        )
}

fn build_close_blurb(count: usize, agent_count: usize, kept_count: usize) -> ElementDef {
    let session_word = if count == 1 { "session" } else { "sessions" };
    let mut blurb = ElementDef::new(Tag::Div)
        .with_class("cd-blurb")
        .with_class("confirm-dialog-body")
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("amber")
                .with_class("tnum")
                .with_text(count.to_string()),
        )
        .with_child(ElementDef::new(Tag::Span).with_text(format!(" running {session_word}")));
    if agent_count > 0 {
        let agent_word = if agent_count == 1 { "agent" } else { "agents" };
        blurb = blurb
            .with_child(ElementDef::new(Tag::Span).with_text(" \u{00B7} ".to_string()))
            .with_child(
                ElementDef::new(Tag::Span)
                    .with_class("violet")
                    .with_class("tnum")
                    .with_text(agent_count.to_string()),
            )
            .with_child(ElementDef::new(Tag::Span).with_text(format!(" attached {agent_word}")));
    }
    let suffix = if count == 0 {
        ". ptyd has no live sessions for this window.".to_string()
    } else if kept_count < count {
        format!(
            ". {kept_count} selected will stay alive; {} unselected will be killed.",
            count - kept_count
        )
    } else {
        ". ptyd will keep them alive in the background unless you kill them.".to_string()
    };
    blurb.with_child(
        ElementDef::new(Tag::Span)
            .with_class("dim")
            .with_text(suffix),
    )
}

fn build_close_session_list(
    entries: Vec<CloseDialogEntry>,
    kept_pane_ids: &BTreeSet<u32>,
    shared: &SharedState,
) -> ElementDef {
    let mut list = ElementDef::new(Tag::Div).with_class("cd-list");
    if entries.is_empty() {
        return list.with_child(
            ElementDef::new(Tag::Div)
                .with_class("cd-row")
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("cd-keep-box")
                        .with_class("status-idle"),
                )
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("cd-label")
                        .with_text("no live sessions".to_string()),
                )
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("cd-meta")
                        .with_class("path")
                        .with_text("ptyd".to_string()),
                )
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("cd-meta")
                        .with_class("dim")
                        .with_class("tnum")
                        .with_text("idle".to_string()),
                ),
        );
    }

    for entry in entries.iter() {
        list = list.with_child(build_close_session_row(
            entry,
            kept_pane_ids.contains(&entry.pane_id),
            shared,
        ));
    }
    list
}

fn build_close_session_row(
    entry: &CloseDialogEntry,
    kept: bool,
    shared: &SharedState,
) -> ElementDef {
    let status_class = if entry.agent.is_some() {
        "status-agent"
    } else {
        "status-running"
    };
    let meta = if let Some(agent) = entry.agent {
        ElementDef::new(Tag::Span).with_class("cd-meta").with_child(
            ElementDef::new(Tag::Span)
                .with_class("badge")
                .with_class("violet")
                .with_text(agent.to_string()),
        )
    } else {
        ElementDef::new(Tag::Span)
            .with_class("cd-meta")
            .with_class("path")
            .with_text(entry.path.clone())
    };

    let toggle_shared = shared.clone();
    let command = format!("dialog.toggle_keep:{}", entry.pane_id);
    ElementDef::new(Tag::Div)
        .with_class("cd-row")
        .with_class("cd-row-selectable")
        .with_class(if kept { "kept" } else { "not-kept" })
        .on_click(move || {
            let cmd = command.clone();
            mutate_with(&toggle_shared, |st| {
                dispatch(st, &cmd);
            });
        })
        .with_child(build_check_box(
            &format!("cd-keep-box {status_class}"),
            kept,
        ))
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("cd-label")
                .with_text(entry.label.clone()),
        )
        .with_child(meta)
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("cd-meta")
                .with_class("dim")
                .with_class("tnum")
                .with_text(entry.meta.clone()),
        )
}

fn build_check_box(classes: &str, checked: bool) -> ElementDef {
    let mut box_el = ElementDef::new(Tag::Span).with_class(classes);
    if checked {
        box_el = box_el.with_child(svg_icon(icon_check()).with_class("cd-check-icon"));
    }
    box_el
}

fn close_dialog_entries(snap: &UiSnapshot) -> Vec<CloseDialogEntry> {
    let mut out = Vec::new();
    for (idx, workspace) in snap.workspaces.iter().enumerate() {
        let path = workspace_display_path(workspace);
        let tabs = if idx == snap.active_workspace {
            &snap.tabs
        } else {
            &workspace.tabs
        };
        for (tab_idx, tab) in tabs.iter().enumerate() {
            let panes = if idx == snap.active_workspace && tab_idx == snap.active_tab {
                &snap.panes
            } else {
                &tab.panes
            };
            for pane in panes.iter().flatten() {
                out.push(CloseDialogEntry {
                    pane_id: pane.id.0,
                    label: pane.title.clone(),
                    path: path.clone(),
                    meta: close_entry_meta(pane),
                    agent: agent_label(pane),
                });
            }
        }
    }
    out
}

fn workspace_display_path(workspace: &crate::state::Workspace) -> String {
    workspace
        .path
        .as_deref()
        .and_then(|path| path.file_name())
        .map(|name| format!("~/{}", name.to_string_lossy()))
        .unwrap_or_else(|| format!("~/{}", workspace.name))
}

fn close_entry_meta(pane: &crate::state::Pane) -> String {
    let shell = pane.subtitle.trim();
    if shell.is_empty() {
        format!("pane {}", pane.id.0)
    } else {
        shell.to_string()
    }
}

fn agent_label(pane: &crate::state::Pane) -> Option<&'static str> {
    let title = pane.title.to_ascii_lowercase();
    let subtitle = pane.subtitle.to_ascii_lowercase();
    if title.contains("claude") || subtitle.contains("claude") {
        Some("CLAUDE")
    } else if title.contains("codex") || subtitle.contains("codex") {
        Some("CODEX")
    } else if title.starts_with("qp:") {
        Some("AGENT")
    } else {
        None
    }
}

/// Card body for the `RenameSession` dialog. The input is seeded with the
/// session's current name (`buffer`) and autofocused so the user can edit
/// or retype immediately; its on_change keeps the dialog buffer in sync so
/// the commit handler can read the typed value. Submitting an empty field
/// clears the custom name (the placeholder is shown only when the field is
/// emptied).
///
/// `error` carries an inline failure message under the input when
/// the most recent rename RPC came back Err. Typing into the input
/// clears it so a retry does not show stale text.
fn build_rename_session_card(
    pane_id: u32,
    buffer: &str,
    error: Option<&str>,
    shared: &SharedState,
) -> ElementDef {
    let input_shared = shared.clone();
    let submit_shared = shared.clone();
    let input = ElementDef::new(Tag::Input)
        .with_class("confirm-dialog-input")
        .with_placeholder("New session name")
        .with_value(buffer)
        .with_autofocus(true)
        .on_change(move |text| {
            let typed = text.to_string();
            mutate_with(&input_shared, |st| {
                if let Some(ConfirmDialog::RenameSession { buffer, error, .. }) =
                    st.confirm_dialog.as_mut()
                {
                    *buffer = typed;
                    *error = None;
                }
            });
        })
        .on_submit(move |text| {
            let typed = text.to_string();
            mutate_with(&submit_shared, |st| {
                if let Some(ConfirmDialog::RenameSession { buffer, .. }) =
                    st.confirm_dialog.as_mut()
                {
                    *buffer = typed;
                }
                dispatch(st, "dialog.rename_commit");
            });
        });

    let cancel_shared = shared.clone();
    let cancel = ElementDef::new(Tag::Div)
        .with_class("confirm-dialog-button")
        .with_class("cancel")
        .on_click(move || {
            mutate_with(&cancel_shared, |st| {
                dispatch(st, "dialog.cancel");
            });
        })
        .with_child(ElementDef::new(Tag::Span).with_text("Cancel".to_string()));

    let save_shared = shared.clone();
    let save = ElementDef::new(Tag::Button)
        .with_class("confirm-dialog-button")
        .with_class("primary")
        .on_click(move || {
            mutate_with(&save_shared, |st| {
                dispatch(st, "dialog.rename_commit");
            });
        })
        .with_child(ElementDef::new(Tag::Span).with_text("Save".to_string()));

    let mut card = ElementDef::new(Tag::Div)
        .with_class("confirm-dialog-card")
        .with_class("confirm-dialog-simple-card")
        .with_class("confirm-dialog-rename-card")
        .with_id(format!("confirm-dialog-rename-{pane_id}"))
        .on_click(|| {})
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("confirm-dialog-title")
                .with_text("Rename session".to_string()),
        )
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("confirm-dialog-body")
                .with_text(
                    "Type a new name for this session. Leave empty to clear the custom name."
                        .to_string(),
                ),
        )
        .with_child(input);
    if let Some(msg) = error {
        card = card.with_child(
            ElementDef::new(Tag::Div)
                .with_class("rename-session-error")
                .with_text(msg.to_string()),
        );
    }
    card.with_child(
        ElementDef::new(Tag::Div)
            .with_class("confirm-dialog-buttons")
            .with_child(cancel)
            .with_child(save),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{seed_state, SharedState};
    use std::sync::{Arc, Mutex};
    use unshit::core::style::types::{Background, Color, Edges, FontWeight};
    use unshit_test::TestHarness;

    fn shared() -> SharedState {
        Arc::new(Mutex::new(seed_state()))
    }

    fn close_app_dialog(count: usize, remember: bool, kept_pane_ids: &[u32]) -> ConfirmDialog {
        ConfirmDialog::CloseApp {
            count,
            remember,
            kept_pane_ids: kept_pane_ids.iter().copied().collect(),
        }
    }

    #[test]
    fn hidden_when_no_dialog_active() {
        let snap = shared().lock().unwrap().ui_snapshot();
        let el = build_confirm_dialog_overlay(&snap, &shared());
        assert!(el.classes.iter().any(|c| c == "confirm-dialog-hidden"));
    }

    #[test]
    fn shows_overlay_when_kill_workspace_dialog_active() {
        let s = shared();
        {
            let mut guard = s.lock().unwrap();
            guard.confirm_dialog = Some(ConfirmDialog::KillWorkspace {
                workspace_idx: 0,
                name: "my-ws".to_string(),
            });
        }
        let snap = s.lock().unwrap().ui_snapshot();
        let el = build_confirm_dialog_overlay(&snap, &s);
        assert!(el.classes.iter().any(|c| c == "confirm-dialog-overlay"));
    }

    #[test]
    fn close_app_dialog_renders_three_buttons() {
        let s = shared();
        {
            let mut guard = s.lock().unwrap();
            guard.confirm_dialog = Some(close_app_dialog(3, false, &[1, 2, 3]));
        }
        let snap = s.lock().unwrap().ui_snapshot();
        let el = build_confirm_dialog_overlay(&snap, &s);
        let buttons = find_by_class(&el, "confirm-dialog-buttons").expect("buttons row");
        assert_eq!(
            buttons
                .children
                .iter()
                .filter(|child| has_class(child, "confirm-dialog-button"))
                .count(),
            3
        );
    }

    #[test]
    fn close_app_dialog_checkbox_reflects_remember_state() {
        let s = shared();
        {
            let mut guard = s.lock().unwrap();
            guard.confirm_dialog = Some(close_app_dialog(0, true, &[]));
        }
        let snap = s.lock().unwrap().ui_snapshot();
        let el = build_confirm_dialog_overlay(&snap, &s);
        let checkbox = find_by_class(&el, "confirm-dialog-checkbox").expect("checkbox element");
        assert!(checkbox.classes.iter().any(|c| c == "checked"));
    }

    #[test]
    fn close_app_dialog_matches_design_system_shell_and_real_rows() {
        let s = shared();
        {
            let mut guard = s.lock().unwrap();
            guard.panes = vec![vec![
                crate::state::Pane {
                    id: crate::state::PaneId(7),
                    title: "frontend-watch".into(),
                    subtitle: "bash".into(),
                    pid: 0,
                    cpu: 0.0,
                },
                crate::state::Pane {
                    id: crate::state::PaneId(8),
                    title: "qp: repair close flow".into(),
                    subtitle: "claude".into(),
                    pid: 0,
                    cpu: 0.0,
                },
            ]];
            guard.confirm_dialog = Some(close_app_dialog(2, false, &[7, 8]));
        }
        let snap = s.lock().unwrap().ui_snapshot();
        let el = build_confirm_dialog_overlay(&snap, &s);

        assert!(has_class(&el, "cd-scrim"));
        assert!(has_class(&el.children[0], "cd-panel"));
        for class in ["cd-head", "cd-body", "cd-list", "cd-foot", "cd-check"] {
            assert!(has_class_anywhere(&el, class), "missing {class}");
        }
        assert!(has_class_anywhere(&el, "status-running"));
        assert!(has_class_anywhere(&el, "status-agent"));
        assert!(has_class_anywhere(&el, "badge"));

        let text = normalized_text(&el);
        assert!(text.contains("close terminal.mgr?"));
        assert!(text.contains("2 running sessions"));
        assert!(text.contains("1 attached agent"));
        assert!(text.contains("ptyd will keep them alive"));
        assert!(text.contains("frontend-watch"));
        assert!(text.contains("CLAUDE"));
        assert!(!text.contains("refactor-userlist"));
        assert!(!text.contains("api-server"));
    }

    #[test]
    fn close_app_dialog_lists_reasonable_session_counts_without_more_row() {
        let entries = (0..6)
            .map(|idx| CloseDialogEntry {
                pane_id: idx,
                label: format!("shell-{idx}"),
                path: "~/main".to_string(),
                meta: "bash".to_string(),
                agent: None,
            })
            .collect::<Vec<_>>();

        let kept_pane_ids = (0..6).collect();
        let el = build_close_session_list(entries, &kept_pane_ids, &shared());
        let text = normalized_text(&el);
        assert!(text.contains("shell-0"));
        assert!(text.contains("shell-5"));
        assert!(!text.contains("more"));
    }

    #[test]
    fn close_app_dialog_rows_render_keep_checkboxes() {
        let entries = vec![
            CloseDialogEntry {
                pane_id: 1,
                label: "keep".to_string(),
                path: "~/main".to_string(),
                meta: "bash".to_string(),
                agent: None,
            },
            CloseDialogEntry {
                pane_id: 2,
                label: "kill".to_string(),
                path: "~/main".to_string(),
                meta: "bash".to_string(),
                agent: None,
            },
        ];
        let kept_pane_ids = BTreeSet::from([1]);
        let el = build_close_session_list(entries, &kept_pane_ids, &shared());
        let rows = find_all_by_class(&el, "cd-row-selectable");

        assert_eq!(rows.len(), 2);
        assert!(has_class(rows[0], "kept"));
        assert!(has_class(rows[1], "not-kept"));
        assert!(has_class_anywhere(rows[0], "cd-keep-box"));
        assert!(has_class_anywhere(rows[0], "cd-check-icon"));
        assert!(!has_class_anywhere(rows[1], "cd-check-icon"));
    }

    #[test]
    fn close_app_dialog_styles_have_visible_layout_with_stylesheet() {
        let s = shared();
        {
            let mut guard = s.lock().unwrap();
            guard.confirm_dialog = Some(close_app_dialog(1, true, &[1]));
        }
        let snap = s.lock().unwrap().ui_snapshot();
        let tree_snap = snap.clone();
        let tree_shared = s.clone();
        let mut harness = TestHarness::new(
            include_str!("../../assets/styles.css"),
            move || ElementTree {
                root: ElementDef::new(Tag::Div)
                    .with_class("app")
                    .with_child(build_confirm_dialog_overlay(&tree_snap, &tree_shared)),
            },
            1280.0,
            800.0,
        );
        harness.step();

        for selector in [
            ".cd-scrim",
            ".cd-panel",
            ".cd-head",
            ".cd-body",
            ".cd-list",
            ".cd-box",
            ".cd-foot",
        ] {
            let snap = harness.query(selector).expect(selector);
            assert!(
                snap.layout_rect.width > 0.0 && snap.layout_rect.height > 0.0,
                "{selector} should have non-zero layout, got {:?}",
                snap.layout_rect
            );
        }

        let title = harness.query(".cd-title").expect(".cd-title");
        assert!((title.computed_style.font_size - 13.0).abs() < 0.01);
        assert!((title.computed_style.line_height - 1.4).abs() < 0.01);
        assert_eq!(title.computed_style.font_weight, FontWeight::W(600));
        assert_eq!(
            title.computed_style.font_family,
            "JetBrains Mono, Berkeley Mono, SF Mono, Menlo, Consolas, monospace"
        );

        let blurb = harness.query(".cd-blurb").expect(".cd-blurb");
        assert!((blurb.computed_style.font_size - 12.0).abs() < 0.01);
        assert!((blurb.computed_style.line_height - 1.55).abs() < 0.01);

        let check = harness.query(".cd-check").expect(".cd-check");
        assert!((check.computed_style.font_size - 10.0).abs() < 0.01);
        assert!((check.computed_style.line_height - 1.4).abs() < 0.01);

        let button = harness
            .query(".cd-foot .confirm-dialog-button")
            .expect(".cd-foot .confirm-dialog-button");
        assert!((button.computed_style.font_size - 11.0).abs() < 0.01);
        assert!((button.computed_style.line_height - 1.4).abs() < 0.01);
        assert_eq!(button.computed_style.font_weight, FontWeight::W(600));

        let panel = harness.query(".cd-panel").expect(".cd-panel");
        assert_eq!(panel.computed_style.border_width, Edges::all(1.0));
        for selector in [".cd-head", ".cd-body", ".cd-foot"] {
            let row = harness.query(selector).expect(selector);
            assert!(
                (row.layout_rect.width - panel.layout_rect.width).abs() < 0.01,
                "{selector} should stretch to panel width: row={:?}, panel={:?}",
                row.layout_rect,
                panel.layout_rect
            );
        }

        let secondary = harness.query(".secondary").expect(".secondary");
        assert_eq!(secondary.computed_style.border_width, Edges::all(1.0));
        assert_eq!(
            secondary.computed_style.border_color,
            Color::rgb(0xb8, 0x85, 0x2c)
        );
        let kbd = harness.query(".secondary .kbd").expect(".secondary .kbd");
        assert!(matches!(
            kbd.computed_style.background,
            Background::Color(Color { a, .. }) if a > 80
        ));

        harness.hover_on(".secondary");
        let hovered_secondary = harness.query(".secondary").expect(".secondary after hover");
        assert_eq!(
            hovered_secondary.computed_style.background,
            Background::Color(Color::rgba(52, 44, 32, 234))
        );
    }

    #[test]
    fn rename_session_dialog_renders_input_and_save_cancel_buttons() {
        let s = shared();
        {
            let mut guard = s.lock().unwrap();
            guard.confirm_dialog = Some(ConfirmDialog::RenameSession {
                pane_id: 3,
                buffer: "old".into(),
                error: None,
            });
        }
        let snap = s.lock().unwrap().ui_snapshot();
        let el = build_confirm_dialog_overlay(&snap, &s);
        let card = &el.children[0];
        assert_eq!(card.id.as_deref(), Some("confirm-dialog-rename-3"));

        let input = card
            .children
            .iter()
            .find(|c| c.classes.iter().any(|cls| cls == "confirm-dialog-input"))
            .expect("input element");
        assert!(input.on_change.is_some());

        let buttons = card
            .children
            .iter()
            .find(|c| c.classes.iter().any(|cls| cls == "confirm-dialog-buttons"))
            .expect("buttons row");
        assert_eq!(buttons.children.len(), 2);
        assert!(buttons.children[1].classes.iter().any(|c| c == "primary"));
    }

    #[test]
    fn rename_dialog_styles_show_light_caret_and_save_hover_feedback() {
        let s = shared();
        {
            let mut guard = s.lock().unwrap();
            guard.confirm_dialog = Some(ConfirmDialog::RenameSession {
                pane_id: 3,
                buffer: "old".into(),
                error: None,
            });
        }
        let tree_shared = s.clone();
        let mut harness = TestHarness::new(
            include_str!("../../assets/styles.css"),
            move || {
                let snap = tree_shared.lock().unwrap().ui_snapshot();
                ElementTree {
                    root: ElementDef::new(Tag::Div)
                        .with_class("app")
                        .with_child(build_confirm_dialog_overlay(&snap, &tree_shared)),
                }
            },
            1280.0,
            800.0,
        );
        harness.step();

        let input = harness
            .query(".confirm-dialog-input")
            .expect("rename input should render");
        assert_eq!(
            input.computed_style.caret_color,
            Color::rgb(0xf6, 0xd9, 0x88)
        );

        let save_before = harness
            .query(".primary")
            .expect("save button before hover")
            .computed_style
            .background
            .clone();
        harness.hover_on(".primary");
        let save_after = harness
            .query(".primary")
            .expect("save button after hover")
            .computed_style
            .background
            .clone();
        assert_ne!(
            save_after, save_before,
            "save button hover must use a concrete background change"
        );
        assert_eq!(save_after, Background::Color(Color::rgb(0xd4, 0xa3, 0x48)));
    }

    #[test]
    fn rename_dialog_seeds_current_name_and_autofocuses_input() {
        let s = shared();
        {
            let mut guard = s.lock().unwrap();
            guard.confirm_dialog = Some(ConfirmDialog::RenameSession {
                pane_id: 3,
                buffer: "build-watch".into(),
                error: None,
            });
        }
        let snap = s.lock().unwrap().ui_snapshot();
        let el = build_confirm_dialog_overlay(&snap, &s);
        let card = &el.children[0];
        let input = card
            .children
            .iter()
            .find(|c| c.classes.iter().any(|cls| cls == "confirm-dialog-input"))
            .expect("input element");
        // The field is prefilled with the session's current name and
        // autofocused so the user can edit/retype without clicking.
        assert_eq!(input.value.as_deref(), Some("build-watch"));
        assert!(input.autofocus, "rename input should autofocus on open");
    }

    #[test]
    fn rename_dialog_input_on_change_updates_buffer_in_state() {
        let s = shared();
        {
            let mut guard = s.lock().unwrap();
            guard.confirm_dialog = Some(ConfirmDialog::RenameSession {
                pane_id: 3,
                buffer: String::new(),
                error: None,
            });
        }
        let snap = s.lock().unwrap().ui_snapshot();
        let el = build_confirm_dialog_overlay(&snap, &s);
        let card = &el.children[0];
        let input = card
            .children
            .iter()
            .find(|c| c.classes.iter().any(|cls| cls == "confirm-dialog-input"))
            .expect("input");
        (input.on_change.as_ref().unwrap())("api-server");
        let guard = s.lock().unwrap();
        match guard.confirm_dialog.as_ref() {
            Some(ConfirmDialog::RenameSession { buffer, .. }) => {
                assert_eq!(buffer, "api-server");
            }
            other => panic!("expected RenameSession dialog, got {other:?}"),
        }
    }

    #[test]
    fn rename_dialog_input_click_keeps_dialog_open() {
        let s = shared();
        {
            let mut guard = s.lock().unwrap();
            guard.confirm_dialog = Some(ConfirmDialog::RenameSession {
                pane_id: 3,
                buffer: String::new(),
                error: None,
            });
        }
        let tree_shared = s.clone();
        let mut harness = TestHarness::new(
            crate::STYLES,
            move || {
                let snap = tree_shared.lock().unwrap().ui_snapshot();
                ElementTree {
                    root: build_confirm_dialog_overlay(&snap, &tree_shared),
                }
            },
            800.0,
            600.0,
        );
        harness.step();

        harness.click_on(".confirm-dialog-input");

        assert!(
            matches!(
                s.lock().unwrap().confirm_dialog,
                Some(ConfirmDialog::RenameSession { .. })
            ),
            "clicking inside the rename input must not hit the backdrop cancel handler"
        );
    }

    #[test]
    fn rename_dialog_cancel_click_clears_dialog() {
        let s = shared();
        {
            let mut guard = s.lock().unwrap();
            guard.confirm_dialog = Some(ConfirmDialog::RenameSession {
                pane_id: 3,
                buffer: "x".into(),
                error: None,
            });
        }
        let snap = s.lock().unwrap().ui_snapshot();
        let el = build_confirm_dialog_overlay(&snap, &s);
        let card = &el.children[0];
        let buttons = card
            .children
            .iter()
            .find(|c| c.classes.iter().any(|cls| cls == "confirm-dialog-buttons"))
            .expect("buttons");
        let cancel = &buttons.children[0];
        (cancel.on_click.as_ref().unwrap())();
        assert!(s.lock().unwrap().confirm_dialog.is_none());
    }

    #[test]
    fn rename_dialog_save_click_commits_via_dispatch() {
        let s = shared();
        {
            let mut guard = s.lock().unwrap();
            guard.panes = vec![vec![crate::state::Pane {
                id: crate::state::PaneId(3),
                title: "old".into(),
                subtitle: "".into(),
                pid: 0,
                cpu: 0.0,
            }]];
            guard.confirm_dialog = Some(ConfirmDialog::RenameSession {
                pane_id: 3,
                buffer: "new-name".into(),
                error: None,
            });
        }
        let snap = s.lock().unwrap().ui_snapshot();
        let el = build_confirm_dialog_overlay(&snap, &s);
        let card = &el.children[0];
        let buttons = card
            .children
            .iter()
            .find(|c| c.classes.iter().any(|cls| cls == "confirm-dialog-buttons"))
            .expect("buttons");
        let save = &buttons.children[1];
        (save.on_click.as_ref().unwrap())();
        let guard = s.lock().unwrap();
        assert!(guard.confirm_dialog.is_none());
        assert_eq!(guard.panes[0][0].title, "new-name");
    }

    #[test]
    fn kill_workspace_dialog_confirm_click_consumes_dialog() {
        let s = shared();
        {
            let mut guard = s.lock().unwrap();
            guard.confirm_dialog = Some(ConfirmDialog::KillWorkspace {
                workspace_idx: 0,
                name: "ws".into(),
            });
        }
        let snap = s.lock().unwrap().ui_snapshot();
        let el = build_confirm_dialog_overlay(&snap, &s);
        let card = &el.children[0];
        let buttons = card
            .children
            .iter()
            .find(|c| c.classes.iter().any(|cls| cls == "confirm-dialog-buttons"))
            .expect("buttons");
        (buttons.children[1].on_click.as_ref().unwrap())();
        assert!(s.lock().unwrap().confirm_dialog.is_none());
    }

    #[test]
    fn close_app_dialog_checkbox_click_toggles_remember_flag() {
        let s = shared();
        {
            let mut guard = s.lock().unwrap();
            guard.confirm_dialog = Some(close_app_dialog(0, false, &[]));
        }
        let snap = s.lock().unwrap().ui_snapshot();
        let el = build_confirm_dialog_overlay(&snap, &s);
        let checkbox = find_by_class(&el, "confirm-dialog-checkbox").expect("checkbox");
        (checkbox.on_click.as_ref().unwrap())();
        assert!(matches!(
            s.lock().unwrap().confirm_dialog,
            Some(ConfirmDialog::CloseApp { remember: true, .. })
        ));
    }

    #[test]
    fn close_app_dialog_session_row_click_toggles_keep_selection() {
        let s = shared();
        {
            let mut guard = s.lock().unwrap();
            guard.panes = vec![vec![crate::state::Pane {
                id: crate::state::PaneId(1),
                title: "shell".into(),
                subtitle: "bash".into(),
                pid: 0,
                cpu: 0.0,
            }]];
            guard.confirm_dialog = Some(close_app_dialog(1, false, &[1]));
        }
        let snap = s.lock().unwrap().ui_snapshot();
        let el = build_confirm_dialog_overlay(&snap, &s);
        let row = find_by_class(&el, "cd-row-selectable").expect("selectable row");

        (row.on_click.as_ref().unwrap())();

        let guard = s.lock().unwrap();
        match guard.confirm_dialog.as_ref() {
            Some(ConfirmDialog::CloseApp { kept_pane_ids, .. }) => {
                assert!(!kept_pane_ids.contains(&1));
            }
            other => panic!("expected CloseApp dialog, got {other:?}"),
        }
    }

    #[test]
    fn rename_dialog_input_submit_commits_with_typed_value() {
        let s = shared();
        {
            let mut guard = s.lock().unwrap();
            guard.panes = vec![vec![crate::state::Pane {
                id: crate::state::PaneId(5),
                title: "old".into(),
                subtitle: "".into(),
                pid: 0,
                cpu: 0.0,
            }]];
            guard.confirm_dialog = Some(ConfirmDialog::RenameSession {
                pane_id: 5,
                buffer: "partial".into(),
                error: None,
            });
        }
        let snap = s.lock().unwrap().ui_snapshot();
        let el = build_confirm_dialog_overlay(&snap, &s);
        let card = &el.children[0];
        let input = card
            .children
            .iter()
            .find(|c| c.classes.iter().any(|cls| cls == "confirm-dialog-input"))
            .expect("input");
        (input.on_submit.as_ref().unwrap())("enter-to-commit");
        let guard = s.lock().unwrap();
        assert!(guard.confirm_dialog.is_none());
        assert_eq!(guard.panes[0][0].title, "enter-to-commit");
    }

    #[test]
    fn overlay_backdrop_click_cancels_dialog() {
        let s = shared();
        {
            let mut guard = s.lock().unwrap();
            guard.confirm_dialog = Some(ConfirmDialog::KillAll { count: 1 });
        }
        let snap = s.lock().unwrap().ui_snapshot();
        let el = build_confirm_dialog_overlay(&snap, &s);
        (el.on_click.as_ref().unwrap())();
        assert!(s.lock().unwrap().confirm_dialog.is_none());
    }

    // refs #130: rename dialog must render an inline error string
    // under the input when ConfirmDialog::RenameSession.error is Some,
    // and must omit it when None.
    #[test]
    fn rename_dialog_renders_inline_error_when_present() {
        let s = shared();
        {
            let mut guard = s.lock().unwrap();
            guard.confirm_dialog = Some(ConfirmDialog::RenameSession {
                pane_id: 3,
                buffer: "x".into(),
                error: None,
            });
        }
        let snap_clean = s.lock().unwrap().ui_snapshot();
        let clean = build_confirm_dialog_overlay(&snap_clean, &s);
        assert!(!has_class_anywhere(&clean, "rename-session-error"));

        {
            let mut guard = s.lock().unwrap();
            guard.confirm_dialog = Some(ConfirmDialog::RenameSession {
                pane_id: 3,
                buffer: "x".into(),
                error: Some("rename failed: not connected".into()),
            });
        }
        let snap_err = s.lock().unwrap().ui_snapshot();
        let with_err = build_confirm_dialog_overlay(&snap_err, &s);
        assert!(has_class_anywhere(&with_err, "rename-session-error"));
        assert!(text_anywhere(&with_err).contains("rename failed: not connected"));
    }

    fn has_class_anywhere(el: &ElementDef, class: &str) -> bool {
        if has_class(el, class) {
            return true;
        }
        el.children.iter().any(|c| has_class_anywhere(c, class))
    }

    fn has_class(el: &ElementDef, class: &str) -> bool {
        el.classes.iter().any(|c| c == class)
    }

    fn find_by_class<'a>(el: &'a ElementDef, class: &str) -> Option<&'a ElementDef> {
        if has_class(el, class) {
            return Some(el);
        }
        el.children.iter().find_map(|c| find_by_class(c, class))
    }

    fn find_all_by_class<'a>(el: &'a ElementDef, class: &str) -> Vec<&'a ElementDef> {
        let mut out = Vec::new();
        if has_class(el, class) {
            out.push(el);
        }
        for child in &el.children {
            out.extend(find_all_by_class(child, class));
        }
        out
    }

    fn text_anywhere(el: &ElementDef) -> String {
        let mut out = String::new();
        if let ElementContent::Text(s) = &el.content {
            out.push_str(s);
        }
        for c in &el.children {
            out.push(' ');
            out.push_str(&text_anywhere(c));
        }
        out
    }

    fn normalized_text(el: &ElementDef) -> String {
        text_anywhere(el)
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }
}
