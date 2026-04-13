//! End to end rendering tests for inline SVG.
//!
//! These tests use a headless GPU context to render a small viewport
//! containing inline SVG primitives, then inspect the RGBA pixel output to
//! confirm that fills and strokes land where expected. They are resilient
//! to exact pixel jitter (antialiasing, tessellation tolerance) by sampling
//! a small inner patch rather than comparing full images.

use unshit_core::element::*;
use unshit_core::style::types::Color;
use unshit_core::svg::types::{SvgAttrs, SvgNode, SvgPaint, SvgPrimitive, ViewBox};
use unshit_test::TestHarness;

fn try_with_gpu(h: TestHarness) -> Option<TestHarness> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| h.with_gpu())).ok()
}

/// Read one RGBA pixel at (x, y) out of a tightly packed row major buffer.
fn pixel_at(pixels: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let idx = ((y * width + x) * 4) as usize;
    [pixels[idx], pixels[idx + 1], pixels[idx + 2], pixels[idx + 3]]
}

fn attrs_fill(color: Color) -> SvgAttrs {
    SvgAttrs { fill: Some(SvgPaint::Solid(color)), ..Default::default() }
}

fn attrs_fill_and_stroke(fill: Color, stroke: Color, stroke_width: f32) -> SvgAttrs {
    SvgAttrs {
        fill: Some(SvgPaint::Solid(fill)),
        stroke: Some(SvgPaint::Solid(stroke)),
        stroke_width: Some(stroke_width),
        ..Default::default()
    }
}

#[test]
fn renders_filled_circle_and_path() {
    // 64x64 viewport with a red filled circle plus a blue square path.
    // The inline SVG uses viewBox 0 0 64 64 so user units map 1 to 1.
    let root_attrs =
        SvgAttrs { view_box: Some(ViewBox::new(0.0, 0.0, 64.0, 64.0)), ..Default::default() };

    let red_circle = SvgNode {
        primitive: SvgPrimitive::Circle { cx: 16.0, cy: 32.0, r: 10.0 },
        attrs: attrs_fill_and_stroke(Color::rgb(255, 0, 0), Color::BLACK, 2.0),
        children: Vec::new(),
    };

    let blue_square = SvgNode {
        primitive: SvgPrimitive::Rect {
            x: 40.0,
            y: 22.0,
            width: 16.0,
            height: 16.0,
            rx: 0.0,
            ry: 0.0,
        },
        attrs: attrs_fill(Color::rgb(0, 0, 255)),
        children: Vec::new(),
    };

    let svg_root = SvgNode {
        primitive: SvgPrimitive::Group,
        attrs: root_attrs,
        children: vec![red_circle, blue_square],
    };

    let css = r#"
        .root {
            display: flex;
            width: 100%;
            height: 100%;
            padding: 0;
            margin: 0;
            background: #000000;
        }
        .icon { width: 64px; height: 64px; }
    "#;

    let tree_fn = move || ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("root")
            .with_child(ElementDef::new(Tag::Div).with_class("icon").with_svg(svg_root.clone())),
    };

    let h = TestHarness::new(css, tree_fn, 64.0, 64.0);
    let Some(mut h) = try_with_gpu(h) else {
        eprintln!("Skipping: no GPU available");
        return;
    };
    h.step();
    let pixels = h.render();
    assert_eq!(pixels.len(), 64 * 64 * 4);

    // Center of the red circle: around (16, 32) in viewBox, which is (16, 32)
    // in the target viewport because the viewBox matches pixel size.
    let red = pixel_at(&pixels, 64, 16, 32);
    assert!(red[0] > 180, "expected red channel dominant in circle center, got {:?}", red);
    assert!(red[2] < 80, "expected low blue in circle center, got {:?}", red);

    // Center of the blue square at around (48, 30).
    let blue = pixel_at(&pixels, 64, 48, 30);
    assert!(blue[2] > 180, "expected blue channel dominant in square center, got {:?}", blue);
    assert!(blue[0] < 80, "expected low red in square center, got {:?}", blue);

    // A corner that should have no SVG drawn: (2, 2). Expect the dark
    // background (either black clear color or whatever the default
    // GpuContext clear is).
    let empty = pixel_at(&pixels, 64, 2, 2);
    assert!(
        empty[0] < 100 && empty[1] < 100 && empty[2] < 100,
        "expected dark background pixel at top left corner, got {:?}",
        empty
    );
}

#[test]
fn cache_hit_reuses_geometry_across_draws() {
    // Two identical circles in a group should both hit the cache after the
    // first tessellation. We can only observe the effect indirectly via the
    // cache length, so we inspect it after the frame is built.
    let inner = SvgNode {
        primitive: SvgPrimitive::Circle { cx: 8.0, cy: 8.0, r: 4.0 },
        attrs: attrs_fill(Color::rgb(0, 200, 0)),
        children: Vec::new(),
    };
    let another = SvgNode {
        primitive: SvgPrimitive::Circle { cx: 24.0, cy: 8.0, r: 4.0 },
        attrs: attrs_fill(Color::rgb(0, 200, 0)),
        children: Vec::new(),
    };
    let svg_root = SvgNode {
        primitive: SvgPrimitive::Group,
        attrs: SvgAttrs {
            view_box: Some(ViewBox::new(0.0, 0.0, 32.0, 16.0)),
            ..Default::default()
        },
        children: vec![inner, another],
    };

    let css = r#"
        .root { display: flex; width: 100%; height: 100%; background: #000; }
        .icon { width: 32px; height: 16px; }
    "#;

    let tree_fn = move || ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("root")
            .with_child(ElementDef::new(Tag::Div).with_class("icon").with_svg(svg_root.clone())),
    };

    let h = TestHarness::new(css, tree_fn, 32.0, 16.0);
    let Some(mut h) = try_with_gpu(h) else {
        eprintln!("Skipping: no GPU available");
        return;
    };
    h.step();
    let _ = h.render();
    // Both circles are identical so the cache should hold exactly one entry.
    assert_eq!(h.gpu_ref().svg_cache.len(), 1);
}

#[test]
fn hand_rolled_path_round_trips_through_parser() {
    // Build a path from the SVG mini language and make sure the tessellator
    // produces something drawable.
    use unshit_core::svg::path_parser::parse_svg_path;
    let d = "M 10 10 L 30 10 L 30 30 L 10 30 Z";
    let commands = parse_svg_path(d).unwrap();
    let node = SvgNode {
        primitive: SvgPrimitive::Path { d: d.to_string(), commands },
        attrs: attrs_fill(Color::rgb(200, 200, 0)),
        children: Vec::new(),
    };
    let svg_root = SvgNode {
        primitive: SvgPrimitive::Group,
        attrs: SvgAttrs {
            view_box: Some(ViewBox::new(0.0, 0.0, 40.0, 40.0)),
            ..Default::default()
        },
        children: vec![node],
    };

    let css = r#"
        .root { display: flex; width: 100%; height: 100%; background: #000; }
        .icon { width: 40px; height: 40px; }
    "#;

    let tree_fn = move || ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("root")
            .with_child(ElementDef::new(Tag::Div).with_class("icon").with_svg(svg_root.clone())),
    };

    let h = TestHarness::new(css, tree_fn, 40.0, 40.0);
    let Some(mut h) = try_with_gpu(h) else {
        eprintln!("Skipping: no GPU available");
        return;
    };
    h.step();
    let pixels = h.render();
    // Center of the yellow square.
    let yellow = pixel_at(&pixels, 40, 20, 20);
    assert!(yellow[0] > 120 && yellow[1] > 120, "expected yellow center pixel, got {:?}", yellow);
    assert!(yellow[2] < 100, "expected low blue in yellow pixel, got {:?}", yellow);
}
