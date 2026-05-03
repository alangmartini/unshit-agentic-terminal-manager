//! Criterion bench gating Quick Prompt autocomplete filter latency.
//!
//! Spec A8.6 requires p99 <1 ms over ~200 entries. This bench keeps
//! the budget honest: if a future change replaces the cheap
//! case-insensitive substring match with something heavier (fuzzy
//! score, regex, fallback to walking the FS on every keystroke), the
//! benchmark numbers move enough to catch it in review.
//!
//! The bench builds a synthetic 200 entry list once and then iterates
//! `filter` against several query shapes: empty (fast path), short
//! query that matches many entries, longer query that matches a few,
//! and a query that matches nothing. The hottest case is the empty
//! query because it returns every index; the slowest is the broad
//! match because it actually walks every name.
//!
//! `autocomplete.rs` has no crate-internal references so we pull it in
//! via `#[path]` rather than turning the package into a library, the
//! same pattern `benches/pty_write.rs` uses.

use criterion::{black_box, criterion_group, criterion_main, Criterion};

#[allow(dead_code, unused_imports)]
#[path = "../src/quick_prompt/autocomplete.rs"]
mod autocomplete;

use autocomplete::{filter, Entry, EntryKind};

fn synthetic_entries(n: usize) -> Vec<Entry> {
    (0..n)
        .map(|i| Entry {
            name: format!("entry-{:03}-{}", i, name_seed(i)),
            kind: if i % 2 == 0 {
                EntryKind::Skill
            } else {
                EntryKind::Command
            },
        })
        .collect()
}

fn name_seed(i: usize) -> &'static str {
    const SEEDS: &[&str] = &[
        "plan", "review", "ship", "fix", "refactor", "audit", "lint", "doc", "merge", "release",
    ];
    SEEDS[i % SEEDS.len()]
}

fn bench_filter(c: &mut Criterion) {
    let entries = synthetic_entries(200);

    c.bench_function("autocomplete::filter empty query (200 entries)", |b| {
        b.iter(|| black_box(filter(black_box(&entries), black_box(""))))
    });

    c.bench_function("autocomplete::filter short hit (200 entries)", |b| {
        // Matches roughly 20 entries (every 10th has the seed "plan").
        b.iter(|| black_box(filter(black_box(&entries), black_box("plan"))))
    });

    c.bench_function("autocomplete::filter long hit (200 entries)", |b| {
        // Matches a smaller slice; exercises the per row to_ascii_lowercase + contains path.
        b.iter(|| black_box(filter(black_box(&entries), black_box("entry-1"))))
    });

    c.bench_function("autocomplete::filter no match (200 entries)", |b| {
        b.iter(|| black_box(filter(black_box(&entries), black_box("zzzzzz"))))
    });
}

criterion_group!(benches, bench_filter);
criterion_main!(benches);
