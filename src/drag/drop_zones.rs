//! Five-zone hit-testing for tab-onto-pane drops (F1).
//!
//! Each pane rectangle is divided into five regions:
//!
//! ```text
//!   +----+------+----+
//!   | TL | Top  | TR |
//!   +----+------+----+
//!   | L  |Center| R  |
//!   +----+------+----+
//!   | BL |Bottom| BR |
//!   +----+------+----+
//! ```
//!
//! The center is the inner 50% of the rect on both axes. The four
//! straight-edge zones (Left/Right/Top/Bottom) are the 25% strips
//! that flank the center band. The corner cells go to whichever edge
//! the cursor is closer to, so a drop near the top-left corner snaps
//! to `Top` or `Left` based on which edge is nearest.

use super::Rect;

/// Which edge of a pane a tab drop would target, or `Center` to move
/// the source tab next to the target without splitting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropZone {
    Left,
    Right,
    Top,
    Bottom,
    Center,
}

impl DropZone {
    /// Lowercase command token used when encoding the zone in a
    /// dispatch string (e.g. `pane.drop_split:7:left`).
    pub fn id(self) -> &'static str {
        match self {
            DropZone::Left => "left",
            DropZone::Right => "right",
            DropZone::Top => "top",
            DropZone::Bottom => "bottom",
            DropZone::Center => "center",
        }
    }

    /// Parse the token emitted by `id`. Returns `None` on any other
    /// value so malformed commands can be rejected safely.
    pub fn from_id(s: &str) -> Option<Self> {
        match s {
            "left" => Some(DropZone::Left),
            "right" => Some(DropZone::Right),
            "top" => Some(DropZone::Top),
            "bottom" => Some(DropZone::Bottom),
            "center" => Some(DropZone::Center),
            _ => None,
        }
    }
}

/// Classify a cursor position relative to a pane rectangle.
/// Returns `None` when the cursor is outside `rect`.
pub fn hit_test(rect: Rect, cursor_x: f32, cursor_y: f32) -> Option<DropZone> {
    if !rect.contains(cursor_x, cursor_y) {
        return None;
    }
    let w = rect.width.max(1.0);
    let h = rect.height.max(1.0);
    let nx = (cursor_x - rect.x) / w;
    let ny = (cursor_y - rect.y) / h;

    let x_center_band = (0.25..=0.75).contains(&nx);
    let y_center_band = (0.25..=0.75).contains(&ny);

    if x_center_band && y_center_band {
        return Some(DropZone::Center);
    }
    if x_center_band {
        return Some(if ny < 0.5 {
            DropZone::Top
        } else {
            DropZone::Bottom
        });
    }
    if y_center_band {
        return Some(if nx < 0.5 {
            DropZone::Left
        } else {
            DropZone::Right
        });
    }

    // Corner cell: the closer edge wins. On a perfect diagonal tie
    // we fall through to the vertical edge deterministically.
    let dl = nx;
    let dr = 1.0 - nx;
    let dt = ny;
    let db = 1.0 - ny;
    let horizontal_edge = if dl < dr {
        DropZone::Left
    } else {
        DropZone::Right
    };
    let vertical_edge = if dt < db {
        DropZone::Top
    } else {
        DropZone::Bottom
    };
    let horizontal_dist = dl.min(dr);
    let vertical_dist = dt.min(db);
    Some(if horizontal_dist < vertical_dist {
        horizontal_edge
    } else {
        vertical_edge
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect() -> Rect {
        Rect {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 100.0,
        }
    }

    #[test]
    fn cursor_outside_rect_returns_none() {
        let r = rect();
        assert_eq!(hit_test(r, -1.0, 50.0), None);
        assert_eq!(hit_test(r, 101.0, 50.0), None);
        assert_eq!(hit_test(r, 50.0, -1.0), None);
        assert_eq!(hit_test(r, 50.0, 101.0), None);
    }

    #[test]
    fn dead_center_is_center() {
        assert_eq!(hit_test(rect(), 50.0, 50.0), Some(DropZone::Center));
    }

    #[test]
    fn center_band_edges_inclusive() {
        // Boundary of the inner 50% square: still center.
        assert_eq!(hit_test(rect(), 25.0, 50.0), Some(DropZone::Center));
        assert_eq!(hit_test(rect(), 75.0, 50.0), Some(DropZone::Center));
        assert_eq!(hit_test(rect(), 50.0, 25.0), Some(DropZone::Center));
        assert_eq!(hit_test(rect(), 50.0, 75.0), Some(DropZone::Center));
    }

    #[test]
    fn left_strip_middle_band_is_left() {
        assert_eq!(hit_test(rect(), 10.0, 50.0), Some(DropZone::Left));
        assert_eq!(hit_test(rect(), 5.0, 40.0), Some(DropZone::Left));
        assert_eq!(hit_test(rect(), 5.0, 60.0), Some(DropZone::Left));
    }

    #[test]
    fn right_strip_middle_band_is_right() {
        assert_eq!(hit_test(rect(), 90.0, 50.0), Some(DropZone::Right));
        assert_eq!(hit_test(rect(), 95.0, 40.0), Some(DropZone::Right));
        assert_eq!(hit_test(rect(), 95.0, 60.0), Some(DropZone::Right));
    }

    #[test]
    fn top_strip_middle_band_is_top() {
        assert_eq!(hit_test(rect(), 50.0, 10.0), Some(DropZone::Top));
        assert_eq!(hit_test(rect(), 40.0, 5.0), Some(DropZone::Top));
        assert_eq!(hit_test(rect(), 60.0, 5.0), Some(DropZone::Top));
    }

    #[test]
    fn bottom_strip_middle_band_is_bottom() {
        assert_eq!(hit_test(rect(), 50.0, 90.0), Some(DropZone::Bottom));
        assert_eq!(hit_test(rect(), 40.0, 95.0), Some(DropZone::Bottom));
        assert_eq!(hit_test(rect(), 60.0, 95.0), Some(DropZone::Bottom));
    }

    #[test]
    fn top_left_corner_closer_to_top_picks_top() {
        // (20, 5): dist_top=0.05, dist_left=0.20; top is closer.
        assert_eq!(hit_test(rect(), 20.0, 5.0), Some(DropZone::Top));
    }

    #[test]
    fn top_left_corner_closer_to_left_picks_left() {
        // (5, 20): dist_left=0.05, dist_top=0.20; left is closer.
        assert_eq!(hit_test(rect(), 5.0, 20.0), Some(DropZone::Left));
    }

    #[test]
    fn top_right_corner_closer_to_top_picks_top() {
        assert_eq!(hit_test(rect(), 80.0, 5.0), Some(DropZone::Top));
    }

    #[test]
    fn top_right_corner_closer_to_right_picks_right() {
        assert_eq!(hit_test(rect(), 95.0, 20.0), Some(DropZone::Right));
    }

    #[test]
    fn bottom_left_corner_closer_to_bottom_picks_bottom() {
        assert_eq!(hit_test(rect(), 20.0, 95.0), Some(DropZone::Bottom));
    }

    #[test]
    fn bottom_left_corner_closer_to_left_picks_left() {
        assert_eq!(hit_test(rect(), 5.0, 80.0), Some(DropZone::Left));
    }

    #[test]
    fn bottom_right_corner_closer_to_bottom_picks_bottom() {
        assert_eq!(hit_test(rect(), 80.0, 95.0), Some(DropZone::Bottom));
    }

    #[test]
    fn bottom_right_corner_closer_to_right_picks_right() {
        assert_eq!(hit_test(rect(), 95.0, 80.0), Some(DropZone::Right));
    }

    #[test]
    fn diagonal_tie_resolves_deterministically() {
        // Exact corner-distance tie: we break toward the vertical edge.
        // (10, 10) and (90, 90) are true ties because both axes use the
        // same computation; the mixed corners fall through to the
        // non-tied corner tests above where one edge is strictly closer.
        assert_eq!(hit_test(rect(), 10.0, 10.0), Some(DropZone::Top));
        assert_eq!(hit_test(rect(), 90.0, 90.0), Some(DropZone::Bottom));
    }

    #[test]
    fn offset_rect_hits_relative_zones() {
        // 200x200 pane at (1000, 500).
        let r = Rect {
            x: 1000.0,
            y: 500.0,
            width: 200.0,
            height: 200.0,
        };
        assert_eq!(hit_test(r, 1100.0, 600.0), Some(DropZone::Center));
        assert_eq!(hit_test(r, 1010.0, 600.0), Some(DropZone::Left));
        assert_eq!(hit_test(r, 1190.0, 600.0), Some(DropZone::Right));
        assert_eq!(hit_test(r, 1100.0, 510.0), Some(DropZone::Top));
        assert_eq!(hit_test(r, 1100.0, 690.0), Some(DropZone::Bottom));
    }

    #[test]
    fn rectangular_pane_respects_its_own_proportions() {
        // Wide pane: edge zones are thicker in pixels horizontally.
        let r = Rect {
            x: 0.0,
            y: 0.0,
            width: 400.0,
            height: 100.0,
        };
        // 60px from the left is 15% of width, so still in the Left strip.
        assert_eq!(hit_test(r, 60.0, 50.0), Some(DropZone::Left));
        // 120px is 30% of width, out of the left strip, into the center band.
        assert_eq!(hit_test(r, 120.0, 50.0), Some(DropZone::Center));
    }

    #[test]
    fn center_of_offset_rect_still_center() {
        let r = Rect {
            x: 250.0,
            y: 100.0,
            width: 600.0,
            height: 400.0,
        };
        assert_eq!(hit_test(r, 550.0, 300.0), Some(DropZone::Center));
    }

    #[test]
    fn zero_size_rect_returns_none() {
        let r = Rect {
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
        };
        assert_eq!(hit_test(r, 0.0, 0.0), None);
    }

    #[test]
    fn drop_zone_id_round_trip() {
        for z in [
            DropZone::Left,
            DropZone::Right,
            DropZone::Top,
            DropZone::Bottom,
            DropZone::Center,
        ] {
            assert_eq!(DropZone::from_id(z.id()), Some(z));
        }
        assert_eq!(DropZone::from_id("nonsense"), None);
        assert_eq!(DropZone::from_id(""), None);
        assert_eq!(DropZone::from_id("LEFT"), None, "casing is strict");
    }

    #[test]
    fn hit_test_is_deterministic_at_all_25_percent_lines() {
        // The four boundary lines of the center band resolve to Center
        // rather than flipping to an edge zone.
        assert_eq!(hit_test(rect(), 25.0, 25.0), Some(DropZone::Center));
        assert_eq!(hit_test(rect(), 75.0, 75.0), Some(DropZone::Center));
        assert_eq!(hit_test(rect(), 25.0, 75.0), Some(DropZone::Center));
        assert_eq!(hit_test(rect(), 75.0, 25.0), Some(DropZone::Center));
    }
}
