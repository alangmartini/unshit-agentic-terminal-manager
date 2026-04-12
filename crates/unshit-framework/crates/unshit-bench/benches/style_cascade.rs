use criterion::{criterion_group, criterion_main, Criterion};
use unshit_bench::{
    build_large_tree_def, generate_cascade_css, materialize_tree, resolve_all_styles,
};
use unshit_core::id::NodeId;
use unshit_core::style::parse::CompiledStylesheet;

fn bench_style_cascade(c: &mut Criterion) {
    let css = generate_cascade_css();
    let stylesheet = CompiledStylesheet::parse(&css);
    let def = build_large_tree_def();

    c.bench_function("style_cascade_500_elements_50_rules", |b| {
        b.iter(|| {
            let (mut arena, _taffy, root) = materialize_tree(&def);
            resolve_all_styles(
                &mut arena,
                &stylesheet,
                root,
                NodeId::DANGLING,
                None,
                NodeId::DANGLING,
            );
            assert!(arena.len() > 400);
        });
    });
}

criterion_group!(benches, bench_style_cascade);
criterion_main!(benches);
