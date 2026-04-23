//! Transient drag state used by pane-to-tab extraction (F4) and the
//! tab-drop flow (F1).
//!
//! Kept separate from `resize_drag` because pane-resize dragging is
//! a local event loop owned by the resizer element, whereas the pane
//! and tab drags need to be tracked globally so the overlay and tab
//! bar can react to cursor movement regardless of which element the
//! pointer is currently over.

use crate::state::PaneId;

/// Global cursor tracking for in-progress drags.
///
/// `Idle` is the resting state; `DraggingPane` is entered when the
/// user presses the pane header grip and exceeds the 4px threshold.
/// The cursor fields are updated on each `drag.update` so overlays
/// can render feedback at the current pointer position.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum DragState {
    #[default]
    Idle,
    DraggingPane {
        pane: PaneId,
        cursor_x: f32,
        cursor_y: f32,
    },
}

impl DragState {
    /// `Some(pane)` while a pane drag is active, `None` otherwise.
    pub fn dragged_pane(&self) -> Option<PaneId> {
        match self {
            DragState::DraggingPane { pane, .. } => Some(*pane),
            DragState::Idle => None,
        }
    }

    /// True while any drag is in progress.
    pub fn is_active(&self) -> bool {
        !matches!(self, DragState::Idle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_idle() {
        assert_eq!(DragState::default(), DragState::Idle);
    }

    #[test]
    fn idle_reports_no_pane() {
        let s = DragState::Idle;
        assert_eq!(s.dragged_pane(), None);
        assert!(!s.is_active());
    }

    #[test]
    fn dragging_pane_reports_pane_and_is_active() {
        let s = DragState::DraggingPane {
            pane: PaneId(7),
            cursor_x: 10.0,
            cursor_y: 20.0,
        };
        assert_eq!(s.dragged_pane(), Some(PaneId(7)));
        assert!(s.is_active());
    }
}
