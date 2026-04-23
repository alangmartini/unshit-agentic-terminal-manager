//! Drop-zone overlay rendered over each pane while a tab drag is in
//! progress (F1).
//!
//! The overlay is a set of fixed-position boxes, one per pane, that
//! visualise where a dropped tab will land. Each pane's overlay
//! contains five children (Left/Right/Top/Bottom/Center) tiled to
//! match `drop_zones::hit_test`, minus the four corner squares which
//! aren't drawn: a cursor in a corner highlights the nearest edge
//! band via the shared `hit_test` logic rather than showing a
//! distinct corner zone.
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
/// overlay hit-test matches the one used by `dispatch_drag_end`.
fn snapshot_pane_rects(state: &UiSnapshot) -> Vec<(PaneId, Rect)> {
    let grid = crate::drag::grid_rect_from_state(
        state.sidebar_width,
        state.tabbar_rect,
        state.last_grid_width,
        state.last_grid_height,
        state.scale_factor,
    );
    crate::drag::compute_pane_rects(&state.panes, &state.row_ratios, &state.col_ratios, grid)
}

/// Find the pane currently under the cursor (if any) during a tab
/// drag, along with which zone of it is hovered. Returns `None` when
/// no tab drag is in progress or the cursor sits outside every pane.
pub fn hovered_zone(state: &UiSnapshot) -> Option<(PaneId, DropZone)> {
    if !matches!(state.drag, DragState::DraggingTab { .. }) {
        return None;
    }
    let (cx, cy) = state.drag.cursor()?;
    for (id, rect) in snapshot_pane_rects(state) {
        if let Some(zone) = hit_test(rect, cx, cy) {
            return Some((id, zone));
        }
    }
    None
}

/// Build one overlay element per pane in the active tab. Returns an
/// empty vec when no tab drag is in progress or when the grid hasn't
/// been measured yet (so pane rects would be zero-sized and there's
/// nothing sensible to draw).
pub fn build_drop_zone_overlay(state: &UiSnapshot) -> Vec<ElementDef> {
    if !matches!(state.drag, DragState::DraggingTab { .. }) {
        return Vec::new();
    }
    let hover = hovered_zone(state);
    snapshot_pane_rects(state)
        .into_iter()
        .map(|(id, rect)| {
            let hovered = hover.and_then(|(p, z)| (p == id).then_some(z));
            build_pane_overlay(rect, hovered)
        })
        .collect()
}

fn build_pane_overlay(rect: Rect, hovered: Option<DropZone>) -> ElementDef {
    let mut container = ElementDef::new(Tag::Div)
        .with_class("drop-zone-overlay")
        .with_style(StyleDeclaration::Position(CssPosition::Fixed))
        .with_style(StyleDeclaration::Left(Dimension::Px(rect.x)))
        .with_style(StyleDeclaration::Top(Dimension::Px(rect.y)))
        .with_style(StyleDeclaration::Width(Dimension::Px(rect.width)))
        .with_style(StyleDeclaration::Height(Dimension::Px(rect.height)));
    for zone in [
        DropZone::Left,
        DropZone::Right,
        DropZone::Top,
        DropZone::Bottom,
        DropZone::Center,
    ] {
        container = container.with_child(build_zone(rect, zone, hovered == Some(zone)));
    }
    container
}

fn build_zone(rect: Rect, zone: DropZone, active: bool) -> ElementDef {
    // Fractional (x, y, w, h) for each zone. These tile the 5 active
    // regions of `hit_test`; the 4 corners are not drawn.
    let (fx, fy, fw, fh) = match zone {
        DropZone::Left => (0.0, 0.25, 0.25, 0.5),
        DropZone::Right => (0.75, 0.25, 0.25, 0.5),
        DropZone::Top => (0.25, 0.0, 0.5, 0.25),
        DropZone::Bottom => (0.25, 0.75, 0.5, 0.25),
        DropZone::Center => (0.25, 0.25, 0.5, 0.5),
    };
    let class = match zone {
        DropZone::Left => "drop-zone-left",
        DropZone::Right => "drop-zone-right",
        DropZone::Top => "drop-zone-top",
        DropZone::Bottom => "drop-zone-bottom",
        DropZone::Center => "drop-zone-center",
    };
    let mut el = ElementDef::new(Tag::Div)
        .with_class("drop-zone")
        .with_class(class)
        .with_style(StyleDeclaration::Position(CssPosition::Fixed))
        .with_style(StyleDeclaration::Left(Dimension::Px(
            rect.x + rect.width * fx,
        )))
        .with_style(StyleDeclaration::Top(Dimension::Px(
            rect.y + rect.height * fy,
        )))
        .with_style(StyleDeclaration::Width(Dimension::Px(rect.width * fw)))
        .with_style(StyleDeclaration::Height(Dimension::Px(rect.height * fh)));
    if active {
        el = el.with_class("hovered");
    }
    el
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
    fn overlay_empty_when_grid_not_measured() {
        // Without calling configure_grid, last_grid_width/height are 0.
        let mut state = seed_state();
        start_tab_drag(&mut state, (0.0, 0.0));
        let snap = state.ui_snapshot();
        assert!(build_drop_zone_overlay(&snap).is_empty());
    }

    #[test]
    fn overlay_has_one_container_per_pane() {
        let mut state = seed_state();
        configure_grid(&mut state, GRID);
        start_tab_drag(&mut state, (56.0, 50.0));
        let snap = state.ui_snapshot();
        let overlays = build_drop_zone_overlay(&snap);
        assert_eq!(overlays.len(), 1);
        assert!(has_class(&overlays[0], "drop-zone-overlay"));
    }

    #[test]
    fn each_overlay_has_five_zone_children() {
        let mut state = seed_state();
        configure_grid(&mut state, GRID);
        start_tab_drag(&mut state, (56.0, 50.0));
        let overlays = build_drop_zone_overlay(&state.ui_snapshot());
        assert_eq!(overlays[0].children.len(), 5);
        let classes: Vec<&str> = overlays[0]
            .children
            .iter()
            .flat_map(|c| c.classes.iter().map(|s| s.as_str()))
            .collect();
        for expected in [
            "drop-zone-left",
            "drop-zone-right",
            "drop-zone-top",
            "drop-zone-bottom",
            "drop-zone-center",
        ] {
            assert!(
                classes.contains(&expected),
                "missing zone class: {}",
                expected
            );
        }
    }

    #[test]
    fn center_cursor_highlights_center_zone() {
        let mut state = seed_state();
        configure_grid(&mut state, GRID);
        // Pane is at (6, 0, 100, 100); dead center is (56, 50).
        start_tab_drag(&mut state, (56.0, 50.0));
        let overlay = &build_drop_zone_overlay(&state.ui_snapshot())[0];
        let hovered: Vec<&str> = overlay
            .children
            .iter()
            .filter(|c| has_class(c, "hovered"))
            .flat_map(|c| c.classes.iter().map(|s| s.as_str()))
            .filter(|c| c.starts_with("drop-zone-") && *c != "drop-zone")
            .collect();
        assert_eq!(hovered, vec!["drop-zone-center"]);
    }

    #[test]
    fn left_edge_cursor_highlights_left_zone() {
        let mut state = seed_state();
        configure_grid(&mut state, GRID);
        // Cursor at (10, 50): nx = (10-6)/100 = 0.04, ny = 0.5 → Left.
        start_tab_drag(&mut state, (10.0, 50.0));
        let overlay = &build_drop_zone_overlay(&state.ui_snapshot())[0];
        let left_child = overlay
            .children
            .iter()
            .find(|c| has_class(c, "drop-zone-left"))
            .unwrap();
        assert!(has_class(left_child, "hovered"));
        let center_child = overlay
            .children
            .iter()
            .find(|c| has_class(c, "drop-zone-center"))
            .unwrap();
        assert!(!has_class(center_child, "hovered"));
    }

    #[test]
    fn cursor_outside_all_panes_highlights_nothing() {
        let mut state = seed_state();
        configure_grid(&mut state, GRID);
        start_tab_drag(&mut state, (500.0, 500.0));
        let overlay = &build_drop_zone_overlay(&state.ui_snapshot())[0];
        let any_hovered = overlay.children.iter().any(|c| has_class(c, "hovered"));
        assert!(!any_hovered);
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
    fn overlay_positions_container_at_pane_rect() {
        let mut state = seed_state();
        let grid = Rect {
            x: 1000.0,
            y: 500.0,
            width: 200.0,
            height: 200.0,
        };
        configure_grid(&mut state, grid);
        start_tab_drag(&mut state, (1100.0, 600.0));
        let overlay = &build_drop_zone_overlay(&state.ui_snapshot())[0];
        let mut left = None;
        let mut top = None;
        let mut width = None;
        let mut height = None;
        for style in &overlay.style_overrides {
            match style {
                StyleDeclaration::Left(Dimension::Px(v)) => left = Some(*v),
                StyleDeclaration::Top(Dimension::Px(v)) => top = Some(*v),
                StyleDeclaration::Width(Dimension::Px(v)) => width = Some(*v),
                StyleDeclaration::Height(Dimension::Px(v)) => height = Some(*v),
                _ => {}
            }
        }
        assert_eq!(left, Some(1000.0));
        assert_eq!(top, Some(500.0));
        assert_eq!(width, Some(200.0));
        assert_eq!(height, Some(200.0));
    }

    #[test]
    fn zones_offset_to_pane_origin() {
        // Non-zero pane origin: zone geometry must be in window coords,
        // not pane-local ones (the overlay is Position::Fixed).
        let mut state = seed_state();
        let grid = Rect {
            x: 1000.0,
            y: 500.0,
            width: 200.0,
            height: 200.0,
        };
        configure_grid(&mut state, grid);
        start_tab_drag(&mut state, (1100.0, 600.0));
        let overlay = &build_drop_zone_overlay(&state.ui_snapshot())[0];
        // Center zone: x=1000+200*0.25=1050, y=500+200*0.25=550, w=100, h=100.
        let center = overlay
            .children
            .iter()
            .find(|c| has_class(c, "drop-zone-center"))
            .unwrap();
        let mut left = None;
        let mut top = None;
        let mut width = None;
        let mut height = None;
        for style in &center.style_overrides {
            match style {
                StyleDeclaration::Left(Dimension::Px(v)) => left = Some(*v),
                StyleDeclaration::Top(Dimension::Px(v)) => top = Some(*v),
                StyleDeclaration::Width(Dimension::Px(v)) => width = Some(*v),
                StyleDeclaration::Height(Dimension::Px(v)) => height = Some(*v),
                _ => {}
            }
        }
        assert_eq!(left, Some(1050.0));
        assert_eq!(top, Some(550.0));
        assert_eq!(width, Some(100.0));
        assert_eq!(height, Some(100.0));
    }

    #[test]
    fn multi_pane_overlay_only_highlights_pane_under_cursor() {
        let mut state = seed_state();
        let initial = state.active_pane;
        crate::state::mutate_split_right(&mut state, initial);
        // Grid at (0, 0) sized 2000x100; two equal columns land panes at
        // (0, 0, 1000, 100) and (1000, 0, 1000, 100). Use sidebar_width
        // of -6 so grid.x resolves to 0 exactly.
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
        let overlays = build_drop_zone_overlay(&snap);
        assert_eq!(overlays.len(), 2);
        // Cursor at (1050, 50) is in pane_b's left strip.
        let _ = pane_a;
        let hover = hovered_zone(&snap);
        assert_eq!(hover, Some((pane_b, DropZone::Left)));
    }
}
