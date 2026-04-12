use unshit_core::element::*;
use unshit_core::style::types::Background;
use unshit_test::TestHarness;

fn make_box_tree() -> ElementTree {
    ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("root")
            .with_child(ElementDef::new(Tag::Div).with_class("box")),
    }
}

fn make_btn_tree() -> ElementTree {
    ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("root")
            .with_child(ElementDef::new(Tag::Div).with_class("btn")),
    }
}

fn assert_bg_color(snap: &unshit_test::ElementSnapshot, r: u8, g: u8, b: u8, msg: &str) {
    match &snap.computed_style.background {
        Background::Color(c) => {
            assert_eq!(c.r, r, "{msg}: expected r={r}, got r={}", c.r);
            assert_eq!(c.g, g, "{msg}: expected g={g}, got g={}", c.g);
            assert_eq!(c.b, b, "{msg}: expected b={b}, got b={}", c.b);
        }
        other => panic!("{msg}: expected Background::Color, got {other:?}"),
    }
}

#[test]
fn hover_applies_style() {
    let css = r#"
        .root { width: 100%; height: 100%; }
        .box { width: 100px; height: 100px; background: #ff0000; }
        .box:hover { background: #00ff00; }
    "#;

    let mut h = TestHarness::new(css, make_box_tree, 800.0, 600.0);
    h.step();

    // Before hover: background should be red
    let snap = h.query(".box").expect(".box not found");
    assert_bg_color(&snap, 255, 0, 0, "before hover");

    // Move mouse to center of .box
    h.mouse_move(50.0, 50.0);
    h.step();

    // After hover: background should be green
    let snap = h.query(".box").expect(".box not found after hover");
    assert_bg_color(&snap, 0, 255, 0, "after hover");
}

#[test]
fn hover_stable_across_frames() {
    let css = r#"
        .root { width: 100%; height: 100%; }
        .box { width: 100px; height: 100px; background: #ff0000; }
        .box:hover { background: #00ff00; }
    "#;

    let mut h = TestHarness::new(css, make_box_tree, 800.0, 600.0);
    h.step();

    h.mouse_move(50.0, 50.0);
    h.step();

    // Hover should remain stable for 5 steps (catches hover blink bug)
    h.assert_hover_stable(5);
}

#[test]
fn hover_leaves_on_move_away() {
    let css = r#"
        .root { width: 100%; height: 100%; }
        .box { width: 100px; height: 100px; background: #ff0000; }
        .box:hover { background: #00ff00; }
    "#;

    let mut h = TestHarness::new(css, make_box_tree, 800.0, 600.0);
    h.step();

    // Hover the box
    h.mouse_move(50.0, 50.0);
    h.step();
    let snap = h.query(".box").expect(".box not found during hover");
    assert_bg_color(&snap, 0, 255, 0, "during hover");

    // Move away to empty area
    h.mouse_move(500.0, 500.0);
    h.step();

    // Should revert to red
    let snap = h.query(".box").expect(".box not found after move away");
    assert_bg_color(&snap, 255, 0, 0, "after move away");
}

#[test]
fn active_on_mouse_down() {
    let css = r#"
        .root { width: 100%; height: 100%; }
        .btn { width: 100px; height: 50px; background: #0000ff; }
        .btn:active { background: #ff00ff; }
    "#;

    let mut h = TestHarness::new(css, make_btn_tree, 800.0, 600.0);
    h.step();

    // Before active: blue
    let snap = h.query(".btn").expect(".btn not found");
    assert_bg_color(&snap, 0, 0, 255, "before active");

    // Mouse down at center of .btn
    h.mouse_down(50.0, 25.0);
    h.step();

    // Should be magenta (:active)
    let snap = h.query(".btn").expect(".btn not found after mouse_down");
    assert_bg_color(&snap, 255, 0, 255, "during active");
}

#[test]
fn active_clears_on_mouse_up() {
    let css = r#"
        .root { width: 100%; height: 100%; }
        .btn { width: 100px; height: 50px; background: #0000ff; }
        .btn:active { background: #ff00ff; }
    "#;

    let mut h = TestHarness::new(css, make_btn_tree, 800.0, 600.0);
    h.step();

    // Activate
    h.mouse_down(50.0, 25.0);
    h.step();
    let snap = h.query(".btn").expect(".btn not found");
    assert_bg_color(&snap, 255, 0, 255, "during active");

    // Release
    h.mouse_up(50.0, 25.0);
    h.step();

    // Should go back to blue
    let snap = h.query(".btn").expect(".btn not found after mouse_up");
    assert_bg_color(&snap, 0, 0, 255, "after mouse_up");
}

#[test]
fn hover_and_active_together() {
    let css = r#"
        .root { width: 100%; height: 100%; }
        .btn { width: 100px; height: 50px; background: #0000ff; }
        .btn:hover { background: #00ff00; }
        .btn:active { background: #ff0000; }
    "#;

    let mut h = TestHarness::new(css, make_btn_tree, 800.0, 600.0);
    h.step();

    // Move to .btn -> hover green
    h.mouse_move(50.0, 25.0);
    h.step();
    let snap = h.query(".btn").expect(".btn not found");
    assert_bg_color(&snap, 0, 255, 0, "hover only");

    // Mouse down -> :active red (higher specificity due to source order, same specificity)
    h.mouse_down(50.0, 25.0);
    h.step();
    let snap = h.query(".btn").expect(".btn not found");
    assert_bg_color(&snap, 255, 0, 0, "hover + active");

    // Mouse up -> back to hover green (cursor still on .btn)
    h.mouse_up(50.0, 25.0);
    h.step();
    let snap = h.query(".btn").expect(".btn not found");
    assert_bg_color(&snap, 0, 255, 0, "hover after active release");
}
