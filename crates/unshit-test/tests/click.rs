use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use unshit_core::element::*;
use unshit_test::TestHarness;

#[test]
fn click_fires_handler() {
    let counter = Arc::new(AtomicU32::new(0));
    let counter_clone = counter.clone();

    let css = ".root { width: 100%; height: 100%; } .btn { width: 100px; height: 50px; background: #0000ff; }";

    let tree_fn = move || ElementTree {
        root: ElementDef::new(Tag::Div).with_class("root").with_child(
            ElementDef::new(Tag::Button).with_class("btn").on_click({
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

    h.click(50.0, 25.0);

    assert_eq!(counter.load(Ordering::SeqCst), 1);
}

#[test]
fn click_miss_does_not_fire() {
    let counter = Arc::new(AtomicU32::new(0));
    let counter_clone = counter.clone();

    let css = ".root { width: 100%; height: 100%; } .btn { width: 100px; height: 50px; }";

    let tree_fn = move || ElementTree {
        root: ElementDef::new(Tag::Div).with_class("root").with_child(
            ElementDef::new(Tag::Button).with_class("btn").on_click({
                let c = counter_clone.clone();
                move || {
                    c.fetch_add(1, Ordering::SeqCst);
                }
            }),
        ),
    };

    let mut h = TestHarness::new(css, tree_fn, 800.0, 600.0);
    h.step();

    // Click far away from the button
    h.click(500.0, 500.0);

    assert_eq!(counter.load(Ordering::SeqCst), 0);
}

#[test]
fn click_bubbles_to_parent() {
    let counter = Arc::new(AtomicU32::new(0));
    let counter_clone = counter.clone();

    let css = ".root { width: 100%; height: 100%; } .parent { width: 200px; height: 200px; } .child { width: 100px; height: 100px; }";

    let tree_fn = move || ElementTree {
        root: ElementDef::new(Tag::Div).with_class("root").with_child(
            ElementDef::new(Tag::Div)
                .with_class("parent")
                .on_click({
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

    // Click on the child, should bubble up to parent's handler
    h.click(50.0, 50.0);

    assert_eq!(counter.load(Ordering::SeqCst), 1);
}

#[test]
fn mousedown_different_element_no_click() {
    let counter = Arc::new(AtomicU32::new(0));
    let counter_clone = counter.clone();

    let css = ".root { width: 100%; height: 100%; } .btn { width: 100px; height: 50px; } .other { width: 100px; height: 50px; margin-top: 100px; }";

    let tree_fn = move || ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("root")
            .with_child(ElementDef::new(Tag::Button).with_class("btn").on_click({
                let c = counter_clone.clone();
                move || {
                    c.fetch_add(1, Ordering::SeqCst);
                }
            }))
            .with_child(ElementDef::new(Tag::Div).with_class("other")),
    };

    let mut h = TestHarness::new(css, tree_fn, 800.0, 600.0);
    h.step();

    // Mouse down on btn, mouse up on other element
    h.mouse_down(50.0, 25.0);
    h.step();
    h.mouse_up(50.0, 200.0);
    h.step();

    assert_eq!(counter.load(Ordering::SeqCst), 0);
}
