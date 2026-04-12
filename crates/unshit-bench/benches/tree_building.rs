use criterion::{criterion_group, criterion_main, Criterion};
use unshit_bench::{build_large_tree_def, materialize_tree};

fn bench_tree_building(c: &mut Criterion) {
    c.bench_function("tree_def_construction_500", |b| {
        b.iter(|| {
            let def = build_large_tree_def();
            assert!(!def.children.is_empty());
        });
    });

    let def = build_large_tree_def();

    c.bench_function("tree_materialize_500", |b| {
        b.iter(|| {
            let (arena, _taffy, root) = materialize_tree(&def);
            assert!(arena.len() > 400);
            assert!(!root.is_dangling());
        });
    });
}

criterion_group!(benches, bench_tree_building);
criterion_main!(benches);
