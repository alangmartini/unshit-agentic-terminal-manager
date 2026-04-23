//! Transient drag state used by pane-to-tab extraction (F4) and the
//! tab-drop flow (F1).
//!
//! Kept separate from `resize_drag` because pane-resize dragging is
//! a local event loop owned by the resizer element, whereas the pane
//! and tab drags need to be tracked globally so the overlay and tab
//! bar can react to cursor movement regardless of which element the
//! pointer is currently over.

pub mod drop_zones;

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

    /// Current cursor position while dragging, or `None` when idle.
    pub fn cursor(&self) -> Option<(f32, f32)> {
        match self {
            DragState::DraggingPane {
                cursor_x, cursor_y, ..
            } => Some((*cursor_x, *cursor_y)),
            DragState::Idle => None,
        }
    }
}

/// Axis-aligned rectangle in window coordinates. Used to describe the
/// tab bar when resolving a pane-drop target.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub fn contains(&self, cursor_x: f32, cursor_y: f32) -> bool {
        cursor_x >= self.x
            && cursor_x < self.x + self.width
            && cursor_y >= self.y
            && cursor_y < self.y + self.height
    }
}

/// Estimated rendered width of one tab in CSS pixels. Tabs clamp to
/// [150, 240] in the stylesheet; 200 splits the middle and gives us a
/// reasonable boundary for drop-index hit-testing without tracking each
/// tab's live rect.
pub const TAB_WIDTH_ESTIMATE: f32 = 200.0;

/// Resolve the drop target for a dragged pane at the given cursor
/// position. Returns the insertion index in the tab bar when the cursor
/// is within `tabbar`, or `None` otherwise.
///
/// Each existing tab is treated as `TAB_WIDTH_ESTIMATE` CSS pixels wide
/// starting at `tabbar.x`; the insertion slot falls on the nearest tab
/// boundary. A cursor past the rightmost tab (i.e. over the empty right
/// half of the tab strip) inserts at `tab_count`, placing the new tab
/// at the end. This matches what a flex-row tab strip renders: tabs are
/// left-aligned with a fixed width, so the midpoint of each tab is
/// `(i + 0.5) * TAB_WIDTH_ESTIMATE` from the strip's start.
pub fn resolve_tabbar_drop(
    cursor_x: f32,
    cursor_y: f32,
    tabbar: Rect,
    tab_count: usize,
) -> Option<usize> {
    if !tabbar.contains(cursor_x, cursor_y) {
        return None;
    }
    if tab_count == 0 {
        return Some(0);
    }
    let local_x = (cursor_x - tabbar.x).max(0.0);
    let raw = (local_x / TAB_WIDTH_ESTIMATE).round() as isize;
    Some(raw.clamp(0, tab_count as isize) as usize)
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
        assert_eq!(s.cursor(), None);
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
        assert_eq!(s.cursor(), Some((10.0, 20.0)));
    }

    fn tabbar(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect {
            x,
            y,
            width: w,
            height: h,
        }
    }

    #[test]
    fn rect_contains_point_inside() {
        let r = tabbar(10.0, 20.0, 100.0, 40.0);
        assert!(r.contains(10.0, 20.0));
        assert!(r.contains(50.0, 40.0));
        assert!(r.contains(109.9, 59.9));
    }

    #[test]
    fn rect_excludes_point_outside() {
        let r = tabbar(10.0, 20.0, 100.0, 40.0);
        assert!(!r.contains(5.0, 40.0));
        assert!(!r.contains(50.0, 10.0));
        assert!(!r.contains(110.0, 40.0));
        assert!(!r.contains(50.0, 60.0));
    }

    #[test]
    fn drop_outside_tabbar_returns_none() {
        let bar = tabbar(0.0, 34.0, 800.0, 38.0);
        assert_eq!(resolve_tabbar_drop(400.0, 200.0, bar, 3), None);
        assert_eq!(resolve_tabbar_drop(400.0, 30.0, bar, 3), None);
    }

    #[test]
    fn drop_on_empty_tabbar_inserts_at_zero() {
        let bar = tabbar(0.0, 34.0, 800.0, 38.0);
        assert_eq!(resolve_tabbar_drop(200.0, 50.0, bar, 0), Some(0));
    }

    #[test]
    fn drop_before_first_tab_inserts_at_zero() {
        let bar = tabbar(0.0, 34.0, 600.0, 38.0);
        assert_eq!(resolve_tabbar_drop(0.0, 50.0, bar, 3), Some(0));
        assert_eq!(resolve_tabbar_drop(50.0, 50.0, bar, 3), Some(0));
    }

    #[test]
    fn drop_between_tabs_inserts_at_boundary() {
        let bar = tabbar(0.0, 34.0, 600.0, 38.0);
        assert_eq!(resolve_tabbar_drop(200.0, 50.0, bar, 3), Some(1));
        assert_eq!(resolve_tabbar_drop(400.0, 50.0, bar, 3), Some(2));
    }

    #[test]
    fn drop_past_last_tab_inserts_at_end() {
        let bar = tabbar(0.0, 34.0, 600.0, 38.0);
        assert_eq!(resolve_tabbar_drop(599.0, 50.0, bar, 3), Some(3));
    }

    #[test]
    fn drop_respects_offset_origin() {
        let bar = tabbar(252.0, 34.0, 548.0, 38.0);
        assert_eq!(resolve_tabbar_drop(252.0, 50.0, bar, 2), Some(0));
        // Middle of tab 0 at x=352, boundary at x=452, middle of tab 1 at x=552.
        assert_eq!(resolve_tabbar_drop(460.0, 50.0, bar, 2), Some(1));
        assert_eq!(resolve_tabbar_drop(700.0, 50.0, bar, 2), Some(2));
    }

    /// The tab bar is typically much wider than `tab_count * TAB_WIDTH`;
    /// a cursor released over the empty right half must still insert at
    /// the end of the strip rather than snapping back to the middle.
    #[test]
    fn drop_in_empty_right_area_still_inserts_at_end() {
        let bar = tabbar(258.0, 34.0, 2000.0, 38.0);
        assert_eq!(resolve_tabbar_drop(1800.0, 50.0, bar, 1), Some(1));
        assert_eq!(resolve_tabbar_drop(1800.0, 50.0, bar, 2), Some(2));
    }

    /// A cursor just past a tab's right edge must insert after that
    /// tab, even when the tab bar has generous empty space.
    #[test]
    fn drop_right_of_only_tab_inserts_after_it() {
        let bar = tabbar(258.0, 34.0, 2000.0, 38.0);
        // With 1 tab at [258, 458] (width 200 estimate) the midpoint is
        // 358. A drop at x=500 is clearly past it and should insert at 1.
        assert_eq!(resolve_tabbar_drop(500.0, 50.0, bar, 1), Some(1));
    }

    /// A cursor to the left of the only tab's midpoint must insert at 0.
    #[test]
    fn drop_left_of_only_tab_inserts_before_it() {
        let bar = tabbar(258.0, 34.0, 2000.0, 38.0);
        assert_eq!(resolve_tabbar_drop(300.0, 50.0, bar, 1), Some(0));
    }
}
