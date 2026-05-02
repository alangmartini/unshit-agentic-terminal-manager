//! Render the Quick Prompt overlay.
//!
//! Returns a fixed-position backdrop with a centered card. When the
//! overlay is closed (`snap.quick_prompt.is_none()`) the function still
//! returns a hidden element so callers can include the result in the
//! root tree unconditionally without a branch in `main.rs`.

use unshit::core::element::*;
use unshit::core::style::parse::StyleDeclaration;
use unshit::core::style::types::{AlignItems, CssPosition, Dimension, JustifyContent};

use crate::state::{dispatch, mutate_with, SharedState, UiSnapshot};

/// Build the overlay tree. Always returns an element so the root render
/// in `main.rs` does not need a conditional branch.
pub fn build_quick_prompt_overlay(snap: &UiSnapshot, shared: &SharedState) -> ElementDef {
    if snap.quick_prompt.is_none() {
        return ElementDef::new(Tag::Div).with_class("quick-prompt-hidden");
    }

    let card = build_quick_prompt_card();

    let backdrop_shared = shared.clone();
    ElementDef::new(Tag::Div)
        .with_class("quick-prompt-overlay")
        .with_id("quick-prompt-overlay")
        .with_style(StyleDeclaration::Position(CssPosition::Fixed))
        .with_style(StyleDeclaration::Top(Dimension::Px(0.0)))
        .with_style(StyleDeclaration::Right(Dimension::Px(0.0)))
        .with_style(StyleDeclaration::Bottom(Dimension::Px(0.0)))
        .with_style(StyleDeclaration::Left(Dimension::Px(0.0)))
        .with_style(StyleDeclaration::AlignItems(AlignItems::Center))
        .with_style(StyleDeclaration::JustifyContent(JustifyContent::Center))
        .on_click(move || {
            mutate_with(&backdrop_shared, |st| {
                dispatch(st, "quick_prompt.close");
            });
        })
        .with_child(card)
}

/// Slice 1 placeholder card: just a title. Slice 2 fills it with the
/// prompt input, agent chips, error chip; later slices add image strip
/// and autocomplete popup.
fn build_quick_prompt_card() -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("quick-prompt-card")
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("quick-prompt-title")
                .with_text("Quick prompt".to_string()),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::seed_state;

    #[test]
    fn returns_hidden_when_overlay_closed() {
        let state = seed_state();
        let shared = std::sync::Arc::new(std::sync::Mutex::new(state));
        let snap = shared.lock().unwrap().ui_snapshot();
        let el = build_quick_prompt_overlay(&snap, &shared);
        assert!(
            el.classes.iter().any(|c| c == "quick-prompt-hidden"),
            "expected hidden class, got classes {:?}",
            el.classes
        );
    }

    #[test]
    fn returns_overlay_when_open() {
        let mut state = seed_state();
        state.quick_prompt = Some(crate::quick_prompt::QuickPromptState::open_default());
        let shared = std::sync::Arc::new(std::sync::Mutex::new(state));
        let snap = shared.lock().unwrap().ui_snapshot();
        let el = build_quick_prompt_overlay(&snap, &shared);
        assert!(
            el.classes.iter().any(|c| c == "quick-prompt-overlay"),
            "expected overlay class, got classes {:?}",
            el.classes
        );
        assert!(
            el.on_click.is_some(),
            "backdrop click handler should be set"
        );
        assert_eq!(el.children.len(), 1, "overlay should contain the card");
        assert!(
            el.children[0]
                .classes
                .iter()
                .any(|c| c == "quick-prompt-card"),
            "child should be the card"
        );
    }
}
