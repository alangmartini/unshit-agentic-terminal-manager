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

use unshit::core::element::*;
use unshit::core::style::parse::StyleDeclaration;
use unshit::core::style::types::{AlignItems, CssPosition, Dimension, JustifyContent};

use crate::state::{dispatch, mutate_with, ConfirmDialog, SharedState, UiSnapshot};

/// Build the confirmation modal overlay. Returns an empty hidden div
/// when no dialog is active so the caller can always include this in
/// the root tree unconditionally.
pub fn build_confirm_dialog_overlay(snap: &UiSnapshot, shared: &SharedState) -> ElementDef {
    let Some(dialog) = snap.confirm_dialog.as_ref() else {
        return ElementDef::new(Tag::Div).with_class("confirm-dialog-hidden");
    };

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
        ConfirmDialog::CloseApp { count, remember } => {
            build_close_app_card(*count, *remember, shared)
        }
        ConfirmDialog::RenameSession { pane_id, buffer } => {
            build_rename_session_card(*pane_id, buffer, shared)
        }
    };

    let backdrop_shared = shared.clone();
    ElementDef::new(Tag::Div)
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
        .with_child(card)
}

fn build_simple_confirm_card(
    title: &str,
    body: &str,
    confirm_label: &str,
    shared: &SharedState,
) -> ElementDef {
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

fn build_close_app_card(count: usize, remember: bool, shared: &SharedState) -> ElementDef {
    let body = if count == 0 {
        "No terminals are currently running. Closing the window does not shut down the session daemon.".to_string()
    } else {
        format!(
            "{} running shell{} {} open. Choose whether to keep them running on the daemon, kill them before quitting, or stay in the app.",
            count,
            if count == 1 { "" } else { "s" },
            if count == 1 { "is" } else { "are" }
        )
    };

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

    let keep_shared = shared.clone();
    let keep_running = ElementDef::new(Tag::Div)
        .with_class("confirm-dialog-button")
        .on_click(move || {
            mutate_with(&keep_shared, |st| {
                dispatch(st, "app.close.keep_running");
            });
            crate::shutdown_now();
        })
        .with_child(ElementDef::new(Tag::Span).with_text("Keep running".to_string()));

    let kill_shared = shared.clone();
    let kill_and_quit = ElementDef::new(Tag::Div)
        .with_class("confirm-dialog-button")
        .with_class("danger")
        .on_click(move || {
            mutate_with(&kill_shared, |st| {
                dispatch(st, "app.close.kill_and_quit");
            });
            crate::shutdown_now();
        })
        .with_child(ElementDef::new(Tag::Span).with_text("Kill all and quit".to_string()));

    let remember_shared = shared.clone();
    let checkbox = ElementDef::new(Tag::Div)
        .with_class("confirm-dialog-checkbox")
        .with_class(if remember { "checked" } else { "unchecked" })
        .on_click(move || {
            mutate_with(&remember_shared, |st| {
                dispatch(st, "dialog.toggle_remember");
            });
        })
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("confirm-dialog-checkbox-box")
                .with_text(if remember {
                    "[x]".to_string()
                } else {
                    "[ ]".to_string()
                }),
        )
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("confirm-dialog-checkbox-label")
                .with_text("Remember my choice".to_string()),
        );

    ElementDef::new(Tag::Div)
        .with_class("confirm-dialog-card")
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("confirm-dialog-title")
                .with_text("Close Godly Terminal?".to_string()),
        )
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("confirm-dialog-body")
                .with_text(body),
        )
        .with_child(checkbox)
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("confirm-dialog-buttons")
                .with_child(cancel)
                .with_child(keep_running)
                .with_child(kill_and_quit),
        )
}

/// Card body for the `RenameSession` dialog. The input's on_change
/// keeps the dialog buffer in sync so the commit handler can read the
/// typed value. The framework does not seed an input's initial value
/// from the ElementDef, so the current name is shown as placeholder
/// text; submitting an empty field clears the custom name.
fn build_rename_session_card(pane_id: u32, _buffer: &str, shared: &SharedState) -> ElementDef {
    let input_shared = shared.clone();
    let input = ElementDef::new(Tag::Input)
        .with_class("confirm-dialog-input")
        .with_placeholder("New session name")
        .on_change(move |text| {
            let typed = text.to_string();
            mutate_with(&input_shared, |st| {
                if let Some(ConfirmDialog::RenameSession { buffer, .. }) =
                    st.confirm_dialog.as_mut()
                {
                    *buffer = typed;
                }
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
    let save = ElementDef::new(Tag::Div)
        .with_class("confirm-dialog-button")
        .with_class("primary")
        .on_click(move || {
            mutate_with(&save_shared, |st| {
                dispatch(st, "dialog.rename_commit");
            });
        })
        .with_child(ElementDef::new(Tag::Span).with_text("Save".to_string()));

    ElementDef::new(Tag::Div)
        .with_class("confirm-dialog-card")
        .with_id(format!("confirm-dialog-rename-{pane_id}"))
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
        .with_child(input)
        .with_child(
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

    fn shared() -> SharedState {
        Arc::new(Mutex::new(seed_state()))
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
            guard.confirm_dialog = Some(ConfirmDialog::CloseApp {
                count: 3,
                remember: false,
            });
        }
        let snap = s.lock().unwrap().ui_snapshot();
        let el = build_confirm_dialog_overlay(&snap, &s);
        // overlay -> card -> [title, body, checkbox, buttons]
        let card = &el.children[0];
        let buttons = card
            .children
            .iter()
            .find(|c| c.classes.iter().any(|cls| cls == "confirm-dialog-buttons"))
            .expect("buttons row");
        assert_eq!(buttons.children.len(), 3);
    }

    #[test]
    fn close_app_dialog_checkbox_reflects_remember_state() {
        let s = shared();
        {
            let mut guard = s.lock().unwrap();
            guard.confirm_dialog = Some(ConfirmDialog::CloseApp {
                count: 0,
                remember: true,
            });
        }
        let snap = s.lock().unwrap().ui_snapshot();
        let el = build_confirm_dialog_overlay(&snap, &s);
        let card = &el.children[0];
        let checkbox = card
            .children
            .iter()
            .find(|c| c.classes.iter().any(|cls| cls == "confirm-dialog-checkbox"))
            .expect("checkbox element");
        assert!(checkbox.classes.iter().any(|c| c == "checked"));
    }

    #[test]
    fn rename_session_dialog_renders_input_and_save_cancel_buttons() {
        let s = shared();
        {
            let mut guard = s.lock().unwrap();
            guard.confirm_dialog = Some(ConfirmDialog::RenameSession {
                pane_id: 3,
                buffer: "old".into(),
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
    fn rename_dialog_input_on_change_updates_buffer_in_state() {
        let s = shared();
        {
            let mut guard = s.lock().unwrap();
            guard.confirm_dialog = Some(ConfirmDialog::RenameSession {
                pane_id: 3,
                buffer: String::new(),
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
    fn rename_dialog_cancel_click_clears_dialog() {
        let s = shared();
        {
            let mut guard = s.lock().unwrap();
            guard.confirm_dialog = Some(ConfirmDialog::RenameSession {
                pane_id: 3,
                buffer: "x".into(),
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
            guard.confirm_dialog = Some(ConfirmDialog::CloseApp {
                count: 0,
                remember: false,
            });
        }
        let snap = s.lock().unwrap().ui_snapshot();
        let el = build_confirm_dialog_overlay(&snap, &s);
        let card = &el.children[0];
        let checkbox = card
            .children
            .iter()
            .find(|c| c.classes.iter().any(|cls| cls == "confirm-dialog-checkbox"))
            .expect("checkbox");
        (checkbox.on_click.as_ref().unwrap())();
        assert!(matches!(
            s.lock().unwrap().confirm_dialog,
            Some(ConfirmDialog::CloseApp { remember: true, .. })
        ));
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
}
