use unshit_core::element::*;
use unshit_core::style::parse::StyleDeclaration;
use unshit_core::style::types::Color;
use unshit_test::TestHarness;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn base_css() -> &'static str {
    r#"
    .root { display: flex; flex-direction: column; width: 100%; height: 100%; gap: 4px; }
    .row  { display: flex; flex-direction: row; gap: 4px; }
    span  { padding: 2px 4px; }
    button { padding: 4px 8px; width: 80px; height: 32px; }
    .scroll-container {
        display: flex;
        flex-direction: column;
        overflow: scroll;
        height: 100px;
        width: 100%;
    }
    .tall { height: 500px; width: 100%; }
    .old { background: #ff0000; }
    .new { background: #00ff00; }
    "#
}

fn make_tree_3_spans() -> ElementTree {
    ElementTree {
        root: ElementDef::new(Tag::Div).with_class("root").with_child(
            ElementDef::new(Tag::Div)
                .with_class("row")
                .with_child(ElementDef::new(Tag::Span).with_text("Alpha"))
                .with_child(ElementDef::new(Tag::Span).with_text("Beta"))
                .with_child(ElementDef::new(Tag::Span).with_text("Gamma")),
        ),
    }
}

// ---------------------------------------------------------------------------
// 1. Reconcile preserves layout
// ---------------------------------------------------------------------------

#[test]
fn test_reconcile_preserves_layout() {
    let mut h = TestHarness::new(base_css(), make_tree_3_spans, 800.0, 600.0);
    h.step();

    // Capture layout rects before reconcile
    let before: Vec<_> = h.query_all("span").iter().map(|s| s.layout_rect).collect();
    assert_eq!(before.len(), 3, "expected 3 spans before rebuild");

    // Rebuild with identical tree
    h.rebuild(make_tree_3_spans);

    let after: Vec<_> = h.query_all("span").iter().map(|s| s.layout_rect).collect();
    assert_eq!(after.len(), 3, "expected 3 spans after rebuild");

    for (i, (b, a)) in before.iter().zip(after.iter()).enumerate() {
        assert!(
            (b.x - a.x).abs() < 1.0 && (b.y - a.y).abs() < 1.0,
            "span {i} position shifted: before=({}, {}), after=({}, {})",
            b.x,
            b.y,
            a.x,
            a.y,
        );
        assert!(
            (b.width - a.width).abs() < 1.0 && (b.height - a.height).abs() < 1.0,
            "span {i} size changed: before=({}, {}), after=({}, {})",
            b.width,
            b.height,
            a.width,
            a.height,
        );
    }
}

// ---------------------------------------------------------------------------
// 2. Reconcile updates text
// ---------------------------------------------------------------------------

#[test]
fn test_reconcile_updates_text() {
    let mut h = TestHarness::new(
        base_css(),
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("root")
                .with_child(ElementDef::new(Tag::Span).with_class("target").with_text("Hello")),
        },
        800.0,
        600.0,
    );
    h.step();

    let snap = h.query(".target").expect("target span exists");
    assert_eq!(snap.content, ElementContent::Text("Hello".into()));

    h.rebuild(|| ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("root")
            .with_child(ElementDef::new(Tag::Span).with_class("target").with_text("World")),
    });

    let snap = h.query(".target").expect("target span exists after rebuild");
    assert_eq!(snap.content, ElementContent::Text("World".into()));
    assert!(snap.layout_rect.width > 0.0, "layout width should be non-zero");
    assert!(snap.layout_rect.height > 0.0, "layout height should be non-zero");
}

#[test]
fn test_reconcile_updates_inline_font_scale_for_descendants() {
    let css = r#"
    .root { display: flex; flex-direction: column; width: 100%; height: 100%; }
    .label { font-size: 10px; width: 100px; height: 24px; }
    "#;
    let mut scale = 1.0_f32;
    let mut h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("root")
                .with_style(StyleDeclaration::FontScale(scale))
                .with_child(ElementDef::new(Tag::Span).with_class("label").with_text("scaled")),
        },
        800.0,
        600.0,
    );
    h.step();
    assert!((h.query(".label").unwrap().computed_style.font_size - 10.0).abs() < 0.01);

    scale = 1.5;
    h.rebuild(|| ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("root")
            .with_style(StyleDeclaration::FontScale(scale))
            .with_child(ElementDef::new(Tag::Span).with_class("label").with_text("scaled")),
    });

    assert!(
        (h.query(".label").unwrap().computed_style.font_size - 15.0).abs() < 0.01,
        "inline font scale changes should recascade through descendants"
    );
}

// ---------------------------------------------------------------------------
// 3. Reconcile adds children
// ---------------------------------------------------------------------------

#[test]
fn test_reconcile_adds_children() {
    let mut h = TestHarness::new(
        base_css(),
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("root")
                .with_child(ElementDef::new(Tag::Span).with_text("A"))
                .with_child(ElementDef::new(Tag::Span).with_text("B")),
        },
        800.0,
        600.0,
    );
    h.step();

    let spans = h.query_all("span");
    assert_eq!(spans.len(), 2, "should start with 2 children");

    h.rebuild(|| ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("root")
            .with_child(ElementDef::new(Tag::Span).with_text("A"))
            .with_child(ElementDef::new(Tag::Span).with_text("B"))
            .with_child(ElementDef::new(Tag::Span).with_text("C")),
    });

    let spans = h.query_all("span");
    assert_eq!(spans.len(), 3, "should have 3 children after rebuild");

    assert_eq!(
        spans[2].content,
        ElementContent::Text("C".into()),
        "third child should have text 'C'"
    );
}

// ---------------------------------------------------------------------------
// 4. Reconcile removes children
// ---------------------------------------------------------------------------

#[test]
fn test_reconcile_removes_children() {
    let mut h = TestHarness::new(
        base_css(),
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("root")
                .with_child(ElementDef::new(Tag::Span).with_text("X"))
                .with_child(ElementDef::new(Tag::Span).with_text("Y"))
                .with_child(ElementDef::new(Tag::Span).with_text("Z")),
        },
        800.0,
        600.0,
    );
    h.step();

    assert_eq!(h.query_all("span").len(), 3, "should start with 3 children");

    h.rebuild(|| ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("root")
            .with_child(ElementDef::new(Tag::Span).with_text("X"))
            .with_child(ElementDef::new(Tag::Span).with_text("Y")),
    });

    let spans = h.query_all("span");
    assert_eq!(spans.len(), 2, "should have 2 children after removing one");
}

// ---------------------------------------------------------------------------
// 5. Reconcile preserves scroll
// ---------------------------------------------------------------------------

#[test]
fn test_reconcile_preserves_scroll() {
    let make_scrollable = || ElementTree {
        root: ElementDef::new(Tag::Div).with_class("root").with_child(
            ElementDef::new(Tag::Div)
                .with_class("scroll-container")
                .with_child(ElementDef::new(Tag::Div).with_class("tall")),
        ),
    };

    let mut h = TestHarness::new(base_css(), make_scrollable, 800.0, 600.0);
    h.step();

    // Set scroll_y via direct arena mutation
    let container = h.query(".scroll-container").expect("scroll-container exists");
    let container_id = container.node_id;
    h.arena_mut().get_mut(container_id).unwrap().scroll_y = 42.0;

    // Rebuild with the same tree
    h.rebuild(make_scrollable);

    let snap = h.query(".scroll-container").expect("scroll-container exists after rebuild");
    assert!(
        (snap.scroll_y - 42.0).abs() < 0.01,
        "scroll_y should be preserved after reconcile, got {}",
        snap.scroll_y,
    );
}

// ---------------------------------------------------------------------------
// 6. Reconcile keyed reorder
// ---------------------------------------------------------------------------

#[test]
fn test_reconcile_keyed_reorder() {
    let mut h = TestHarness::new(
        base_css(),
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("root")
                .with_child(ElementDef::new(Tag::Span).with_id("a").with_text("Item A"))
                .with_child(ElementDef::new(Tag::Span).with_id("b").with_text("Item B"))
                .with_child(ElementDef::new(Tag::Span).with_id("c").with_text("Item C")),
        },
        800.0,
        600.0,
    );
    h.step();

    // Reorder: c, a, b
    h.rebuild(|| ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("root")
            .with_child(ElementDef::new(Tag::Span).with_id("c").with_text("Item C"))
            .with_child(ElementDef::new(Tag::Span).with_id("a").with_text("Item A"))
            .with_child(ElementDef::new(Tag::Span).with_id("b").with_text("Item B")),
    });

    // Walk children in sibling-chain order via the arena to verify reorder
    let root_id = h.root();
    let child_ids = h.arena().children(root_id);
    assert_eq!(child_ids.len(), 3, "should still have 3 keyed children");

    let first = h.query_node(child_ids[0]).expect("first child");
    let second = h.query_node(child_ids[1]).expect("second child");
    let third = h.query_node(child_ids[2]).expect("third child");

    assert_eq!(first.id.as_deref(), Some("c"), "first child should be 'c'");
    assert_eq!(second.id.as_deref(), Some("a"), "second child should be 'a'");
    assert_eq!(third.id.as_deref(), Some("b"), "third child should be 'b'");

    // Verify layout positions reflect new order
    assert!(
        first.layout_rect.y <= second.layout_rect.y,
        "first child should be above or equal to second"
    );
    assert!(
        second.layout_rect.y <= third.layout_rect.y,
        "second child should be above or equal to third"
    );
}

// ---------------------------------------------------------------------------
// 7. Reconcile tag change replaces element
// ---------------------------------------------------------------------------

#[test]
fn test_reconcile_tag_change_replaces() {
    let mut h = TestHarness::new(
        base_css(),
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("root")
                .with_child(ElementDef::new(Tag::Div).with_class("target")),
        },
        800.0,
        600.0,
    );
    h.step();

    let snap = h.query(".target").expect("target exists");
    assert_eq!(snap.tag, Tag::Div, "initial tag should be Div");

    h.rebuild(|| ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("root")
            .with_child(ElementDef::new(Tag::Button).with_class("target")),
    });

    let snap = h.query(".target").expect("target exists after rebuild");
    assert_eq!(snap.tag, Tag::Button, "tag should be Button after reconcile");
}

// ---------------------------------------------------------------------------
// 8. Reconcile class change
// ---------------------------------------------------------------------------

#[test]
fn test_reconcile_class_change() {
    let mut h = TestHarness::new(
        base_css(),
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("root")
                .with_child(ElementDef::new(Tag::Div).with_class("old")),
        },
        800.0,
        600.0,
    );
    h.step();

    let snap = h.query(".old").expect("element with class 'old' exists");
    assert!(snap.classes.contains(&"old".to_string()));

    h.rebuild(|| ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("root")
            .with_child(ElementDef::new(Tag::Div).with_class("new")),
    });

    // Old class should be gone
    assert!(h.query(".old").is_none(), "element should no longer have class 'old'");

    // New class should be present
    let snap = h.query(".new").expect("element with class 'new' exists");
    assert!(snap.classes.contains(&"new".to_string()));
}

#[test]
fn test_reconcile_ancestor_class_change_restyles_descendant_selectors() {
    let css = r#"
        .root { display: flex; width: 100%; height: 100%; }
        .theme-amber .label { color: #d4a348; width: 100px; height: 20px; }
        .theme-hacker .label { color: #39ff88; width: 100px; height: 20px; }
    "#;

    let mut h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("root")
                .with_class("theme-amber")
                .with_child(ElementDef::new(Tag::Span).with_class("label").with_text("theme")),
        },
        800.0,
        600.0,
    );

    let before = h.query(".label").expect("label exists before rebuild");
    assert_eq!(before.computed_style.color, Color::rgb(0xd4, 0xa3, 0x48));

    h.rebuild(|| ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("root")
            .with_class("theme-hacker")
            .with_child(ElementDef::new(Tag::Span).with_class("label").with_text("theme")),
    });

    let after = h.query(".label").expect("label exists after rebuild");
    assert_eq!(
        after.computed_style.color,
        Color::rgb(0x39, 0xff, 0x88),
        "descendant selectors must recascade when an ancestor class changes"
    );
}

// ---------------------------------------------------------------------------
// 9. Reconcile does not regress hover
// ---------------------------------------------------------------------------

#[test]
fn test_reconcile_no_regression_hover() {
    let css = r#"
        .root { display: flex; width: 100%; height: 100%; }
        button { width: 100px; height: 50px; }
    "#;

    let make_tree = || ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("root")
            .with_child(ElementDef::new(Tag::Button).with_class("btn")),
    };

    let mut h = TestHarness::new(css, make_tree, 800.0, 600.0);
    h.step();

    // Move mouse over the button
    h.mouse_move(50.0, 25.0);
    h.step();

    let hovered_before = h.hovered();
    assert!(!hovered_before.is_dangling(), "button should be hovered before rebuild");

    // Rebuild with the same tree
    h.rebuild(make_tree);

    // Re-trigger hover at the same position
    h.mouse_move(50.0, 25.0);
    h.step();

    let hovered_after = h.hovered();
    assert!(!hovered_after.is_dangling(), "button should still be hovered after rebuild");

    let snap = h.query("button").expect("button exists after rebuild");
    assert_eq!(hovered_after, snap.node_id, "hovered element should be the button");
}

// ---------------------------------------------------------------------------
// 10..13. Pseudo element reconciler safety (issue #121).
// ---------------------------------------------------------------------------

fn pseudo_css() -> &'static str {
    r#"
    .root { display: flex; flex-direction: column; }
    .card::before { content: "*"; }
    .card::after { content: "!"; }
    "#
}

fn make_card_tree_spans(texts: &[&'static str]) -> ElementTree {
    let mut card = ElementDef::new(Tag::Div).with_class("card");
    for t in texts {
        card = card.with_child(ElementDef::new(Tag::Span).with_text(*t));
    }
    ElementTree { root: ElementDef::new(Tag::Div).with_class("root").with_child(card) }
}

/// The reconciler must not mistake the synthetic ::before for a user child
/// when walking positional matches. Rebuilding with the same user tree should
/// leave both the user spans and the pseudo nodes untouched.
#[test]
fn test_reconcile_ignores_synthetic_before() {
    let mut h =
        TestHarness::new(pseudo_css(), || make_card_tree_spans(&["alpha", "beta"]), 800.0, 600.0);
    h.step();

    // Grab the child ids as they currently are in the arena.
    let card_id = h.query(".card").expect("card exists").node_id;
    let children_before: Vec<_> = h.arena().children(card_id).to_vec();

    // There should be exactly 4 children: before, alpha, beta, after.
    assert_eq!(children_before.len(), 4, "expected 4 children including pseudo");

    h.rebuild(|| make_card_tree_spans(&["alpha", "beta"]));
    h.step();

    let children_after: Vec<_> = h.arena().children(card_id).to_vec();
    assert_eq!(
        children_after, children_before,
        "reconcile with the same user children must preserve pseudo and user ids"
    );
}

/// User child at position 0 must align with the first user child, not the
/// leading synthetic ::before, so reconcile keeps text content stable.
#[test]
fn test_reconcile_user_child_after_synthetic_before() {
    let mut h = TestHarness::new(pseudo_css(), || make_card_tree_spans(&["one"]), 800.0, 600.0);
    h.step();

    let card_id = h.query(".card").expect("card exists").node_id;
    let ids = h.arena().children(card_id);
    assert_eq!(ids.len(), 3);
    // Middle child (position 1 in the full sibling list) is the user span.
    let user_span_id = ids[1];
    let user_elem = h.arena().get(user_span_id).expect("user span");
    assert!(!user_elem.synthetic);
    assert_eq!(user_elem.content, ElementContent::Text("one".into()));

    // Rebuild with a different text on the single user child.
    h.rebuild(|| make_card_tree_spans(&["two"]));
    h.step();

    let ids_after = h.arena().children(card_id);
    assert_eq!(ids_after.len(), 3, "still before + 1 user + after");
    assert_eq!(
        ids_after[1], user_span_id,
        "user child should remain the same NodeId across reconcile"
    );
    assert_eq!(h.arena().get(user_span_id).unwrap().content, ElementContent::Text("two".into()),);
}

/// Wiping all user children must leave the pseudo ::before and ::after
/// intact.
#[test]
fn test_reconcile_removes_all_user_children_keeps_pseudo() {
    let mut h = TestHarness::new(pseudo_css(), || make_card_tree_spans(&["a", "b"]), 800.0, 600.0);
    h.step();

    let card_id = h.query(".card").expect("card exists").node_id;
    let before_id = h.arena().children(card_id)[0];
    let after_id = *h.arena().children(card_id).last().unwrap();

    // Sanity: both pseudo nodes are marked synthetic.
    assert!(h.arena().get(before_id).unwrap().synthetic);
    assert!(h.arena().get(after_id).unwrap().synthetic);

    h.rebuild(|| ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("root")
            .with_child(ElementDef::new(Tag::Div).with_class("card")),
    });
    h.step();

    let children = h.arena().children(card_id);
    assert_eq!(children.len(), 2, "only pseudo nodes remain");
    assert_eq!(children[0], before_id, "before NodeId preserved");
    assert_eq!(children[1], after_id, "after NodeId preserved");
}

/// Running reconcile twice with the same tree must not duplicate pseudo
/// children.
#[test]
fn test_reconcile_rebuild_does_not_duplicate_pseudo() {
    let mut h = TestHarness::new(pseudo_css(), || make_card_tree_spans(&["only"]), 800.0, 600.0);
    h.step();

    h.rebuild(|| make_card_tree_spans(&["only"]));
    h.step();
    h.rebuild(|| make_card_tree_spans(&["only"]));
    h.step();

    let card_id = h.query(".card").expect("card exists").node_id;
    let children = h.arena().children(card_id);
    // Still exactly 3 children: before, user span, after.
    assert_eq!(children.len(), 3);
    // Exactly one synthetic at each end.
    assert!(h.arena().get(children[0]).unwrap().synthetic);
    assert!(!h.arena().get(children[1]).unwrap().synthetic);
    assert!(h.arena().get(children[2]).unwrap().synthetic);
}

// ---------------------------------------------------------------------------
// 14. Keyed children via `key` attribute (not `id`) -- reorder preserves NodeIds
// ---------------------------------------------------------------------------

#[test]
fn test_reconcile_key_reorder_preserves_node_ids() {
    // Build initial list keyed with `key` (not CSS id).
    let mut h = TestHarness::new(
        base_css(),
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("root")
                .with_child(ElementDef::new(Tag::Span).with_key("a").with_text("Item A"))
                .with_child(ElementDef::new(Tag::Span).with_key("b").with_text("Item B"))
                .with_child(ElementDef::new(Tag::Span).with_key("c").with_text("Item C")),
        },
        800.0,
        600.0,
    );
    h.step();

    let root_id = h.root();
    let ids_before: Vec<_> = h.arena().children(root_id).to_vec();
    assert_eq!(ids_before.len(), 3);

    // Verify keys were stored on the live elements.
    assert_eq!(h.arena().get(ids_before[0]).unwrap().key.as_deref(), Some("a"));
    assert_eq!(h.arena().get(ids_before[1]).unwrap().key.as_deref(), Some("b"));
    assert_eq!(h.arena().get(ids_before[2]).unwrap().key.as_deref(), Some("c"));

    // Reorder: c, a, b
    h.rebuild(|| ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("root")
            .with_child(ElementDef::new(Tag::Span).with_key("c").with_text("Item C"))
            .with_child(ElementDef::new(Tag::Span).with_key("a").with_text("Item A"))
            .with_child(ElementDef::new(Tag::Span).with_key("b").with_text("Item B")),
    });

    let ids_after: Vec<_> = h.arena().children(root_id).to_vec();
    assert_eq!(ids_after.len(), 3, "should still have 3 keyed children");

    // The NodeId for each key must be preserved (no re-allocation on reorder).
    let node_a_before = ids_before[0]; // was "a"
    let node_b_before = ids_before[1]; // was "b"
    let node_c_before = ids_before[2]; // was "c"

    // After reorder the order is c, a, b.
    assert_eq!(ids_after[0], node_c_before, "first slot should be the original 'c' NodeId");
    assert_eq!(ids_after[1], node_a_before, "second slot should be the original 'a' NodeId");
    assert_eq!(ids_after[2], node_b_before, "third slot should be the original 'b' NodeId");
}

// ---------------------------------------------------------------------------
// 15. Keyed children -- insert into middle preserves flanking NodeIds
// ---------------------------------------------------------------------------

#[test]
fn test_reconcile_key_insert_preserves_flanking_node_ids() {
    let mut h = TestHarness::new(
        base_css(),
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("root")
                .with_child(ElementDef::new(Tag::Span).with_key("a").with_text("A"))
                .with_child(ElementDef::new(Tag::Span).with_key("b").with_text("B")),
        },
        800.0,
        600.0,
    );
    h.step();

    let root_id = h.root();
    let ids_before: Vec<_> = h.arena().children(root_id).to_vec();
    assert_eq!(ids_before.len(), 2);
    let node_a = ids_before[0];
    let node_b = ids_before[1];

    // Insert "x" between "a" and "b".
    h.rebuild(|| ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("root")
            .with_child(ElementDef::new(Tag::Span).with_key("a").with_text("A"))
            .with_child(ElementDef::new(Tag::Span).with_key("x").with_text("X"))
            .with_child(ElementDef::new(Tag::Span).with_key("b").with_text("B")),
    });

    let ids_after: Vec<_> = h.arena().children(root_id).to_vec();
    assert_eq!(ids_after.len(), 3, "should have 3 children after insert");

    // "a" and "b" must keep their original NodeIds.
    assert_eq!(ids_after[0], node_a, "'a' should keep its NodeId");
    assert_eq!(ids_after[2], node_b, "'b' should keep its NodeId");

    // "x" is new -- its NodeId must differ from both flanking nodes.
    let node_x = ids_after[1];
    assert_ne!(node_x, node_a, "new 'x' should have a fresh NodeId");
    assert_ne!(node_x, node_b, "new 'x' should have a fresh NodeId");
    assert_eq!(
        h.arena().get(ids_after[1]).unwrap().key.as_deref(),
        Some("x"),
        "'x' element key should be set"
    );
}

// ---------------------------------------------------------------------------
// 16. Keyed children -- remove middle element deallocates it
// ---------------------------------------------------------------------------

#[test]
fn test_reconcile_key_remove_deallocates_removed_node() {
    let mut h = TestHarness::new(
        base_css(),
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("root")
                .with_child(ElementDef::new(Tag::Span).with_key("a").with_text("A"))
                .with_child(ElementDef::new(Tag::Span).with_key("b").with_text("B"))
                .with_child(ElementDef::new(Tag::Span).with_key("c").with_text("C")),
        },
        800.0,
        600.0,
    );
    h.step();

    let root_id = h.root();
    let ids_before: Vec<_> = h.arena().children(root_id).to_vec();
    let node_b = ids_before[1];
    assert_eq!(h.arena().get(node_b).unwrap().key.as_deref(), Some("b"));

    // Remove "b".
    h.rebuild(|| ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("root")
            .with_child(ElementDef::new(Tag::Span).with_key("a").with_text("A"))
            .with_child(ElementDef::new(Tag::Span).with_key("c").with_text("C")),
    });

    let ids_after: Vec<_> = h.arena().children(root_id).to_vec();
    assert_eq!(ids_after.len(), 2, "should have 2 children after removal");

    // "b" must no longer exist in the arena.
    assert!(h.arena().get(node_b).is_none(), "'b' NodeId should be deallocated after removal");
}

// ---------------------------------------------------------------------------
// 17. Mixed keyed/unkeyed: unkeyed elements still match positionally
// ---------------------------------------------------------------------------

#[test]
fn test_reconcile_key_mixed_keyed_and_unkeyed() {
    // Three children: keyed "k1", unkeyed, keyed "k2".
    let mut h = TestHarness::new(
        base_css(),
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("root")
                .with_child(ElementDef::new(Tag::Span).with_key("k1").with_text("Keyed 1"))
                .with_child(ElementDef::new(Tag::Span).with_text("Unkeyed"))
                .with_child(ElementDef::new(Tag::Span).with_key("k2").with_text("Keyed 2")),
        },
        800.0,
        600.0,
    );
    h.step();

    let root_id = h.root();
    let ids_before: Vec<_> = h.arena().children(root_id).to_vec();
    assert_eq!(ids_before.len(), 3);

    let node_k1 = ids_before[0];
    let node_unkeyed = ids_before[1];
    let node_k2 = ids_before[2];

    // Rebuild with same structure -- all nodes should survive with same NodeIds.
    h.rebuild(|| ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("root")
            .with_child(ElementDef::new(Tag::Span).with_key("k1").with_text("Keyed 1 updated"))
            .with_child(ElementDef::new(Tag::Span).with_text("Unkeyed updated"))
            .with_child(ElementDef::new(Tag::Span).with_key("k2").with_text("Keyed 2 updated")),
    });

    let ids_after: Vec<_> = h.arena().children(root_id).to_vec();
    assert_eq!(ids_after.len(), 3);

    // Keyed elements keep their NodeIds.
    assert_eq!(ids_after[0], node_k1, "k1 NodeId preserved");
    assert_eq!(ids_after[2], node_k2, "k2 NodeId preserved");

    // Unkeyed element also keeps its NodeId (positional match).
    assert_eq!(ids_after[1], node_unkeyed, "unkeyed NodeId preserved by position");
}
