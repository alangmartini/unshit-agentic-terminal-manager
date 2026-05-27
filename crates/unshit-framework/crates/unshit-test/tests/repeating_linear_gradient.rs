//! Renderer checks for `repeating-linear-gradient` (issue #128).
//!
//! These tests render a small element with the exact terminal-manager
//! CRT scanline overlay (`repeating-linear-gradient(0deg, transparent 0,
//! transparent 2px, rgba(0,0,0,0.12) 2px, rgba(0,0,0,0.12) 3px)`) and probe
//! pixels along the vertical axis to confirm the shader's `fract` based
//! wrapping produces a 3 pixel tile pattern: two transparent rows followed
//! by a translucent black row, repeating.
//!
//! Each test is resilient to GPU absence: if a headless GPU context cannot
//! be created we skip rather than fail. The pixel probes target small
//! patches around row centers to avoid sub pixel sampling jitter at exact
//! tile boundaries.

use unshit_core::element::*;
use unshit_test::TestHarness;

fn try_with_gpu(mut h: TestHarness) -> Option<TestHarness> {
    // This test exercises the quad pipeline directly, so request the
    // adapter's real limits instead of relying on panic-based GPU skipping.
    std::env::set_var("TM_HEADLESS_ADAPTER_LIMITS", "1");
    if h.try_with_gpu() {
        Some(h)
    } else {
        None
    }
}

fn pixel_at(pixels: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let idx = ((y * width + x) * 4) as usize;
    [pixels[idx], pixels[idx + 1], pixels[idx + 2], pixels[idx + 3]]
}

/// Renders a 10 by 30 element filled with a 3 pixel scanline tile and
/// verifies that the visible vertical stripe pattern repeats correctly.
///
/// The 0deg axis runs from the bottom edge to the top edge in CSS terms,
/// so the first stop sits on the bottom row. The pattern per 3 pixel tile
/// is two transparent rows then one translucent black row, all painted on
/// top of a solid white root background. After compositing the expected
/// per row colors are bright white for the transparent rows and a slightly
/// darker shade for the translucent black row.
#[test]
fn repeating_linear_gradient_scanline_tiles_every_three_pixels() {
    let css = r#"
        .root {
            display: flex;
            width: 10px;
            height: 30px;
            background: #ffffff;
            padding: 0;
            margin: 0;
        }
        .scanlines {
            width: 10px;
            height: 30px;
            background-image: repeating-linear-gradient(
                0deg,
                transparent 0,
                transparent 2px,
                rgba(0, 0, 0, 0.12) 2px,
                rgba(0, 0, 0, 0.12) 3px
            );
        }
    "#;

    let tree_fn = || ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("root")
            .with_child(ElementDef::new(Tag::Div).with_class("scanlines")),
    };

    let h = TestHarness::new(css, tree_fn, 10.0, 30.0);
    let Some(mut h) = try_with_gpu(h) else {
        eprintln!("Skipping repeating_linear_gradient_scanline: no GPU available");
        return;
    };
    h.step();
    let pixels = h.render();
    assert_eq!(pixels.len(), 10 * 30 * 4);

    // Sample column 5 (center column) at the centers of every row in the
    // first three tiles. The shader projects 0deg as bottom to top, so
    // pixel rows from the bottom upward see the pattern repeat starting at
    // the bottom edge. Sample three full tiles starting at the bottom for
    // a total of 9 rows of probes.
    let width = 10_u32;
    // Build a per row mean luminance vector across the rendered image so
    // we can spot the periodic dip caused by the translucent rows.
    let mut row_means = [0.0_f32; 30];
    for y in 0..30_u32 {
        let mut sum: u32 = 0;
        for x in 0..width {
            let p = pixel_at(&pixels, width, x, y);
            sum += p[0] as u32 + p[1] as u32 + p[2] as u32;
        }
        row_means[y as usize] = sum as f32 / (width as f32 * 3.0);
    }

    // Find the row with the minimum brightness in each consecutive group
    // of 3 rows. These should land on the translucent rows of the tiles.
    // We expect three repeating darker rows in the 30 row image. To make
    // the test robust to which boundary the bottom edge maps to, we just
    // assert that there exist at least 8 rows where the value is lower
    // than a fully white sample by a margin large enough to attribute it
    // to the translucent overlay (12 percent of 255 is about 30 units).
    let bright_count = row_means.iter().filter(|&&v| v >= 250.0).count();
    let dim_count = row_means.iter().filter(|&&v| v < 245.0).count();
    assert!(
        bright_count >= 18,
        "expected at least 18 fully bright rows in the scanline image, got row means {:?}",
        row_means
    );
    assert!(
        dim_count >= 8,
        "expected at least 8 dimmed rows from the translucent scanline tile, got row means {:?}",
        row_means
    );

    // The dimmed rows should appear at a regular cadence: pick any pair
    // of dimmed rows and assert their distance is a multiple of 3, which
    // is the tile length in pixels. This catches a regression where the
    // shader uses `clamp` instead of `fract` and emits a single dim row
    // followed by a saturated stripe at the bottom.
    let dim_rows: Vec<usize> =
        row_means.iter().enumerate().filter(|(_, &v)| v < 245.0).map(|(i, _)| i).collect();
    assert!(
        dim_rows.len() >= 2,
        "need at least two dimmed rows to verify cadence, got {:?}",
        dim_rows
    );
    let first = dim_rows[0];
    let mut spacings_ok = 0;
    for &row in dim_rows.iter().skip(1) {
        let spacing = row - first;
        if spacing % 3 == 0 {
            spacings_ok += 1;
        }
    }
    assert!(
        spacings_ok >= 1,
        "expected dimmed row spacing to be a multiple of 3 (the tile length), \
         got rows {:?} with row means {:?}",
        dim_rows,
        row_means
    );
}

/// Sanity check that a non repeating linear gradient still renders end to
/// end after the parser and packer changes for issue #128. We use a simple
/// vertical gradient from white at the top to black at the bottom and
/// confirm the top row is bright while the bottom row is dark.
#[test]
fn non_repeating_linear_gradient_still_works() {
    let css = r#"
        .root {
            display: flex;
            width: 10px;
            height: 30px;
            padding: 0;
            margin: 0;
            background: linear-gradient(180deg, #ffffff 0%, #000000 100%);
        }
    "#;

    let tree_fn = || ElementTree { root: ElementDef::new(Tag::Div).with_class("root") };

    let h = TestHarness::new(css, tree_fn, 10.0, 30.0);
    let Some(mut h) = try_with_gpu(h) else {
        eprintln!("Skipping non_repeating_linear_gradient: no GPU available");
        return;
    };
    h.step();
    let pixels = h.render();
    assert_eq!(pixels.len(), 10 * 30 * 4);

    let top = pixel_at(&pixels, 10, 5, 1);
    let bottom = pixel_at(&pixels, 10, 5, 28);
    assert!(
        top[0] > 200 && top[1] > 200 && top[2] > 200,
        "expected bright top of 180deg gradient, got {:?}",
        top
    );
    assert!(
        bottom[0] < 60 && bottom[1] < 60 && bottom[2] < 60,
        "expected dark bottom of 180deg gradient, got {:?}",
        bottom
    );
}

/// Regression: gradient backgrounds used to disappear when the same element
/// had any border side, because the quad shader composited the border over
/// the solid-color slot instead of the resolved gradient color.
#[test]
fn linear_gradient_with_border_keeps_gradient_fill() {
    let css = r#"
        .root {
            display: flex;
            width: 30px;
            height: 20px;
            padding: 0;
            margin: 0;
            background: #101820;
        }
        .chip {
            width: 30px;
            height: 20px;
            background: linear-gradient(135deg, #7dd3fc 0%, #a5f3fc 100%);
            border-bottom: 1px solid #101a25;
        }
    "#;

    let tree_fn = || ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("root")
            .with_child(ElementDef::new(Tag::Div).with_class("chip")),
    };

    let h = TestHarness::new(css, tree_fn, 30.0, 20.0);
    let Some(mut h) = try_with_gpu(h) else {
        eprintln!("Skipping linear_gradient_with_border: no GPU available");
        return;
    };
    h.step();
    let pixels = h.render();

    let middle = pixel_at(&pixels, 30, 15, 8);
    assert!(
        middle[1] > 150 && middle[2] > 180,
        "expected cyan gradient fill above bordered edge, got {:?}",
        middle
    );

    let bottom = pixel_at(&pixels, 30, 15, 19);
    assert!(
        bottom[0] < 60 && bottom[1] < 70 && bottom[2] < 80,
        "expected dark bottom border, got {:?}",
        bottom
    );
}
