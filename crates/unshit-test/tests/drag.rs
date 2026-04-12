use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use unshit_core::element::*;
use unshit_core::event::{DragEvent, DragPhase};
use unshit_test::TestHarness;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn drag_css() -> &'static str {
    r#"
    .root { display: flex; flex-direction: column; width: 100%; height: 100%; }
    .handle {
        width: 100px;
        height: 50px;
        background: #444444;
    }
    .other {
        width: 100px;
        height: 50px;
        margin-top: 100px;
        background: #222222;
    }
    "#
}

/// Collects all DragEvent phases/values for later assertion.
#[derive(Clone, Debug)]
struct DragLog {
    entries: Arc<Mutex<Vec<DragSnapshot>>>,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
struct DragSnapshot {
    phase: DragPhase,
    x: f32,
    y: f32,
    delta_x: f32,
    delta_y: f32,
    total_delta_x: f32,
    total_delta_y: f32,
}

impl DragLog {
    fn new() -> Self {
        Self { entries: Arc::new(Mutex::new(Vec::new())) }
    }

    fn handler(&self) -> Arc<dyn Fn(&DragEvent) + Send + Sync> {
        let entries = self.entries.clone();
        Arc::new(move |ev: &DragEvent| {
            entries.lock().unwrap().push(DragSnapshot {
                phase: ev.phase,
                x: ev.x,
                y: ev.y,
                delta_x: ev.delta_x,
                delta_y: ev.delta_y,
                total_delta_x: ev.total_delta_x,
                total_delta_y: ev.total_delta_y,
            });
        })
    }

    fn entries(&self) -> Vec<DragSnapshot> {
        self.entries.lock().unwrap().clone()
    }

    fn len(&self) -> usize {
        self.entries.lock().unwrap().len()
    }

    fn phases(&self) -> Vec<DragPhase> {
        self.entries.lock().unwrap().iter().map(|e| e.phase).collect()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn move_below_threshold_fires_click_not_drag() {
    let click_count = Arc::new(AtomicU32::new(0));
    let drag_log = DragLog::new();

    let click_c = click_count.clone();
    let drag_h = drag_log.handler();

    let tree_fn = move || ElementTree {
        root: ElementDef::new(Tag::Div).with_class("root").with_child(
            ElementDef::new(Tag::Div)
                .with_class("handle")
                .on_click({
                    let c = click_c.clone();
                    move || {
                        c.fetch_add(1, Ordering::SeqCst);
                    }
                })
                .on_drag({
                    let h = drag_h.clone();
                    move |ev| h(ev)
                }),
        ),
    };

    let mut h = TestHarness::new(drag_css(), tree_fn, 800.0, 600.0);
    h.step();

    // Mouse down, move only 2px (below 4px threshold), then release
    h.mouse_down(50.0, 25.0);
    h.step();
    h.mouse_move(51.0, 26.0); // distance ~1.4px
    h.step();
    h.mouse_up(51.0, 26.0);
    h.step();

    // Click should have fired, drag should NOT
    assert_eq!(click_count.load(Ordering::SeqCst), 1, "click should fire when below threshold");
    assert_eq!(drag_log.len(), 0, "no drag events when below threshold");
}

#[test]
fn move_above_threshold_fires_drag_not_click() {
    let click_count = Arc::new(AtomicU32::new(0));
    let drag_log = DragLog::new();

    let click_c = click_count.clone();
    let drag_h = drag_log.handler();

    let tree_fn = move || ElementTree {
        root: ElementDef::new(Tag::Div).with_class("root").with_child(
            ElementDef::new(Tag::Div)
                .with_class("handle")
                .on_click({
                    let c = click_c.clone();
                    move || {
                        c.fetch_add(1, Ordering::SeqCst);
                    }
                })
                .on_drag({
                    let h = drag_h.clone();
                    move |ev| h(ev)
                }),
        ),
    };

    let mut h = TestHarness::new(drag_css(), tree_fn, 800.0, 600.0);
    h.step();

    // Mouse down, move 10px (above 4px threshold), then release
    h.mouse_down(50.0, 25.0);
    h.step();
    h.mouse_move(60.0, 25.0); // distance = 10px, triggers DragStart + DragUpdate
    h.step();
    h.mouse_up(60.0, 25.0);
    h.step();

    // Drag should have fired (Start, Update, End), click should NOT
    assert_eq!(click_count.load(Ordering::SeqCst), 0, "click must not fire when drag occurred");
    let phases = drag_log.phases();
    assert!(phases.contains(&DragPhase::Start), "DragStart should fire");
    assert!(phases.contains(&DragPhase::End), "DragEnd should fire");
}

#[test]
fn drag_lifecycle_start_update_end() {
    let drag_log = DragLog::new();
    let drag_h = drag_log.handler();

    let tree_fn = move || ElementTree {
        root: ElementDef::new(Tag::Div).with_class("root").with_child(
            ElementDef::new(Tag::Div).with_class("handle").on_drag({
                let h = drag_h.clone();
                move |ev| h(ev)
            }),
        ),
    };

    let mut h = TestHarness::new(drag_css(), tree_fn, 800.0, 600.0);
    h.step();

    // Mouse down at (50, 25)
    h.mouse_down(50.0, 25.0);
    h.step();

    // Move past threshold
    h.mouse_move(55.0, 25.0); // 5px, above threshold
    h.step();

    // Continue moving
    h.mouse_move(70.0, 30.0);
    h.step();

    h.mouse_move(80.0, 40.0);
    h.step();

    // Release
    h.mouse_up(80.0, 40.0);
    h.step();

    let entries = drag_log.entries();
    assert!(
        entries.len() >= 3,
        "should have at least Start, Update(s), End; got {}",
        entries.len()
    );

    // First event must be Start
    assert_eq!(entries[0].phase, DragPhase::Start);

    // Last event must be End
    assert_eq!(entries.last().unwrap().phase, DragPhase::End);

    // Middle events must be Update
    for entry in &entries[1..entries.len() - 1] {
        assert_eq!(entry.phase, DragPhase::Update);
    }
}

#[test]
fn drag_delta_values_are_correct() {
    let drag_log = DragLog::new();
    let drag_h = drag_log.handler();

    let tree_fn = move || ElementTree {
        root: ElementDef::new(Tag::Div).with_class("root").with_child(
            ElementDef::new(Tag::Div).with_class("handle").on_drag({
                let h = drag_h.clone();
                move |ev| h(ev)
            }),
        ),
    };

    let mut h = TestHarness::new(drag_css(), tree_fn, 800.0, 600.0);
    h.step();

    // Mouse down at (10, 20)
    h.mouse_down(10.0, 20.0);
    h.step();

    // Move to (20, 20) - triggers Start (delta from origin)
    h.mouse_move(20.0, 20.0);
    h.step();

    // Move to (30, 25) - Update
    h.mouse_move(30.0, 25.0);
    h.step();

    // Release at (30, 25)
    h.mouse_up(30.0, 25.0);
    h.step();

    let entries = drag_log.entries();

    // Start event: total_delta = (10, 0) from origin (10, 20)
    let start = &entries[0];
    assert_eq!(start.phase, DragPhase::Start);
    assert!(
        (start.total_delta_x - 10.0).abs() < 0.01,
        "start total_delta_x = {}",
        start.total_delta_x
    );
    assert!(
        (start.total_delta_y - 0.0).abs() < 0.01,
        "start total_delta_y = {}",
        start.total_delta_y
    );

    // Find the Update event with the move from (20,20) to (30,25)
    let updates: Vec<_> = entries.iter().filter(|e| e.phase == DragPhase::Update).collect();
    assert!(!updates.is_empty(), "should have at least one Update event");

    // The last update should be at (30, 25)
    let last_update = updates.last().unwrap();
    assert!((last_update.x - 30.0).abs() < 0.01);
    assert!((last_update.y - 25.0).abs() < 0.01);
    // Total delta from origin (10, 20) to (30, 25)
    assert!((last_update.total_delta_x - 20.0).abs() < 0.01);
    assert!((last_update.total_delta_y - 5.0).abs() < 0.01);

    // End event
    let end = entries.last().unwrap();
    assert_eq!(end.phase, DragPhase::End);
    assert!((end.total_delta_x - 20.0).abs() < 0.01, "end total_delta_x = {}", end.total_delta_x);
    assert!((end.total_delta_y - 5.0).abs() < 0.01, "end total_delta_y = {}", end.total_delta_y);
}

#[test]
fn drag_continues_outside_element_bounds_pointer_capture() {
    let drag_log = DragLog::new();
    let drag_h = drag_log.handler();

    let tree_fn = move || ElementTree {
        root: ElementDef::new(Tag::Div).with_class("root").with_child(
            // handle is 100x50 at (0, 0)
            ElementDef::new(Tag::Div).with_class("handle").on_drag({
                let h = drag_h.clone();
                move |ev| h(ev)
            }),
        ),
    };

    let mut h = TestHarness::new(drag_css(), tree_fn, 800.0, 600.0);
    h.step();

    // Mouse down inside the handle
    h.mouse_down(50.0, 25.0);
    h.step();

    // Move past threshold
    h.mouse_move(55.0, 25.0);
    h.step();

    // Move far outside the element bounds (pointer capture should keep sending events)
    h.mouse_move(500.0, 500.0);
    h.step();

    // Release outside the element
    h.mouse_up(500.0, 500.0);
    h.step();

    let entries = drag_log.entries();
    assert!(entries.len() >= 3, "drag events should fire even when pointer leaves element");

    // Verify we got events at the out-of-bounds position
    let end = entries.last().unwrap();
    assert_eq!(end.phase, DragPhase::End);
    assert!((end.x - 500.0).abs() < 0.01, "end x should be at release position");
    assert!((end.y - 500.0).abs() < 0.01, "end y should be at release position");
}

#[test]
fn no_drag_handler_means_no_drag_events() {
    let click_count = Arc::new(AtomicU32::new(0));
    let click_c = click_count.clone();

    // Element with only on_click, no on_drag
    let tree_fn = move || ElementTree {
        root: ElementDef::new(Tag::Div).with_class("root").with_child(
            ElementDef::new(Tag::Div).with_class("handle").on_click({
                let c = click_c.clone();
                move || {
                    c.fetch_add(1, Ordering::SeqCst);
                }
            }),
        ),
    };

    let mut h = TestHarness::new(drag_css(), tree_fn, 800.0, 600.0);
    h.step();

    // Move past drag threshold
    h.mouse_down(50.0, 25.0);
    h.step();
    h.mouse_move(60.0, 25.0);
    h.step();
    h.mouse_up(60.0, 25.0);
    h.step();

    // No drag handler, so this should still work as a normal click-miss-by-move.
    // The click dispatches when mousedown and mouseup are on the same element.
    // Since we moved, the hovered element might differ. With the handle at 100x50,
    // (60, 25) is still inside. So click should fire.
    assert_eq!(click_count.load(Ordering::SeqCst), 1, "click should fire when no drag handler");
}

#[test]
fn drag_bubbles_to_parent() {
    let drag_log = DragLog::new();
    let drag_h = drag_log.handler();

    let css = r#"
    .root { display: flex; flex-direction: column; width: 100%; height: 100%; }
    .parent { width: 200px; height: 200px; }
    .child { width: 100px; height: 100px; }
    "#;

    let tree_fn = move || ElementTree {
        root: ElementDef::new(Tag::Div).with_class("root").with_child(
            ElementDef::new(Tag::Div)
                .with_class("parent")
                .on_drag({
                    let h = drag_h.clone();
                    move |ev| h(ev)
                })
                .with_child(ElementDef::new(Tag::Div).with_class("child")),
        ),
    };

    let mut h = TestHarness::new(css, tree_fn, 800.0, 600.0);
    h.step();

    // Mousedown on child, drag past threshold
    h.mouse_down(50.0, 50.0);
    h.step();
    h.mouse_move(60.0, 50.0);
    h.step();
    h.mouse_up(60.0, 50.0);
    h.step();

    // The drag handler on the parent should have received events
    let phases = drag_log.phases();
    assert!(phases.contains(&DragPhase::Start), "parent handler should receive DragStart");
    assert!(phases.contains(&DragPhase::End), "parent handler should receive DragEnd");
}

#[test]
fn threshold_is_euclidean_not_axis_aligned() {
    let drag_log = DragLog::new();
    let drag_h = drag_log.handler();

    let tree_fn = move || ElementTree {
        root: ElementDef::new(Tag::Div).with_class("root").with_child(
            ElementDef::new(Tag::Div).with_class("handle").on_drag({
                let h = drag_h.clone();
                move |ev| h(ev)
            }),
        ),
    };

    let mut h = TestHarness::new(drag_css(), tree_fn, 800.0, 600.0);
    h.step();

    // Move 3px in X and 3px in Y: euclidean distance ~4.24px, above 4px threshold
    h.mouse_down(50.0, 25.0);
    h.step();
    h.mouse_move(53.0, 28.0); // sqrt(9 + 9) = 4.24
    h.step();
    h.mouse_up(53.0, 28.0);
    h.step();

    assert!(drag_log.len() > 0, "4.24px euclidean distance should exceed 4px threshold");

    // Reset: move only 2px in each axis: sqrt(4+4) = 2.83px, below threshold
    let drag_log2 = DragLog::new();
    let drag_h2 = drag_log2.handler();

    let tree_fn2 = move || ElementTree {
        root: ElementDef::new(Tag::Div).with_class("root").with_child(
            ElementDef::new(Tag::Div).with_class("handle").on_drag({
                let h = drag_h2.clone();
                move |ev| h(ev)
            }),
        ),
    };

    let mut h2 = TestHarness::new(drag_css(), tree_fn2, 800.0, 600.0);
    h2.step();

    h2.mouse_down(50.0, 25.0);
    h2.step();
    h2.mouse_move(52.0, 27.0); // sqrt(4+4) = 2.83
    h2.step();
    h2.mouse_up(52.0, 27.0);
    h2.step();

    assert_eq!(drag_log2.len(), 0, "2.83px euclidean distance should not exceed 4px threshold");
}
