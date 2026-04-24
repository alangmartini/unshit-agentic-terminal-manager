//! Confirmation dialog overlay for destructive actions.
//!
//! Rendered whenever `AppState.confirm_dialog` is populated. Two
//! buttons: Confirm dispatches the queued action via
//! `dialog.confirm`; Cancel clears the dialog via `dialog.cancel`.
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

    let (title, body, confirm_label) = match dialog {
        ConfirmDialog::KillWorkspace { name, .. } => (
            "Kill all terminals in workspace".to_string(),
            format!(
                "Every shell in workspace \"{}\" will be killed and the workspace will be left empty. This cannot be undone.",
                name
            ),
            "Kill all".to_string(),
        ),
        ConfirmDialog::KillAll { count } => (
            "Kill all terminals".to_string(),
            format!(
                "{} running shell{} across every workspace will be killed. All workspaces will be emptied. This cannot be undone.",
                count,
                if *count == 1 { "" } else { "s" }
            ),
            "Kill everything".to_string(),
        ),
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

    let confirm_shared = shared.clone();
    let confirm = ElementDef::new(Tag::Div)
        .with_class("confirm-dialog-button")
        .with_class("danger")
        .on_click(move || {
            mutate_with(&confirm_shared, |st| {
                dispatch(st, "dialog.confirm");
            });
        })
        .with_child(ElementDef::new(Tag::Span).with_text(confirm_label));

    let card = ElementDef::new(Tag::Div)
        .with_class("confirm-dialog-card")
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("confirm-dialog-title")
                .with_text(title),
        )
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("confirm-dialog-body")
                .with_text(body),
        )
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("confirm-dialog-buttons")
                .with_child(cancel)
                .with_child(confirm),
        );

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
            // Clicking the backdrop cancels, matching the settings-modal
            // pattern. The card stops event propagation so clicks inside
            // the card do not cancel.
            mutate_with(&backdrop_shared, |st| {
                dispatch(st, "dialog.cancel");
            });
        })
        .with_child(card)
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
}
