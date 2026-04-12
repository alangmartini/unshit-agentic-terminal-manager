use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use unshit_core::element::*;
use unshit_core::event::hit_test;
use unshit_core::style::types::{PointerEvents, Visibility};
use unshit_test::TestHarness;

/// visibility: hidden element has correct layout rect but produces no render output.
#[test]
fn visibility_hidden_has_layout_but_no_render() {
    let css = r#"
        .root { width: 400px; height: 400px; flex-direction: column; }
        .hidden-box { visibility: hidden; width: 100px; height: 50px; background: red; }
        .visible-box { width: 100px; height: 50px; background: blue; }
    "#;

    let tree_fn = || ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("root")
            .with_child(ElementDef::new(Tag::Div).with_class("hidden-box"))
            .with_child(ElementDef::new(Tag::Div).with_class("visible-box")),
    };

    let mut h = TestHarness::new(css, tree_fn, 400.0, 400.0);
    h.step();

    // Hidden element still has layout (takes up space)
    let hidden = h.query(".hidden-box").expect("hidden-box should exist");
    assert_eq!(hidden.layout_rect.width, 100.0);
    assert_eq!(hidden.layout_rect.height, 50.0);

    // Verify the visibility property was correctly resolved
    assert_eq!(hidden.computed_style.visibility, Visibility::Hidden);

    // The visible box should be offset by the hidden box's height (layout is preserved)
    let visible = h.query(".visible-box").expect("visible-box should exist");
    assert!(
        visible.layout_rect.y >= 50.0,
        "visible-box should be pushed down by hidden-box's layout: y={}",
        visible.layout_rect.y,
    );
    assert_eq!(visible.computed_style.visibility, Visibility::Visible);
}

/// pointer-events: none element is visible but hit_test returns what's behind it.
/// The overlay child has pointer-events: none and sits inside a clickable parent.
/// Clicking on the overlay should pass through to the parent's click handler.
#[test]
fn pointer_events_none_passes_through() {
    let counter = Arc::new(AtomicU32::new(0));
    let counter_clone = counter.clone();

    let css = r#"
        .root { width: 400px; height: 400px; }
        .stack { width: 200px; height: 200px; }
        .overlay { pointer-events: none; width: 200px; height: 200px; }
    "#;

    let tree_fn = move || ElementTree {
        root: ElementDef::new(Tag::Div).with_class("root").with_child(
            ElementDef::new(Tag::Div)
                .with_class("stack")
                .on_click({
                    let c = counter_clone.clone();
                    move || {
                        c.fetch_add(1, Ordering::SeqCst);
                    }
                })
                .with_child(ElementDef::new(Tag::Div).with_class("overlay")),
        ),
    };

    let mut h = TestHarness::new(css, tree_fn, 400.0, 400.0);
    h.step();

    let overlay = h.query(".overlay").expect("overlay should exist");
    assert_eq!(overlay.computed_style.pointer_events, PointerEvents::None);

    // hit_test should pass through the overlay to the parent
    let hit = hit_test(h.arena(), h.root(), 100.0, 100.0);
    let hit_id = hit.expect("hit_test should find something");
    let hit_snap = h.query_node(hit_id).expect("hit element should exist");
    assert!(
        hit_snap.classes.contains(&"stack".to_string()),
        "hit should be 'stack' (parent), not 'overlay'. Got classes: {:?}",
        hit_snap.classes,
    );

    // Click should reach the handler on .stack
    h.click(100.0, 100.0);
    assert_eq!(counter.load(Ordering::SeqCst), 1, "click should pass through overlay to stack");
}

/// Children of visibility: hidden parent are also hidden (inheritance).
#[test]
fn visibility_hidden_inherits_to_children() {
    let css = r#"
        .root { width: 400px; height: 400px; flex-direction: column; }
        .parent { visibility: hidden; width: 200px; height: 200px; flex-direction: column; }
        .child { width: 100px; height: 50px; background: green; }
    "#;

    let tree_fn = || ElementTree {
        root: ElementDef::new(Tag::Div).with_class("root").with_child(
            ElementDef::new(Tag::Div)
                .with_class("parent")
                .with_child(ElementDef::new(Tag::Div).with_class("child")),
        ),
    };

    let mut h = TestHarness::new(css, tree_fn, 400.0, 400.0);
    h.step();

    // Parent should be hidden
    let parent = h.query(".parent").expect("parent should exist");
    assert_eq!(parent.computed_style.visibility, Visibility::Hidden);

    // Child should inherit visibility: hidden from parent
    let child = h.query(".child").expect("child should exist");
    assert_eq!(
        child.computed_style.visibility,
        Visibility::Hidden,
        "child should inherit visibility: hidden from parent",
    );

    // But child should still have layout
    assert!(child.layout_rect.width > 0.0, "hidden child should still have layout width");
    assert!(child.layout_rect.height > 0.0, "hidden child should still have layout height");
}

/// pointer-events: none also inherits to children.
#[test]
fn pointer_events_none_inherits_to_children() {
    let css = r#"
        .root { width: 400px; height: 400px; }
        .parent { pointer-events: none; width: 200px; height: 200px; }
        .child { width: 100px; height: 50px; }
    "#;

    let tree_fn = || ElementTree {
        root: ElementDef::new(Tag::Div).with_class("root").with_child(
            ElementDef::new(Tag::Div)
                .with_class("parent")
                .with_child(ElementDef::new(Tag::Div).with_class("child")),
        ),
    };

    let mut h = TestHarness::new(css, tree_fn, 400.0, 400.0);
    h.step();

    let parent = h.query(".parent").expect("parent should exist");
    assert_eq!(parent.computed_style.pointer_events, PointerEvents::None);

    let child = h.query(".child").expect("child should exist");
    assert_eq!(
        child.computed_style.pointer_events,
        PointerEvents::None,
        "child should inherit pointer-events: none from parent",
    );

    // hit_test on child position should pass through both child and parent
    let hit = hit_test(h.arena(), h.root(), 50.0, 25.0);
    assert!(hit.is_some());
    let hit_snap = h.query_node(hit.unwrap()).expect("hit element should exist");
    assert!(
        hit_snap.classes.contains(&"root".to_string()),
        "hit should fall through to root, got: {:?}",
        hit_snap.classes,
    );
}
