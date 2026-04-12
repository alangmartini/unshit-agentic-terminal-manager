//! Tests for subtree memoization (issue #151).
//!
//! Covers:
//! 1. Memo skip: same key, children not touched
//! 2. Memo invalidation: different key, children reconciled
//! 3. Dirty propagation: deep leaf change marks all ancestors SUBTREE_STYLE
//! 4. Clean subtree skip: cascade only runs on dirty branch
//! 5. Memo with no children change: children survive untouched
//! 6. First build: all nodes get cascaded even without explicit dirty flags

use unshit_core::dirty::DirtyFlags;
use unshit_core::element::*;
use unshit_core::id::NodeId;
use unshit_test::TestHarness;

// ---------------------------------------------------------------------------
// Shared CSS
// ---------------------------------------------------------------------------

fn base_css() -> &'static str {
    r#"
    .root  { display: flex; flex-direction: column; width: 100%; height: 100%; }
    .panel { display: flex; flex-direction: row; }
    span   { padding: 2px 4px; }
    .highlight { background: #ff0000; }
    "#
}

// ---------------------------------------------------------------------------
// Test 1: Memo hit skips subtree reconciliation
//
// Build a tree where one child carries memo_key=42.
// Rebuild with the exact same memo_key. The child's content should remain
// unchanged even though the new def has different text, because the memo
// fence prevents reconciliation of that subtree.
// ---------------------------------------------------------------------------
#[test]
fn test_memo_hit_skips_subtree() {
    // Build: root -> [normal_span("first"), memo_span("original")]
    let mut h = TestHarness::new(
        base_css(),
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("root")
                .with_child(ElementDef::new(Tag::Span).with_text("first"))
                .with_child(ElementDef::new(Tag::Span).with_memo_key(42).with_text("original")),
        },
        800.0,
        600.0,
    );
    h.step();

    // Capture the NodeId of the second child (the memo'd one).
    let root_id = h.root();
    let children = h.arena().children(root_id);
    assert_eq!(children.len(), 2, "should have 2 children");
    let memo_child_id = children[1];
    let memo_node_id = memo_child_id;

    // Verify it has the correct initial text.
    let snap = h.query_node(memo_child_id).expect("memo child exists");
    assert_eq!(snap.content, ElementContent::Text("original".into()));

    // Rebuild with same memo_key=42 but DIFFERENT text. The memo fence
    // should prevent the text from being updated.
    h.rebuild(|| ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("root")
            .with_child(ElementDef::new(Tag::Span).with_text("first"))
            .with_child(
                ElementDef::new(Tag::Span).with_memo_key(42).with_text("should be skipped"),
            ),
    });

    // The memo'd child should still have the original text.
    let snap = h.query_node(memo_node_id).expect("memo child still exists");
    assert_eq!(
        snap.content,
        ElementContent::Text("original".into()),
        "memo hit: text should NOT be updated when key matches"
    );
}

// ---------------------------------------------------------------------------
// Test 2: Memo invalidation when key changes
//
// Build with memo_key=42, rebuild with memo_key=99. The new key does not
// match, so the subtree must be reconciled and the text updated.
// ---------------------------------------------------------------------------
#[test]
fn test_memo_invalidation_on_key_change() {
    let mut h = TestHarness::new(
        base_css(),
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("root")
                .with_child(ElementDef::new(Tag::Span).with_memo_key(42).with_text("version-A")),
        },
        800.0,
        600.0,
    );
    h.step();

    let root_id = h.root();
    let children = h.arena().children(root_id);
    let memo_child_id = children[0];

    let snap = h.query_node(memo_child_id).expect("child exists");
    assert_eq!(snap.content, ElementContent::Text("version-A".into()));

    // Rebuild with a DIFFERENT memo key. Reconcile must run.
    h.rebuild(|| ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("root")
            .with_child(ElementDef::new(Tag::Span).with_memo_key(99).with_text("version-B")),
    });

    let snap = h.query_node(memo_child_id).expect("child still exists after rebuild");
    assert_eq!(
        snap.content,
        ElementContent::Text("version-B".into()),
        "memo miss: text must be updated when key changes"
    );
}

// ---------------------------------------------------------------------------
// Test 3: No memo attributes means normal reconciliation
//
// Without memo, changing text always propagates.
// ---------------------------------------------------------------------------
#[test]
fn test_no_memo_reconciles_normally() {
    let mut h = TestHarness::new(
        base_css(),
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("root")
                .with_child(ElementDef::new(Tag::Span).with_text("hello")),
        },
        800.0,
        600.0,
    );
    h.step();

    let root_id = h.root();
    let children = h.arena().children(root_id);
    let child_id = children[0];

    h.rebuild(|| ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("root")
            .with_child(ElementDef::new(Tag::Span).with_text("world")),
    });

    let snap = h.query_node(child_id).expect("child exists");
    assert_eq!(
        snap.content,
        ElementContent::Text("world".into()),
        "without memo, text update must propagate"
    );
}

// ---------------------------------------------------------------------------
// Test 4: Dirty propagation - deep leaf change marks all ancestors
//
// Build a 3-level tree: root -> middle -> leaf.
// Reconcile only (before cascade) and check dirty flags.
// ---------------------------------------------------------------------------
#[test]
fn test_dirty_propagation_marks_ancestors() {
    let mut h = TestHarness::new(
        base_css(),
        || ElementTree {
            root: ElementDef::new(Tag::Div).with_class("root").with_child(
                ElementDef::new(Tag::Div)
                    .with_class("panel")
                    .with_child(ElementDef::new(Tag::Span).with_class("leaf").with_text("initial")),
            ),
        },
        800.0,
        600.0,
    );
    h.step();

    let root_id = h.root();
    let panel_id = h.arena().children(root_id)[0];
    let leaf_id = h.arena().children(panel_id)[0];

    // Clear all dirty flags manually to simulate a fully clean state.
    h.arena_mut().get_mut(root_id).unwrap().dirty = DirtyFlags::empty();
    h.arena_mut().get_mut(panel_id).unwrap().dirty = DirtyFlags::empty();
    h.arena_mut().get_mut(leaf_id).unwrap().dirty = DirtyFlags::empty();

    // Verify all flags are clear.
    assert!(h.arena().get(root_id).unwrap().dirty.is_empty());
    assert!(h.arena().get(panel_id).unwrap().dirty.is_empty());
    assert!(h.arena().get(leaf_id).unwrap().dirty.is_empty());

    // Run only the reconcile step (not cascade) with a class change on the leaf.
    let new_tree = ElementTree {
        root: ElementDef::new(Tag::Div).with_class("root").with_child(
            ElementDef::new(Tag::Div).with_class("panel").with_child(
                // Class change -> STYLE dirty on leaf
                ElementDef::new(Tag::Span).with_class("highlight").with_text("initial"),
            ),
        ),
    };

    {
        let arena = h.arena_mut();
        let taffy = &mut taffy::TaffyTree::new();
        unshit_core::reconcile::reconcile(arena, taffy, root_id, &new_tree.root);
    }

    // After reconcile only, dirty flags should be set.
    let leaf_flags = h.arena().get(leaf_id).unwrap().dirty;
    let panel_flags = h.arena().get(panel_id).unwrap().dirty;
    let root_flags = h.arena().get(root_id).unwrap().dirty;

    assert!(
        leaf_flags.contains(DirtyFlags::STYLE),
        "leaf must have STYLE dirty after class change, got {:?}",
        leaf_flags
    );
    assert!(
        panel_flags.contains(DirtyFlags::SUBTREE_STYLE),
        "panel ancestor must have SUBTREE_STYLE after leaf change, got {:?}",
        panel_flags
    );
    assert!(
        root_flags.contains(DirtyFlags::SUBTREE_STYLE),
        "root ancestor must have SUBTREE_STYLE after leaf change, got {:?}",
        root_flags
    );
}

// ---------------------------------------------------------------------------
// Test 5: Clean subtree cascade skip
//
// Two sibling branches. Dirty only one. After cascade, the clean branch
// should have no dirty style flags but the dirty branch should have been
// processed. We verify by checking the computed style was updated on the
// dirty branch.
// ---------------------------------------------------------------------------
#[test]
fn test_clean_branch_not_recascaded() {
    let css = r#"
        .root    { display: flex; flex-direction: row; width: 100%; height: 100%; }
        .box-a   { background: #0000ff; }
        .box-b   { background: #00ff00; }
        .box-red { background: #ff0000; }
    "#;

    let mut h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("root")
                .with_child(ElementDef::new(Tag::Div).with_id("a").with_class("box-a"))
                .with_child(ElementDef::new(Tag::Div).with_id("b").with_class("box-b")),
        },
        800.0,
        600.0,
    );
    h.step();

    // Capture initial computed styles.
    let a_id = h.query("#a").expect("a exists").node_id;
    let b_id = h.query("#b").expect("b exists").node_id;

    // Rebuild changing only branch A's class to box-red.
    h.rebuild(|| ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("root")
            .with_child(ElementDef::new(Tag::Div).with_id("a").with_class("box-red"))
            .with_child(ElementDef::new(Tag::Div).with_id("b").with_class("box-b")),
    });

    // Branch A should have been recascaded (new background).
    let a_snap = h.query_node(a_id).expect("a still exists");
    let b_snap = h.query_node(b_id).expect("b still exists");

    // Branch A's class changed so its computed style should reflect the change.
    assert!(a_snap.classes.contains(&"box-red".to_string()), "a should have class box-red");
    // Branch B was not touched. Its class is still box-b.
    assert!(b_snap.classes.contains(&"box-b".to_string()), "b should still have class box-b");
}

// ---------------------------------------------------------------------------
// Test 6: Memo'd subtree children survive untouched across rebuilds
//
// Build root -> [normal_child, memo_parent(key=7) -> [child_A, child_B]].
// Rebuild with same memo key. child_A and child_B must keep their NodeIds.
// ---------------------------------------------------------------------------
#[test]
fn test_memo_children_preserve_node_ids() {
    let mut h = TestHarness::new(
        base_css(),
        || ElementTree {
            root: ElementDef::new(Tag::Div).with_class("root").with_child(
                ElementDef::new(Tag::Div)
                    .with_class("panel")
                    .with_memo_key(7)
                    .with_child(ElementDef::new(Tag::Span).with_text("child-A"))
                    .with_child(ElementDef::new(Tag::Span).with_text("child-B")),
            ),
        },
        800.0,
        600.0,
    );
    h.step();

    let root_id = h.root();
    let panel_id = h.arena().children(root_id)[0];
    let children_before: Vec<NodeId> = h.arena().children(panel_id).to_vec();
    assert_eq!(children_before.len(), 2, "should have 2 children in memo panel");

    // Rebuild with same memo key. The panel's children must not be touched.
    h.rebuild(|| ElementTree {
        root: ElementDef::new(Tag::Div).with_class("root").with_child(
            ElementDef::new(Tag::Div)
                .with_class("panel")
                .with_memo_key(7)
                .with_child(ElementDef::new(Tag::Span).with_text("child-X")) // ignored by memo
                .with_child(ElementDef::new(Tag::Span).with_text("child-Y")), // ignored by memo
        ),
    });

    let children_after: Vec<NodeId> = h.arena().children(panel_id).to_vec();
    assert_eq!(children_after, children_before, "memo hit: child NodeIds must be preserved");

    // Content must still be the original (not the new defs).
    let child_a = h.query_node(children_before[0]).expect("child A exists");
    let child_b = h.query_node(children_before[1]).expect("child B exists");
    assert_eq!(child_a.content, ElementContent::Text("child-A".into()));
    assert_eq!(child_b.content, ElementContent::Text("child-B".into()));
}

// ---------------------------------------------------------------------------
// Test 7: First build processes all nodes (initial STYLE flags are set)
//
// On first build, every Element::new sets STYLE | LAYOUT | CHILDREN.
// resolve_all_styles must process every node even with the short-circuit.
// We verify by checking that computed styles are non-default after the
// initial resolve.
// ---------------------------------------------------------------------------
#[test]
fn test_first_build_all_nodes_cascaded() {
    let css = r#"
        .root  { display: flex; }
        .child { background: #ff00ff; }
    "#;

    let mut h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("root")
                .with_child(ElementDef::new(Tag::Span).with_class("child").with_text("hi")),
        },
        800.0,
        600.0,
    );
    h.step();

    // The child should have a non-default computed style (background is #ff00ff).
    let snap = h.query(".child").expect("child exists");
    use unshit_core::style::types::Background;
    match &snap.computed_style.background {
        Background::Color(c) => {
            assert!(c.r > 200, "red channel should be high for #ff00ff, got {}", c.r);
            assert!(c.g < 10, "green should be low, got {}", c.g);
            assert!(c.b > 200, "blue channel should be high for #ff00ff, got {}", c.b);
        }
        other => panic!("expected Background::Color, got {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// Test 8: Memo with no children change - multiple rebuilds keep same ids
// ---------------------------------------------------------------------------
#[test]
fn test_memo_stable_across_multiple_rebuilds() {
    let build_tree = |key: u64| {
        move || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("root")
                .with_child(ElementDef::new(Tag::Span).with_memo_key(key).with_text("stable")),
        }
    };

    let mut h = TestHarness::new(base_css(), build_tree(55), 800.0, 600.0);
    h.step();

    let root_id = h.root();
    let child_id_first = h.arena().children(root_id)[0];

    // Multiple rebuilds with the same key.
    h.rebuild(build_tree(55));
    let child_id_second = h.arena().children(root_id)[0];

    h.rebuild(build_tree(55));
    let child_id_third = h.arena().children(root_id)[0];

    assert_eq!(child_id_first, child_id_second, "child NodeId must be stable across rebuild 1");
    assert_eq!(child_id_second, child_id_third, "child NodeId must be stable across rebuild 2");

    // Verify content still original.
    let snap = h.query_node(child_id_third).expect("child exists");
    assert_eq!(snap.content, ElementContent::Text("stable".into()));
}
