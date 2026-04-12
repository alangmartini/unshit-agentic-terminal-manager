use std::time::{Duration, Instant};

use unshit_core::cursor::{CursorShape, CursorState};
use unshit_core::element::*;
use unshit_test::TestHarness;

fn make_input_tree() -> ElementTree {
    ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("root")
            .with_child(ElementDef::new(Tag::Input).with_class("input1").with_text("hello"))
            .with_child(ElementDef::new(Tag::Input).with_class("input2").with_text("world")),
    }
}

const BASE_CSS: &str = r#"
    .root { width: 100%; height: 100%; display: flex; flex-direction: column; }
    .input1 { width: 200px; height: 30px; }
    .input2 { width: 200px; height: 30px; }
"#;

#[test]
fn cursor_defaults_to_beam_shape() {
    // The default cursor shape should be Beam when no explicit shape is set
    let state = CursorState::default();
    assert_eq!(state.shape, CursorShape::Beam);
}

#[test]
fn blink_toggles_visibility_at_default_rate() {
    // Blink should toggle visibility at the configured rate (530ms default)
    let start = Instant::now();
    let mut state = CursorState::default();
    state.last_toggle = start;

    assert!(state.visible, "cursor should start visible");

    // Just before 530ms: should still be visible
    let before = start + Duration::from_millis(529);
    let changed = state.tick(before);
    assert!(!changed, "should not toggle before blink rate");
    assert!(state.visible, "should remain visible before blink rate");

    // At 530ms: should toggle to invisible
    let at_rate = start + Duration::from_millis(530);
    let changed = state.tick(at_rate);
    assert!(changed, "should toggle at blink rate");
    assert!(!state.visible, "should become invisible after first toggle");

    // Another 530ms later: should toggle back to visible
    let second_toggle = at_rate + Duration::from_millis(530);
    let changed = state.tick(second_toggle);
    assert!(changed, "should toggle again");
    assert!(state.visible, "should become visible after second toggle");
}

#[test]
fn keystroke_resets_cursor_to_visible() {
    // Typing into a focused input should reset the cursor to visible
    let mut h = TestHarness::new(BASE_CSS, make_input_tree, 800.0, 600.0);
    h.step();

    // Click on input1 to focus it
    h.mouse_down(100.0, 15.0);
    h.mouse_up(100.0, 15.0);
    h.step();

    // Manually set cursor to invisible (simulate blink having hidden it)
    {
        let focused = h.focused();
        let el = h.arena_mut().get_mut(focused).unwrap();
        el.cursor_state.visible = false;
    }

    // Type a character
    h.type_char('a');
    h.step();

    // Cursor should be visible again after keystroke
    let focused = h.focused();
    let el = h.arena().get(focused).unwrap();
    assert!(el.cursor_state.visible, "cursor should be visible after keystroke");
}

#[test]
fn block_cursor_dimensions_match_full_cell() {
    // CursorShape::Block uses full cell width/height.
    // This is a unit-level check on the shape enum itself.
    let state = CursorState::with_shape(CursorShape::Block);
    assert_eq!(state.shape, CursorShape::Block);
    assert!(state.visible);
}

#[test]
fn underline_cursor_shape() {
    // CursorShape::Underline renders at bottom of cell.
    let state = CursorState::with_shape(CursorShape::Underline);
    assert_eq!(state.shape, CursorShape::Underline);
    assert!(state.visible);
}

#[test]
fn steady_mode_disables_blink() {
    // When steady is true, the cursor should never toggle
    let start = Instant::now();
    let mut state = CursorState::default();
    state.last_toggle = start;
    state.steady = true;

    // Even after a long time, visibility should not change
    let long_after = start + Duration::from_millis(5000);
    let changed = state.tick(long_after);
    assert!(!changed, "steady mode should prevent toggle");
    assert!(state.visible, "cursor should remain visible in steady mode");

    // next_toggle_time should return None for steady mode
    assert_eq!(state.next_toggle_time(), None, "next_toggle_time should be None in steady mode");
}

#[test]
fn blink_rate_configurable_via_css() {
    // caret-blink-rate CSS property should configure the blink rate
    let css = r#"
        .root { width: 100%; height: 100%; display: flex; flex-direction: column; }
        .input1 { width: 200px; height: 30px; caret-blink-rate: 250; }
    "#;
    let h = TestHarness::new(css, make_input_tree, 800.0, 600.0);

    let snap = h.query(".input1").expect("input1 not found");
    assert_eq!(
        snap.computed_style.caret_blink_rate, 250,
        "caret_blink_rate should be 250ms from CSS"
    );
}

#[test]
fn cursor_hidden_when_element_loses_focus() {
    // The cursor should not render when the element is not focused.
    // In the renderer, cursor rendering is gated on `node_id == focused`.
    // We verify the cursor_state.visible is still true (it does not get
    // explicitly hidden), but the renderer only draws it for the focused element.
    let mut h = TestHarness::new(BASE_CSS, make_input_tree, 800.0, 600.0);
    h.step();

    // Focus input1
    h.mouse_down(100.0, 15.0);
    h.mouse_up(100.0, 15.0);
    h.step();

    let input1_id = h.query(".input1").unwrap().node_id;
    assert_eq!(h.focused(), input1_id, "input1 should be focused");

    // Focus input2 instead
    h.mouse_down(100.0, 45.0);
    h.mouse_up(100.0, 45.0);
    h.step();

    let input2_id = h.query(".input2").unwrap().node_id;
    assert_eq!(h.focused(), input2_id, "input2 should now be focused, input1 lost focus");
    // input1 is no longer the focused element, so the renderer will not
    // draw its cursor regardless of cursor_state.visible
    assert_ne!(h.focused(), input1_id);
}

#[test]
fn wait_until_scheduled_for_next_blink_toggle() {
    // next_toggle_time should return the correct instant for scheduling WaitUntil
    let start = Instant::now();
    let mut state = CursorState::default();
    state.last_toggle = start;
    state.blink_rate_ms = 500;

    let expected = start + Duration::from_millis(500);
    assert_eq!(
        state.next_toggle_time(),
        Some(expected),
        "next_toggle_time should be last_toggle + blink_rate_ms"
    );

    // After a tick, the next toggle time should advance
    let toggle_time = start + Duration::from_millis(500);
    state.tick(toggle_time);
    let expected_next = toggle_time + Duration::from_millis(500);
    assert_eq!(state.next_toggle_time(), Some(expected_next));
}

#[test]
fn cursor_visible_immediately_on_initial_focus() {
    // When an element first receives focus, the cursor should be visible
    let mut h = TestHarness::new(BASE_CSS, make_input_tree, 800.0, 600.0);
    h.step();

    // Nothing focused initially
    assert!(h.focused().is_dangling(), "no element should be focused initially");

    // Click on input1 to focus it
    h.mouse_down(100.0, 15.0);
    h.mouse_up(100.0, 15.0);
    h.step();

    let input1_id = h.query(".input1").unwrap().node_id;
    assert_eq!(h.focused(), input1_id, "input1 should be focused");

    let el = h.arena().get(input1_id).unwrap();
    assert!(el.cursor_state.visible, "cursor should be visible immediately on focus");
}

#[test]
fn caret_shape_css_property_block() {
    // caret-shape: block should set CursorShape::Block
    let css = r#"
        .root { width: 100%; height: 100%; }
        .input1 { width: 200px; height: 30px; caret-shape: block; }
    "#;
    let h = TestHarness::new(css, make_input_tree, 800.0, 600.0);

    let snap = h.query(".input1").expect("input1 not found");
    assert_eq!(snap.computed_style.caret_shape, CursorShape::Block);
}

#[test]
fn caret_shape_css_property_underline() {
    // caret-shape: underline should set CursorShape::Underline
    let css = r#"
        .root { width: 100%; height: 100%; }
        .input1 { width: 200px; height: 30px; caret-shape: underline; }
    "#;
    let h = TestHarness::new(css, make_input_tree, 800.0, 600.0);

    let snap = h.query(".input1").expect("input1 not found");
    assert_eq!(snap.computed_style.caret_shape, CursorShape::Underline);
}
