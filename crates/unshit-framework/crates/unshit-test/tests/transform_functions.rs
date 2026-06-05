//! Tests for the full CSS `transform` function list — `scale`, `rotate`,
//! `translateY`/`translate`, the combined `translateY(..) scale(..)` form, and
//! `none`. The `translateX`-only behavior (and sibling-layout invariance) is
//! covered by `transform_translate_x.rs`; this file covers everything the app
//! stylesheet additionally authors, at both the parse and the paint layer.

use std::f32::consts::PI;

use unshit_core::element::*;
use unshit_core::style::types::{Transform, TransformX};
use unshit_test::TestHarness;

fn try_with_gpu(h: TestHarness) -> Option<TestHarness> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| h.with_gpu())).ok()
}

fn pixel_at(pixels: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let idx = ((y * width + x) * 4) as usize;
    [pixels[idx], pixels[idx + 1], pixels[idx + 2], pixels[idx + 3]]
}

fn transform_of(css: &str, selector: &str) -> Transform {
    let h = TestHarness::new(
        css,
        || ElementTree { root: ElementDef::new(Tag::Div).with_class("x") },
        100.0,
        100.0,
    );
    h.query(selector).expect("element exists").computed_style.transform
}

#[test]
fn parse_uniform_scale() {
    let t = transform_of(".x { transform: scale(0.5); }", ".x");
    assert_eq!(t.scale_x, 0.5);
    assert_eq!(t.scale_y, 0.5);
    assert!(t.translate_x.is_none() && t.translate_y.is_none());
    assert_eq!(t.rotate, 0.0);
}

#[test]
fn parse_non_uniform_scale() {
    let t = transform_of(".x { transform: scale(2, 3); }", ".x");
    assert_eq!(t.scale_x, 2.0);
    assert_eq!(t.scale_y, 3.0);
}

#[test]
fn parse_scale_x_and_y_axis_functions() {
    let tx = transform_of(".x { transform: scaleX(1.5); }", ".x");
    assert_eq!(tx.scale_x, 1.5);
    assert_eq!(tx.scale_y, 1.0);

    let ty = transform_of(".x { transform: scaleY(0.25); }", ".x");
    assert_eq!(ty.scale_x, 1.0);
    assert_eq!(ty.scale_y, 0.25);
}

#[test]
fn parse_rotate_degrees_to_radians() {
    let t = transform_of(".x { transform: rotate(90deg); }", ".x");
    assert!((t.rotate - PI / 2.0).abs() < 1e-5, "rotate was {}", t.rotate);
    // Negative rotation (the chevron uses -90deg).
    let neg = transform_of(".x { transform: rotate(-90deg); }", ".x");
    assert!((neg.rotate + PI / 2.0).abs() < 1e-5, "rotate was {}", neg.rotate);
}

#[test]
fn parse_translate_y_and_two_arg_translate() {
    let ty = transform_of(".x { transform: translateY(10px); }", ".x");
    assert_eq!(ty.translate_y, Some(TransformX::Px(10.0)));
    assert!(ty.translate_x.is_none());

    let two = transform_of(".x { transform: translate(4px, -8px); }", ".x");
    assert_eq!(two.translate_x, Some(TransformX::Px(4.0)));
    assert_eq!(two.translate_y, Some(TransformX::Px(-8.0)));
}

#[test]
fn parse_combined_translate_and_scale() {
    // The modal-in keyframe authors `translateY(-12px) scale(0.98)`.
    let t = transform_of(".x { transform: translateY(-12px) scale(0.98); }", ".x");
    assert_eq!(t.translate_y, Some(TransformX::Px(-12.0)));
    assert_eq!(t.scale_x, 0.98);
    assert_eq!(t.scale_y, 0.98);
}

#[test]
fn parse_none_is_identity() {
    let t = transform_of(".x { transform: none; }", ".x");
    assert!(t.is_identity());
}

#[test]
fn scale_down_renders_smaller_box_centered() {
    // A 40x40 red box at left/top 20, so it spans [20,60) and its center is
    // (40, 40). `scale(0.5)` about the (default) center shrinks it to 20x20
    // spanning [30, 50). So a point at x=25 (inside the un-scaled box, outside
    // the scaled one) becomes background, while the center stays red.
    let css = r#"
        .page { width: 100%; height: 100%; background: #000000; position: relative; }
        .box {
            position: absolute;
            left: 20px; top: 20px; width: 40px; height: 40px;
            background: #ff0000;
            transform: scale(0.5);
        }
    "#;
    let tree_fn = || ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("page")
            .with_child(ElementDef::new(Tag::Div).with_class("box")),
    };
    let h = TestHarness::new(css, tree_fn, 80.0, 80.0);
    let Some(mut h) = try_with_gpu(h) else {
        eprintln!("Skipping: no GPU available");
        return;
    };
    h.step();
    let pixels = h.render();

    // Center of the box stays red.
    let center = pixel_at(&pixels, 80, 40, 40);
    assert!(
        center[0] > 200 && center[1] < 50 && center[2] < 50,
        "expected red at the box center after scale, got {:?}",
        center
    );
    // A point near the original left edge (x=24) is now outside the shrunk box.
    let edge = pixel_at(&pixels, 80, 24, 40);
    assert!(
        edge[0] < 40 && edge[1] < 40 && edge[2] < 40,
        "expected background just inside the un-scaled left edge after scale(0.5), got {:?}",
        edge
    );
}

#[test]
fn translate_y_renders_at_shifted_position() {
    // A red box at top 10, height 20 (spans y in [10,30)). `translateY(30px)`
    // moves it to y in [40,60). So y=20 becomes background and y=50 becomes red.
    let css = r#"
        .page { width: 100%; height: 100%; background: #000000; position: relative; }
        .box {
            position: absolute;
            left: 20px; top: 10px; width: 30px; height: 20px;
            background: #ff0000;
            transform: translateY(30px);
        }
    "#;
    let tree_fn = || ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("page")
            .with_child(ElementDef::new(Tag::Div).with_class("box")),
    };
    let h = TestHarness::new(css, tree_fn, 80.0, 80.0);
    let Some(mut h) = try_with_gpu(h) else {
        eprintln!("Skipping: no GPU available");
        return;
    };
    h.step();
    let pixels = h.render();

    let pre = pixel_at(&pixels, 80, 30, 20);
    assert!(
        pre[0] < 40 && pre[1] < 40 && pre[2] < 40,
        "expected background at the pre-transform position, got {:?}",
        pre
    );
    let post = pixel_at(&pixels, 80, 30, 50);
    assert!(
        post[0] > 200 && post[1] < 50 && post[2] < 50,
        "expected red at the translateY(30px) position, got {:?}",
        post
    );
}

#[test]
fn scale_propagates_to_child_subtree() {
    // A transform establishes a coordinate system for the whole subtree: a
    // scaled parent must scale its child. The parent is a 40x40 box at [20,60);
    // its child fills it with a different color. Under `scale(0.5)` about the
    // center (40,40) the child shrinks to [30,50) too, so x=24 is background.
    let css = r#"
        .page { width: 100%; height: 100%; background: #000000; position: relative; }
        .parent {
            position: absolute;
            left: 20px; top: 20px; width: 40px; height: 40px;
            background: #003300;
            transform: scale(0.5);
        }
        .child { width: 100%; height: 100%; background: #00ff00; }
    "#;
    let tree_fn = || ElementTree {
        root: ElementDef::new(Tag::Div).with_class("page").with_child(
            ElementDef::new(Tag::Div)
                .with_class("parent")
                .with_child(ElementDef::new(Tag::Div).with_class("child")),
        ),
    };
    let h = TestHarness::new(css, tree_fn, 80.0, 80.0);
    let Some(mut h) = try_with_gpu(h) else {
        eprintln!("Skipping: no GPU available");
        return;
    };
    h.step();
    let pixels = h.render();

    // Child (green) is visible at the scaled center.
    let center = pixel_at(&pixels, 80, 40, 40);
    assert!(
        center[1] > 200 && center[0] < 50,
        "expected the green child at center after the parent scales, got {:?}",
        center
    );
    // The child does not paint outside the shrunk parent box.
    let edge = pixel_at(&pixels, 80, 24, 40);
    assert!(
        edge[0] < 40 && edge[1] < 40 && edge[2] < 40,
        "expected background outside the scaled subtree, got {:?}",
        edge
    );
}
