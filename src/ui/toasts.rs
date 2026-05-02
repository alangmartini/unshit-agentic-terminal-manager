//! Ephemeral toast overlay rendered above every other UI surface.
//!
//! The overlay is fixed to the bottom-right of the window and stacks
//! one card per live toast in `state.toasts`. Each card is clickable
//! and dispatches `toast.dismiss:<id>` to remove itself; the cursor
//! blink subscription in `bridge.rs` drives auto-dismiss by calling
//! `state.toasts.advance_ticks(1)` every 500 ms.
//!
//! Returns a hidden div when there are no live toasts so the caller
//! can include the overlay in the root tree unconditionally.
//!
//! Accessibility (`role` / `aria-live` announcement) is deferred until
//! the framework grows an attribute path and an AT bridge. See
//! `unshit-rust-framework#228`.

use unshit::core::element::*;
use unshit::core::style::parse::StyleDeclaration;
use unshit::core::style::types::{CssPosition, Dimension};

use crate::state::{dispatch, mutate_with, SharedState, UiSnapshot};

/// Build the toast stack overlay. Empty toasts produce a hidden div so
/// the caller does not have to special-case the empty list.
pub fn build_toast_overlay(snap: &UiSnapshot, shared: &SharedState) -> ElementDef {
    if snap.toasts.is_empty() {
        return ElementDef::new(Tag::Div).with_class("toast-overlay-hidden");
    }

    let mut overlay = ElementDef::new(Tag::Div)
        .with_class("toast-overlay")
        .with_id("toast-overlay")
        .with_style(StyleDeclaration::Position(CssPosition::Fixed))
        .with_style(StyleDeclaration::Right(Dimension::Px(16.0)))
        .with_style(StyleDeclaration::Bottom(Dimension::Px(40.0)));

    for toast in &snap.toasts {
        overlay = overlay.with_child(build_toast_card(toast, shared));
    }
    overlay
}

fn build_toast_card(view: &crate::state::ToastView, shared: &SharedState) -> ElementDef {
    let dispatch_shared = shared.clone();
    let id = view.id;
    let command = if view.target.is_some() {
        format!("notification.activate:{id}")
    } else {
        format!("toast.dismiss:{id}")
    };
    let mut card = ElementDef::new(Tag::Div)
        .with_class("toast")
        .with_class(if view.target.is_some() {
            "toast-notification"
        } else {
            "toast-error"
        })
        .with_id(format!("toast-{id}"))
        .on_click(move || {
            mutate_with(&dispatch_shared, |st| {
                dispatch(st, &command);
            });
        });
    if let Some(title) = &view.title {
        card = card.with_child(
            ElementDef::new(Tag::Div)
                .with_class("toast-title")
                .with_text(title.clone()),
        );
    }
    card.with_child(
        ElementDef::new(Tag::Div)
            .with_class("toast-text")
            .with_text(view.message.clone()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{push_error_toast, seed_state};
    use std::sync::{Arc, Mutex};

    fn make_shared_with_toasts(messages: &[&str]) -> (UiSnapshot, SharedState) {
        let mut state = seed_state();
        for m in messages {
            push_error_toast(&mut state, *m);
        }
        let snap = state.ui_snapshot();
        let shared = Arc::new(Mutex::new(state));
        (snap, shared)
    }

    #[test]
    fn toast_overlay_empty_returns_hidden_div() {
        let (snap, shared) = make_shared_with_toasts(&[]);
        let el = build_toast_overlay(&snap, &shared);
        assert!(matches!(el.tag, Tag::Div));
        assert!(el.classes.contains(&"toast-overlay-hidden".to_string()));
        assert!(el.children.is_empty());
    }

    #[test]
    fn toast_overlay_renders_one_card_per_toast() {
        let (snap, shared) = make_shared_with_toasts(&["a", "b", "c"]);
        let el = build_toast_overlay(&snap, &shared);
        assert!(el.classes.contains(&"toast-overlay".to_string()));
        assert_eq!(el.children.len(), 3);
        let messages: Vec<String> = el
            .children
            .iter()
            .filter_map(|c| c.children.last())
            .filter_map(|body| match &body.content {
                ElementContent::Text(s) => Some(s.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(messages, vec!["a", "b", "c"]);
    }

    #[test]
    fn toast_card_click_dispatches_dismiss() {
        let (snap, shared) = make_shared_with_toasts(&["bye"]);
        let el = build_toast_overlay(&snap, &shared);
        let card = el.children.first().expect("one card");
        let handler = card.on_click.as_ref().expect("click handler");
        handler();
        let after = shared.lock().expect("lock").toasts.len();
        assert_eq!(after, 0);
    }

    #[test]
    fn notification_card_renders_title_and_body() {
        let mut state = seed_state();
        crate::state::push_notification_toast(&mut state, "Build done", "Tests passed", 1, 1);
        let snap = state.ui_snapshot();
        let shared = Arc::new(Mutex::new(state));

        let el = build_toast_overlay(&snap, &shared);
        let card = el.children.first().expect("one card");
        assert!(card.classes.contains(&"toast-notification".to_string()));
        let title = card.children.first().expect("title");
        let body = card.children.last().expect("body");
        assert!(matches!(&title.content, ElementContent::Text(s) if s == "Build done"));
        assert!(matches!(&body.content, ElementContent::Text(s) if s == "Tests passed"));
    }
}
