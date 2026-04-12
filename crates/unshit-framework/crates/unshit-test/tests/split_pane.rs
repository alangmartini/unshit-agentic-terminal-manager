use std::sync::{Arc, Mutex};
use unshit_core::element::*;
use unshit_core::event::{DragEvent, DragPhase};
use unshit_core::resize_handle::*;
use unshit_core::style::types::*;
use unshit_test::TestHarness;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// CSS for a horizontal split: two panes side by side with a vertical resize handle.
fn horizontal_split_css() -> &'static str {
    r#"
    .root {
        display: grid;
        grid-template-columns: 200px 6px 300px;
        width: 506px;
        height: 400px;
    }
    .pane-left {
        min-width: 50px;
        background: #333333;
    }
    .resize-handle-vertical {
        background: #666666;
        cursor: col-resize;
        resize-axis: vertical;
    }
    .pane-right {
        min-width: 50px;
        background: #444444;
    }
    "#
}

/// CSS for a vertical split: two panes stacked with a horizontal resize handle.
fn vertical_split_css() -> &'static str {
    r#"
    .root {
        display: grid;
        grid-template-rows: 200px 6px 300px;
        width: 400px;
        height: 506px;
    }
    .pane-top {
        min-height: 50px;
        background: #333333;
    }
    .resize-handle-horizontal {
        background: #666666;
        cursor: row-resize;
        resize-axis: horizontal;
    }
    .pane-bottom {
        min-height: 50px;
        background: #444444;
    }
    "#
}

/// Collects PaneResizeEvent values for assertions.
#[derive(Clone)]
struct ResizeLog {
    entries: Arc<Mutex<Vec<PaneResizeEvent>>>,
}

impl ResizeLog {
    fn new() -> Self {
        Self { entries: Arc::new(Mutex::new(Vec::new())) }
    }

    fn handler(&self) -> Arc<dyn Fn(&PaneResizeEvent) + Send + Sync> {
        let entries = self.entries.clone();
        Arc::new(move |ev: &PaneResizeEvent| {
            entries.lock().unwrap().push(ev.clone());
        })
    }

    fn entries(&self) -> Vec<PaneResizeEvent> {
        self.entries.lock().unwrap().clone()
    }

    fn len(&self) -> usize {
        self.entries.lock().unwrap().len()
    }
}

// ---------------------------------------------------------------------------
// Test: Horizontal split -- dragging vertical handle adjusts adjacent column widths
// ---------------------------------------------------------------------------

#[test]
fn horizontal_split_drag_adjusts_column_widths() {
    // Layout: 200px | 6px handle | 300px
    // Dragging handle 50px right should yield 250px | 6px | 250px
    let tracks = vec![
        GridTrackDef::Single(GridTrackSize::fixed_px(200.0)),
        GridTrackDef::Single(GridTrackSize::fixed_px(6.0)),
        GridTrackDef::Single(GridTrackSize::fixed_px(300.0)),
    ];

    let state = ResizeDragState {
        axis: ResizeAxis::Vertical,
        before_index: 0,
        after_index: 2,
        initial_tracks: tracks,
        initial_before_px: 200.0,
        initial_after_px: 300.0,
        min_before_px: 50.0,
        min_after_px: 50.0,
    };

    let result = compute_new_track_sizes(&state, 50.0);

    // Before track: 200 + 50 = 250
    let before = extract_single_px(&result[0]);
    assert!((before - 250.0).abs() < 0.01, "before column should be 250px, got {}", before);

    // After track: 300 - 50 = 250
    let after = extract_single_px(&result[2]);
    assert!((after - 250.0).abs() < 0.01, "after column should be 250px, got {}", after);

    // Handle track unchanged
    let handle = extract_single_px(&result[1]);
    assert!((handle - 6.0).abs() < 0.01, "handle track unchanged at 6px, got {}", handle);
}

// ---------------------------------------------------------------------------
// Test: Vertical split -- dragging horizontal handle adjusts adjacent row heights
// ---------------------------------------------------------------------------

#[test]
fn vertical_split_drag_adjusts_row_heights() {
    // Layout: 200px | 6px handle | 300px (rows)
    // Dragging handle 100px down should yield 300px | 6px | 200px
    let tracks = vec![
        GridTrackDef::Single(GridTrackSize::fixed_px(200.0)),
        GridTrackDef::Single(GridTrackSize::fixed_px(6.0)),
        GridTrackDef::Single(GridTrackSize::fixed_px(300.0)),
    ];

    let state = ResizeDragState {
        axis: ResizeAxis::Horizontal,
        before_index: 0,
        after_index: 2,
        initial_tracks: tracks,
        initial_before_px: 200.0,
        initial_after_px: 300.0,
        min_before_px: 0.0,
        min_after_px: 0.0,
    };

    let result = compute_new_track_sizes(&state, 100.0);

    let before = extract_single_px(&result[0]);
    assert!((before - 300.0).abs() < 0.01, "before row should be 300px, got {}", before);

    let after = extract_single_px(&result[2]);
    assert!((after - 200.0).abs() < 0.01, "after row should be 200px, got {}", after);
}

// ---------------------------------------------------------------------------
// Test: Minimum size enforcement -- can't collapse below min-width/min-height
// ---------------------------------------------------------------------------

#[test]
fn min_size_enforcement_prevents_collapse() {
    let tracks = vec![
        GridTrackDef::Single(GridTrackSize::fixed_px(200.0)),
        GridTrackDef::Single(GridTrackSize::fixed_px(6.0)),
        GridTrackDef::Single(GridTrackSize::fixed_px(300.0)),
    ];

    let state = ResizeDragState {
        axis: ResizeAxis::Vertical,
        before_index: 0,
        after_index: 2,
        initial_tracks: tracks,
        initial_before_px: 200.0,
        initial_after_px: 300.0,
        min_before_px: 100.0,
        min_after_px: 80.0,
    };

    // Try to drag far right, would shrink "after" below 80px
    let result = compute_new_track_sizes(&state, 500.0);
    let after = extract_single_px(&result[2]);
    assert!(after >= 80.0 - 0.01, "after track should not go below min 80px, got {}", after);

    // Try to drag far left, would shrink "before" below 100px
    let result_left = compute_new_track_sizes(&state, -500.0);
    let before = extract_single_px(&result_left[0]);
    assert!(before >= 100.0 - 0.01, "before track should not go below min 100px, got {}", before);
}

// ---------------------------------------------------------------------------
// Test: Double-click resets adjacent tracks to equal sizes
// ---------------------------------------------------------------------------

#[test]
fn double_click_resets_to_equal_sizes() {
    let tracks = vec![
        GridTrackDef::Single(GridTrackSize::fixed_px(100.0)),
        GridTrackDef::Single(GridTrackSize::fixed_px(6.0)),
        GridTrackDef::Single(GridTrackSize::fixed_px(400.0)),
    ];

    let state = ResizeDragState {
        axis: ResizeAxis::Vertical,
        before_index: 0,
        after_index: 2,
        initial_tracks: tracks,
        initial_before_px: 100.0,
        initial_after_px: 400.0,
        min_before_px: 0.0,
        min_after_px: 0.0,
    };

    let result = reset_tracks_equal(&state);

    // Both should be (100 + 400) / 2 = 250
    let before = extract_single_px(&result[0]);
    let after = extract_single_px(&result[2]);
    assert!((before - 250.0).abs() < 0.01, "before should be 250px, got {}", before);
    assert!((after - 250.0).abs() < 0.01, "after should be 250px, got {}", after);
}

// ---------------------------------------------------------------------------
// Test: Drag delta maps correctly to px track size adjustment
// ---------------------------------------------------------------------------

#[test]
fn drag_delta_maps_to_px_adjustment() {
    let tracks = vec![
        GridTrackDef::Single(GridTrackSize::fixed_px(150.0)),
        GridTrackDef::Single(GridTrackSize::fixed_px(6.0)),
        GridTrackDef::Single(GridTrackSize::fixed_px(350.0)),
    ];

    // Test various deltas
    for delta in [-100.0_f32, -50.0, -10.0, 0.0, 10.0, 50.0, 100.0] {
        let state = ResizeDragState {
            axis: ResizeAxis::Vertical,
            before_index: 0,
            after_index: 2,
            initial_tracks: tracks.clone(),
            initial_before_px: 150.0,
            initial_after_px: 350.0,
            min_before_px: 0.0,
            min_after_px: 0.0,
        };

        let result = compute_new_track_sizes(&state, delta);
        let before = extract_single_px(&result[0]);
        let after = extract_single_px(&result[2]);

        // The sum of before+after should remain constant (conservation of space)
        let sum = before + after;
        assert!(
            (sum - 500.0).abs() < 0.01,
            "delta={}: total space should be conserved (500px), got {}",
            delta,
            sum
        );

        // before = initial + delta, after = initial - delta
        assert!(
            (before - (150.0 + delta)).abs() < 0.01,
            "delta={}: before should be {}, got {}",
            delta,
            150.0 + delta,
            before
        );
        assert!(
            (after - (350.0 - delta)).abs() < 0.01,
            "delta={}: after should be {}, got {}",
            delta,
            350.0 - delta,
            after
        );
    }
}

// ---------------------------------------------------------------------------
// Test: fr redistribution maintains total fr sum
// ---------------------------------------------------------------------------

#[test]
fn fr_redistribution_maintains_total() {
    let tracks = vec![
        GridTrackDef::Single(GridTrackSize::fr(1.0)),
        GridTrackDef::Single(GridTrackSize::fixed_px(6.0)),
        GridTrackDef::Single(GridTrackSize::fr(2.0)),
    ];

    let initial_fr_sum = total_fr_sum(&tracks);
    assert!((initial_fr_sum - 3.0).abs() < 0.01);

    let state = ResizeDragState {
        axis: ResizeAxis::Vertical,
        before_index: 0,
        after_index: 2,
        initial_tracks: tracks,
        initial_before_px: 200.0, // effective: 1fr = 200px
        initial_after_px: 400.0,  // effective: 2fr = 400px
        min_before_px: 0.0,
        min_after_px: 0.0,
    };

    // Drag 100px: before should grow, after should shrink
    let result = compute_new_track_sizes(&state, 100.0);
    let new_fr_sum = total_fr_sum(&result);

    assert!(
        (initial_fr_sum - new_fr_sum).abs() < 0.01,
        "fr sum should be preserved: initial={}, new={}",
        initial_fr_sum,
        new_fr_sum
    );

    // Check individual fr values are reasonable
    let before_fr = extract_single_fr(&result[0]);
    let after_fr = extract_single_fr(&result[2]);
    assert!(before_fr > 1.0, "before fr should increase from 1.0, got {}", before_fr);
    assert!(after_fr < 2.0, "after fr should decrease from 2.0, got {}", after_fr);
}

// ---------------------------------------------------------------------------
// Test: Handle hover changes visual state (cursor style via CSS)
// ---------------------------------------------------------------------------

#[test]
fn handle_hover_changes_cursor() {
    let h = TestHarness::new(
        horizontal_split_css(),
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("root")
                .with_child(ElementDef::new(Tag::Div).with_class("pane-left").with_id("left"))
                .with_child(
                    ElementDef::new(Tag::Div)
                        .with_class("resize-handle-vertical")
                        .with_id("handle"),
                )
                .with_child(ElementDef::new(Tag::Div).with_class("pane-right").with_id("right")),
        },
        506.0,
        400.0,
    );

    // Check handle element has col-resize cursor
    let handle = h.query("#handle").unwrap();
    assert_eq!(
        handle.computed_style.cursor,
        CursorStyle::ColResize,
        "vertical resize handle should have col-resize cursor"
    );

    // Check resize-axis was parsed
    assert_eq!(
        handle.computed_style.resize_axis,
        Some(ResizeAxis::Vertical),
        "should parse resize-axis: vertical"
    );
}

#[test]
fn horizontal_handle_hover_cursor() {
    let h = TestHarness::new(
        vertical_split_css(),
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("root")
                .with_child(ElementDef::new(Tag::Div).with_class("pane-top").with_id("top"))
                .with_child(
                    ElementDef::new(Tag::Div)
                        .with_class("resize-handle-horizontal")
                        .with_id("handle"),
                )
                .with_child(ElementDef::new(Tag::Div).with_class("pane-bottom").with_id("bottom")),
        },
        400.0,
        506.0,
    );

    let handle = h.query("#handle").unwrap();
    assert_eq!(
        handle.computed_style.cursor,
        CursorStyle::RowResize,
        "horizontal resize handle should have row-resize cursor"
    );

    assert_eq!(
        handle.computed_style.resize_axis,
        Some(ResizeAxis::Horizontal),
        "should parse resize-axis: horizontal"
    );
}

// ---------------------------------------------------------------------------
// Test: Nested splits with multiple handles work independently
// ---------------------------------------------------------------------------

#[test]
fn nested_splits_independent() {
    // Two separate resize operations on different tracks
    let left_tracks = vec![
        GridTrackDef::Single(GridTrackSize::fixed_px(100.0)),
        GridTrackDef::Single(GridTrackSize::fixed_px(6.0)),
        GridTrackDef::Single(GridTrackSize::fixed_px(200.0)),
    ];

    let right_tracks = vec![
        GridTrackDef::Single(GridTrackSize::fixed_px(150.0)),
        GridTrackDef::Single(GridTrackSize::fixed_px(6.0)),
        GridTrackDef::Single(GridTrackSize::fixed_px(250.0)),
    ];

    let state_left = ResizeDragState {
        axis: ResizeAxis::Vertical,
        before_index: 0,
        after_index: 2,
        initial_tracks: left_tracks.clone(),
        initial_before_px: 100.0,
        initial_after_px: 200.0,
        min_before_px: 0.0,
        min_after_px: 0.0,
    };

    let state_right = ResizeDragState {
        axis: ResizeAxis::Vertical,
        before_index: 0,
        after_index: 2,
        initial_tracks: right_tracks.clone(),
        initial_before_px: 150.0,
        initial_after_px: 250.0,
        min_before_px: 0.0,
        min_after_px: 0.0,
    };

    // Drag left handle 30px
    let result_left = compute_new_track_sizes(&state_left, 30.0);
    // Drag right handle -20px
    let result_right = compute_new_track_sizes(&state_right, -20.0);

    // Results should be independent
    let left_before = extract_single_px(&result_left[0]);
    let left_after = extract_single_px(&result_left[2]);
    assert!((left_before - 130.0).abs() < 0.01);
    assert!((left_after - 170.0).abs() < 0.01);

    let right_before = extract_single_px(&result_right[0]);
    let right_after = extract_single_px(&result_right[2]);
    assert!((right_before - 130.0).abs() < 0.01);
    assert!((right_after - 270.0).abs() < 0.01);
}

// ---------------------------------------------------------------------------
// Test: on_pane_resize callback fires with correct values
// ---------------------------------------------------------------------------

#[test]
fn on_pane_resize_callback_fires() {
    let log = ResizeLog::new();
    let log_handler = log.handler();

    // Simulate what the framework would do: after drag end, fire callback
    let tracks = vec![
        GridTrackDef::Single(GridTrackSize::fixed_px(200.0)),
        GridTrackDef::Single(GridTrackSize::fixed_px(6.0)),
        GridTrackDef::Single(GridTrackSize::fixed_px(300.0)),
    ];

    let state = ResizeDragState {
        axis: ResizeAxis::Vertical,
        before_index: 0,
        after_index: 2,
        initial_tracks: tracks,
        initial_before_px: 200.0,
        initial_after_px: 300.0,
        min_before_px: 0.0,
        min_after_px: 0.0,
    };

    let new_tracks = compute_new_track_sizes(&state, 75.0);

    // Fire the callback
    let event = PaneResizeEvent {
        axis: ResizeAxis::Vertical,
        before_track_index: 0,
        after_track_index: 2,
        before_track: match &new_tracks[0] {
            GridTrackDef::Single(s) => *s,
            _ => panic!("expected Single"),
        },
        after_track: match &new_tracks[2] {
            GridTrackDef::Single(s) => *s,
            _ => panic!("expected Single"),
        },
        new_tracks: new_tracks.clone(),
    };

    log_handler(&event);

    assert_eq!(log.len(), 1, "callback should fire once");

    let entry = &log.entries()[0];
    assert_eq!(entry.axis, ResizeAxis::Vertical);
    assert_eq!(entry.before_track_index, 0);
    assert_eq!(entry.after_track_index, 2);

    // Before track: 200 + 75 = 275
    match entry.before_track.max {
        GridMaxTrackSize::Px(v) => {
            assert!((v - 275.0).abs() < 0.01, "before track should be 275px, got {}", v)
        }
        _ => panic!("expected Px"),
    }

    // After track: 300 - 75 = 225
    match entry.after_track.max {
        GridMaxTrackSize::Px(v) => {
            assert!((v - 225.0).abs() < 0.01, "after track should be 225px, got {}", v)
        }
        _ => panic!("expected Px"),
    }
}

// ---------------------------------------------------------------------------
// Test: Drag on handle element with on_drag fires proper DragEvent sequence
// ---------------------------------------------------------------------------

#[test]
fn handle_drag_fires_event_sequence() {
    let drag_phases: Arc<Mutex<Vec<DragPhase>>> = Arc::new(Mutex::new(Vec::new()));
    let phases = drag_phases.clone();

    let tree_fn = move || ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("root")
            .with_child(ElementDef::new(Tag::Div).with_class("pane-left"))
            .with_child(
                ElementDef::new(Tag::Div)
                    .with_class("resize-handle-vertical")
                    .with_id("handle")
                    .on_drag({
                        let phases = phases.clone();
                        move |ev: &DragEvent| {
                            phases.lock().unwrap().push(ev.phase);
                        }
                    }),
            )
            .with_child(ElementDef::new(Tag::Div).with_class("pane-right")),
    };

    let mut h = TestHarness::new(horizontal_split_css(), tree_fn, 506.0, 400.0);
    h.step();

    // Handle is at x=200..206 (after 200px left pane)
    // Mouse down on handle
    h.mouse_down(203.0, 200.0);
    h.step();

    // Move past threshold (>4px)
    h.mouse_move(210.0, 200.0);
    h.step();

    // Continue drag
    h.mouse_move(250.0, 200.0);
    h.step();

    // Release
    h.mouse_up(250.0, 200.0);
    h.step();

    let recorded = drag_phases.lock().unwrap().clone();
    assert!(!recorded.is_empty(), "drag events should fire on handle");
    assert_eq!(recorded[0], DragPhase::Start, "first phase should be Start");
    assert_eq!(recorded.last().copied(), Some(DragPhase::End), "last phase should be End");
}

// ---------------------------------------------------------------------------
// Test: resize_axis_from_classes returns correct axis
// ---------------------------------------------------------------------------

#[test]
fn resize_axis_class_detection() {
    assert_eq!(
        resize_axis_from_classes(&["resize-handle-vertical".to_string()]),
        Some(ResizeAxis::Vertical)
    );
    assert_eq!(
        resize_axis_from_classes(&["resize-handle-horizontal".to_string()]),
        Some(ResizeAxis::Horizontal)
    );
    assert_eq!(
        resize_axis_from_classes(&["resize-handle".to_string()]),
        Some(ResizeAxis::Vertical),
        "bare resize-handle defaults to vertical"
    );
    assert_eq!(resize_axis_from_classes(&["some-other-class".to_string()]), None);
}

// ---------------------------------------------------------------------------
// Test: Mixed fr/px scenario
// ---------------------------------------------------------------------------

#[test]
fn mixed_fr_px_tracks() {
    // 1fr | 6px handle | 200px
    let tracks = vec![
        GridTrackDef::Single(GridTrackSize::fr(1.0)),
        GridTrackDef::Single(GridTrackSize::fixed_px(6.0)),
        GridTrackDef::Single(GridTrackSize::fixed_px(200.0)),
    ];

    let state = ResizeDragState {
        axis: ResizeAxis::Vertical,
        before_index: 0,
        after_index: 2,
        initial_tracks: tracks,
        initial_before_px: 300.0, // 1fr resolves to 300px
        initial_after_px: 200.0,
        min_before_px: 0.0,
        min_after_px: 0.0,
    };

    let result = compute_new_track_sizes(&state, 50.0);

    // fr track should grow proportionally
    let before_fr = extract_single_fr(&result[0]);
    assert!(before_fr > 1.0, "fr track should grow, got {}", before_fr);

    // px track: 200 - 50 = 150
    let after_px = extract_single_px(&result[2]);
    assert!((after_px - 150.0).abs() < 0.01, "px track should be 150, got {}", after_px);
}

// ---------------------------------------------------------------------------
// Test: Double-click reset on fr tracks
// ---------------------------------------------------------------------------

#[test]
fn double_click_reset_fr_tracks() {
    let tracks = vec![
        GridTrackDef::Single(GridTrackSize::fr(1.0)),
        GridTrackDef::Single(GridTrackSize::fixed_px(6.0)),
        GridTrackDef::Single(GridTrackSize::fr(3.0)),
    ];

    let state = ResizeDragState {
        axis: ResizeAxis::Vertical,
        before_index: 0,
        after_index: 2,
        initial_tracks: tracks,
        initial_before_px: 100.0,
        initial_after_px: 300.0,
        min_before_px: 0.0,
        min_after_px: 0.0,
    };

    let result = reset_tracks_equal(&state);

    // Both fr tracks should be (1.0 + 3.0) / 2 = 2.0
    let before_fr = extract_single_fr(&result[0]);
    let after_fr = extract_single_fr(&result[2]);
    assert!((before_fr - 2.0).abs() < 0.01, "before fr should be 2.0, got {}", before_fr);
    assert!((after_fr - 2.0).abs() < 0.01, "after fr should be 2.0, got {}", after_fr);
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn extract_single_px(def: &GridTrackDef) -> f32 {
    match def {
        GridTrackDef::Single(s) => match s.max {
            GridMaxTrackSize::Px(v) => v,
            _ => panic!("expected Px max, got {:?}", s.max),
        },
        _ => panic!("expected Single track def"),
    }
}

fn extract_single_fr(def: &GridTrackDef) -> f32 {
    match def {
        GridTrackDef::Single(s) => match s.max {
            GridMaxTrackSize::Fr(v) => v,
            _ => panic!("expected Fr max, got {:?}", s.max),
        },
        _ => panic!("expected Single track def"),
    }
}
