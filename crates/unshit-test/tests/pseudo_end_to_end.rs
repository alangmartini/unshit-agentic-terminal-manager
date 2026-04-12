//! End to end tests for ::before and ::after that exercise the full
//! harness pipeline (style resolve, pseudo resolver, layout sync).

use unshit_core::element::*;
use unshit_test::TestHarness;

fn hover_css() -> &'static str {
    r#"
    .root {
        display: flex;
        flex-direction: row;
        width: 800px;
        height: 200px;
    }
    .card {
        display: flex;
        flex-direction: row;
        width: 200px;
        height: 100px;
    }
    .card:hover::before {
        content: "->";
        width: 30px;
        height: 100px;
    }
    "#
}

fn hover_tree() -> ElementTree {
    ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("root")
            .with_child(ElementDef::new(Tag::Div).with_class("card")),
    }
}

/// A `.card:hover::before` rule should only produce a synthetic child while
/// the host is hovered, and the synthetic node must be cleanly torn down
/// when the hover is released.
#[test]
fn test_pseudo_before_with_hover() {
    let mut h = TestHarness::new(hover_css(), hover_tree, 800.0, 200.0);
    h.step();

    let card = h.query(".card").expect("card exists");
    let card_id = card.node_id;

    // Initial frame has no hover, so ::before must not be allocated.
    let children = h.arena().children(card_id);
    assert_eq!(children.len(), 0, "no pseudo child should exist before hover: {:?}", children);

    // Move mouse into the card, triggering :hover::before.
    let rect = card.layout_rect;
    h.mouse_move(rect.x + rect.width * 0.5, rect.y + rect.height * 0.5);
    h.step();

    let children_hover = h.arena().children(card_id);
    assert_eq!(
        children_hover.len(),
        1,
        "::before should be allocated while hovered: {:?}",
        children_hover
    );
    let before_id = children_hover[0];
    let before = h.arena().get(before_id).expect("before element");
    assert!(before.synthetic, "pseudo child must be flagged synthetic");
    assert_eq!(before.content, ElementContent::Text("->".into()));

    // Move mouse off the card. The card starts at the root origin so we
    // move well beyond its width to be safely outside the hit region.
    h.mouse_move(700.0, 150.0);
    h.step();

    let children_release = h.arena().children(card_id);
    assert_eq!(
        children_release.len(),
        0,
        "::before should be removed when hover releases: {:?}",
        children_release
    );

    // The old pseudo node must be gone from the arena entirely, not just
    // unlinked, so no leak accrues across hover cycles.
    assert!(h.arena().get(before_id).is_none(), "stale pseudo node must be deallocated");
}

/// Specificity chain with `a:hover::before`: the rule must match when the
/// host (tag `a`, class `card`) is hovered and not otherwise.
#[test]
fn test_pseudo_specificity_with_hover_chain() {
    let css = r#"
    .root { display: flex; width: 800px; height: 200px; }
    button { display: flex; width: 200px; height: 100px; }
    button::before { content: "x"; }
    button:hover::before { content: "y"; }
    "#;

    let tree = || ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("root")
            .with_child(ElementDef::new(Tag::Button).with_class("btn")),
    };

    let mut h = TestHarness::new(css, tree, 800.0, 200.0);
    h.step();

    let button = h.query("button").expect("button exists");
    let button_id = button.node_id;

    // Without hover, the base rule should apply: content x.
    let children = h.arena().children(button_id);
    assert_eq!(children.len(), 1);
    assert_eq!(h.arena().get(children[0]).unwrap().content, ElementContent::Text("x".into()),);

    // Hover the button: the more specific `:hover::before` rule wins.
    let rect = button.layout_rect;
    h.mouse_move(rect.x + rect.width * 0.5, rect.y + rect.height * 0.5);
    h.step();

    let children_hover = h.arena().children(button_id);
    assert_eq!(children_hover.len(), 1);
    let hovered_content = h.arena().get(children_hover[0]).unwrap().content.clone();
    assert_eq!(
        hovered_content,
        ElementContent::Text("y".into()),
        "more specific hover rule should overwrite the base content",
    );
}
