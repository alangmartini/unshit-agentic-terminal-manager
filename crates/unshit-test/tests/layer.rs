use unshit_core::element::*;
use unshit_core::event::hit_test;
use unshit_core::style::types::{Layer, RenderTarget};
use unshit_test::TestHarness;

/// CSS `layer: modal` is correctly parsed and stored in ComputedStyle.
#[test]
fn layer_property_parsed() {
    let css = r#"
        .root { width: 400px; height: 400px; }
        .modal { layer: modal; width: 100px; height: 100px; }
    "#;
    let tree_fn = || ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("root")
            .with_child(ElementDef::new(Tag::Div).with_class("modal")),
    };
    let mut h = TestHarness::new(css, tree_fn, 400.0, 400.0);
    h.step();
    let modal = h.query(".modal").expect("modal should exist");
    assert_eq!(modal.computed_style.layer, Layer::Modal);
}

/// Elements without a layer property default to Layer::Content.
#[test]
fn layer_defaults_to_content() {
    let css = r#"
        .root { width: 400px; height: 400px; }
        .plain { width: 100px; height: 100px; }
    "#;
    let tree_fn = || ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("root")
            .with_child(ElementDef::new(Tag::Div).with_class("plain")),
    };
    let mut h = TestHarness::new(css, tree_fn, 400.0, 400.0);
    h.step();
    let plain = h.query(".plain").expect("plain should exist");
    assert_eq!(plain.computed_style.layer, Layer::Content);
}

/// An element on Layer::Modal wins hit test over Layer::Content
/// even if the Content element is later in DOM order.
#[test]
fn layered_hit_test_higher_layer_wins() {
    let css = r#"
        .root { width: 400px; height: 400px; }
        .modal-bg { layer: modal; width: 400px; height: 400px; }
        .content-on-top { width: 400px; height: 400px; }
    "#;
    let tree_fn = || ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("root")
            .with_child(ElementDef::new(Tag::Div).with_class("modal-bg"))
            .with_child(ElementDef::new(Tag::Div).with_class("content-on-top")),
    };
    let mut h = TestHarness::new(css, tree_fn, 400.0, 400.0);
    h.step();

    // The modal element is first in DOM but on a higher layer.
    // The content-on-top element is later in DOM but on Content layer.
    // hit_test should return the modal element since Modal > Content.
    let hit = hit_test(h.arena(), h.root(), 50.0, 50.0);
    let hit_id = hit.expect("hit_test should find something");
    let hit_snap = h.query_node(hit_id).expect("hit element should exist");
    assert!(
        hit_snap.classes.contains(&"modal-bg".to_string()),
        "hit should be 'modal-bg' (higher layer), got classes: {:?}",
        hit_snap.classes,
    );
}

/// Elements on Layer::Tooltip are skipped by hit_test.
#[test]
fn tooltip_layer_not_interactive() {
    let css = r#"
        .root { width: 400px; height: 400px; }
        .tooltip { layer: tooltip; width: 400px; height: 400px; }
    "#;
    let tree_fn = || ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("root")
            .with_child(ElementDef::new(Tag::Div).with_class("tooltip")),
    };
    let mut h = TestHarness::new(css, tree_fn, 400.0, 400.0);
    h.step();

    // The tooltip covers the root, but tooltip layer is non-interactive.
    // hit_test should skip it and return the root.
    let hit = hit_test(h.arena(), h.root(), 50.0, 50.0);
    let hit_id = hit.expect("hit_test should find something");
    let hit_snap = h.query_node(hit_id).expect("hit element should exist");
    assert!(
        hit_snap.classes.contains(&"root".to_string()),
        "hit should pass through tooltip layer to root, got classes: {:?}",
        hit_snap.classes,
    );
}

/// CSS `render-target: overlay` sets RenderTarget::Portal(Layer::Overlay).
#[test]
fn render_target_portal_parsed() {
    let css = r#"
        .root { width: 400px; height: 400px; }
        .portal { render-target: overlay; width: 100px; height: 100px; }
    "#;
    let tree_fn = || ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("root")
            .with_child(ElementDef::new(Tag::Div).with_class("portal")),
    };
    let mut h = TestHarness::new(css, tree_fn, 400.0, 400.0);
    h.step();
    let portal = h.query(".portal").expect("portal should exist");
    assert_eq!(portal.computed_style.render_target, RenderTarget::Portal(Layer::Overlay),);
}
