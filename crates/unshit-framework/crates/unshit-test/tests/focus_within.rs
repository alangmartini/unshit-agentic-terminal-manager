use unshit_core::element::*;
use unshit_core::style::types::Background;
use unshit_test::TestHarness;

fn make_tree() -> ElementTree {
    ElementTree {
        root: ElementDef::new(Tag::Div).with_class("root").with_child(
            ElementDef::new(Tag::Div)
                .with_class("container")
                .with_child(ElementDef::new(Tag::Button).with_class("btn").with_text("Click")),
        ),
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

const CSS: &str = r#"
    .root { width: 100%; height: 100%; }
    .container { width: 200px; height: 100px; background: #ff0000; }
    .container:focus-within { background: #00ff00; }
    .btn { width: 100px; height: 40px; }
"#;

#[test]
fn focus_within_applies_on_child_focus() {
    let mut h = TestHarness::new(CSS, make_tree, 800.0, 600.0);
    h.step();

    // Before focus: container should be red.
    let snap = h.query(".container").expect(".container not found");
    assert_bg_color(&snap, 255, 0, 0, "container before focus");

    // Click on the button to focus it. The button is inside the container.
    h.mouse_move(50.0, 20.0);
    h.click(50.0, 20.0);
    h.step();

    // After focusing the child button, :focus-within on the container should
    // make it green.
    let snap = h.query(".container").expect(".container not found after click");
    assert_bg_color(&snap, 0, 255, 0, "container after child focus");
}

#[test]
fn focus_within_removed_on_blur() {
    let mut h = TestHarness::new(CSS, make_tree, 800.0, 600.0);
    h.step();

    // Focus the button.
    h.mouse_move(50.0, 20.0);
    h.click(50.0, 20.0);
    h.step();
    let snap = h.query(".container").expect(".container not found");
    assert_bg_color(&snap, 0, 255, 0, "container while button focused");

    // Click outside to blur.
    h.mouse_move(500.0, 500.0);
    h.click(500.0, 500.0);
    h.step();

    // Container should revert to red.
    let snap = h.query(".container").expect(".container not found after blur");
    assert_bg_color(&snap, 255, 0, 0, "container after blur");
}
