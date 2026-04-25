//! Drop-zone overlay rendered over each pane while a drag is in
//! progress (F1).
//!
//! While a drag is active, every non-source pane gets a translucent
//! amber rectangle showing exactly where the dropped pane will land:
//! a half-pane strip for the four edge zones, the entire pane for
//! Center. The hit-test runs in `drop_zones::hit_test` so the visible
//! preview always matches what `dispatch_drag_end` will execute.
//!
//! The overlay is rendered inside each pane element using
//! percentage-based absolute positioning so it tracks the pane's
//! real layout without any window-coordinate math.
//!
//! The overlay is non-interactive (CSS `pointer-events: none`) so it
//! never blocks the drag cursor from the underlying pane.

use unshit::core::element::*;
use unshit::core::style::parse::StyleDeclaration;
use unshit::core::style::types::{CssPosition, Dimension};

use super::drop_zones::{hit_test, DropZone};
use super::{DragState, Rect};
use crate::state::{PaneId, UiSnapshot};

/// Compute each pane's window-coordinate rect from the snapshot's
/// layout fields. Forwards to `drag::compute_pane_rects` so the
/// hit-test used by `dispatch_drag_end` stays in sync with the pane
/// layout the overlay overlays.
pub(crate) fn snapshot_pane_rects(state: &UiSnapshot) -> Vec<(PaneId, Rect)> {
    let grid = crate::drag::grid_rect_from_state(
        state.sidebar_width,
        state.tabbar_rect,
        state.last_grid_width,
        state.last_grid_height,
        state.scale_factor,
    );
    crate::drag::compute_pane_rects(&state.panes, &state.row_ratios, &state.col_ratios, grid)
}

/// Find the pane currently under the cursor (if any) during a drag,
/// along with which zone of it is hovered. Works for both tab and
/// pane drags: during a pane drag the dragged pane itself is
/// excluded from hit-testing so you can't drop a pane onto its own
/// edge. Returns `None` when no drag is in progress or the cursor
/// sits outside every (non-self) pane.
pub fn hovered_zone(state: &UiSnapshot) -> Option<(PaneId, DropZone)> {
    if matches!(state.drag, DragState::Idle) {
        return None;
    }
    let (cx, cy) = state.drag.cursor()?;
    let dragged_pane = state.drag.dragged_pane();
    for (id, rect) in snapshot_pane_rects(state) {
        if Some(id) == dragged_pane {
            continue;
        }
        if let Some(zone) = hit_test(rect, cx, cy) {
            return Some((id, zone));
        }
    }
    None
}

/// Build the drop-zone overlay for a single pane, to be added as a
/// child of its `.pane` element. Returns `None` when no drag is
/// active, or when the pane is itself the drag source (which would
/// render drop zones on top of the element being moved).
pub fn build_pane_drop_zone_overlay(state: &UiSnapshot, pane_id: PaneId) -> Option<ElementDef> {
    if matches!(state.drag, DragState::Idle) {
        return None;
    }
    if state.drag.dragged_pane() == Some(pane_id) {
        return None;
    }
    let hover = hovered_zone(state);
    let hovered = hover.and_then(|(p, z)| (p == pane_id).then_some(z));
    Some(build_in_pane_overlay(hovered))
}

/// Kept for backward compatibility with callers that previously
/// mounted the overlay at the root. Always returns an empty vec now
/// that the overlay is rendered inside each pane.
pub fn build_drop_zone_overlay(_state: &UiSnapshot) -> Vec<ElementDef> {
    Vec::new()
}

fn build_in_pane_overlay(hovered: Option<DropZone>) -> ElementDef {
    let mut container = ElementDef::new(Tag::Div)
        .with_class("drop-zone-overlay")
        .with_style(StyleDeclaration::Position(CssPosition::Absolute))
        .with_style(StyleDeclaration::Left(Dimension::Px(0.0)))
        .with_style(StyleDeclaration::Top(Dimension::Px(0.0)))
        .with_style(StyleDeclaration::Width(Dimension::Percent(100.0)))
        .with_style(StyleDeclaration::Height(Dimension::Percent(100.0)));
    if let Some(zone) = hovered {
        container = container.with_child(build_zone_preview(zone));
    }
    container
}

/// Translucent amber rectangle covering the area of the pane the drop
/// will land in. Shown only while a zone is hovered. The geometry
/// mirrors what each mutation produces: edge zones cover the half-pane
/// the new split takes, Center covers the whole pane (since the source
/// pane swaps into the target's slot).
fn build_zone_preview(zone: DropZone) -> ElementDef {
    let (left, top, width, height) = preview_rect(zone);
    ElementDef::new(Tag::Div)
        .with_class("drop-zone-preview")
        .with_class(zone_class(zone))
        .with_style(StyleDeclaration::Position(CssPosition::Absolute))
        .with_style(StyleDeclaration::Left(Dimension::Percent(left)))
        .with_style(StyleDeclaration::Top(Dimension::Percent(top)))
        .with_style(StyleDeclaration::Width(Dimension::Percent(width)))
        .with_style(StyleDeclaration::Height(Dimension::Percent(height)))
}

fn preview_rect(zone: DropZone) -> (f32, f32, f32, f32) {
    match zone {
        DropZone::Left => (0.0, 0.0, 50.0, 100.0),
        DropZone::Right => (50.0, 0.0, 50.0, 100.0),
        DropZone::Top => (0.0, 0.0, 100.0, 50.0),
        DropZone::Bottom => (0.0, 50.0, 100.0, 50.0),
        DropZone::Center => (0.0, 0.0, 100.0, 100.0),
    }
}

fn zone_class(zone: DropZone) -> &'static str {
    match zone {
        DropZone::Left => "drop-zone-left",
        DropZone::Right => "drop-zone-right",
        DropZone::Top => "drop-zone-top",
        DropZone::Bottom => "drop-zone-bottom",
        DropZone::Center => "drop-zone-center",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drag::{DragState, Rect, SIDEBAR_RESIZER_WIDTH};
    use crate::state::{seed_state, AppState};

    fn has_class(el: &ElementDef, class: &str) -> bool {
        el.classes.iter().any(|c| c == class)
    }

    /// Configure an AppState so the active tab's grid occupies `grid`
    /// in CSS coordinates. With a single-pane active tab this places
    /// the target pane at exactly `grid`; split layouts distribute
    /// across the grid per row_ratios/col_ratios.
    fn configure_grid(state: &mut AppState, grid: Rect) {
        state.sidebar_width = grid.x - SIDEBAR_RESIZER_WIDTH;
        state.scale_factor = 1.0;
        state.last_grid_width = grid.width;
        state.last_grid_height = grid.height;
        state.tabbar_rect = Rect {
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: grid.y,
        };
    }

    fn start_tab_drag(state: &mut AppState, cursor: (f32, f32)) {
        state.drag = DragState::DraggingTab {
            source_tab: "src".into(),
            cursor_x: cursor.0,
            cursor_y: cursor.1,
        };
    }

    /// Standard one-pane grid at (6, 0) sized 100x100 so cursor coords
    /// compose cleanly with nx/ny calculations in hit_test.
    const GRID: Rect = Rect {
        x: 6.0,
        y: 0.0,
        width: 100.0,
        height: 100.0,
    };

    #[test]
    fn overlay_empty_when_idle() {
        let snap = seed_state().ui_snapshot();
        assert!(build_drop_zone_overlay(&snap).is_empty());
    }

    #[test]
    fn overlay_empty_when_pane_drag_only() {
        let mut state = seed_state();
        state.drag = DragState::DraggingPane {
            pane: state.active_pane,
            cursor_x: 10.0,
            cursor_y: 10.0,
        };
        let snap = state.ui_snapshot();
        assert!(
            build_drop_zone_overlay(&snap).is_empty(),
            "pane drag must not trigger the tab-drop overlay"
        );
    }

    #[test]
    fn pane_overlay_none_when_idle() {
        let snap = seed_state().ui_snapshot();
        let pane_id = snap.panes[0][0].id;
        assert!(build_pane_drop_zone_overlay(&snap, pane_id).is_none());
    }

    fn find_preview(overlay: &ElementDef) -> Option<&ElementDef> {
        overlay
            .children
            .iter()
            .find(|c| has_class(c, "drop-zone-preview"))
    }

    #[test]
    fn hovered_zone_returns_pane_and_zone() {
        let mut state = seed_state();
        configure_grid(&mut state, GRID);
        // Cursor at (56, 10): nx = 0.5 (center band), ny = 0.1 → Top.
        start_tab_drag(&mut state, (56.0, 10.0));
        let pane_id = state.panes[0][0].id;
        let snap = state.ui_snapshot();
        assert_eq!(hovered_zone(&snap), Some((pane_id, DropZone::Top)));
    }

    #[test]
    fn hovered_zone_returns_center_for_dead_center_cursor() {
        let mut state = seed_state();
        configure_grid(&mut state, GRID);
        start_tab_drag(&mut state, (56.0, 50.0));
        let pane_id = state.panes[0][0].id;
        let snap = state.ui_snapshot();
        assert_eq!(hovered_zone(&snap), Some((pane_id, DropZone::Center)));
    }

    #[test]
    fn hovered_zone_none_when_idle() {
        let snap = seed_state().ui_snapshot();
        assert_eq!(hovered_zone(&snap), None);
    }

    #[test]
    fn hovered_zone_none_when_cursor_outside_all_rects() {
        let mut state = seed_state();
        configure_grid(&mut state, GRID);
        start_tab_drag(&mut state, (-10.0, -10.0));
        assert_eq!(hovered_zone(&state.ui_snapshot()), None);
    }

    #[test]
    fn no_preview_when_cursor_outside_pane() {
        let mut state = seed_state();
        configure_grid(&mut state, GRID);
        start_tab_drag(&mut state, (500.0, 500.0));
        let pane_id = state.panes[0][0].id;
        let snap = state.ui_snapshot();
        let overlay = build_pane_drop_zone_overlay(&snap, pane_id).unwrap();
        assert!(
            find_preview(&overlay).is_none(),
            "no zone hovered → no preview rect"
        );
    }

    #[test]
    fn preview_for_left_zone_covers_left_half() {
        let mut state = seed_state();
        configure_grid(&mut state, GRID);
        start_tab_drag(&mut state, (10.0, 50.0));
        let pane_id = state.panes[0][0].id;
        let snap = state.ui_snapshot();
        let overlay = build_pane_drop_zone_overlay(&snap, pane_id).unwrap();
        let preview = find_preview(&overlay).expect("preview missing for hovered zone");
        assert!(has_class(preview, "drop-zone-left"));
        let (left, top, w, h) = percent_rect(preview);
        assert_eq!(
            (left, top, w, h),
            (Some(0.0), Some(0.0), Some(50.0), Some(100.0))
        );
    }

    #[test]
    fn preview_for_bottom_zone_covers_bottom_half() {
        let mut state = seed_state();
        configure_grid(&mut state, GRID);
        // Bottom strip middle band: nx = 0.5, ny ~ 0.9 → Bottom.
        start_tab_drag(&mut state, (56.0, 90.0));
        let pane_id = state.panes[0][0].id;
        let snap = state.ui_snapshot();
        let overlay = build_pane_drop_zone_overlay(&snap, pane_id).unwrap();
        let preview = find_preview(&overlay).expect("preview missing");
        let (left, top, w, h) = percent_rect(preview);
        assert_eq!(
            (left, top, w, h),
            (Some(0.0), Some(50.0), Some(100.0), Some(50.0))
        );
    }

    #[test]
    fn preview_for_center_zone_covers_full_pane() {
        let mut state = seed_state();
        configure_grid(&mut state, GRID);
        start_tab_drag(&mut state, (56.0, 50.0));
        let pane_id = state.panes[0][0].id;
        let snap = state.ui_snapshot();
        let overlay = build_pane_drop_zone_overlay(&snap, pane_id).unwrap();
        let preview = find_preview(&overlay).expect("preview missing");
        assert!(has_class(preview, "drop-zone-center"));
        let (left, top, w, h) = percent_rect(preview);
        assert_eq!(
            (left, top, w, h),
            (Some(0.0), Some(0.0), Some(100.0), Some(100.0))
        );
    }

    #[test]
    fn multi_pane_only_shows_preview_on_hovered_pane() {
        let mut state = seed_state();
        let initial = state.active_pane;
        crate::state::mutate_split_right(&mut state, initial);
        let grid = Rect {
            x: 0.0,
            y: 0.0,
            width: 2000.0,
            height: 100.0,
        };
        configure_grid(&mut state, grid);
        let pane_a = state.panes[0][0].id;
        let pane_b = state.panes[0][1].id;
        start_tab_drag(&mut state, (1050.0, 50.0));
        let snap = state.ui_snapshot();
        // Cursor at (1050, 50) is in pane_b's left strip.
        assert_eq!(hovered_zone(&snap), Some((pane_b, DropZone::Left)));
        let overlay_a = build_pane_drop_zone_overlay(&snap, pane_a).unwrap();
        let overlay_b = build_pane_drop_zone_overlay(&snap, pane_b).unwrap();
        assert!(find_preview(&overlay_a).is_none());
        let preview_b = find_preview(&overlay_b).expect("preview on hovered pane");
        assert!(has_class(preview_b, "drop-zone-left"));
    }

    /// Helper to pull out the four percent-valued position/size props.
    fn percent_rect(el: &ElementDef) -> (Option<f32>, Option<f32>, Option<f32>, Option<f32>) {
        let mut left = None;
        let mut top = None;
        let mut width = None;
        let mut height = None;
        for style in &el.style_overrides {
            match style {
                StyleDeclaration::Left(Dimension::Percent(v)) => left = Some(*v),
                StyleDeclaration::Top(Dimension::Percent(v)) => top = Some(*v),
                StyleDeclaration::Width(Dimension::Percent(v)) => width = Some(*v),
                StyleDeclaration::Height(Dimension::Percent(v)) => height = Some(*v),
                _ => {}
            }
        }
        (left, top, width, height)
    }
}
