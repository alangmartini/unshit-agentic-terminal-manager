use cosmic_text::FontSystem;
use criterion::{criterion_group, criterion_main, Criterion};
use unshit_bench::{
    build_large_tree_def, generate_cascade_css, materialize_tree, resolve_all_styles,
    run_layout_pipeline,
};
use unshit_core::event::hit_test;
use unshit_core::id::NodeId;
use unshit_core::layout::TextMeasureCache;
use unshit_core::style::parse::CompiledStylesheet;

fn bench_hit_test(c: &mut Criterion) {
    let css = generate_cascade_css();
    let stylesheet = CompiledStylesheet::parse(&css);
    let def = build_large_tree_def();

    let (mut arena, mut taffy, root) = materialize_tree(&def);
    resolve_all_styles(&mut arena, &stylesheet, root, NodeId::DANGLING, None, NodeId::DANGLING);
    let mut font_system = FontSystem::new();
    let mut cache = TextMeasureCache::new();
    run_layout_pipeline(&mut arena, &mut taffy, root, &mut font_system, 1200.0, 800.0, &mut cache);

    let test_coords: Vec<(f32, f32)> = vec![
        (0.0, 0.0),
        (600.0, 400.0),
        (1199.0, 799.0),
        (100.0, 50.0),
        (900.0, 600.0),
        (300.0, 200.0),
        (800.0, 100.0),
        (50.0, 700.0),
        (1100.0, 300.0),
        (500.0, 500.0),
    ];

    c.bench_function("hit_test_500_elements_10_coords", |b| {
        b.iter(|| {
            for &(x, y) in &test_coords {
                let _ = hit_test(&arena, root, x, y);
            }
        });
    });

    c.bench_function("hit_test_500_elements_center", |b| {
        b.iter(|| {
            let _ = hit_test(&arena, root, 600.0, 400.0);
        });
    });

    c.bench_function("hit_test_500_elements_miss", |b| {
        b.iter(|| {
            let _ = hit_test(&arena, root, 5000.0, 5000.0);
        });
    });
}

criterion_group!(benches, bench_hit_test);
criterion_main!(benches);
