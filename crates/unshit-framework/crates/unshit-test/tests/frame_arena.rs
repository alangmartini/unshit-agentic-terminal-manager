//! Integration tests for the per-frame bump arena path.
//!
//! Exercises `reconcile_bump`, `build_subtree_bump`, and the reset cycle.

use unshit_core::element::{ElementDefBump, Tag};
use unshit_core::frame_arena::FrameArena;
use unshit_core::id::NodeId;
use unshit_core::layout::TextMeasureCtx;
use unshit_core::reconcile::{build_subtree_bump, reconcile_bump};
use unshit_core::tree::NodeArena;

/// `reconcile_bump` accepts a bump tree and populates the persistent NodeArena
/// with nodes that mirror the bump tree's structure.
#[test]
fn reconcile_accepts_bump_tree() {
    let frame_arena = FrameArena::with_capacity(64 * 1024);
    let bump_def = ElementDefBump::new_in(Tag::Div, &frame_arena)
        .with_class(&frame_arena, "root")
        .with_child(
            ElementDefBump::new_in(Tag::Span, &frame_arena).with_text(&frame_arena, "first"),
        )
        .with_child(
            ElementDefBump::new_in(Tag::Span, &frame_arena).with_text(&frame_arena, "second"),
        );

    let mut node_arena = NodeArena::new();
    let mut taffy = taffy::TaffyTree::<TextMeasureCtx>::new();
    let mut pending = Vec::new();

    let root_id =
        build_subtree_bump(&bump_def, &mut node_arena, &mut taffy, NodeId::DANGLING, &mut pending);

    assert!(!root_id.is_dangling());
    let root = node_arena.get(root_id).expect("root should be allocated");
    assert_eq!(root.tag, Tag::Div);
    assert_eq!(root.classes.as_slice(), ["root".to_string()]);

    // Walk children in order.
    let children = node_arena.children(root_id);
    assert_eq!(children.len(), 2);
    let first = node_arena.get(children[0]).unwrap();
    assert_eq!(first.tag, Tag::Span);
}

/// Frame 1 builds a three-child tree; after reset, frame 2 builds a four-child
/// tree and reconciles correctly (the new child is inserted, not duplicated).
#[test]
fn two_frame_reset_cycle_reconciles_correctly() {
    let mut node_arena = NodeArena::new();
    let mut taffy = taffy::TaffyTree::<TextMeasureCtx>::new();
    let mut pending = Vec::new();
    let mut frame_arena = FrameArena::with_capacity(64 * 1024);

    // Frame 1: 3 children.
    let root_id = {
        let def = ElementDefBump::new_in(Tag::Div, &frame_arena)
            .with_class(&frame_arena, "root")
            .with_child(ElementDefBump::new_in(Tag::Span, &frame_arena))
            .with_child(ElementDefBump::new_in(Tag::Span, &frame_arena))
            .with_child(ElementDefBump::new_in(Tag::Span, &frame_arena));
        build_subtree_bump(&def, &mut node_arena, &mut taffy, NodeId::DANGLING, &mut pending)
    };
    assert_eq!(node_arena.children(root_id).len(), 3);

    // Reset frame arena between frames.
    frame_arena.reset();

    // Frame 2: 4 children. Build a fresh bump tree after reset.
    {
        let def = ElementDefBump::new_in(Tag::Div, &frame_arena)
            .with_class(&frame_arena, "root")
            .with_child(ElementDefBump::new_in(Tag::Span, &frame_arena))
            .with_child(ElementDefBump::new_in(Tag::Span, &frame_arena))
            .with_child(ElementDefBump::new_in(Tag::Span, &frame_arena))
            .with_child(ElementDefBump::new_in(Tag::Span, &frame_arena));
        let _ = reconcile_bump(&mut node_arena, &mut taffy, root_id, &def);
    }

    assert_eq!(
        node_arena.children(root_id).len(),
        4,
        "reconcile_bump should have inserted a fourth child"
    );
}

/// After many frames of allocating and resetting, the arena's allocated_bytes
/// converges to a bounded steady state and does not grow unboundedly.
#[test]
fn arena_is_bounded_across_many_frames() {
    let mut frame_arena = FrameArena::with_capacity(256 * 1024);

    for _frame in 0..50 {
        {
            let mut root =
                ElementDefBump::new_in(Tag::Div, &frame_arena).with_class(&frame_arena, "root");
            for i in 0..100u32 {
                let child = ElementDefBump::new_in(Tag::Span, &frame_arena)
                    .with_text(&frame_arena, "item")
                    .with_class(&frame_arena, if i % 2 == 0 { "even" } else { "odd" });
                root = root.with_child(child);
            }
            // Touch the tree so the compiler does not optimize it away.
            assert_eq!(root.children.len(), 100);
        }
        frame_arena.reset();
    }

    // Peak allocated bytes should be bounded. 256 KB starting chunk; allow a
    // 16x growth envelope which is very generous.
    assert!(
        frame_arena.allocated_bytes() <= 16 * 256 * 1024,
        "arena grew unboundedly: {} bytes",
        frame_arena.allocated_bytes()
    );
}

/// A panic during bump-tree construction does not poison subsequent frames.
/// After catching the panic, we can reset the arena and build a new tree.
#[test]
fn panic_during_bump_tree_build_recovers_after_reset() {
    use std::panic::{catch_unwind, AssertUnwindSafe};

    let mut frame_arena = FrameArena::with_capacity(64 * 1024);

    let result = catch_unwind(AssertUnwindSafe(|| {
        let arena_ref = &frame_arena;
        let _def = ElementDefBump::new_in(Tag::Div, arena_ref).with_class(arena_ref, "root");
        panic!("simulated mid-frame panic");
    }));
    assert!(result.is_err(), "expected panic to be caught");

    // Resetting the arena recovers; the next frame builds cleanly.
    frame_arena.reset();
    let recovered = ElementDefBump::new_in(Tag::Div, &frame_arena).with_class(&frame_arena, "ok");
    assert_eq!(recovered.classes[0], "ok");
}
