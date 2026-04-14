use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use unshit_core::element::*;
use unshit_test::TestHarness;

fn make_button_harness(counter: Arc<AtomicU32>) -> TestHarness {
    let css = ".root { width: 100%; height: 100%; } \
               .btn { width: 100px; height: 50px; }";
    let c = counter.clone();
    TestHarness::new(
        css,
        move || ElementTree {
            root: ElementDef::new(Tag::Div).with_class("root").with_child(
                ElementDef::new(Tag::Button).with_class("btn").on_click({
                    let c = c.clone();
                    move || {
                        c.fetch_add(1, Ordering::SeqCst);
                    }
                }),
            ),
        },
        800.0,
        600.0,
    )
}

#[test]
fn click_on_fires_handler() {
    let counter = Arc::new(AtomicU32::new(0));
    let mut h = make_button_harness(counter.clone());
    h.step();

    h.click_on(".btn");
    assert_eq!(counter.load(Ordering::SeqCst), 1);
}

#[test]
fn double_click_on_fires_handler_twice() {
    let counter = Arc::new(AtomicU32::new(0));
    let mut h = make_button_harness(counter.clone());
    h.step();

    h.double_click_on(".btn");
    assert_eq!(counter.load(Ordering::SeqCst), 2);
}

#[test]
fn right_click_on_fires_context_menu() {
    let counter = Arc::new(AtomicU32::new(0));
    let c = counter.clone();
    let css = ".root { width: 100%; height: 100%; } \
               .menu { width: 100px; height: 50px; }";

    let mut h = TestHarness::new(
        css,
        move || ElementTree {
            root: ElementDef::new(Tag::Div).with_class("root").with_child(
                ElementDef::new(Tag::Div).with_class("menu").on_context_menu({
                    let c = c.clone();
                    move |_x, _y| {
                        c.fetch_add(1, Ordering::SeqCst);
                    }
                }),
            ),
        },
        800.0,
        600.0,
    );
    h.step();

    h.right_click_on(".menu");
    assert_eq!(counter.load(Ordering::SeqCst), 1);
}

#[test]
fn fill_replaces_text_in_input() {
    let css = ".root { width: 100%; height: 100%; } \
               .inp { width: 200px; height: 30px; }";

    let mut h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("root")
                .with_child(ElementDef::new(Tag::Input).with_class("inp").with_text("old text")),
        },
        800.0,
        600.0,
    );
    h.step();

    h.fill(".inp", "new text");

    let snap = h.query(".inp").unwrap();
    assert_eq!(snap.input_value.as_deref(), Some("new text"));
}

#[test]
fn clear_empties_input() {
    let css = ".root { width: 100%; height: 100%; } \
               .inp { width: 200px; height: 30px; }";

    let mut h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("root")
                .with_child(ElementDef::new(Tag::Input).with_class("inp").with_text("some text")),
        },
        800.0,
        600.0,
    );
    h.step();

    // First fill it so the input state has a value
    h.fill(".inp", "hello");
    assert_eq!(h.query(".inp").unwrap().input_value.as_deref(), Some("hello"));

    h.clear(".inp");
    assert_eq!(h.query(".inp").unwrap().input_value.as_deref(), Some(""));
}

#[test]
fn hover_on_changes_hovered_element() {
    let css = ".root { width: 100%; height: 100%; } \
               .box { width: 100px; height: 100px; }";

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
    h.step();

    h.hover_on(".box");

    // After hover_on, the hovered element should have the "box" class
    let classes = h.hovered_classes();
    assert!(classes.contains(&"box".to_string()), "expected hovered to be .box, got {:?}", classes);
}

#[test]
fn select_option_on_changes_value() {
    let css = ".root { width: 100%; height: 100%; } \
               .sel { width: 200px; height: 30px; }";

    let mut h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div).with_class("root").with_child(
                ElementDef::new(Tag::Select).with_class("sel").with_options(vec![
                    ("a".into(), "Alpha".into()),
                    ("b".into(), "Beta".into()),
                    ("c".into(), "Charlie".into()),
                ]),
            ),
        },
        800.0,
        600.0,
    );
    h.step();

    let node_id = h.query(".sel").unwrap().node_id;
    assert_eq!(h.select_selected_value(node_id).as_deref(), Some("a"));

    h.select_option_on(".sel", "c");
    assert_eq!(h.select_selected_value(node_id).as_deref(), Some("c"));
}

#[test]
fn select_option_by_index_on_changes_selection() {
    let css = ".root { width: 100%; height: 100%; } \
               .sel { width: 200px; height: 30px; }";

    let mut h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div).with_class("root").with_child(
                ElementDef::new(Tag::Select).with_class("sel").with_options(vec![
                    ("x".into(), "X-ray".into()),
                    ("y".into(), "Yankee".into()),
                ]),
            ),
        },
        800.0,
        600.0,
    );
    h.step();

    h.select_option_by_index_on(".sel", 1);
    let node_id = h.query(".sel").unwrap().node_id;
    assert_eq!(h.select_selected_value(node_id).as_deref(), Some("y"));
}

#[test]
#[should_panic(expected = "no element matches selector")]
fn action_on_missing_element_panics() {
    let css = ".root { width: 100%; height: 100%; }";

    let mut h = TestHarness::new(
        css,
        || ElementTree { root: ElementDef::new(Tag::Div).with_class("root") },
        800.0,
        600.0,
    );
    h.step();

    h.click_on(".nonexistent");
}

#[test]
fn press_on_sends_enter_key() {
    let submitted = Arc::new(AtomicU32::new(0));
    let s = submitted.clone();

    let css = ".root { width: 100%; height: 100%; } \
               .inp { width: 200px; height: 30px; }";

    let mut h = TestHarness::new(
        css,
        move || ElementTree {
            root: ElementDef::new(Tag::Div).with_class("root").with_child(
                ElementDef::new(Tag::Input).with_class("inp").on_submit({
                    let s = s.clone();
                    move |_| {
                        s.fetch_add(1, Ordering::SeqCst);
                    }
                }),
            ),
        },
        800.0,
        600.0,
    );
    h.step();

    h.press_on(".inp", "Enter");
    assert_eq!(submitted.load(Ordering::SeqCst), 1);
}
