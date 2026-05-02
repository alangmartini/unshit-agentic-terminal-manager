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

    ElementDef::new(Tag::Div)
        .with_class("quick-prompt-card")
        .on_click(|| {
            // Prevent backdrop click from also firing when the user
            // clicks inside the card. Without this, every click on the
            // input would also dispatch quick_prompt.close.
        })
        .with_child(agent_row)
        .with_child(prompt_input)
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
    let input_shared = shared.clone();
    ElementDef::new(Tag::Input)
        .with_class("quick-prompt-input")
        .with_placeholder("What should the agent do?")
        .on_change(move |text| {
            let typed = text.to_string();
            mutate_with(&input_shared, |st| {
                if let Some(qp) = st.quick_prompt.as_mut() {
                    qp.prompt = typed;
                    qp.error = None;
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
