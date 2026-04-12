use criterion::{criterion_group, criterion_main, Criterion};
use unshit_bench::generate_large_css;
use unshit_core::style::parse::CompiledStylesheet;

fn bench_css_parsing(c: &mut Criterion) {
    let css = generate_large_css();

    c.bench_function("css_parse_100_rules", |b| {
        b.iter(|| {
            let stylesheet = CompiledStylesheet::parse(&css);
            assert!(!stylesheet.rules.is_empty());
        });
    });
}

criterion_group!(benches, bench_css_parsing);
criterion_main!(benches);
