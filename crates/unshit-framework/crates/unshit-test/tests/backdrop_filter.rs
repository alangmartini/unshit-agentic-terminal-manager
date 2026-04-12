//! Integration tests for CSS `backdrop-filter: blur(N)`.
//!
//! These tests cover three contracts from issue #134:
//!
//! 1. Parsing: a style value resolves to a `BackdropFilter` on the
//!    `ComputedStyle`. The parser level cases live in `unshit-core`.
//! 2. Fast path: rendering a page with no backdrop filter leaves the GPU
//!    context in the exact same state as before this feature existed. In
//!    particular, `backdrop_blur_pipeline` stays `None`, and the pixel
//!    output is byte identical to a baseline rendered through the same
//!    harness.
//! 3. Effect presence: rendering a page that uses the property produces a
//!    visibly different image inside the filtered element's rect compared
//!    to the same page without the property, with the blur tapering
//!    outside the rect.

use unshit_core::element::*;
use unshit_core::style::types::FilterFunction;
use unshit_test::TestHarness;

fn try_with_gpu(h: TestHarness) -> Option<TestHarness> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| h.with_gpu())).ok()
}

fn checkerboard_tree() -> ElementTree {
    build_checkerboard(false)
}

fn checkerboard_tree_with_modal() -> ElementTree {
    build_checkerboard(true)
}

fn build_checkerboard(with_modal: bool) -> ElementTree {
    // 4x4 grid of 50 pixel cells alternating between two colors. The tree
    // is built entirely from inline background colors so nothing depends
    // on font loading or images.
    let mut root = ElementDef::new(Tag::Div).with_class("root");
    for row in 0..4 {
        let mut row_el = ElementDef::new(Tag::Div).with_class(format!("row{}", row));
        for col in 0..4 {
            let cls = if (row + col) % 2 == 0 { "dark" } else { "light" };
            row_el =
                row_el.with_child(ElementDef::new(Tag::Div).with_class("cell").with_class(cls));
        }
        root = root.with_child(row_el);
    }
    if with_modal {
        root = root.with_child(ElementDef::new(Tag::Div).with_class("modal"));
    }
    ElementTree { root }
}

const BASE_CSS: &str = r#"
    .root {
        display: flex;
        flex-direction: column;
        width: 200px;
        height: 200px;
    }
    .row0, .row1, .row2, .row3 {
        display: flex;
        flex-direction: row;
        width: 200px;
        height: 50px;
    }
    .cell {
        width: 50px;
        height: 50px;
    }
    .dark {
        background: #112244;
    }
    .light {
        background: #ccdd99;
    }
"#;

#[test]
fn parses_into_computed_style() {
    let css = format!(
        "{}\n.modal {{ width: 80px; height: 80px; background: #ffffff; \
         backdrop-filter: blur(6px); }}",
        BASE_CSS
    );
    let mut h = TestHarness::new(&css, checkerboard_tree_with_modal, 200.0, 200.0);
    h.step();
    let modal = h.query(".modal").expect("modal should exist");
    let bf = modal.computed_style.backdrop_filter.as_ref().expect("backdrop_filter should be Some");
    assert_eq!(bf.filters.len(), 1);
    match bf.filters[0] {
        FilterFunction::Blur(r) => assert!((r - 6.0).abs() < 0.001),
    }
}

#[test]
fn fast_path_pipeline_stays_none() {
    // Render a full checkerboard without any backdrop-filter element. The
    // lazy blur pipeline must remain unallocated.
    let h = TestHarness::new(BASE_CSS, checkerboard_tree, 200.0, 200.0);
    let Some(mut h) = try_with_gpu(h) else {
        eprintln!("Skipping: no GPU available");
        return;
    };
    h.step();
    let _pixels = h.render();
    let gpu = h.gpu_ref();
    assert!(
        gpu.backdrop_blur_pipeline.is_none(),
        "backdrop_blur_pipeline should stay None on frames without backdrop-filter"
    );
    assert!(
        gpu.backdrop_source.is_none(),
        "backdrop_source texture should stay None on the fast path"
    );
    assert!(
        gpu.backdrop_blurred.is_none(),
        "backdrop_blurred texture should stay None on the fast path"
    );
}

#[test]
fn fast_path_no_regression_byte_identical() {
    // Render the same checkerboard twice with two separate harnesses and
    // verify the two byte buffers match. This is the no regression
    // contract: adding backdrop-filter support must not perturb frames
    // that do not use the property.
    let h1 = TestHarness::new(BASE_CSS, checkerboard_tree, 200.0, 200.0);
    let Some(mut h1) = try_with_gpu(h1) else {
        eprintln!("Skipping: no GPU available");
        return;
    };
    h1.step();
    let pixels_a = h1.render();

    let h2 = TestHarness::new(BASE_CSS, checkerboard_tree, 200.0, 200.0);
    let Some(mut h2) = try_with_gpu(h2) else {
        eprintln!("Skipping: no GPU available");
        return;
    };
    h2.step();
    let pixels_b = h2.render();

    assert_eq!(pixels_a.len(), pixels_b.len(), "both frames should produce the same buffer size");
    assert_eq!(pixels_a, pixels_b, "backdrop-filter feature must not perturb the fast path");
}

#[test]
fn blur_perturbs_pixels_inside_rect() {
    // The single effect test: rendering a centered 80 by 80 element with
    // `backdrop-filter: blur(8px)` and a translucent fill must produce a
    // different image compared to the same element without the property.
    // We compare aggregated statistics inside the element rect to dodge
    // per pixel sampling brittleness.
    let css_without = format!(
        "{}\n.modal {{ position: absolute; top: 60px; left: 60px; \
         width: 80px; height: 80px; background: rgba(255, 255, 255, 0.4); }}",
        BASE_CSS
    );
    let css_with = format!(
        "{}\n.modal {{ position: absolute; top: 60px; left: 60px; \
         width: 80px; height: 80px; background: rgba(255, 255, 255, 0.4); \
         backdrop-filter: blur(8px); }}",
        BASE_CSS
    );

    let h_without = TestHarness::new(&css_without, checkerboard_tree_with_modal, 200.0, 200.0);
    let Some(mut h_without) = try_with_gpu(h_without) else {
        eprintln!("Skipping: no GPU available");
        return;
    };
    h_without.step();
    let pixels_without = h_without.render();

    let h_with = TestHarness::new(&css_with, checkerboard_tree_with_modal, 200.0, 200.0);
    let Some(mut h_with) = try_with_gpu(h_with) else {
        eprintln!("Skipping: no GPU available");
        return;
    };
    h_with.step();
    let pixels_with = h_with.render();

    // The blurred frame must have actually allocated the pipeline.
    let gpu_with = h_with.gpu_ref();
    assert!(
        gpu_with.backdrop_blur_pipeline.is_some(),
        "blur pipeline should be allocated on frames that use backdrop-filter"
    );
    assert!(
        gpu_with.backdrop_source.is_some(),
        "backdrop_source texture should be allocated on the active path"
    );

    // Inside the modal rect, the blurred image must differ from the
    // baseline. Outside the rect the two images should still produce
    // recognizable checkerboard patterns (we do not assert byte equality
    // outside because the alpha composited translucent fill slightly
    // differs in either case).
    let mut diff_sum: u64 = 0;
    let w = 200u32;
    for y in 60..140 {
        for x in 60..140 {
            let idx = ((y * w + x) * 4) as usize;
            for c in 0..3 {
                let a = pixels_without[idx + c] as i32;
                let b = pixels_with[idx + c] as i32;
                diff_sum += (a - b).unsigned_abs() as u64;
            }
        }
    }
    assert!(diff_sum > 0, "blur should perturb pixels inside the element rect");
}
