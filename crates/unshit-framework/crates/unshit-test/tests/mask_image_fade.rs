//! Tests for CSS `mask-image: linear-gradient(...)` — issue #5.
//!
//! The feature paints a rectangular element, then multiplies its output
//! alpha by the alpha channel of a linear gradient, so the rect fades out
//! toward the edges. The tests cover:
//!
//! 1. The parser extracting a `LinearGradient` from the `mask-image`
//!    declaration.
//! 2. The value landing on `ComputedStyle::mask_image` after cascade.
//! 3. Pixel level correctness: for `mask-image: linear-gradient(to right,
//!    transparent, #000)` the left edge must be near zero alpha and the
//!    right edge near full alpha.

use unshit_core::element::*;
use unshit_core::style::types::GradientStopPosition;
use unshit_test::TestHarness;

fn try_with_gpu(h: TestHarness) -> Option<TestHarness> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| h.with_gpu())).ok()
}

fn pixel_at(pixels: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let idx = ((y * width + x) * 4) as usize;
    [pixels[idx], pixels[idx + 1], pixels[idx + 2], pixels[idx + 3]]
}

#[test]
fn parse_mask_image_linear_gradient_to_right() {
    // The parser must accept the CSS Images Level 3 `to right` keyword
    // and translate it into a 90deg angle. Stops survive intact.
    let css = r#"
        .t { mask-image: linear-gradient(to right, transparent, #000000); }
    "#;
    let h = TestHarness::new(
        css,
        || ElementTree { root: ElementDef::new(Tag::Div).with_class("t") },
        100.0,
        100.0,
    );
    let mask = h.query(".t").expect("element").computed_style.mask_image.expect("mask-image parsed");
    assert!((mask.angle_deg - 90.0).abs() < 0.01, "expected 90deg, got {}", mask.angle_deg);
    assert_eq!(mask.stops.len(), 2);
    // The first stop is `transparent` (alpha 0), the last is `#000000`
    // (solid black, alpha 255).
    assert_eq!(mask.stops[0].color.a, 0);
    assert_eq!(mask.stops[1].color.a, 255);
}

#[test]
fn parse_mask_image_with_explicit_percent_stops() {
    // The canonical edge fade pattern uses four stops with explicit
    // percentages. All four stops must be preserved.
    let css = r#"
        .t {
            mask-image: linear-gradient(
                to right,
                transparent,
                #000000 10%,
                #000000 90%,
                transparent
            );
        }
    "#;
    let h = TestHarness::new(
        css,
        || ElementTree { root: ElementDef::new(Tag::Div).with_class("t") },
        100.0,
        100.0,
    );
    let mask = h
        .query(".t")
        .expect("element")
        .computed_style
        .mask_image
        .expect("mask-image parsed");
    assert_eq!(mask.stops.len(), 4);
    // Verify the middle two stops sit at 10% and 90%.
    match mask.stops[1].position {
        GradientStopPosition::Percent(p) => assert!((p - 0.10).abs() < 0.001),
        other => panic!("expected percent, got {:?}", other),
    }
    match mask.stops[2].position {
        GradientStopPosition::Percent(p) => assert!((p - 0.90).abs() < 0.001),
        other => panic!("expected percent, got {:?}", other),
    }
}

#[test]
fn parse_mask_image_with_angle_in_degrees() {
    // CSS Images Level 3 also allows angle syntax. Verify a 45deg
    // diagonal mask parses without errors.
    let css = r#"
        .t { mask-image: linear-gradient(45deg, transparent, #000000); }
    "#;
    let h = TestHarness::new(
        css,
        || ElementTree { root: ElementDef::new(Tag::Div).with_class("t") },
        100.0,
        100.0,
    );
    let mask = h
        .query(".t")
        .expect("element")
        .computed_style
        .mask_image
        .expect("mask-image parsed");
    assert!((mask.angle_deg - 45.0).abs() < 0.01, "expected 45deg, got {}", mask.angle_deg);
}

#[test]
fn default_mask_image_is_none() {
    // Absence of `mask-image` in CSS must leave the computed style's
    // mask slot as `None` so the renderer stays on the fast path.
    let css = r#"
        .t { background: #000000; }
    "#;
    let h = TestHarness::new(
        css,
        || ElementTree { root: ElementDef::new(Tag::Div).with_class("t") },
        100.0,
        100.0,
    );
    let style = h.query(".t").expect("element").computed_style;
    assert!(style.mask_image.is_none());
}

#[test]
fn mask_image_edge_fade_renders_correctly() {
    // Pixel level: a 100px wide black rectangle with
    // `mask-image: linear-gradient(to right, transparent, #000)`.
    // The leftmost pixel should be fully transparent (alpha ~0) and the
    // rightmost pixel should be fully opaque (alpha ~255). Pixels in
    // between should be gradually rising alpha values.
    //
    // The background of the page is a distinctive color so we can tell
    // whether the rectangle's pixels got overlaid. The blend mode is
    // premultiplied alpha so a transparent fragment leaves the page
    // color untouched.

    let css = r#"
        .page {
            width: 100%;
            height: 100%;
            background: #ffffff;
            margin: 0;
            padding: 0;
        }
        .fade {
            width: 100%;
            height: 100%;
            background: #000000;
            mask-image: linear-gradient(to right, transparent, #000000);
        }
    "#;

    let tree_fn = || ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("page")
            .with_child(ElementDef::new(Tag::Div).with_class("fade")),
    };
    let h = TestHarness::new(css, tree_fn, 100.0, 20.0);
    let Some(mut h) = try_with_gpu(h) else {
        eprintln!("Skipping: no GPU available");
        return;
    };
    h.step();
    let pixels = h.render();
    assert_eq!(pixels.len(), (100 * 20 * 4) as usize);

    // Leftmost pixel (x = 2, mid height) should show the white page
    // background because the mask alpha at this position is ~0.
    let left = pixel_at(&pixels, 100, 2, 10);
    assert!(
        left[0] > 200 && left[1] > 200 && left[2] > 200,
        "expected white (page background visible at masked edge), got {:?}",
        left
    );

    // Rightmost pixel (x = 97, mid height) should show the black
    // rectangle because the mask alpha is ~1 there.
    let right = pixel_at(&pixels, 100, 97, 10);
    assert!(
        right[0] < 40 && right[1] < 40 && right[2] < 40,
        "expected black (mask fully solid at right edge), got {:?}",
        right
    );

    // A pixel near the right side should be darker than one near the left.
    // This holds even if the first two assertions are loose on a soft
    // skip: the gradient fade must produce monotonically increasing
    // "blackness" from left to right.
    assert!(
        (left[0] as i32) > (right[0] as i32) + 100,
        "expected left pixel to be much brighter than right pixel (mask fade), \
         got left={:?}, right={:?}",
        left,
        right
    );
}
