use unshit_core::element::*;
use unshit_core::style::types::Background;
use unshit_test::TestHarness;

fn make_focus_tree() -> ElementTree {
    ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("root")
            .with_child(ElementDef::new(Tag::Button).with_class("btn1").with_text("One"))
            .with_child(ElementDef::new(Tag::Div).with_class("spacer"))
            .with_child(ElementDef::new(Tag::Button).with_class("btn2").with_text("Two"))
            .with_child(ElementDef::new(Tag::Div).with_class("plain"))
            .with_child(ElementDef::new(Tag::Button).with_class("btn3").with_text("Three")),
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

const FOCUS_CSS: &str = r#"
    .root { width: 100%; height: 100%; display: flex; flex-direction: column; }
    .btn1 { width: 100px; height: 40px; background: #0000ff; }
    .btn2 { width: 100px; height: 40px; background: #0000ff; }
    .btn3 { width: 100px; height: 40px; background: #0000ff; }
    .btn1:focus { background: #ff0000; }
    .btn2:focus { background: #ff0000; }
    .btn3:focus { background: #ff0000; }
    .spacer { width: 100px; height: 20px; }
    .plain { width: 100px; height: 20px; }
"#;

#[test]
fn click_button_applies_focus_style() {
    // Clicking on a button should set :focus and apply the focused style.
    let mut h = TestHarness::new(FOCUS_CSS, make_focus_tree, 800.0, 600.0);
    h.step();

    // Before focus: btn1 should be blue
    let snap = h.query(".btn1").expect(".btn1 not found");
    assert_bg_color(&snap, 0, 0, 255, "btn1 before focus");

    // Click on btn1 (center of a 100x40 element at top)
    h.mouse_down(50.0, 20.0);
    h.step();

    // After click: btn1 should be red (:focus)
    let snap = h.query(".btn1").expect(".btn1 not found after click");
    assert_bg_color(&snap, 255, 0, 0, "btn1 after focus");
}

#[test]
fn tab_moves_focus_to_next_focusable() {
    // Pressing Tab should move focus to the next focusable element (Button/Input).
    let mut h = TestHarness::new(FOCUS_CSS, make_focus_tree, 800.0, 600.0);
    h.step();

    // Focus the first button by clicking
    h.mouse_down(50.0, 20.0);
    h.step();

    let btn1 = h.query(".btn1").expect(".btn1 not found");
    assert_eq!(h.focused(), btn1.node_id, "btn1 should be focused after click");

    // Tab to next
    h.tab();
    h.step();

    let btn2 = h.query(".btn2").expect(".btn2 not found");
    assert_eq!(h.focused(), btn2.node_id, "btn2 should be focused after tab");
    assert_bg_color(&btn2, 255, 0, 0, "btn2 should have :focus style");

    // btn1 should no longer have focus style
    let btn1 = h.query(".btn1").expect(".btn1 not found after tab");
    assert_bg_color(&btn1, 0, 0, 255, "btn1 should lose :focus style");
}

#[test]
fn shift_tab_moves_focus_backward() {
    // Pressing Shift+Tab should move focus to the previous focusable element.
    let mut h = TestHarness::new(FOCUS_CSS, make_focus_tree, 800.0, 600.0);
    h.step();

    // Focus btn2 first via two tabs
    h.tab(); // -> btn1
    h.step();
    h.tab(); // -> btn2
    h.step();

    let btn2 = h.query(".btn2").expect(".btn2 not found");
    assert_eq!(h.focused(), btn2.node_id, "btn2 should be focused");

    // Shift+Tab back to btn1
    h.shift_tab();
    h.step();

    let btn1 = h.query(".btn1").expect(".btn1 not found");
    assert_eq!(h.focused(), btn1.node_id, "btn1 should be focused after shift+tab");
    assert_bg_color(&btn1, 255, 0, 0, "btn1 should have :focus style");
}

#[test]
fn focus_disappears_when_other_element_focused() {
    // When a different element gains focus, the previous one loses :focus style.
    let mut h = TestHarness::new(FOCUS_CSS, make_focus_tree, 800.0, 600.0);
    h.step();

    // Focus btn1
    h.tab();
    h.step();

    let btn1 = h.query(".btn1").expect(".btn1 not found");
    assert_bg_color(&btn1, 255, 0, 0, "btn1 with focus");

    // Tab to btn2
    h.tab();
    h.step();

    // btn1 should lose focus style, btn2 should gain it
    let btn1 = h.query(".btn1").expect(".btn1 not found");
    assert_bg_color(&btn1, 0, 0, 255, "btn1 after losing focus");

    let btn2 = h.query(".btn2").expect(".btn2 not found");
    assert_bg_color(&btn2, 255, 0, 0, "btn2 with focus");
}

#[test]
fn tab_skips_non_focusable_elements() {
    // Plain divs (non-button, non-input, no tab_index) should be skipped.
    let mut h = TestHarness::new(FOCUS_CSS, make_focus_tree, 800.0, 600.0);
    h.step();

    // Tab from nothing -> btn1
    h.tab();
    h.step();
    let btn1 = h.query(".btn1").expect(".btn1 not found");
    assert_eq!(h.focused(), btn1.node_id);

    // Tab -> btn2 (skips .spacer)
    h.tab();
    h.step();
    let btn2 = h.query(".btn2").expect(".btn2 not found");
    assert_eq!(h.focused(), btn2.node_id);

    // Tab -> btn3 (skips .plain)
    h.tab();
    h.step();
    let btn3 = h.query(".btn3").expect(".btn3 not found");
    assert_eq!(h.focused(), btn3.node_id);

    // Tab wraps around -> btn1
    h.tab();
    h.step();
    assert_eq!(h.focused(), btn1.node_id, "focus should wrap around to btn1");
}

#[test]
fn tab_index_makes_div_focusable() {
    // A plain div with tab_index should be focusable via Tab.
    let tree_fn = || ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("root")
            .with_child(ElementDef::new(Tag::Button).with_class("btn1"))
            .with_child(ElementDef::new(Tag::Div).with_class("focusable-div").with_tab_index(0))
            .with_child(ElementDef::new(Tag::Button).with_class("btn2")),
    };

    let css = r#"
        .root { width: 100%; height: 100%; display: flex; flex-direction: column; }
        .btn1, .btn2 { width: 100px; height: 40px; }
        .focusable-div { width: 100px; height: 40px; }
    "#;

    let mut h = TestHarness::new(css, tree_fn, 800.0, 600.0);
    h.step();

    // Tab -> btn1
    h.tab();
    h.step();
    let btn1 = h.query(".btn1").expect(".btn1 not found");
    assert_eq!(h.focused(), btn1.node_id);

    // Tab -> focusable-div (has tab_index)
    h.tab();
    h.step();
    let div = h.query(".focusable-div").expect(".focusable-div not found");
    assert_eq!(h.focused(), div.node_id, "div with tab_index should be focusable");

    // Tab -> btn2
    h.tab();
    h.step();
    let btn2 = h.query(".btn2").expect(".btn2 not found");
    assert_eq!(h.focused(), btn2.node_id);
}

#[test]
fn direct_focus_sets_node() {
    // The focus() test helper should directly set the focused element.
    let mut h = TestHarness::new(FOCUS_CSS, make_focus_tree, 800.0, 600.0);
    h.step();

    let btn2 = h.query(".btn2").expect(".btn2 not found");
    h.focus(btn2.node_id);
    h.step();

    assert_eq!(h.focused(), btn2.node_id);
    let snap = h.query(".btn2").expect(".btn2 not found");
    assert_bg_color(&snap, 255, 0, 0, "btn2 after direct focus");
}
