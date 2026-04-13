use unshit_core::element::*;
use unshit_test::TestHarness;

#[test]
fn harness_builds_and_steps() {
    let css = r#"
        .root { display: flex; width: 100%; height: 100%; }
        .child { flex-grow: 1; }
    "#;
    let mut h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("root")
                .with_child(ElementDef::new(Tag::Div).with_class("child")),
        },
        800.0,
        600.0,
    );

    h.step();
    let root = h.query(".root").unwrap();
    assert!(root.layout_rect.width > 0.0);

    let child = h.query(".child").unwrap();
    assert!(child.layout_rect.width > 0.0);
}

#[test]
fn query_by_id() {
    let css = "#main { width: 200px; height: 100px; }";
    let h = TestHarness::new(
        css,
        || ElementTree { root: ElementDef::new(Tag::Div).with_id("main") },
        800.0,
        600.0,
    );

    let snap = h.query("#main").unwrap();
    assert_eq!(snap.id.as_deref(), Some("main"));
    assert!((snap.layout_rect.width - 200.0).abs() < 1.0);
}

#[test]
fn query_by_tag() {
    let css = "button { width: 80px; height: 40px; }";
    let h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_child(ElementDef::new(Tag::Button).with_class("btn")),
        },
        800.0,
        600.0,
    );

    let buttons = h.query_all("button");
    assert_eq!(buttons.len(), 1);
    assert!((buttons[0].layout_rect.width - 80.0).abs() < 1.0);
}

#[test]
fn mouse_move_updates_hover() {
    let css = r#"
        .root { display: flex; width: 100%; height: 100%; }
        .box { width: 100px; height: 100px; }
    "#;
    let mut h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("root")
                .with_child(ElementDef::new(Tag::Div).with_class("box")),
        },
        800.0,
        600.0,
    );

    // Move into the box area
    h.mouse_move(50.0, 50.0);
    h.step();

    let classes = h.hovered_classes();
    assert!(classes.contains(&"box".to_string()));
}

#[test]
fn click_sets_and_clears_active() {
    let css = r#"
        .root { display: flex; width: 100%; height: 100%; }
        .target { width: 100px; height: 100px; }
    "#;
    let mut h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("root")
                .with_child(ElementDef::new(Tag::Div).with_class("target")),
        },
        800.0,
        600.0,
    );

    // After click, active should be cleared (mouse_up clears it)
    h.click(50.0, 50.0);
    assert!(h.active().is_none());
}

#[test]
fn hover_stable_does_not_panic() {
    let css = ".root { display: flex; width: 100%; height: 100%; }";
    let mut h = TestHarness::new(
        css,
        || ElementTree { root: ElementDef::new(Tag::Div).with_class("root") },
        800.0,
        600.0,
    );

    h.mouse_move(50.0, 50.0);
    h.step();
    h.assert_hover_stable(5);
}
