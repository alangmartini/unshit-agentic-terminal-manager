//! End to end rendering tests for CSS `radial-gradient` backgrounds.
//!
//! These tests render a small viewport that has a single element styled
//! with a radial gradient, then sample pixels at known positions to verify
//! that the gradient resolves and rasterizes correctly. They are resilient
//! to small antialiasing differences by sampling channels rather than
//! exact RGBA values.
//!
//! In addition to the GPU paths, we cover the pure resolver math via
//! `RadialGradient::resolve`, which lives in unshit-core and does not need
//! a GPU at all. That part runs even on machines without a working WGPU
//! adapter so the parser plus resolver always have coverage.

use unshit_core::element::*;
use unshit_core::style::types::{
    Color, GradientStop, GradientStopPosition, LengthOrPercent, RadialGradient, RadialPosition,
    RadialShape, RadialSize,
};
use unshit_test::TestHarness;

fn try_with_gpu(h: TestHarness) -> Option<TestHarness> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| h.with_gpu())).ok()
}

fn pixel_at(pixels: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let idx = ((y * width + x) * 4) as usize;
    [pixels[idx], pixels[idx + 1], pixels[idx + 2], pixels[idx + 3]]
}

#[test]
fn radial_centered_ellipse_resolves_in_box() {
    // Pure resolver test: 200x100 box, default everything. This is the
    // CSS default radial gradient (ellipse, farthest corner, center). The
    // farthest corner from the center (100, 50) is at (200, 100) with
    // distances (100, 50). Ellipse scales by sqrt(2).
    let g = RadialGradient {
        shape: RadialShape::Ellipse,
        size: RadialSize::FarthestCorner,
        center: RadialPosition::CENTER,
        stops: smallvec::smallvec![
            GradientStop { color: Color::WHITE, position: GradientStopPosition::Percent(0.0) },
            GradientStop {
                color: Color::TRANSPARENT,
                position: GradientStopPosition::Percent(1.0)
            },
        ],
    };
    let r = g.resolve(200.0, 100.0);
    let k = std::f32::consts::SQRT_2;
    assert!((r.center_x - 100.0).abs() < 1e-3);
    assert!((r.center_y - 50.0).abs() < 1e-3);
    assert!((r.rx - 100.0 * k).abs() < 1e-3);
    assert!((r.ry - 50.0 * k).abs() < 1e-3);
    assert_eq!(r.shape, RadialShape::Ellipse);
}

#[test]
fn radial_off_center_circle_resolves_radius_zero_when_on_edge() {
    // Edge case from the issue: a closest-side circle with the center
    // sitting on the box edge collapses to radius zero. The renderer must
    // not divide by zero; the shader collapses to the last stop color.
    let g = RadialGradient {
        shape: RadialShape::Circle,
        size: RadialSize::ClosestSide,
        center: RadialPosition {
            x: LengthOrPercent::Percent(0.0),
            y: LengthOrPercent::Percent(0.5),
        },
        stops: smallvec::smallvec![
            GradientStop { color: Color::WHITE, position: GradientStopPosition::Percent(0.0) },
            GradientStop {
                color: Color::TRANSPARENT,
                position: GradientStopPosition::Percent(1.0)
            },
        ],
    };
    let r = g.resolve(100.0, 100.0);
    // x distance is 0 because the center sits on the left edge.
    assert!((r.center_x - 0.0).abs() < 1e-3);
    assert!((r.rx - 0.0).abs() < 1e-3);
    assert!((r.ry - 0.0).abs() < 1e-3);
}

#[test]
fn radial_center_outside_box_is_not_clamped() {
    // CSS allows centers outside the box. The resolver must not clamp.
    let g = RadialGradient {
        shape: RadialShape::Ellipse,
        size: RadialSize::Explicit { rx: LengthOrPercent::Px(50.0), ry: LengthOrPercent::Px(40.0) },
        center: RadialPosition {
            x: LengthOrPercent::Percent(1.5),
            y: LengthOrPercent::Percent(-0.25),
        },
        stops: smallvec::smallvec![
            GradientStop { color: Color::WHITE, position: GradientStopPosition::Percent(0.0) },
            GradientStop {
                color: Color::TRANSPARENT,
                position: GradientStopPosition::Percent(1.0)
            },
        ],
    };
    let r = g.resolve(100.0, 100.0);
    assert!((r.center_x - 150.0).abs() < 1e-3);
    assert!((r.center_y - -25.0).abs() < 1e-3);
    assert!((r.rx - 50.0).abs() < 1e-3);
    assert!((r.ry - 40.0).abs() < 1e-3);
}

#[test]
fn radial_render_centered_white_to_black_gradient_pixels() {
    // GPU path: build a 100x100 viewport with a centered ellipse that
    // goes from opaque white at the center to opaque black at the edge.
    // The center pixel must be near white, the corner pixels near black,
    // and the alpha channel must be fully opaque everywhere because both
    // stops are opaque.
    let css = r#"
        .root {
            display: flex;
            width: 100%;
            height: 100%;
            margin: 0;
            padding: 0;
            background: #000000;
        }
        .blob {
            width: 100%;
            height: 100%;
            background: radial-gradient(circle, #ffffff, #000000);
        }
    "#;
    let tree_fn = || ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("root")
            .with_child(ElementDef::new(Tag::Div).with_class("blob")),
    };
    let h = TestHarness::new(css, tree_fn, 100.0, 100.0);
    let Some(mut h) = try_with_gpu(h) else {
        eprintln!("Skipping: no GPU available");
        return;
    };
    h.step();
    let pixels = h.render();
    assert_eq!(pixels.len(), 100 * 100 * 4);

    // Center pixel should be near white (high brightness on all channels).
    let center = pixel_at(&pixels, 100, 50, 50);
    assert!(
        center[0] > 200 && center[1] > 200 && center[2] > 200,
        "expected near white center pixel, got {:?}",
        center
    );

    // A corner pixel should be near black. We sample (5, 5) rather than
    // exactly (0, 0) so antialiasing on the rect edge does not muddy the
    // reading.
    let corner = pixel_at(&pixels, 100, 5, 5);
    assert!(
        corner[0] < 80 && corner[1] < 80 && corner[2] < 80,
        "expected near black corner pixel, got {:?}",
        corner
    );
}

#[test]
fn radial_render_terminal_manager_pane_overlay_is_visible_at_top() {
    // Reproduces the .pane::before overlay shape from terminal-manager
    // styles.css line 948: `radial-gradient(ellipse at top, ...)`. The
    // first stop sits at (0, 0%) so the very top of the box should be
    // brighter than the very bottom. We use a higher first stop alpha
    // than the real terminal-manager value (0.5 instead of 0.015) so the
    // GPU output crosses the visible pixel threshold for an assert.
    let css = r#"
        .root {
            display: flex;
            width: 100%;
            height: 100%;
            margin: 0;
            padding: 0;
            background: #000000;
        }
        .pane {
            width: 100%;
            height: 100%;
            background: radial-gradient(ellipse at top, rgba(212, 163, 72, 0.5), transparent 60%);
        }
    "#;
    let tree_fn = || ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("root")
            .with_child(ElementDef::new(Tag::Div).with_class("pane")),
    };
    let h = TestHarness::new(css, tree_fn, 200.0, 100.0);
    let Some(mut h) = try_with_gpu(h) else {
        eprintln!("Skipping: no GPU available");
        return;
    };
    h.step();
    let pixels = h.render();
    assert_eq!(pixels.len(), 200 * 100 * 4);

    // Sample one pixel just below the top, near the horizontal center.
    // The warm tint should be present in the red channel.
    let near_top = pixel_at(&pixels, 200, 100, 5);
    // Sample a pixel near the bottom, same column. The radial fade plus
    // the explicit `transparent 60%` stop means this pixel should land at
    // background black.
    let near_bottom = pixel_at(&pixels, 200, 100, 95);

    assert!(
        near_top[0] >= near_bottom[0],
        "expected the top of the radial overlay to have at least as much red as the bottom: top {:?} bottom {:?}",
        near_top,
        near_bottom
    );
    // Strict inequality: with a 0.5 alpha first stop and 60% transparent
    // tail, the difference must be observable.
    assert!(
        near_top[0] > near_bottom[0] + 8,
        "expected a clear warm tint at the top of the gradient: top {:?} bottom {:?}",
        near_top,
        near_bottom
    );
}
