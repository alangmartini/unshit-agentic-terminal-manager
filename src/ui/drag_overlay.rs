//! Floating cursor ghost rendered while a pane drag is in progress.
//!
//! Matches web drag-and-drop expectations: the user sees a translucent
//! label tracking the pointer so it's obvious what is being moved and
//! where it will land. The overlay is non-interactive (pointer-events
//! are disabled via CSS) so it never blocks hit-testing of the tab bar
//! drop target below it.

use unshit::core::element::*;
use unshit::core::style::parse::StyleDeclaration;
use unshit::core::style::types::{CssPosition, Dimension};

use crate::state::UiSnapshot;

/// Return the drag ghost element for the current snapshot, or `None`
/// when no drag is in progress. Resolves the label (title + subtitle)
/// from either the dragged pane or the dragged tab, depending on the
/// drag variant. Missing ids fall through to `None` rather than
/// panicking so a torn-down pane/tab doesn't blow up rendering.
pub fn build_drag_overlay(state: &UiSnapshot) -> Option<ElementDef> {
    let (cursor_x, cursor_y) = state.drag.cursor()?;
    let (title, subtitle) = if let Some(pane_id) = state.drag.dragged_pane() {
        let pane = state
            .panes
            .iter()
            .flat_map(|row| row.iter())
            .find(|p| p.id == pane_id)?;
        (pane.title.clone(), pane.subtitle.clone())
    } else if let Some(tab_id) = state.drag.dragged_tab() {
        let tab = state.tabs.iter().find(|t| t.id == tab_id)?;
        (tab.name.clone(), tab.subtitle.clone())
    } else {
        return None;
    };

    // Offset so the ghost hangs down/right of the cursor instead of
    // covering the arrow tip. The 12/8 offset mirrors common OS drag
    // cursor conventions.
    let ghost_x = cursor_x + 12.0;
    let ghost_y = cursor_y + 8.0;

    Some(
        ElementDef::new(Tag::Div)
            .with_class("drag-ghost")
            .with_style(StyleDeclaration::Position(CssPosition::Fixed))
            .with_style(StyleDeclaration::Left(Dimension::Px(ghost_x)))
            .with_style(StyleDeclaration::Top(Dimension::Px(ghost_y)))
            .with_child(
                ElementDef::new(Tag::Span)
                    .with_class("drag-ghost-title")
                    .with_text(title),
            )
            .with_child(
                ElementDef::new(Tag::Span)
                    .with_class("drag-ghost-subtitle")
                    .with_text(format!("\u{00B7} {}", subtitle)),
            ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{seed_state, PaneId};

    fn has_class(el: &ElementDef, class: &str) -> bool {
        el.classes.iter().any(|c| c == class)
    }

    fn find_by_class<'a>(el: &'a ElementDef, class: &str) -> Option<&'a ElementDef> {
        if has_class(el, class) {
            return Some(el);
        }
        for child in &el.children {
            if let Some(found) = find_by_class(child, class) {
                return Some(found);
            }
        }
        None
    }

    fn text_of(el: &ElementDef) -> Option<&str> {
        match &el.content {
            ElementContent::Text(t) => Some(t.as_str()),
            _ => None,
        }
    }

    #[test]
    fn overlay_is_none_when_idle() {
        let snap = seed_state().ui_snapshot();
        assert!(build_drag_overlay(&snap).is_none());
    }

    #[test]
    fn overlay_rendered_when_dragging() {
        let mut state = seed_state();
        let pane = state.active_pane;
        state.drag = crate::drag::DragState::DraggingPane {
            pane,
            cursor_x: 200.0,
            cursor_y: 150.0,
        };
        let snap = state.ui_snapshot();
        let el = build_drag_overlay(&snap).expect("overlay expected while dragging");
        assert!(has_class(&el, "drag-ghost"));
    }

    #[test]
    fn overlay_shows_dragged_pane_title_and_subtitle() {
        let mut state = seed_state();
        state.panes[0][0].title = "build".into();
        state.panes[0][0].subtitle = "cargo".into();
        let pane = state.panes[0][0].id;
        state.drag = crate::drag::DragState::DraggingPane {
            pane,
            cursor_x: 0.0,
            cursor_y: 0.0,
        };
        let snap = state.ui_snapshot();
        let el = build_drag_overlay(&snap).unwrap();

        let title = find_by_class(&el, "drag-ghost-title").unwrap();
        assert_eq!(text_of(title), Some("build"));
        let subtitle = find_by_class(&el, "drag-ghost-subtitle").unwrap();
        assert_eq!(text_of(subtitle), Some("\u{00B7} cargo"));
    }

    #[test]
    fn overlay_tracks_cursor_with_offset() {
        let mut state = seed_state();
        let pane = state.active_pane;
        state.drag = crate::drag::DragState::DraggingPane {
            pane,
            cursor_x: 400.0,
            cursor_y: 250.0,
        };
        let snap = state.ui_snapshot();
        let el = build_drag_overlay(&snap).unwrap();

        let left = el
            .style_overrides
            .iter()
            .find_map(|s| match s {
                StyleDeclaration::Left(Dimension::Px(v)) => Some(*v),
                _ => None,
            })
            .expect("left style must be set");
        let top = el
            .style_overrides
            .iter()
            .find_map(|s| match s {
                StyleDeclaration::Top(Dimension::Px(v)) => Some(*v),
                _ => None,
            })
            .expect("top style must be set");
        assert!(
            (left - 412.0).abs() < 1e-3,
            "left should track cursor + offset, got {}",
            left
        );
        assert!(
            (top - 258.0).abs() < 1e-3,
            "top should track cursor + offset, got {}",
            top
        );
    }

    #[test]
    fn overlay_is_none_when_dragged_pane_not_in_live_state() {
        let mut state = seed_state();
        state.drag = crate::drag::DragState::DraggingPane {
            pane: PaneId(9999),
            cursor_x: 100.0,
            cursor_y: 100.0,
        };
        let snap = state.ui_snapshot();
        assert!(build_drag_overlay(&snap).is_none());
    }

    #[test]
    fn overlay_renders_for_tab_drag_with_tab_label() {
        let mut state = seed_state();
        let tab_id = state.tabs[0].id.clone();
        state.tabs[0].name = "alpha".into();
        state.tabs[0].subtitle = "zsh".into();
        state.drag = crate::drag::DragState::DraggingTab {
            source_tab: tab_id,
            cursor_x: 300.0,
            cursor_y: 100.0,
        };
        let snap = state.ui_snapshot();
        let el = build_drag_overlay(&snap).expect("tab drag must render a ghost");
        let title = find_by_class(&el, "drag-ghost-title").unwrap();
        assert_eq!(text_of(title), Some("alpha"));
        let subtitle = find_by_class(&el, "drag-ghost-subtitle").unwrap();
        assert_eq!(text_of(subtitle), Some("\u{00B7} zsh"));
    }

    #[test]
    fn overlay_is_none_when_dragged_tab_not_in_state() {
        let mut state = seed_state();
        state.drag = crate::drag::DragState::DraggingTab {
            source_tab: "does-not-exist".into(),
            cursor_x: 50.0,
            cursor_y: 50.0,
        };
        let snap = state.ui_snapshot();
        assert!(build_drag_overlay(&snap).is_none());
    }
}
