//! Regression test: SVGs must remain visible when a sibling element with
//! `backdrop-filter` is in the same tree.
//!
//! User-reported bug: opening the settings modal (which has
//! `backdrop-filter: blur(6px)`) makes ALL SVGs in the app disappear and
//! reappear in a rhythmic pulse, including SVGs not inside the modal.
//!
//! Suspected root cause: the GPU backdrop render path at
//! `crates/unshit-framework/crates/unshit-renderer/src/gpu.rs` mixes
//! pipelines. When MSAA is enabled (production), it sets
//! `backdrop_svg_pipeline.pipeline` on the render pass but draws via
//! `self.svg_pipeline.draw(...)`, which sets bind group 1 from
//! `svg_pipeline.current_instance_bind_group`. That bind group was built
//! against `svg_pipeline.instance_bind_group_layout`, a distinct handle
//! from `backdrop_svg_pipeline.instance_bind_group_layout` even though
//! the layouts are structurally identical. This is fragile and is the
//! prime suspect for the disappearing SVGs.
//!
//! This test exercises the contract: with a backdrop-filter overlay
//! present, an SVG drawn earlier in the tree still produces colored
//! pixels at its position. If the SVG draws fail under the backdrop
//! path, the pixels at the SVG location collapse to the overlay
//! background and the test fails.

use unshit_core::element::*;
use unshit_core::style::types::Color;
use unshit_core::svg::types::{SvgAttrs, SvgNode, SvgPaint, SvgPrimitive, ViewBox};
use unshit_test::TestHarness;

fn try_with_gpu(h: TestHarness) -> Option<TestHarness> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| h.with_gpu())).ok()
}

fn pixel_at(pixels: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let idx = ((y * width + x) * 4) as usize;
    [pixels[idx], pixels[idx + 1], pixels[idx + 2], pixels[idx + 3]]
}

/// A solid red filled circle SVG. Easy to detect in pixel output.
fn red_circle_svg() -> SvgNode {
    let circle = SvgNode {
        primitive: SvgPrimitive::Circle { cx: 16.0, cy: 16.0, r: 12.0 },
        attrs: SvgAttrs {
            fill: Some(SvgPaint::Solid(Color::rgb(255, 0, 0))),
            ..Default::default()
        },
        children: Vec::new(),
    };
    SvgNode {
        primitive: SvgPrimitive::Group,
        attrs: SvgAttrs {
            view_box: Some(ViewBox::new(0.0, 0.0, 32.0, 32.0)),
            ..Default::default()
        },
        children: vec![circle],
    }
}

const CSS_NO_OVERLAY: &str = r#"
    .root {
        display: flex;
        width: 200px;
        height: 200px;
        background: #000000;
    }
    .icon { width: 32px; height: 32px; }
"#;

const CSS_WITH_BACKDROP_OVERLAY: &str = r#"
    .root {
        display: flex;
        width: 200px;
        height: 200px;
        background: #000000;
        position: relative;
    }
    .icon { width: 32px; height: 32px; }
    .modal-overlay {
        position: absolute;
        top: 0;
        left: 0;
        right: 0;
        bottom: 0;
        background: rgba(20, 20, 30, 0.4);
        backdrop-filter: blur(6px);
    }
"#;

fn tree_with_icon_only() -> ElementTree {
    let icon = ElementDef::new(Tag::Div).with_class("icon").with_svg(red_circle_svg());
    ElementTree { root: ElementDef::new(Tag::Div).with_class("root").with_child(icon) }
}

fn tree_with_icon_and_overlay() -> ElementTree {
    let icon = ElementDef::new(Tag::Div).with_class("icon").with_svg(red_circle_svg());
    let overlay = ElementDef::new(Tag::Div).with_class("modal-overlay");
    ElementTree {
        root: ElementDef::new(Tag::Div).with_class("root").with_child(icon).with_child(overlay),
    }
}

/// Baseline: without any backdrop-filter overlay, the red circle is clearly
/// visible. This proves the SVG renders correctly through the simple
/// (non-backdrop) render path.
#[test]
fn red_circle_visible_without_overlay() {
    let h = TestHarness::new(CSS_NO_OVERLAY, tree_with_icon_only, 200.0, 200.0);
    let Some(mut h) = try_with_gpu(h) else {
        eprintln!("Skipping: no GPU available");
        return;
    };
    h.step();
    let pixels = h.render();
    let center = pixel_at(&pixels, 200, 16, 16);
    assert!(
        center[0] > 180,
        "baseline: SVG circle should be visible (red dominant) without overlay; got {:?}",
        center
    );
}

/// The bug repro: with a sibling `backdrop-filter: blur(6px)` overlay covering
/// the whole viewport, the SVG circle is drawn first (earlier in the tree)
/// and the overlay is composited on top. Because the overlay is only 40 pct
/// opaque, the red circle should remain visibly red (with some darkening
/// from the overlay tint and some blurring from the filter).
///
/// If the SVG draws fail under the backdrop render path, the pixel at the
/// circle center collapses to the overlay background and the red channel
/// drops below the visibility threshold.
#[test]
fn red_circle_visible_with_sibling_backdrop_overlay() {
    let h = TestHarness::new(CSS_WITH_BACKDROP_OVERLAY, tree_with_icon_and_overlay, 200.0, 200.0);
    let Some(mut h) = try_with_gpu(h) else {
        eprintln!("Skipping: no GPU available");
        return;
    };
    h.step();
    let pixels = h.render();

    {
        let gpu = h.gpu_ref();
        assert!(
            gpu.backdrop_blur_pipeline.is_some(),
            "test must exercise the backdrop path; pipeline was not allocated"
        );
    }

    let center = pixel_at(&pixels, 200, 16, 16);
    assert!(
        center[0] > 60,
        "BUG: SVG circle disappeared under sibling backdrop-filter overlay; \
         expected red channel > 60 (visible through 40 pct opaque overlay), \
         got {:?}. The overlay-only color would be roughly (8, 8, 12).",
        center
    );
}

/// The pulse repro: simulate the cursor-blink subscription by rebuilding the
/// tree multiple times and rendering each frame. The SVG visibility must be
/// stable across rebuilds; if it pulses on/off, the rebuild path and the
/// no-rebuild path treat the SVG draws differently.
#[test]
fn red_circle_stays_visible_across_repeated_rebuilds() {
    let h = TestHarness::new(CSS_WITH_BACKDROP_OVERLAY, tree_with_icon_and_overlay, 200.0, 200.0);
    let Some(mut h) = try_with_gpu(h) else {
        eprintln!("Skipping: no GPU available");
        return;
    };

    let mut visibility_samples: Vec<u8> = Vec::new();
    for _ in 0..6 {
        h.rebuild(tree_with_icon_and_overlay);
        h.step();
        let pixels = h.render();
        visibility_samples.push(pixel_at(&pixels, 200, 16, 16)[0]);
    }

    let min = *visibility_samples.iter().min().unwrap();
    let max = *visibility_samples.iter().max().unwrap();
    assert!(
        max - min < 30,
        "BUG: SVG circle pulses across rebuilds; red channel samples = {:?} \
         (min={}, max={}, spread={})",
        visibility_samples,
        min,
        max,
        max - min
    );
    assert!(
        min > 60,
        "BUG: SVG circle disappears on at least one rebuild; samples = {:?}",
        visibility_samples
    );
}
