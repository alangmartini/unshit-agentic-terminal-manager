//! Tests for CSS `transform: translateX(...)` — issue #4.
//!
//! The feature should behave exactly like the CSS spec: layout places the
//! element in the normal flow, then the paint step shifts the element's
//! pixels by the translation amount without affecting siblings.
//!
//! We cover the parser, the cascade plumbing, and the renderer side with
//! pixel level checks. The sibling invariance test is the headline of the
//! acceptance criteria in the issue.

use unshit_core::element::*;
use unshit_core::style::parse::{apply_declaration, StyleDeclaration};
use unshit_core::style::types::{ComputedStyle, TransformX};
use unshit_test::TestHarness;

fn try_with_gpu(h: TestHarness) -> Option<TestHarness> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| h.with_gpu())).ok()
}

fn pixel_at(pixels: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let idx = ((y * width + x) * 4) as usize;
    [pixels[idx], pixels[idx + 1], pixels[idx + 2], pixels[idx + 3]]
}

#[test]
fn parse_translate_x_px_populates_declaration() {
    // Parsing `transform: translateX(50px)` must land on the computed
    // style as `TransformX::Px(50.0)`. This is the smallest possible
    // test of the parser + apply_declaration seam.
    let css = r#"
        .shift { transform: translateX(50px); }
    "#;
    let h = TestHarness::new(
        css,
        || ElementTree { root: ElementDef::new(Tag::Div).with_class("shift") },
        100.0,
        100.0,
    );
    let tx = h
        .query(".shift")
        .expect("shift exists")
        .computed_style
        .transform_translate_x
        .expect("transform parsed");
    assert_eq!(tx, TransformX::Px(50.0));
}

#[test]
fn parse_translate_x_percent_populates_declaration() {
    // CSS translateX accepts percentages of the element's own width. We
    // store them as a unit fraction so `50%` becomes `Percent(0.5)`.
    let css = r#"
        .shift { transform: translateX(50%); }
    "#;
    let h = TestHarness::new(
        css,
        || ElementTree { root: ElementDef::new(Tag::Div).with_class("shift") },
        100.0,
        100.0,
    );
    let tx = h
        .query(".shift")
        .expect("shift exists")
        .computed_style
        .transform_translate_x
        .expect("transform parsed");
    assert_eq!(tx, TransformX::Percent(0.5));
}

#[test]
fn parse_translate_x_negative_px() {
    // Negative translations are valid: they shift the element left.
    let css = r#"
        .shift { transform: translateX(-25px); }
    "#;
    let h = TestHarness::new(
        css,
        || ElementTree { root: ElementDef::new(Tag::Div).with_class("shift") },
        100.0,
        100.0,
    );
    let tx = h
        .query(".shift")
        .expect("shift exists")
        .computed_style
        .transform_translate_x
        .expect("transform parsed");
    assert_eq!(tx, TransformX::Px(-25.0));
}

#[test]
fn transform_x_resolve_resolves_px_and_percent() {
    // Unit test of the TransformX resolver. Pixel values pass through;
    // percentages multiply the element width.
    assert_eq!(TransformX::Px(42.0).resolve(100.0), 42.0);
    assert_eq!(TransformX::Percent(0.5).resolve(200.0), 100.0);
    assert_eq!(TransformX::Percent(-0.25).resolve(80.0), -20.0);
}

#[test]
fn apply_declaration_sets_transform_translate_x() {
    // apply_declaration wires the parsed StyleDeclaration into the
    // computed style. A fresh style starts with `None` and must come out
    // as `Some(Px(10))` after application.
    let mut style = ComputedStyle::default();
    assert!(style.transform_translate_x.is_none());
    apply_declaration(
        &mut style,
        &StyleDeclaration::TransformTranslateX(TransformX::Px(10.0)),
    );
    assert_eq!(style.transform_translate_x, Some(TransformX::Px(10.0)));
}

#[test]
fn parse_unsupported_transform_function_drops_declaration() {
    // `transform: scale(2)` is not supported today. The declaration is
    // dropped rather than panicking, so other declarations on the same
    // selector still apply.
    let css = r#"
        .unsupported { color: #ff0000; transform: scale(2); }
    "#;
    let h = TestHarness::new(
        css,
        || ElementTree { root: ElementDef::new(Tag::Div).with_class("unsupported") },
        100.0,
        100.0,
    );
    let style = h.query(".unsupported").expect("element").computed_style;
    assert!(style.transform_translate_x.is_none());
    // Color should still be red because apply_declaration keeps running
    // even though the transform entry failed.
    assert_eq!(style.color.r, 255);
}

#[test]
fn translate_x_does_not_shift_sibling_layout() {
    // Two absolutely positioned divs, both anchored at `left: 20px`.
    // One has `transform: translateX(50px)`. Because transforms do not
    // participate in flow layout, the transformed element's LayoutRect
    // must still reflect the pre transform position. The shift only
    // appears in paint. Siblings must be unaffected.

    let css = r#"
        .row {
            display: flex;
            position: relative;
            width: 300px;
            height: 50px;
            margin: 0;
            padding: 0;
        }
        .a {
            position: absolute;
            left: 20px;
            top: 0;
            width: 30px;
            height: 30px;
            transform: translateX(50px);
            background: #ff0000;
        }
        .b {
            position: absolute;
            left: 20px;
            top: 30px;
            width: 30px;
            height: 20px;
            background: #00ff00;
        }
    "#;

    let tree_fn = || ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("row")
            .with_child(ElementDef::new(Tag::Div).with_class("a"))
            .with_child(ElementDef::new(Tag::Div).with_class("b")),
    };
    let h = TestHarness::new(css, tree_fn, 300.0, 50.0);

    let a = h.query(".a").expect(".a exists");
    let b = h.query(".b").expect(".b exists");

    // Both rects come straight from taffy so the translation does not
    // affect either one's layout x position.
    assert!(
        (a.layout_rect.x - 20.0).abs() < 0.5,
        "transformed element's layout_rect.x should still be the pre-transform 20px, got {}",
        a.layout_rect.x,
    );
    assert!(
        (b.layout_rect.x - 20.0).abs() < 0.5,
        "sibling layout_rect.x unchanged at 20px, got {}",
        b.layout_rect.x,
    );
    // The transform parse must have landed on `.a` but not on `.b`.
    assert_eq!(a.computed_style.transform_translate_x, Some(TransformX::Px(50.0)));
    assert_eq!(b.computed_style.transform_translate_x, None);
}

#[test]
fn translate_x_renders_at_shifted_position() {
    // GPU render test: a red rectangle placed at `left: 20px` with
    // `width: 30px` and `transform: translateX(50px)`. Without the
    // transform the rect would paint at x in [20, 50). With the
    // transform applied it should paint at x in [70, 100).
    //
    // We assert the paint by sampling pixel columns: pixels at x=30
    // (inside the pre transform rect) should be the page background;
    // pixels at x=80 (inside the post transform rect) should be red.

    let css = r#"
        .page {
            width: 100%;
            height: 100%;
            background: #000000;
            position: relative;
        }
        .box {
            position: absolute;
            left: 20px;
            top: 10px;
            width: 30px;
            height: 30px;
            background: #ff0000;
            transform: translateX(50px);
        }
    "#;

    let tree_fn = || ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("page")
            .with_child(ElementDef::new(Tag::Div).with_class("box")),
    };
    let h = TestHarness::new(css, tree_fn, 150.0, 60.0);
    let Some(mut h) = try_with_gpu(h) else {
        eprintln!("Skipping: no GPU available");
        return;
    };

    h.step();
    let pixels = h.render();
    assert_eq!(pixels.len(), (150 * 60 * 4) as usize);

    // Pixel at x=30, y=25 — inside where the rect WOULD have been without
    // a transform. This should now be the black page background.
    let pre = pixel_at(&pixels, 150, 30, 25);
    assert!(
        pre[0] < 40 && pre[1] < 40 && pre[2] < 40,
        "expected black at pre transform position, got {:?}",
        pre
    );

    // Pixel at x=80, y=25 — inside where the rect paints AFTER the
    // transform: translateX(50px) shift (rect starts at 20+50=70, width 30).
    let post = pixel_at(&pixels, 150, 80, 25);
    assert!(
        post[0] > 200 && post[1] < 50 && post[2] < 50,
        "expected red at post transform position, got {:?}",
        post
    );
}
