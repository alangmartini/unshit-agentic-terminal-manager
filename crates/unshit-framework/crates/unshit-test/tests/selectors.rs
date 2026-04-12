use unshit_core::element::*;
use unshit_core::style::types::Background;
use unshit_test::TestHarness;

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

fn make_three_children() -> ElementTree {
    ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("root")
            .with_child(ElementDef::new(Tag::Div).with_class("item").with_class("first"))
            .with_child(ElementDef::new(Tag::Div).with_class("item").with_class("middle"))
            .with_child(ElementDef::new(Tag::Div).with_class("item").with_class("last")),
    }
}

// ---- :first-child ----

#[test]
fn first_child_matches_first_sibling() {
    let css = r#"
        .root { display: flex; width: 100%; height: 100%; }
        .item { width: 50px; height: 50px; background: #ff0000; }
        .item:first-child { background: #00ff00; }
    "#;

    let h = TestHarness::new(css, make_three_children, 800.0, 600.0);

    let first = h.query(".first").expect(".first not found");
    assert_bg_color(&first, 0, 255, 0, "first-child should be green");

    let middle = h.query(".middle").expect(".middle not found");
    assert_bg_color(&middle, 255, 0, 0, "middle should stay red");

    let last = h.query(".last").expect(".last not found");
    assert_bg_color(&last, 255, 0, 0, "last should stay red");
}

// ---- :last-child ----

#[test]
fn last_child_matches_last_sibling() {
    let css = r#"
        .root { display: flex; width: 100%; height: 100%; }
        .item { width: 50px; height: 50px; background: #ff0000; }
        .item:last-child { background: #0000ff; }
    "#;

    let h = TestHarness::new(css, make_three_children, 800.0, 600.0);

    let first = h.query(".first").expect(".first not found");
    assert_bg_color(&first, 255, 0, 0, "first should stay red");

    let middle = h.query(".middle").expect(".middle not found");
    assert_bg_color(&middle, 255, 0, 0, "middle should stay red");

    let last = h.query(".last").expect(".last not found");
    assert_bg_color(&last, 0, 0, 255, "last-child should be blue");
}

#[test]
fn first_and_last_child_on_only_child() {
    let css = r#"
        .root { display: flex; width: 100%; height: 100%; }
        .item { width: 50px; height: 50px; background: #ff0000; }
        .item:first-child { background: #00ff00; }
        .item:last-child { background: #0000ff; }
    "#;

    let h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("root")
                .with_child(ElementDef::new(Tag::Div).with_class("item")),
        },
        800.0,
        600.0,
    );

    // Both :first-child and :last-child match. :last-child appears later in source order
    // so it wins (same specificity).
    let item = h.query(".item").expect(".item not found");
    assert_bg_color(&item, 0, 0, 255, "only child should get last-child (later source order)");
}

// ---- :nth-child(n) ----

#[test]
fn nth_child_matches_correct_position() {
    let css = r#"
        .root { display: flex; width: 100%; height: 100%; }
        .item { width: 50px; height: 50px; background: #ff0000; }
        .item:nth-child(2) { background: #00ff00; }
    "#;

    let h = TestHarness::new(css, make_three_children, 800.0, 600.0);

    let first = h.query(".first").expect(".first not found");
    assert_bg_color(&first, 255, 0, 0, "1st child should stay red");

    let middle = h.query(".middle").expect(".middle not found");
    assert_bg_color(&middle, 0, 255, 0, "2nd child (nth-child(2)) should be green");

    let last = h.query(".last").expect(".last not found");
    assert_bg_color(&last, 255, 0, 0, "3rd child should stay red");
}

#[test]
fn nth_child_first_and_third() {
    let css = r#"
        .root { display: flex; width: 100%; height: 100%; }
        .item { width: 50px; height: 50px; background: #ff0000; }
        .item:nth-child(1) { background: #00ff00; }
        .item:nth-child(3) { background: #0000ff; }
    "#;

    let h = TestHarness::new(css, make_three_children, 800.0, 600.0);

    let first = h.query(".first").expect(".first not found");
    assert_bg_color(&first, 0, 255, 0, "nth-child(1) should be green");

    let middle = h.query(".middle").expect(".middle not found");
    assert_bg_color(&middle, 255, 0, 0, "2nd child should stay red");

    let last = h.query(".last").expect(".last not found");
    assert_bg_color(&last, 0, 0, 255, "nth-child(3) should be blue");
}

// ---- :not() ----

#[test]
fn not_class_excludes_matching_elements() {
    let css = r#"
        .root { display: flex; width: 100%; height: 100%; }
        .item { width: 50px; height: 50px; background: #ff0000; }
        .item:not(.middle) { background: #00ff00; }
    "#;

    let h = TestHarness::new(css, make_three_children, 800.0, 600.0);

    let first = h.query(".first").expect(".first not found");
    assert_bg_color(&first, 0, 255, 0, "first (not .middle) should be green");

    let middle = h.query(".middle").expect(".middle not found");
    assert_bg_color(&middle, 255, 0, 0, "middle should stay red (excluded by :not)");

    let last = h.query(".last").expect(".last not found");
    assert_bg_color(&last, 0, 255, 0, "last (not .middle) should be green");
}

#[test]
fn not_tag_excludes_matching_tag() {
    let css = r#"
        .root { display: flex; width: 100%; height: 100%; }
        div { width: 50px; height: 50px; background: #ff0000; }
        div:not(button) { background: #00ff00; }
    "#;

    let h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("root")
                .with_child(ElementDef::new(Tag::Div).with_class("child"))
                .with_child(ElementDef::new(Tag::Button).with_class("btn")),
        },
        800.0,
        600.0,
    );

    // div:not(button) should match divs but not buttons
    let child = h.query(".child").expect(".child not found");
    assert_bg_color(&child, 0, 255, 0, "div:not(button) should match div");
}

// ---- Combined selectors ----

#[test]
fn first_child_combined_with_class() {
    let css = r#"
        .root { display: flex; width: 100%; height: 100%; }
        .item { width: 50px; height: 50px; background: #ff0000; }
        .first:first-child { background: #00ff00; }
    "#;

    let h = TestHarness::new(css, make_three_children, 800.0, 600.0);

    let first = h.query(".first").expect(".first not found");
    assert_bg_color(&first, 0, 255, 0, ".first:first-child should be green");

    let middle = h.query(".middle").expect(".middle not found");
    assert_bg_color(&middle, 255, 0, 0, ".middle should stay red");
}
