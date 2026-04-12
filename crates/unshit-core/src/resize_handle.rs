//! Split pane resize handles for CSS Grid layouts.
//!
//! A resize handle is a thin draggable divider placed between CSS Grid tracks.
//! Dragging it adjusts the sizes of the two adjacent tracks (columns or rows).
//!
//! This module provides:
//! - `ResizeAxis`: whether the handle resizes columns (vertical divider) or rows (horizontal divider)
//! - `PaneResizeEvent`: data sent to `on_pane_resize` callbacks
//! - `compute_new_track_sizes`: the core algorithm that converts drag pixel deltas into track size adjustments

use crate::style::types::{GridMaxTrackSize, GridMinTrackSize, GridTrackDef, GridTrackSize};

/// Which axis the resize handle operates on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResizeAxis {
    /// Vertical divider: dragging left/right adjusts adjacent column widths.
    Vertical,
    /// Horizontal divider: dragging up/down adjusts adjacent row heights.
    Horizontal,
}

/// Data emitted by the `on_pane_resize` callback after a resize operation.
#[derive(Clone, Debug)]
pub struct PaneResizeEvent {
    /// The axis that was resized.
    pub axis: ResizeAxis,
    /// Index of the track before the handle (in the grid-template-columns/rows list).
    pub before_track_index: usize,
    /// Index of the track after the handle.
    pub after_track_index: usize,
    /// New size of the track before the handle.
    pub before_track: GridTrackSize,
    /// New size of the track after the handle.
    pub after_track: GridTrackSize,
    /// Full updated track list.
    pub new_tracks: Vec<GridTrackDef>,
}

/// Initial state captured at drag start, used throughout the drag operation.
#[derive(Clone, Debug)]
pub struct ResizeDragState {
    /// The axis being resized.
    pub axis: ResizeAxis,
    /// Index into the track definition list for the track before the handle.
    pub before_index: usize,
    /// Index into the track definition list for the track after the handle.
    pub after_index: usize,
    /// Snapshot of track defs at drag start.
    pub initial_tracks: Vec<GridTrackDef>,
    /// Effective pixel size of the "before" track at drag start.
    pub initial_before_px: f32,
    /// Effective pixel size of the "after" track at drag start.
    pub initial_after_px: f32,
    /// Minimum pixel size for the "before" pane (from CSS min-width/min-height).
    pub min_before_px: f32,
    /// Minimum pixel size for the "after" pane (from CSS min-width/min-height).
    pub min_after_px: f32,
}

/// Apply a drag pixel delta to two adjacent tracks, returning the updated track list.
///
/// Supports `px` tracks (direct pixel adjustment), `fr` tracks (proportional redistribution),
/// and mixed units.
///
/// `delta` is positive when moving toward the "after" track (right for vertical axis,
/// down for horizontal axis) and negative when moving toward the "before" track.
pub fn compute_new_track_sizes(state: &ResizeDragState, delta: f32) -> Vec<GridTrackDef> {
    let mut tracks = state.initial_tracks.clone();

    // Clamp delta to respect minimum sizes.
    // Moving positive shrinks "after", moving negative shrinks "before".
    let max_positive = state.initial_after_px - state.min_after_px;
    let max_negative = -(state.initial_before_px - state.min_before_px);
    let clamped = delta.clamp(max_negative, max_positive);

    let before_def = &state.initial_tracks[state.before_index];
    let after_def = &state.initial_tracks[state.after_index];

    let new_before = adjust_track_def(before_def, clamped, state.initial_before_px);
    let new_after = adjust_track_def(after_def, -clamped, state.initial_after_px);

    tracks[state.before_index] = new_before;
    tracks[state.after_index] = new_after;

    tracks
}

/// Reset two adjacent tracks to equal sizes.
/// For `fr` tracks: both get the average fr value.
/// For `px` tracks: both get the average pixel value.
/// For mixed: both convert to px at the average of their effective pixels.
pub fn reset_tracks_equal(state: &ResizeDragState) -> Vec<GridTrackDef> {
    let mut tracks = state.initial_tracks.clone();

    let before_def = &state.initial_tracks[state.before_index];
    let after_def = &state.initial_tracks[state.after_index];

    let (new_before, new_after) =
        equalize_track_defs(before_def, after_def, state.initial_before_px, state.initial_after_px);

    tracks[state.before_index] = new_before;
    tracks[state.after_index] = new_after;

    tracks
}

/// Adjust a single track definition by a pixel delta.
fn adjust_track_def(def: &GridTrackDef, delta_px: f32, effective_px: f32) -> GridTrackDef {
    match def {
        GridTrackDef::Single(size) => {
            GridTrackDef::Single(adjust_track_size(size, delta_px, effective_px))
        }
        GridTrackDef::Repeat(count, sizes) => {
            // For repeat tracks, adjust the first track (simplified)
            let mut new_sizes = sizes.clone();
            if !new_sizes.is_empty() {
                new_sizes[0] = adjust_track_size(&new_sizes[0], delta_px, effective_px);
            }
            GridTrackDef::Repeat(*count, new_sizes)
        }
    }
}

/// Adjust a GridTrackSize by a pixel delta.
fn adjust_track_size(size: &GridTrackSize, delta_px: f32, effective_px: f32) -> GridTrackSize {
    match (&size.min, &size.max) {
        // Pure px track: adjust pixel values directly
        (GridMinTrackSize::Px(min_v), GridMaxTrackSize::Px(max_v)) => {
            let new_min = (*min_v + delta_px).max(0.0);
            let new_max = (*max_v + delta_px).max(0.0);
            GridTrackSize { min: GridMinTrackSize::Px(new_min), max: GridMaxTrackSize::Px(new_max) }
        }
        // Pure fr track: adjust fr proportionally based on pixel delta
        (GridMinTrackSize::Auto, GridMaxTrackSize::Fr(fr_val)) => {
            if effective_px > 0.0 {
                let ratio = (effective_px + delta_px) / effective_px;
                let new_fr = (*fr_val * ratio).max(0.01);
                GridTrackSize::fr(new_fr)
            } else {
                // Cannot compute ratio with zero effective size
                *size
            }
        }
        // For any other combination, convert to px
        _ => {
            let new_px = (effective_px + delta_px).max(0.0);
            GridTrackSize::fixed_px(new_px)
        }
    }
}

/// Equalize two track definitions so they have equal sizes.
fn equalize_track_defs(
    before: &GridTrackDef,
    after: &GridTrackDef,
    before_px: f32,
    after_px: f32,
) -> (GridTrackDef, GridTrackDef) {
    let auto_default = GridTrackSize::auto();
    let before_size = match before {
        GridTrackDef::Single(s) => s,
        GridTrackDef::Repeat(_, sizes) => sizes.first().unwrap_or(&auto_default),
    };
    let after_size = match after {
        GridTrackDef::Single(s) => s,
        GridTrackDef::Repeat(_, sizes) => sizes.first().unwrap_or(&auto_default),
    };

    let (new_before_size, new_after_size) =
        equalize_track_sizes(before_size, after_size, before_px, after_px);

    let new_before = match before {
        GridTrackDef::Single(_) => GridTrackDef::Single(new_before_size),
        GridTrackDef::Repeat(count, sizes) => {
            let mut new_sizes = sizes.clone();
            if !new_sizes.is_empty() {
                new_sizes[0] = new_before_size;
            }
            GridTrackDef::Repeat(*count, new_sizes)
        }
    };

    let new_after = match after {
        GridTrackDef::Single(_) => GridTrackDef::Single(new_after_size),
        GridTrackDef::Repeat(count, sizes) => {
            let mut new_sizes = sizes.clone();
            if !new_sizes.is_empty() {
                new_sizes[0] = new_after_size;
            }
            GridTrackDef::Repeat(*count, new_sizes)
        }
    };

    (new_before, new_after)
}

/// Equalize two track sizes.
fn equalize_track_sizes(
    before: &GridTrackSize,
    after: &GridTrackSize,
    before_px: f32,
    after_px: f32,
) -> (GridTrackSize, GridTrackSize) {
    match (&before.max, &after.max) {
        // Both fr: average the fr values
        (GridMaxTrackSize::Fr(fr_a), GridMaxTrackSize::Fr(fr_b)) => {
            let avg_fr = (*fr_a + *fr_b) / 2.0;
            (GridTrackSize::fr(avg_fr), GridTrackSize::fr(avg_fr))
        }
        // Both px: average the pixel values
        (GridMaxTrackSize::Px(_), GridMaxTrackSize::Px(_)) => {
            let avg_px = (before_px + after_px) / 2.0;
            (GridTrackSize::fixed_px(avg_px), GridTrackSize::fixed_px(avg_px))
        }
        // Mixed: convert to px at the average of effective pixels
        _ => {
            let avg_px = (before_px + after_px) / 2.0;
            (GridTrackSize::fixed_px(avg_px), GridTrackSize::fixed_px(avg_px))
        }
    }
}

/// Extract the effective pixel size of a track from layout results.
/// This is needed to convert fr units to pixels for delta application.
///
/// `container_size` is the total size of the grid container along the relevant axis.
/// `track_sizes` are the resolved pixel sizes of all tracks (from taffy layout).
pub fn effective_track_px(track_sizes: &[f32], index: usize) -> f32 {
    track_sizes.get(index).copied().unwrap_or(0.0)
}

/// Determine whether a given class name marks an element as a resize handle.
pub fn is_resize_handle_class(class: &str) -> bool {
    class == "resize-handle"
        || class == "resize-handle-vertical"
        || class == "resize-handle-horizontal"
}

/// Parse the resize axis from class names.
/// Returns `Some(ResizeAxis)` if a resize-handle class is present.
pub fn resize_axis_from_classes(classes: &[String]) -> Option<ResizeAxis> {
    for class in classes {
        match class.as_str() {
            "resize-handle-vertical" | "resize-handle-v" => return Some(ResizeAxis::Vertical),
            "resize-handle-horizontal" | "resize-handle-h" => return Some(ResizeAxis::Horizontal),
            "resize-handle" => {
                // Default to vertical (column resize) if no axis specified
                return Some(ResizeAxis::Vertical);
            }
            _ => {}
        }
    }
    None
}

/// Compute the total fr sum from a list of track definitions.
/// Useful for verifying fr redistribution maintains the total.
pub fn total_fr_sum(tracks: &[GridTrackDef]) -> f32 {
    tracks.iter().map(|t| track_def_fr(t)).sum()
}

fn track_def_fr(def: &GridTrackDef) -> f32 {
    match def {
        GridTrackDef::Single(size) => track_size_fr(size),
        GridTrackDef::Repeat(_, sizes) => sizes.iter().map(track_size_fr).sum(),
    }
}

fn track_size_fr(size: &GridTrackSize) -> f32 {
    match size.max {
        GridMaxTrackSize::Fr(v) => v,
        _ => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_px_tracks(sizes: &[f32]) -> Vec<GridTrackDef> {
        sizes.iter().map(|s| GridTrackDef::Single(GridTrackSize::fixed_px(*s))).collect()
    }

    fn make_state(
        tracks: Vec<GridTrackDef>,
        before_idx: usize,
        after_idx: usize,
        before_px: f32,
        after_px: f32,
    ) -> ResizeDragState {
        ResizeDragState {
            axis: ResizeAxis::Vertical,
            before_index: before_idx,
            after_index: after_idx,
            initial_tracks: tracks,
            initial_before_px: before_px,
            initial_after_px: after_px,
            min_before_px: 0.0,
            min_after_px: 0.0,
        }
    }

    #[test]
    fn px_tracks_adjust_by_delta() {
        // 200px | handle | 300px  -- drag 50px right
        let tracks = make_px_tracks(&[200.0, 6.0, 300.0]);
        let state = make_state(tracks, 0, 2, 200.0, 300.0);
        let result = compute_new_track_sizes(&state, 50.0);

        // Before track: 200 + 50 = 250
        match &result[0] {
            GridTrackDef::Single(s) => {
                assert_eq!(s.max, GridMaxTrackSize::Px(250.0));
            }
            _ => panic!("expected Single"),
        }
        // After track: 300 - 50 = 250
        match &result[2] {
            GridTrackDef::Single(s) => {
                assert_eq!(s.max, GridMaxTrackSize::Px(250.0));
            }
            _ => panic!("expected Single"),
        }
        // Handle track unchanged
        match &result[1] {
            GridTrackDef::Single(s) => {
                assert_eq!(s.max, GridMaxTrackSize::Px(6.0));
            }
            _ => panic!("expected Single"),
        }
    }

    #[test]
    fn fr_tracks_maintain_total_sum() {
        // 1fr | handle | 2fr  -- drag 100px right (out of 300px "before" effective)
        let tracks = vec![
            GridTrackDef::Single(GridTrackSize::fr(1.0)),
            GridTrackDef::Single(GridTrackSize::fixed_px(6.0)),
            GridTrackDef::Single(GridTrackSize::fr(2.0)),
        ];
        let state = make_state(tracks.clone(), 0, 2, 200.0, 400.0);
        let result = compute_new_track_sizes(&state, 100.0);

        let initial_sum = total_fr_sum(&tracks);
        let new_sum = total_fr_sum(&result);
        // Fr sum should be preserved
        assert!(
            (initial_sum - new_sum).abs() < 0.01,
            "fr sum changed: initial={}, new={}",
            initial_sum,
            new_sum
        );
    }

    #[test]
    fn min_size_enforcement_clamps_delta() {
        // 200px | handle | 300px  -- drag 350px right (would over-shrink "after")
        let tracks = make_px_tracks(&[200.0, 6.0, 300.0]);
        let mut state = make_state(tracks, 0, 2, 200.0, 300.0);
        state.min_after_px = 50.0;

        let result = compute_new_track_sizes(&state, 350.0);

        // After track should not go below 50px: clamped delta = 250
        match &result[2] {
            GridTrackDef::Single(s) => {
                assert!(
                    (extract_px_max(s) - 50.0).abs() < 0.01,
                    "after track should be clamped to min 50px, got {}",
                    extract_px_max(s)
                );
            }
            _ => panic!("expected Single"),
        }
        // Before track: 200 + 250 = 450
        match &result[0] {
            GridTrackDef::Single(s) => {
                assert!(
                    (extract_px_max(s) - 450.0).abs() < 0.01,
                    "before track should be 450px, got {}",
                    extract_px_max(s)
                );
            }
            _ => panic!("expected Single"),
        }
    }

    #[test]
    fn min_size_enforcement_clamps_negative_delta() {
        // 200px | handle | 300px  -- drag 250px left (would over-shrink "before")
        let tracks = make_px_tracks(&[200.0, 6.0, 300.0]);
        let mut state = make_state(tracks, 0, 2, 200.0, 300.0);
        state.min_before_px = 100.0;

        let result = compute_new_track_sizes(&state, -250.0);

        // Before track should not go below 100px: clamped delta = -100
        match &result[0] {
            GridTrackDef::Single(s) => {
                assert!(
                    (extract_px_max(s) - 100.0).abs() < 0.01,
                    "before track should be clamped to min 100px, got {}",
                    extract_px_max(s)
                );
            }
            _ => panic!("expected Single"),
        }
    }

    #[test]
    fn reset_equal_px_tracks() {
        let tracks = make_px_tracks(&[200.0, 6.0, 400.0]);
        let state = make_state(tracks, 0, 2, 200.0, 400.0);
        let result = reset_tracks_equal(&state);

        // Both should be (200+400)/2 = 300
        let before_px = extract_px_max(match &result[0] {
            GridTrackDef::Single(s) => s,
            _ => panic!("expected Single"),
        });
        let after_px = extract_px_max(match &result[2] {
            GridTrackDef::Single(s) => s,
            _ => panic!("expected Single"),
        });
        assert!((before_px - 300.0).abs() < 0.01);
        assert!((after_px - 300.0).abs() < 0.01);
    }

    #[test]
    fn reset_equal_fr_tracks() {
        let tracks = vec![
            GridTrackDef::Single(GridTrackSize::fr(1.0)),
            GridTrackDef::Single(GridTrackSize::fixed_px(6.0)),
            GridTrackDef::Single(GridTrackSize::fr(3.0)),
        ];
        let state = make_state(tracks, 0, 2, 100.0, 300.0);
        let result = reset_tracks_equal(&state);

        // Both should be (1.0+3.0)/2 = 2.0 fr
        match &result[0] {
            GridTrackDef::Single(s) => {
                assert_eq!(s.max, GridMaxTrackSize::Fr(2.0));
            }
            _ => panic!("expected Single"),
        }
        match &result[2] {
            GridTrackDef::Single(s) => {
                assert_eq!(s.max, GridMaxTrackSize::Fr(2.0));
            }
            _ => panic!("expected Single"),
        }
    }

    #[test]
    fn resize_axis_from_classes_vertical() {
        let classes = vec!["resize-handle-vertical".to_string()];
        assert_eq!(resize_axis_from_classes(&classes), Some(ResizeAxis::Vertical));
    }

    #[test]
    fn resize_axis_from_classes_horizontal() {
        let classes = vec!["resize-handle-horizontal".to_string()];
        assert_eq!(resize_axis_from_classes(&classes), Some(ResizeAxis::Horizontal));
    }

    #[test]
    fn resize_axis_from_classes_default() {
        let classes = vec!["resize-handle".to_string()];
        assert_eq!(resize_axis_from_classes(&classes), Some(ResizeAxis::Vertical));
    }

    #[test]
    fn resize_axis_from_classes_none() {
        let classes = vec!["button".to_string()];
        assert_eq!(resize_axis_from_classes(&classes), None);
    }

    #[test]
    fn zero_delta_produces_no_change() {
        let tracks = make_px_tracks(&[200.0, 6.0, 300.0]);
        let state = make_state(tracks.clone(), 0, 2, 200.0, 300.0);
        let result = compute_new_track_sizes(&state, 0.0);

        assert_eq!(tracks, result);
    }

    #[test]
    fn negative_delta_on_px_tracks() {
        // 200px | handle | 300px  -- drag 50px left
        let tracks = make_px_tracks(&[200.0, 6.0, 300.0]);
        let state = make_state(tracks, 0, 2, 200.0, 300.0);
        let result = compute_new_track_sizes(&state, -50.0);

        match &result[0] {
            GridTrackDef::Single(s) => {
                assert!((extract_px_max(s) - 150.0).abs() < 0.01);
            }
            _ => panic!("expected Single"),
        }
        match &result[2] {
            GridTrackDef::Single(s) => {
                assert!((extract_px_max(s) - 350.0).abs() < 0.01);
            }
            _ => panic!("expected Single"),
        }
    }

    fn extract_px_max(size: &GridTrackSize) -> f32 {
        match size.max {
            GridMaxTrackSize::Px(v) => v,
            _ => panic!("expected Px, got {:?}", size.max),
        }
    }
}
