use unshit_core::element::*;
use unshit_core::style::types::Background;
use unshit_test::TestHarness;

fn make_tree() -> ElementTree {
    ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("root")
            .with_child(ElementDef::new(Tag::Button).with_class("btn1").with_text("One"))
            .with_child(ElementDef::new(Tag::Button).with_class("btn2").with_text("Two")),
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
    .root { width: 100%; height: 100%; display: flex; flex-direction: column; }
    .btn1 { width: 100px; height: 40px; background: #ff0000; }
    .btn2 { width: 100px; height: 40px; background: #ff0000; }
    .btn1:focus-visible { background: #00ff00; }
    .btn2:focus-visible { background: #00ff00; }
"#;

#[test]
fn focus_visible_applies_on_tab() {
    let mut h = TestHarness::new(CSS, make_tree, 800.0, 600.0);
    h.step();

    // Before focus: btn1 should be red.
    let snap = h.query(".btn1").expect(".btn1 not found");
    assert_bg_color(&snap, 255, 0, 0, "btn1 before focus");

    // Tab to btn1 (keyboard focus).
    h.tab();
    h.step();

    // After Tab: :focus-visible should apply, btn1 should be green.
    let snap = h.query(".btn1").expect(".btn1 not found after tab");
    assert_bg_color(&snap, 0, 255, 0, "btn1 after tab (focus-visible)");
}

#[test]
fn focus_visible_not_applied_on_click() {
    let mut h = TestHarness::new(CSS, make_tree, 800.0, 600.0);
    h.step();

    // Click on btn1 (mouse focus).
    h.mouse_move(50.0, 20.0);
    h.click(50.0, 20.0);
    h.step();

    // After click: :focus-visible should NOT apply (mouse focus), btn1 stays red.
    let snap = h.query(".btn1").expect(".btn1 not found after click");
    assert_bg_color(&snap, 255, 0, 0, "btn1 after click (no focus-visible)");
}
