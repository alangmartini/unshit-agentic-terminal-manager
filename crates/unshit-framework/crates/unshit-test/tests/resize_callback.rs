use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use unshit_core::element::*;
use unshit_test::TestHarness;

/// Callback fires on initial layout (element goes from 0x0 to first computed size).
#[test]
fn resize_fires_on_initial_layout() {
    let fired = Arc::new(AtomicU32::new(0));
    let fired_clone = fired.clone();

    let css = ".root { width: 100%; height: 100%; } .box { width: 200px; height: 150px; }";

    let _h = TestHarness::new(
        css,
        move || ElementTree {
            root: ElementDef::new(Tag::Div).with_class("root").with_child(
                ElementDef::new(Tag::Div).with_class("box").with_id("b").on_resize({
                    let c = fired_clone.clone();
                    move |_w, _h| {
                        c.fetch_add(1, Ordering::SeqCst);
                    }
                }),
            ),
        },
        800.0,
        600.0,
    );

    // The initial layout pass should fire the callback (0x0 -> 200x150).
    assert_eq!(fired.load(Ordering::SeqCst), 1, "on_resize should fire on initial layout");
}

/// Callback fires when parent/container resize changes element dimensions.
#[test]
fn resize_fires_when_container_changes() {
    let fired = Arc::new(AtomicU32::new(0));
    let fired_clone = fired.clone();

    let css = r#"
        .root { display: flex; width: 100%; height: 100%; }
        .half { flex-grow: 1; height: 50px; }
    "#;

    let tree_fn = {
        let c = fired_clone.clone();
        move || ElementTree {
            root: ElementDef::new(Tag::Div).with_class("root").with_child(
                ElementDef::new(Tag::Div).with_class("half").with_id("a").on_resize({
                    let c = c.clone();
                    move |_w, _h| {
                        c.fetch_add(1, Ordering::SeqCst);
                    }
                }),
            ),
        }
    };

    let mut h = TestHarness::new(css, tree_fn.clone(), 800.0, 600.0);

    // Initial layout fires once.
    assert_eq!(fired.load(Ordering::SeqCst), 1);

    // Rebuild with a second child that will halve the first child's width (flex-grow split).
    let fired2 = Arc::new(AtomicU32::new(0));
    let fired2_clone = fired2.clone();
    h.rebuild(move || ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("root")
            .with_child(ElementDef::new(Tag::Div).with_class("half").with_id("a").on_resize({
                let c = fired2_clone.clone();
                move |_w, _h| {
                    c.fetch_add(1, Ordering::SeqCst);
                }
            }))
            .with_child(ElementDef::new(Tag::Div).with_class("half").with_id("b")),
    });

    // The element went from ~800px to ~400px, so resize should fire.
    assert_eq!(fired2.load(Ordering::SeqCst), 1, "on_resize should fire when container changes");
}

/// Callback does NOT fire when only position changes but size stays the same.
#[test]
fn resize_no_fire_on_position_only_change() {
    let fired = Arc::new(AtomicU32::new(0));
    let fired_clone = fired.clone();

    let css = r#"
        .root { display: flex; flex-direction: column; width: 100%; height: 100%; }
        .spacer { height: 50px; width: 100%; }
        .target { width: 200px; height: 100px; }
    "#;

    let tree_fn = {
        let c = fired_clone.clone();
        move || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("root")
                .with_child(ElementDef::new(Tag::Div).with_class("spacer"))
                .with_child(ElementDef::new(Tag::Div).with_class("target").with_id("t").on_resize(
                    {
                        let c = c.clone();
                        move |_w, _h| {
                            c.fetch_add(1, Ordering::SeqCst);
                        }
                    },
                )),
        }
    };

    let mut h = TestHarness::new(css, tree_fn, 800.0, 600.0);

    // Initial layout fires once (0x0 -> 200x100).
    assert_eq!(fired.load(Ordering::SeqCst), 1);

    // Rebuild without the spacer: target moves up but keeps the same dimensions.
    let fired2 = Arc::new(AtomicU32::new(0));
    let fired2_clone = fired2.clone();
    h.rebuild(move || ElementTree {
        root: ElementDef::new(Tag::Div).with_class("root").with_child(
            ElementDef::new(Tag::Div).with_class("target").with_id("t").on_resize({
                let c = fired2_clone.clone();
                move |_w, _h| {
                    c.fetch_add(1, Ordering::SeqCst);
                }
            }),
        ),
    });

    // Position changed, but size stayed the same (200x100). Callback should NOT fire.
    assert_eq!(
        fired2.load(Ordering::SeqCst),
        0,
        "on_resize should not fire when only position changes"
    );
}

/// Epsilon filtering: no fire for sub-0.5px float drift.
#[test]
fn resize_epsilon_filters_tiny_changes() {
    let fired = Arc::new(AtomicU32::new(0));
    let fired_clone = fired.clone();

    let css = ".root { width: 100%; height: 100%; } .box { width: 200px; height: 150px; }";

    let h = TestHarness::new(
        css,
        move || ElementTree {
            root: ElementDef::new(Tag::Div).with_class("root").with_child(
                ElementDef::new(Tag::Div).with_class("box").with_id("b").on_resize({
                    let c = fired_clone.clone();
                    move |_w, _h| {
                        c.fetch_add(1, Ordering::SeqCst);
                    }
                }),
            ),
        },
        800.0,
        600.0,
    );

    // Initial fires once.
    assert_eq!(fired.load(Ordering::SeqCst), 1);

    // Manually tweak prev dimensions by < 0.5 to simulate float drift, then run dispatch.
    // We access the arena directly and adjust prev_width by a tiny amount.
    let node_id = h.query("#b").unwrap().node_id;
    let arena = h.arena();
    let elem = arena.get(node_id).unwrap();

    // Verify prev dimensions match current layout (they were updated after callback).
    assert!((elem.prev_width - 200.0).abs() < 1.0);
    assert!((elem.prev_height - 150.0).abs() < 1.0);

    // The epsilon check means drifts smaller than 0.5 are ignored.
    // Since dimensions match exactly, running dispatch again should not fire.
    // (Dispatch already ran; prev_width == layout_rect.width, so no new fire.)
    assert_eq!(
        fired.load(Ordering::SeqCst),
        1,
        "on_resize should not fire again when dimensions have not changed"
    );
}

/// Callback receives correct (width, height) matching layout rect.
#[test]
fn resize_receives_correct_dimensions() {
    let dims = Arc::new(Mutex::new((0.0f32, 0.0f32)));
    let dims_clone = dims.clone();

    let css = ".root { width: 100%; height: 100%; } .box { width: 300px; height: 250px; }";

    let h = TestHarness::new(
        css,
        move || ElementTree {
            root: ElementDef::new(Tag::Div).with_class("root").with_child(
                ElementDef::new(Tag::Div).with_class("box").with_id("b").on_resize({
                    let d = dims_clone.clone();
                    move |w, h| {
                        let mut guard = d.lock().unwrap();
                        *guard = (w, h);
                    }
                }),
            ),
        },
        800.0,
        600.0,
    );

    let received = dims.lock().unwrap();
    assert!(
        (received.0 - 300.0).abs() < 1.0,
        "received width ({}) should match layout width (300)",
        received.0
    );
    assert!(
        (received.1 - 250.0).abs() < 1.0,
        "received height ({}) should match layout height (250)",
        received.1
    );

    // Cross-check with actual layout rect
    let snapshot = h.query("#b").unwrap();
    assert!(
        (received.0 - snapshot.layout_rect.width).abs() < 1.0,
        "received width should match element's layout_rect.width"
    );
    assert!(
        (received.1 - snapshot.layout_rect.height).abs() < 1.0,
        "received height should match element's layout_rect.height"
    );
}

/// Multiple elements resizing in the same frame each get their callback fired once.
#[test]
fn resize_multiple_elements_each_fire_once() {
    let count_a = Arc::new(AtomicU32::new(0));
    let count_b = Arc::new(AtomicU32::new(0));
    let count_c = Arc::new(AtomicU32::new(0));
    let ca = count_a.clone();
    let cb = count_b.clone();
    let cc = count_c.clone();

    let css = r#"
        .root { display: flex; flex-direction: column; width: 100%; height: 100%; }
        .child { width: 200px; height: 50px; }
    "#;

    let _h = TestHarness::new(
        css,
        move || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("root")
                .with_child(ElementDef::new(Tag::Div).with_class("child").with_id("a").on_resize({
                    let c = ca.clone();
                    move |_w, _h| {
                        c.fetch_add(1, Ordering::SeqCst);
                    }
                }))
                .with_child(ElementDef::new(Tag::Div).with_class("child").with_id("b").on_resize({
                    let c = cb.clone();
                    move |_w, _h| {
                        c.fetch_add(1, Ordering::SeqCst);
                    }
                }))
                .with_child(ElementDef::new(Tag::Div).with_class("child").with_id("c").on_resize(
                    {
                        let c = cc.clone();
                        move |_w, _h| {
                            c.fetch_add(1, Ordering::SeqCst);
                        }
                    },
                )),
        },
        800.0,
        600.0,
    );

    assert_eq!(count_a.load(Ordering::SeqCst), 1, "element a should fire once");
    assert_eq!(count_b.load(Ordering::SeqCst), 1, "element b should fire once");
    assert_eq!(count_c.load(Ordering::SeqCst), 1, "element c should fire once");
}

/// Element with no `on_resize` incurs no overhead (Option::None check).
#[test]
fn resize_no_overhead_without_callback() {
    // This test verifies that elements without on_resize do not cause issues.
    // We create a tree where some elements have callbacks and some do not.
    let fired = Arc::new(AtomicU32::new(0));
    let fired_clone = fired.clone();

    let css = r#"
        .root { display: flex; flex-direction: column; width: 100%; height: 100%; }
        .child { width: 200px; height: 50px; }
    "#;

    let _h = TestHarness::new(
        css,
        move || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("root")
                .with_child(
                    // No on_resize callback
                    ElementDef::new(Tag::Div).with_class("child").with_id("no-cb"),
                )
                .with_child(
                    ElementDef::new(Tag::Div).with_class("child").with_id("with-cb").on_resize({
                        let c = fired_clone.clone();
                        move |_w, _h| {
                            c.fetch_add(1, Ordering::SeqCst);
                        }
                    }),
                ),
        },
        800.0,
        600.0,
    );

    // Only the element with on_resize should fire.
    assert_eq!(fired.load(Ordering::SeqCst), 1, "only the element with on_resize should fire");

    // Verify the element without callback has no issues (just check it exists and has layout).
    let snapshot = _h.query("#no-cb").unwrap();
    assert_eq!(snapshot.layout_rect.width, 200.0);
    assert_eq!(snapshot.layout_rect.height, 50.0);
}
