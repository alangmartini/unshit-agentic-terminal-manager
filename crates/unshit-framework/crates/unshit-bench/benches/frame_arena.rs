//! Benchmarks comparing owned vs bump-arena tree construction.
//!
//! Pairs `tree_def_construction_500_owned` (existing baseline) with
//! `tree_def_construction_500_bump` (arena path). Expected outcome: the
//! bump path is materially faster and triggers far fewer allocator
//! calls per iteration thanks to single pointer-bump allocations per
//! node.

use criterion::{criterion_group, criterion_main, Criterion};
use unshit_bench::{build_large_tree_def, build_large_tree_def_bump, materialize_tree};
use unshit_core::frame_arena::FrameArena;

fn bench_tree_construction(c: &mut Criterion) {
    c.bench_function("tree_def_construction_500_owned", |b| {
        b.iter(|| {
            let def = build_large_tree_def();
            assert!(!def.children.is_empty());
        });
    });

    // Build once outside the timed loop to confirm the arena path produces
    // the expected shape; re-using the same arena across iterations with
    // per-iteration reset is the realistic frame pattern.
    c.bench_function("tree_def_construction_500_bump", |b| {
        let mut arena = FrameArena::with_capacity(1024 * 1024);
        b.iter(|| {
            let def = build_large_tree_def_bump(&arena);
            assert!(!def.children.is_empty());
            // Dropping the tree is a no-op because all storage is in the
            // arena. Explicit drop scoping is needed so that the reset
            // call that follows is not aliasing the borrow.
            drop(def);
            arena.reset();
        });
    });
}

fn bench_frame_round_trip(c: &mut Criterion) {
    c.bench_function("frame_round_trip_owned", |b| {
        // Build an initial tree and arena once, then reconcile fresh
        // trees against it per iteration. Mirrors a rebuild frame.
        let base_def = build_large_tree_def();
        let (mut node_arena, mut taffy, root) = materialize_tree(&base_def);
        b.iter(|| {
            let new_def = build_large_tree_def();
            unshit_core::reconcile::reconcile(&mut node_arena, &mut taffy, root, &new_def);
        });
    });

    c.bench_function("frame_round_trip_bump", |b| {
        // Initial build uses the owned path to keep the baseline arena
        // identical; the reconcile per iteration uses the bump path.
        let base_def = build_large_tree_def();
        let (mut node_arena, mut taffy, root) = materialize_tree(&base_def);
        let mut frame_arena = FrameArena::with_capacity(1024 * 1024);
        b.iter(|| {
            {
                let bump_def = build_large_tree_def_bump(&frame_arena);
                unshit_core::reconcile::reconcile_bump(
                    &mut node_arena,
                    &mut taffy,
                    root,
                    &bump_def,
                );
            }
            frame_arena.reset();
        });
    });
}

criterion_group!(benches, bench_tree_construction, bench_frame_round_trip);
criterion_main!(benches);
