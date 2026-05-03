//! Render the Quick Prompt overlay.
//!
//! Returns a fixed-position backdrop with a centered card. When the
//! overlay is closed (`snap.quick_prompt.is_none()`) the function still
//! returns a hidden element so callers can include the result in the
//! root tree unconditionally without a branch in `main.rs`.

use unshit::core::element::*;
use unshit::core::style::parse::StyleDeclaration;
use unshit::core::style::types::{AlignItems, CssPosition, Dimension, JustifyContent};

use crate::quick_prompt::state::{Agent, QuickPromptState};
use crate::state::{dispatch, mutate_with, SharedState, UiSnapshot};

/// Build the overlay tree. Always returns an element so the root render
/// in `main.rs` does not need a conditional branch.
pub fn build_quick_prompt_overlay(snap: &UiSnapshot, shared: &SharedState) -> ElementDef {
    let Some(qp) = snap.quick_prompt.as_ref() else {
        return ElementDef::new(Tag::Div).with_class("quick-prompt-hidden");
    };

    let card = build_quick_prompt_card(qp, shared);

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

fn build_quick_prompt_card(qp: &QuickPromptState, shared: &SharedState) -> ElementDef {
    let agent_row = build_agent_row(qp.agent, shared);
    let prompt_input = build_prompt_input(shared);
    let toolbar = build_toolbar(shared);

    let mut card = ElementDef::new(Tag::Div)
        .with_class("quick-prompt-card")
        .on_click(|| {
            // Prevent backdrop click from also firing when the user
            // clicks inside the card. Without this, every click on the
            // input would also dispatch quick_prompt.close.
        })
        .with_child(agent_row)
        .with_child(prompt_input)
        .with_child(toolbar);

    if let Some(popup) = qp.popup.as_ref() {
        card = card.with_child(build_autocomplete_popup(popup, shared));
    }

    if !qp.images.is_empty() {
        card = card.with_child(build_image_strip(&qp.images, shared));
    }

    if let Some(msg) = qp.error.as_ref() {
        card = card.with_child(
            ElementDef::new(Tag::Div)
                .with_class("quick-prompt-error")
                .with_text(msg.clone()),
        );
    }

    card
}

fn build_autocomplete_popup(
    popup: &crate::quick_prompt::Popup,
    shared: &SharedState,
) -> ElementDef {
    let mut container = ElementDef::new(Tag::Div).with_class("quick-prompt-autocomplete");
    if popup.matches.is_empty() {
        container = container.with_child(
            ElementDef::new(Tag::Div)
                .with_class("quick-prompt-autocomplete-empty")
                .with_text("No matches".to_string()),
        );
        return container;
    }
    for (row_index, &entry_index) in popup.matches.iter().enumerate() {
        let Some(entry) = popup.entries.get(entry_index) else {
            continue;
        };
        container = container.with_child(build_autocomplete_row(
            entry,
            row_index,
            row_index == popup.selected,
            shared,
        ));
    }
    container
}

fn build_autocomplete_row(
    entry: &crate::quick_prompt::Entry,
    row_index: usize,
    is_selected: bool,
    shared: &SharedState,
) -> ElementDef {
    let row_shared = shared.clone();
    let mut row = ElementDef::new(Tag::Div)
        .with_class("quick-prompt-autocomplete-row")
        .on_click(move || {
            mutate_with(&row_shared, |st| {
                if let Some(qp) = st.quick_prompt.as_mut() {
                    if let Some(popup) = qp.popup.as_mut() {
                        if row_index < popup.matches.len() {
                            popup.selected = row_index;
                        }
                    }
                }
                dispatch(st, "quick_prompt.autocomplete_confirm");
            });
        })
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("quick-prompt-autocomplete-name")
                .with_text(format!("/{}", entry.name)),
        )
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("quick-prompt-autocomplete-kind")
                .with_text(entry.kind.label().to_string()),
        );
    if is_selected {
        row = row.with_class("selected");
    }
    row
}

fn build_toolbar(shared: &SharedState) -> ElementDef {
    let paste_shared = shared.clone();
    let paste_button = ElementDef::new(Tag::Div)
        .with_class("quick-prompt-toolbar-button")
        .on_click(move || {
            mutate_with(&paste_shared, |st| {
                dispatch(st, "quick_prompt.image_paste");
            });
        })
        .with_child(ElementDef::new(Tag::Span).with_text("Attach image".to_string()));

    ElementDef::new(Tag::Div)
        .with_class("quick-prompt-toolbar")
        .with_child(paste_button)
}

fn build_image_strip(
    images: &[crate::quick_prompt::QuickPromptImage],
    shared: &SharedState,
) -> ElementDef {
    let mut strip = ElementDef::new(Tag::Div).with_class("quick-prompt-image-strip");
    for img in images {
        strip = strip.with_child(build_image_chip(img, shared));
    }
    strip
}

fn build_image_chip(
    image: &crate::quick_prompt::QuickPromptImage,
    shared: &SharedState,
) -> ElementDef {
    let hash = image.hash.clone();
    let remove_shared = shared.clone();
    let thumb_path = image.thumb_path.to_string_lossy().to_string();

    let thumb = ElementDef::new(Tag::Div)
        .with_class("quick-prompt-image-thumb")
        .with_image(thumb_path);

    let remove = ElementDef::new(Tag::Div)
        .with_class("quick-prompt-image-remove")
        .on_click(move || {
            let h = hash.clone();
            mutate_with(&remove_shared, |st| {
                if let Some(qp) = st.quick_prompt.as_mut() {
                    if let Some(idx) = qp.images.iter().position(|i| i.hash == h) {
                        let img = qp.images.remove(idx);
                        let _ = std::fs::remove_file(&img.temp_path);
                        let _ = std::fs::remove_file(&img.thumb_path);
                    }
                }
            });
        })
        .with_child(ElementDef::new(Tag::Span).with_text("×".to_string()));

    ElementDef::new(Tag::Div)
        .with_class("quick-prompt-image-chip")
        .with_child(thumb)
        .with_child(remove)
}

fn build_agent_row(active: Agent, shared: &SharedState) -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("quick-prompt-agent-row")
        .with_child(build_agent_chip(Agent::Claude, active, shared))
        .with_child(build_agent_chip(Agent::Codex, active, shared))
}

fn build_agent_chip(chip: Agent, active: Agent, shared: &SharedState) -> ElementDef {
    let chip_shared = shared.clone();
    let mut el = ElementDef::new(Tag::Div)
        .with_class("quick-prompt-chip")
        .on_click(move || {
            mutate_with(&chip_shared, |st| {
                let needs_toggle = st
                    .quick_prompt
                    .as_ref()
                    .map(|qp| qp.agent != chip)
                    .unwrap_or(false);
                if needs_toggle {
                    dispatch(st, "quick_prompt.toggle_agent");
                }
            });
        })
        .with_child(ElementDef::new(Tag::Span).with_text(chip.label().to_string()));
    if chip == active {
        el = el.with_class("active");
    }
    el
}

fn build_prompt_input(shared: &SharedState) -> ElementDef {
    let change_shared = shared.clone();
    let submit_shared = shared.clone();
    ElementDef::new(Tag::Input)
        .with_class("quick-prompt-input")
        .with_placeholder("What should the agent do?")
        .on_change(move |text| {
            let typed = text.to_string();
            mutate_with(&change_shared, |st| {
                let agent = st.quick_prompt.as_ref().map(|qp| qp.agent);
                let prev_prompt = st
                    .quick_prompt
                    .as_ref()
                    .map(|qp| qp.prompt.clone())
                    .unwrap_or_default();

                if let Some(qp) = st.quick_prompt.as_mut() {
                    qp.prompt = typed.clone();
                    qp.error = None;

                    if let Some(popup) = qp.popup.as_mut() {
                        // Recompute the live query against the new
                        // buffer; if the user backspaced past the
                        // anchor or typed whitespace inside the query
                        // window, drop the popup.
                        let keep =
                            crate::quick_prompt::autocomplete::rederive_query(popup, &qp.prompt);
                        if !keep {
                            qp.popup = None;
                        }
                    } else {
                        match agent {
                            Some(crate::quick_prompt::Agent::Claude) => {
                                if let Some(anchor) =
                                    crate::quick_prompt::autocomplete::detect_claude_trigger(
                                        &prev_prompt,
                                        &qp.prompt,
                                    )
                                {
                                    let entries =
                                        crate::quick_prompt::autocomplete::cached_claude_sources();
                                    if !entries.is_empty() {
                                        qp.popup = Some(crate::quick_prompt::Popup::open(
                                            entries, anchor,
                                        ));
                                    }
                                }
                            }
                            Some(crate::quick_prompt::Agent::Codex) => {
                                if let Some((anchor, kind)) =
                                    crate::quick_prompt::autocomplete::detect_codex_trigger(
                                        &prev_prompt,
                                        &qp.prompt,
                                    )
                                {
                                    let (entries, trigger) = match kind {
                                        crate::quick_prompt::EntryKind::Skill => (
                                            crate::quick_prompt::autocomplete::cached_codex_skill_sources(),
                                            '`',
                                        ),
                                        crate::quick_prompt::EntryKind::Command => (
                                            crate::quick_prompt::autocomplete::cached_codex_command_sources(),
                                            '/',
                                        ),
                                    };
                                    if !entries.is_empty() {
                                        qp.popup = Some(
                                            crate::quick_prompt::Popup::open_with_trigger(
                                                entries, anchor, trigger,
                                            ),
                                        );
                                    }
                                }
                            }
                            None => {}
                        }
                    }
                }
            });
        })
        .on_submit(move |text| {
            let typed = text.to_string();
            mutate_with(&submit_shared, |st| {
                // Sync the buffer in case on_change is debounced or
                // missed an event before the user pressed Enter.
                if let Some(qp) = st.quick_prompt.as_mut() {
                    qp.prompt = typed;
                }
                let popup_open = st
                    .quick_prompt
                    .as_ref()
                    .map(|qp| qp.popup.is_some())
                    .unwrap_or(false);
                if popup_open {
                    // Enter confirms the selected popup row instead of
                    // submitting the prompt.
                    dispatch(st, "quick_prompt.autocomplete_confirm");
                } else {
                    dispatch(st, "quick_prompt.submit");
                }
            });
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::seed_state;
    use std::sync::{Arc, Mutex};

    fn shared_with(state: crate::state::AppState) -> Arc<Mutex<crate::state::AppState>> {
        Arc::new(Mutex::new(state))
    }

    #[test]
    fn returns_hidden_when_overlay_closed() {
        let state = seed_state();
        let shared = shared_with(state);
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
        state.quick_prompt = Some(QuickPromptState::open_with_agent(Agent::Claude));
        let shared = shared_with(state);
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
        assert!(el.children[0]
            .classes
            .iter()
            .any(|c| c == "quick-prompt-card"));
    }

    #[test]
    fn card_renders_two_agent_chips() {
        let mut state = seed_state();
        state.quick_prompt = Some(QuickPromptState::open_with_agent(Agent::Claude));
        let shared = shared_with(state);
        let snap = shared.lock().unwrap().ui_snapshot();
        let overlay = build_quick_prompt_overlay(&snap, &shared);
        let card = &overlay.children[0];
        let agent_row = card
            .children
            .iter()
            .find(|c| c.classes.iter().any(|cl| cl == "quick-prompt-agent-row"))
            .expect("agent row");
        assert_eq!(agent_row.children.len(), 2);
        let chip_classes: Vec<bool> = agent_row
            .children
            .iter()
            .map(|c| c.classes.iter().any(|cl| cl == "active"))
            .collect();
        // Exactly one chip is active.
        assert_eq!(chip_classes.iter().filter(|b| **b).count(), 1);
    }

    #[test]
    fn active_chip_matches_state_agent() {
        let mut state = seed_state();
        state.quick_prompt = Some(QuickPromptState::open_with_agent(Agent::Codex));
        let shared = shared_with(state);
        let snap = shared.lock().unwrap().ui_snapshot();
        let overlay = build_quick_prompt_overlay(&snap, &shared);
        let card = &overlay.children[0];
        let agent_row = card
            .children
            .iter()
            .find(|c| c.classes.iter().any(|cl| cl == "quick-prompt-agent-row"))
            .expect("agent row");
        // Codex chip is rendered second, so it should be the active one.
        assert!(!agent_row.children[0].classes.iter().any(|c| c == "active"));
        assert!(agent_row.children[1].classes.iter().any(|c| c == "active"));
    }

    #[test]
    fn chip_click_toggles_agent_in_state() {
        let mut state = seed_state();
        state.quick_prompt = Some(QuickPromptState::open_with_agent(Agent::Claude));
        let shared = shared_with(state);
        let snap = shared.lock().unwrap().ui_snapshot();
        let overlay = build_quick_prompt_overlay(&snap, &shared);
        let card = &overlay.children[0];
        let agent_row = card
            .children
            .iter()
            .find(|c| c.classes.iter().any(|cl| cl == "quick-prompt-agent-row"))
            .expect("agent row");
        // Click the Codex chip (second one).
        let codex_chip = &agent_row.children[1];
        (codex_chip.on_click.as_ref().unwrap())();

        let agent_after = shared.lock().unwrap().quick_prompt.as_ref().unwrap().agent;
        assert_eq!(agent_after, Agent::Codex);
    }

    #[test]
    fn chip_click_on_active_chip_does_not_toggle() {
        let mut state = seed_state();
        state.quick_prompt = Some(QuickPromptState::open_with_agent(Agent::Claude));
        let shared = shared_with(state);
        let snap = shared.lock().unwrap().ui_snapshot();
        let overlay = build_quick_prompt_overlay(&snap, &shared);
        let card = &overlay.children[0];
        let agent_row = card
            .children
            .iter()
            .find(|c| c.classes.iter().any(|cl| cl == "quick-prompt-agent-row"))
            .expect("agent row");
        // Click the Claude chip (first one, currently active).
        let claude_chip = &agent_row.children[0];
        (claude_chip.on_click.as_ref().unwrap())();

        // Agent should still be Claude (no toggle).
        let agent_after = shared.lock().unwrap().quick_prompt.as_ref().unwrap().agent;
        assert_eq!(agent_after, Agent::Claude);
    }

    #[test]
    fn input_on_change_updates_prompt_buffer() {
        let mut state = seed_state();
        state.quick_prompt = Some(QuickPromptState::open_with_agent(Agent::Claude));
        let shared = shared_with(state);
        let snap = shared.lock().unwrap().ui_snapshot();
        let overlay = build_quick_prompt_overlay(&snap, &shared);
        let card = &overlay.children[0];
        let input = card
            .children
            .iter()
            .find(|c| c.classes.iter().any(|cl| cl == "quick-prompt-input"))
            .expect("prompt input");
        (input.on_change.as_ref().unwrap())("hello world");

        let prompt = shared
            .lock()
            .unwrap()
            .quick_prompt
            .as_ref()
            .unwrap()
            .prompt
            .clone();
        assert_eq!(prompt, "hello world");
    }

    #[test]
    fn card_renders_error_chip_when_state_has_error() {
        let mut state = seed_state();
        let mut qp = QuickPromptState::open_with_agent(Agent::Claude);
        qp.error = Some("Codex coming soon".into());
        state.quick_prompt = Some(qp);
        let shared = shared_with(state);
        let snap = shared.lock().unwrap().ui_snapshot();
        let overlay = build_quick_prompt_overlay(&snap, &shared);
        let card = &overlay.children[0];
        assert!(
            card.children
                .iter()
                .any(|c| c.classes.iter().any(|cl| cl == "quick-prompt-error")),
            "expected error chip, got {:?}",
            card.children
                .iter()
                .map(|c| c.classes.clone())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn card_omits_error_chip_when_state_has_no_error() {
        let mut state = seed_state();
        state.quick_prompt = Some(QuickPromptState::open_with_agent(Agent::Claude));
        let shared = shared_with(state);
        let snap = shared.lock().unwrap().ui_snapshot();
        let overlay = build_quick_prompt_overlay(&snap, &shared);
        let card = &overlay.children[0];
        assert!(!card
            .children
            .iter()
            .any(|c| c.classes.iter().any(|cl| cl == "quick-prompt-error")));
    }

    #[test]
    fn input_on_submit_dispatches_quick_prompt_submit() {
        // The on_submit handler dispatches quick_prompt.submit through
        // the existing dispatcher; we verify the side effect (overlay
        // gets the empty-prompt error chip when the buffer is empty).
        let mut state = seed_state();
        state.quick_prompt = Some(QuickPromptState::open_with_agent(Agent::Claude));
        let shared = shared_with(state);
        let snap = shared.lock().unwrap().ui_snapshot();
        let overlay = build_quick_prompt_overlay(&snap, &shared);
        let card = &overlay.children[0];
        let input = card
            .children
            .iter()
            .find(|c| c.classes.iter().any(|cl| cl == "quick-prompt-input"))
            .expect("prompt input");
        // Submit with an empty buffer; the dispatch arm sets the error
        // chip and keeps the overlay open.
        (input.on_submit.as_ref().unwrap())("");
        let qp = shared.lock().unwrap().quick_prompt.clone().unwrap();
        assert_eq!(qp.error.as_deref(), Some("Type a prompt to continue."));
    }

    #[test]
    fn input_on_change_clears_error() {
        let mut state = seed_state();
        let mut qp = QuickPromptState::open_with_agent(Agent::Claude);
        qp.error = Some("stale".into());
        state.quick_prompt = Some(qp);
        let shared = shared_with(state);
        let snap = shared.lock().unwrap().ui_snapshot();
        let overlay = build_quick_prompt_overlay(&snap, &shared);
        let card = &overlay.children[0];
        let input = card
            .children
            .iter()
            .find(|c| c.classes.iter().any(|cl| cl == "quick-prompt-input"))
            .expect("prompt input");
        (input.on_change.as_ref().unwrap())("retry");

        assert!(shared
            .lock()
            .unwrap()
            .quick_prompt
            .as_ref()
            .unwrap()
            .error
            .is_none());
    }
}
