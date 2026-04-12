use cosmic_text::FontSystem;
use criterion::{criterion_group, criterion_main, Criterion};
use unshit_bench::{
    build_large_tree_def, generate_cascade_css, materialize_tree, resolve_all_styles,
    run_layout_pipeline,
};
use unshit_core::id::NodeId;
use unshit_core::layout::TextMeasureCache;
use unshit_core::style::parse::CompiledStylesheet;

fn bench_layout(c: &mut Criterion) {
    let css = generate_cascade_css();
    let stylesheet = CompiledStylesheet::parse(&css);
    let def = build_large_tree_def();
    let mut font_system = FontSystem::new();

    c.bench_function("layout_500_elements_flex", |b| {
        b.iter(|| {
            let (mut arena, mut taffy, root) = materialize_tree(&def);
            resolve_all_styles(
                &mut arena,
                &stylesheet,
                root,
                NodeId::DANGLING,
                None,
                NodeId::DANGLING,
            );
            let mut cache = TextMeasureCache::new();
            run_layout_pipeline(
                &mut arena,
                &mut taffy,
                root,
                &mut font_system,
                1200.0,
                800.0,
                &mut cache,
            );
            let root_elem = arena.get(root).unwrap();
            assert!(root_elem.layout_rect.width > 0.0);
        });
    });
}

criterion_group!(benches, bench_layout);
criterion_main!(benches);
