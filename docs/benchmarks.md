# Benchmarks

Criterion benchmarks covering the CPU-bound hot paths in the rendering pipeline. Located in `crates/unshit-test/benches/`.

## Running

```bash
# All suites
cargo bench -p unshit-test

# Single suite
cargo bench -p unshit-test --bench layout

# Filter by benchmark name
cargo bench -p unshit-test -- "measure_text_cached"
```

HTML reports with charts are generated in `target/criterion/`. Open `target/criterion/report/index.html` after a run.

## Suites

| Suite | File | What it measures |
|-------|------|-----------------|
| `text_measurement` | `benches/text_measurement.rs` | `measure_text` cached vs uncached, varying text lengths, container widths, and font sizes |
| `layout` | `benches/layout.rs` | Flat trees (10-500 elements), nested trees (5-50 depth), grid layouts (5x5 to 20x10), relayout |
| `css_parsing` | `benches/css_parsing.rs` | `CompiledStylesheet::parse` throughput scaling from 1 to 200 rules |
| `style_resolution` | `benches/style_resolution.rs` | CSS cascade with varying tree sizes, hover state impact, rule count scaling |
| `hit_testing` | `benches/hit_testing.rs` | Text position hit testing, glyph range extraction, line selection ranges |
| `pipeline` | `benches/pipeline.rs` | Full end-to-end (CSS parse + tree build + style resolve + layout), frame stepping, minimal overhead baseline |

## Comparing runs

Criterion automatically compares against the previous run. To save a baseline for future comparison:

```bash
# Save a named baseline
cargo bench -p unshit-test -- --save-baseline before-change

# Compare against it later
cargo bench -p unshit-test -- --baseline before-change
```
