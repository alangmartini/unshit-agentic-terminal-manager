use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use unshit_core::element::*;
use unshit_core::event::word_boundary_at;
use unshit_test::TestHarness;

// ---------------------------------------------------------------------------
// word_boundary_at unit tests
// ---------------------------------------------------------------------------

#[test]
fn word_boundary_simple_word() {
    let text = "Hello, world!";
    // Clicking inside "Hello" (byte 2)
    let (start, end) = word_boundary_at(text, 2);
    assert_eq!(&text[start..end], "Hello");
}

#[test]
fn word_boundary_second_word() {
    let text = "Hello, world!";
    // Clicking inside "world" (byte 7)
    let (start, end) = word_boundary_at(text, 7);
    assert_eq!(&text[start..end], "world");
}

#[test]
fn word_boundary_on_punctuation() {
    let text = "Hello, world!";
    // Clicking on the comma (byte 5)
    let (start, end) = word_boundary_at(text, 5);
    assert_eq!(&text[start..end], ",");
}

#[test]
fn word_boundary_on_space() {
    let text = "Hello, world!";
    // Clicking on the space after comma (byte 6)
    let (start, end) = word_boundary_at(text, 6);
    assert_eq!(&text[start..end], " ");
}

#[test]
fn word_boundary_at_start() {
    let text = "Hello world";
    let (start, end) = word_boundary_at(text, 0);
    assert_eq!(&text[start..end], "Hello");
}

#[test]
fn word_boundary_at_end() {
    let text = "Hello world";
    let (start, end) = word_boundary_at(text, text.len());
    assert_eq!(&text[start..end], "world");
}

#[test]
fn word_boundary_empty_text() {
    let (start, end) = word_boundary_at("", 0);
    assert_eq!(start, 0);
    assert_eq!(end, 0);
}

#[test]
fn word_boundary_underscore_counts_as_word() {
    let text = "foo_bar baz";
    let (start, end) = word_boundary_at(text, 4);
    assert_eq!(&text[start..end], "foo_bar");
}

// ---------------------------------------------------------------------------
// Double-click: word selection
// ---------------------------------------------------------------------------

fn make_text_tree() -> ElementTree {
    ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("root")
            .with_child(ElementDef::new(Tag::Span).with_class("label").with_text("Hello, world!")),
    }
}

#[test]
fn double_click_selects_word() {
    let css = r#"
        .root { width: 100%; height: 100%; }
        .label { font-size: 16px; line-height: 1.2; padding: 4px; }
    "#;

    let mut h = TestHarness::new(css, make_text_tree, 800.0, 600.0);
    h.step();

    // Double-click on the text area (should select a word)
    h.double_click(20.0, 10.0);

    let sel = h.text_selection();
    assert!(sel.is_some(), "Double-click on text should create a selection");
    let sel = sel.unwrap();
    assert!(!sel.is_collapsed(), "Double-click should create a non-collapsed (word) selection");

    let (start, end) = sel.ordered_range();
    assert!(end > start, "Word selection should span at least one character");
}

#[test]
fn single_click_does_not_select_word() {
    let css = r#"
        .root { width: 100%; height: 100%; }
        .label { font-size: 16px; line-height: 1.2; padding: 4px; }
    "#;

    let mut h = TestHarness::new(css, make_text_tree, 800.0, 600.0);
    h.step();

    // Single click should produce a collapsed selection (caret), not a word
    h.click(20.0, 10.0);

    let sel = h.text_selection();
    assert!(sel.is_some(), "Click on text should create a selection");
    let sel = sel.unwrap();
    assert!(
        sel.is_collapsed(),
        "Single click should create a collapsed selection, not word selection"
    );
}

// ---------------------------------------------------------------------------
// Right-click: on_context_menu handler
// ---------------------------------------------------------------------------

#[test]
fn right_click_fires_context_menu_handler() {
    let counter = Arc::new(AtomicU32::new(0));
    let counter_clone = counter.clone();

    let css = ".root { width: 100%; height: 100%; } .btn { width: 100px; height: 50px; }";

    let tree_fn = move || ElementTree {
        root: ElementDef::new(Tag::Div).with_class("root").with_child(
            ElementDef::new(Tag::Button).with_class("btn").on_context_menu({
                let c = counter_clone.clone();
                move || {
                    c.fetch_add(1, Ordering::SeqCst);
                }
            }),
        ),
    };

    let mut h = TestHarness::new(css, tree_fn, 800.0, 600.0);
    h.step();

    assert_eq!(counter.load(Ordering::SeqCst), 0);

    h.right_click(50.0, 25.0);

    assert_eq!(counter.load(Ordering::SeqCst), 1);
}

#[test]
fn right_click_miss_does_not_fire() {
    let counter = Arc::new(AtomicU32::new(0));
    let counter_clone = counter.clone();

    let css = ".root { width: 100%; height: 100%; } .btn { width: 100px; height: 50px; }";

    let tree_fn = move || ElementTree {
        root: ElementDef::new(Tag::Div).with_class("root").with_child(
            ElementDef::new(Tag::Button).with_class("btn").on_context_menu({
                let c = counter_clone.clone();
                move || {
                    c.fetch_add(1, Ordering::SeqCst);
                }
            }),
        ),
    };

    let mut h = TestHarness::new(css, tree_fn, 800.0, 600.0);
    h.step();

    // Right-click far away from the button
    h.right_click(500.0, 500.0);

    assert_eq!(counter.load(Ordering::SeqCst), 0);
}

#[test]
fn right_click_bubbles_to_parent() {
    let counter = Arc::new(AtomicU32::new(0));
    let counter_clone = counter.clone();

    let css = ".root { width: 100%; height: 100%; } .parent { width: 200px; height: 200px; } .child { width: 100px; height: 100px; }";

    let tree_fn = move || ElementTree {
        root: ElementDef::new(Tag::Div).with_class("root").with_child(
            ElementDef::new(Tag::Div)
                .with_class("parent")
                .on_context_menu({
                    let c = counter_clone.clone();
                    move || {
                        c.fetch_add(1, Ordering::SeqCst);
                    }
                })
                .with_child(ElementDef::new(Tag::Div).with_class("child")),
        ),
    };

    let mut h = TestHarness::new(css, tree_fn, 800.0, 600.0);
    h.step();

    // Right-click on the child; should bubble up to parent's handler
    h.right_click(50.0, 50.0);

    assert_eq!(counter.load(Ordering::SeqCst), 1);
}

#[test]
fn right_click_does_not_fire_on_click() {
    let click_counter = Arc::new(AtomicU32::new(0));
    let ctx_counter = Arc::new(AtomicU32::new(0));
    let click_clone = click_counter.clone();
    let ctx_clone = ctx_counter.clone();

    let css = ".root { width: 100%; height: 100%; } .btn { width: 100px; height: 50px; }";

    let tree_fn = move || ElementTree {
        root: ElementDef::new(Tag::Div).with_class("root").with_child(
            ElementDef::new(Tag::Button)
                .with_class("btn")
                .on_click({
                    let c = click_clone.clone();
                    move || {
                        c.fetch_add(1, Ordering::SeqCst);
                    }
                })
                .on_context_menu({
                    let c = ctx_clone.clone();
                    move || {
                        c.fetch_add(1, Ordering::SeqCst);
                    }
                }),
        ),
    };

    let mut h = TestHarness::new(css, tree_fn, 800.0, 600.0);
    h.step();

    // Right-click should only fire context menu, not on_click
    h.right_click(50.0, 25.0);
    assert_eq!(click_counter.load(Ordering::SeqCst), 0, "on_click should not fire on right-click");
    assert_eq!(ctx_counter.load(Ordering::SeqCst), 1, "on_context_menu should fire on right-click");

    // Left-click should only fire on_click, not context menu
    h.click(50.0, 25.0);
    assert_eq!(click_counter.load(Ordering::SeqCst), 1, "on_click should fire on left-click");
    assert_eq!(
        ctx_counter.load(Ordering::SeqCst),
        1,
        "on_context_menu should not fire on left-click"
    );
}
