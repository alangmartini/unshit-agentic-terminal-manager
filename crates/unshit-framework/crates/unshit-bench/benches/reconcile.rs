use criterion::{criterion_group, criterion_main, Criterion};
use unshit_bench::*;
use unshit_core::element::{ElementDef, Tag};

fn bench_reconcile(c: &mut Criterion) {
    let def = build_large_tree_def();

    // 1. Reconcile with no changes (identical def)
    c.bench_function("reconcile_no_change_500", |b| {
        let identical = build_large_tree_def();
        b.iter(|| {
            let (mut arena, mut taffy, root) = materialize_tree(&def);
            reconcile_tree(&mut arena, &mut taffy, root, &identical);
        });
    });

    // 2. Reconcile with a single text change in a leaf
    c.bench_function("reconcile_single_text_change_500", |b| {
        let mut mutated = build_large_tree_def();
        // Mutate the first section's first row's label text
        if let Some(section) = mutated.children.first_mut() {
            // children[0] is the header, children[1] is the first row
            if let Some(row) = section.children.get_mut(1) {
                // First child of the row is the label
                if let Some(label) = row.children.first_mut() {
                    label.content =
                        unshit_core::element::ElementContent::Text("CHANGED".to_string());
                }
            }
        }

        b.iter(|| {
            let (mut arena, mut taffy, root) = materialize_tree(&def);
            reconcile_tree(&mut arena, &mut taffy, root, &mutated);
        });
    });

    // 3. Reconcile adding 10 children to the first section
    c.bench_function("reconcile_add_children", |b| {
        let mut with_extra = build_large_tree_def();
        if let Some(section) = with_extra.children.first_mut() {
            for i in 0..10 {
                section.children.push(
                    ElementDef::new(Tag::Span)
                        .with_class("extra")
                        .with_text(format!("Extra {}", i)),
                );
            }
        }

        b.iter(|| {
            let (mut arena, mut taffy, root) = materialize_tree(&def);
            reconcile_tree(&mut arena, &mut taffy, root, &with_extra);
        });
    });

    // 4. Reconcile removing 10 children (start with extra, reconcile to base)
    c.bench_function("reconcile_remove_children", |b| {
        let mut with_extra = build_large_tree_def();
        if let Some(section) = with_extra.children.first_mut() {
            for i in 0..10 {
                section.children.push(
                    ElementDef::new(Tag::Span)
                        .with_class("extra")
                        .with_text(format!("Extra {}", i)),
                );
            }
        }
        let base = build_large_tree_def();

        b.iter(|| {
            let (mut arena, mut taffy, root) = materialize_tree(&with_extra);
            reconcile_tree(&mut arena, &mut taffy, root, &base);
        });
    });

    // 5. Full rebuild baseline for comparison
    c.bench_function("full_rebuild_500_baseline", |b| {
        b.iter(|| {
            let (_arena, _taffy, root) = materialize_tree(&def);
            assert!(!root.is_dangling());
        });
    });
}

criterion_group!(benches, bench_reconcile);
criterion_main!(benches);
