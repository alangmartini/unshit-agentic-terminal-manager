//! Layout integration tests for ::before and ::after pseudo elements.
//!
//! Verifies that synthetic pseudo nodes flow through the normal layout
//! pipeline (sync to taffy, compute layout, read back rects) and that text
//! content inside them is measured correctly, including attr() driven text.

use unshit_core::element::*;
use unshit_test::TestHarness;

fn css() -> &'static str {
    r#"
    .root {
        display: flex;
        flex-direction: row;
        width: 800px;
        height: 200px;
        gap: 0;
        padding: 0;
        margin: 0;
    }
    .card {
        display: flex;
        flex-direction: row;
        width: 400px;
        height: 100px;
        gap: 0;
        padding: 0;
        margin: 0;
    }
    .card::before {
        content: "<<";
        width: 40px;
        height: 100px;
        font-size: 16px;
        padding: 0;
        margin: 0;
    }
    .card::after {
        content: ">>";
        width: 40px;
        height: 100px;
        font-size: 16px;
        padding: 0;
        margin: 0;
    }
    .attr-card::before {
        content: attr(id);
        font-size: 16px;
        padding: 0;
        margin: 0;
    }
    span, div {
        padding: 0;
        margin: 0;
    }
    .user-span {
        width: 120px;
        height: 100px;
    }
    "#
}

fn flex_tree() -> ElementTree {
    ElementTree {
        root: ElementDef::new(Tag::Div).with_class("root").with_child(
            ElementDef::new(Tag::Div)
                .with_class("card")
                .with_child(ElementDef::new(Tag::Div).with_class("user-span").with_text("mid")),
        ),
    }
}

/// The ::before pseudo node must sit at x = 0 within the card and the user
/// child must begin after the pseudo node's width.
#[test]
fn test_pseudo_before_participates_in_flex_layout() {
    let mut h = TestHarness::new(css(), flex_tree, 800.0, 200.0);
    h.step();

    let card = h.query(".card").expect("card exists");
    let children = h.arena().children(card.node_id);
    // before, user-span, after
    assert_eq!(children.len(), 3);

    let before = h.query_node(children[0]).expect("before");
    let user = h.query_node(children[1]).expect("user span");

    assert!(before.layout_rect.x >= card.layout_rect.x - 0.5);
    assert!(
        before.layout_rect.width > 0.0,
        "pseudo before must have a nonzero width: {:?}",
        before.layout_rect
    );
    assert!(
        user.layout_rect.x >= before.layout_rect.x + before.layout_rect.width - 0.5,
        "user span must begin after the ::before width: before={:?}, user={:?}",
        before.layout_rect,
        user.layout_rect,
    );
}

/// The ::after pseudo node must sit at the right of the row, after the user
/// children.
#[test]
fn test_pseudo_after_last_child() {
    let mut h = TestHarness::new(css(), flex_tree, 800.0, 200.0);
    h.step();

    let card = h.query(".card").expect("card exists");
    let children = h.arena().children(card.node_id);
    let user = h.query_node(children[1]).unwrap();
    let after = h.query_node(children[2]).unwrap();

    assert!(
        after.layout_rect.x >= user.layout_rect.x + user.layout_rect.width - 0.5,
        "::after must begin after the user span: user={:?}, after={:?}",
        user.layout_rect,
        after.layout_rect,
    );
    assert!(after.layout_rect.width > 0.0, "::after must have nonzero width");
}

/// `content: attr(id)` must produce a text node whose measurement drives a
/// nonzero layout width.
#[test]
fn test_pseudo_attr_text_measures() {
    let tree = || ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("root")
            .with_child(ElementDef::new(Tag::Div).with_class("attr-card").with_id("hello-world")),
    };
    let mut h = TestHarness::new(css(), tree, 800.0, 200.0);
    h.step();

    let card = h.query(".attr-card").expect("attr-card exists");
    let children = h.arena().children(card.node_id);
    assert!(!children.is_empty(), "pseudo before must exist");
    let before = h.query_node(children[0]).unwrap();
    match before.content {
        ElementContent::Text(ref t) => assert_eq!(t, "hello-world"),
        _ => panic!("pseudo before must carry the attr text"),
    }
    assert!(
        before.layout_rect.width > 0.0,
        "attr text should measure to a nonzero width, got {:?}",
        before.layout_rect,
    );
}
