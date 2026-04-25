//! Isolation repro for the missing corner-frame decoration in the
//! Organiza Nota landing example. The frame pattern is a decorative
//! rectangle inset from the viewport edges with thin left/right
//! borders, thin top/bottom horizontal lines, and 6x6 tick squares in
//! each corner. None of it was rendering.
//!
//! Two framework gaps feed the bug:
//!
//! 1. `with_class("a b")` pushed the raw string `"a b"` as a single
//!    class, so neither `.a` nor `.b` selectors matched the element.
//!    HTML's `class="a b"` attribute behavior splits on whitespace.
//!
//! 2. The per-side border-width longhands (`border-left-width`,
//!    `border-right-width`, `border-top-width`, `border-bottom-width`)
//!    were not parsed at all. Only the `border-width` shorthand worked.
//!    The landing CSS uses just the left/right longhands on `.frame`
//!    and just the top longhand on `.backed`, so those borders never
//!    rendered.
//!
//! This test asserts the fixed behavior for both gaps in isolation.

use unshit_core::element::*;
use unshit_test::TestHarness;

fn try_with_gpu(h: TestHarness) -> Option<TestHarness> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| h.with_gpu())).ok()
}

fn pixel_at(pixels: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let idx = ((y * width + x) * 4) as usize;
    [pixels[idx], pixels[idx + 1], pixels[idx + 2], pixels[idx + 3]]
}

#[test]
fn with_class_splits_on_whitespace_and_matches_each_class() {
    // A single element built with `with_class("a b")` should match
    // both `.a` and `.b` rules. We use one rule to paint the background
    // and another to paint a left border; if class-splitting works,
    // both apply to the same element.
    let css = r#"
        .viewport {
            position: relative;
            width: 100px;
            height: 100px;
            background: #000000;
        }
        .box {
            position: absolute;
            top: 10px;
            left: 10px;
            width: 80px;
            height: 80px;
            background: rgb(50, 50, 200);
        }
        .variant {
            border-left-width: 4px;
            border-color: rgb(255, 200, 0);
        }
    "#;

    let tree_fn = || ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("viewport")
            .with_child(ElementDef::new(Tag::Div).with_class("box variant")),
    };

    let h = TestHarness::new(css, tree_fn, 100.0, 100.0);
    let Some(mut h) = try_with_gpu(h) else {
        eprintln!("Skipping: no GPU available");
        return;
    };
    h.step();
    let pixels = h.render();

    // The background from `.box` should be blue inside the rect.
    let interior = pixel_at(&pixels, 100, 50, 50);
    assert!(
        interior[2] > 150 && interior[0] < 100 && interior[1] < 100,
        "expected blue interior (from .box), got {interior:?}",
    );

    // The left edge of the rect (x=10..14) should show the yellow
    // left border painted by `.variant`. Sample x=11, y=50.
    let left_border = pixel_at(&pixels, 100, 11, 50);
    assert!(
        left_border[0] > 200 && left_border[1] > 150 && left_border[2] < 80,
        "expected yellow left border (from .variant), got {left_border:?}",
    );
}

#[test]
fn border_left_and_right_width_longhands_render() {
    // A box with only the left and right border longhands set should
    // paint vertical lines on both sides, with no border on top or
    // bottom. The `.frame` element in the landing example uses exactly
    // this: border-left-width and border-right-width, no shorthand.
    let css = r#"
        .viewport {
            position: relative;
            width: 100px;
            height: 100px;
            background: #000000;
        }
        .framed {
            position: absolute;
            top: 10px;
            left: 10px;
            width: 80px;
            height: 80px;
            border-left-width: 2px;
            border-right-width: 2px;
            border-color: rgb(255, 255, 255);
        }
    "#;

    let tree_fn = || ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("viewport")
            .with_child(ElementDef::new(Tag::Div).with_class("framed")),
    };

    let h = TestHarness::new(css, tree_fn, 100.0, 100.0);
    let Some(mut h) = try_with_gpu(h) else {
        eprintln!("Skipping: no GPU available");
        return;
    };
    h.step();
    let pixels = h.render();

    // Left border: x=11, y=50 should be white.
    let left = pixel_at(&pixels, 100, 11, 50);
    assert!(
        left[0] > 200 && left[1] > 200 && left[2] > 200,
        "expected white left border at x=11, got {left:?}",
    );

    // Right border: the box spans x=10..90, so x=88 should be inside
    // the 2px right border (x=88..90).
    let right = pixel_at(&pixels, 100, 88, 50);
    assert!(
        right[0] > 200 && right[1] > 200 && right[2] > 200,
        "expected white right border at x=88, got {right:?}",
    );

    // Top and bottom should NOT have a border; those pixels should be
    // the black viewport background (the box has no bg, and only left
    // and right borders are set).
    let top = pixel_at(&pixels, 100, 50, 11);
    assert!(
        top[0] < 40 && top[1] < 40 && top[2] < 40,
        "expected no top border (black bg), got {top:?}",
    );

    let bottom = pixel_at(&pixels, 100, 50, 88);
    assert!(
        bottom[0] < 40 && bottom[1] < 40 && bottom[2] < 40,
        "expected no bottom border (black bg), got {bottom:?}",
    );
}

#[test]
fn frame_rule_parses_with_exact_landing_css_snippet() {
    // Exact snippet lifted from the landing CSS, including the
    // `:root` custom property block, comments, and the surrounding
    // rules. Lets us confirm the rule parses end-to-end in context
    // rather than only in a minimal shape.
    let css = r#"
:root {
    --bg: #0a0a0b;
    --line: rgba(255,255,255,0.08);
}

.viewport {
    position: relative;
    display: flex;
    flex-direction: column;
    width: 200px;
    height: 200px;
    padding: 0 10px;
    background: #0a0a0b;
    color: #ffffff;
    overflow: hidden;
}

.frame {
    position: absolute;
    top: 20px;
    left: 10px;
    right: 10px;
    bottom: 20px;
    border-left-width: 2px;
    border-right-width: 2px;
    border-color: rgba(255,255,255,1.0);
    pointer-events: none;
}
"#;

    let tree_fn = || ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("viewport")
            .with_child(ElementDef::new(Tag::Div).with_class("frame")),
    };

    let h = TestHarness::new(css, tree_fn, 200.0, 200.0);
    let Some(mut h) = try_with_gpu(h) else {
        eprintln!("Skipping: no GPU available");
        return;
    };
    h.step();
    let pixels = h.render();

    // Frame is at x=10..190, y=20..180. Left border at x=11, y=100.
    let left_border = pixel_at(&pixels, 200, 11, 100);
    assert!(
        left_border[0] > 200 && left_border[1] > 200 && left_border[2] > 200,
        "expected white left border at x=11 y=100, got {left_border:?}",
    );

    // Confirm the frame is actually inset from the top (y=5 should
    // still be black background).
    let top_margin = pixel_at(&pixels, 200, 100, 5);
    assert!(
        top_margin[0] < 40 && top_margin[1] < 40 && top_margin[2] < 40,
        "expected black top margin at y=5, got {top_margin:?}",
    );
}

#[test]
fn frame_rule_still_applies_with_full_landing_css() {
    // The landing CSS, verbatim. Previously the `.frame` rule
    // dropped out of the cascade because of an earlier bug in
    // declaration error recovery or rule parsing. This test is the
    // single biggest context signal that the bug lives in the full
    // stylesheet and not in any minimal reduction.
    let css = r#"
:root {
    --bg: #0a0a0b;
    --bg-2: #111113;
    --ink: #ffffff;
    --ink-2: #c7c7cc;
    --ink-3: #8e8e93;
    --ink-4: #48484a;
    --line: rgba(255,255,255,0.08);
    --line-2: rgba(255,255,255,0.14);
    --accent: #ff6b3d;
    --accent-glow: rgba(255,107,61,0.4);
}

.viewport {
    position: relative;
    display: flex;
    flex-direction: column;
    width: 1440px;
    height: 900px;
    padding: 0 40px;
    background: #0a0a0b;
    color: #ffffff;
    overflow: hidden;
}

/* corner frame (emulating position: fixed via absolute against viewport) */
.frame {
    position: absolute;
    top: 60px;
    left: 40px;
    right: 40px;
    bottom: 60px;
    border-left-width: 1px;
    border-right-width: 1px;
    border-color: rgba(255,255,255,0.08);
    pointer-events: none;
}
.frame-edge-top {
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    height: 1px;
    background: rgba(255,255,255,0.08);
}
.frame-edge-bottom {
    position: absolute;
    bottom: 0;
    left: 0;
    right: 0;
    height: 1px;
    background: rgba(255,255,255,0.08);
}
.frame-tick {
    position: absolute;
    width: 6px;
    height: 6px;
    border-width: 1px;
    border-color: rgba(255,255,255,0.14);
}
.frame-tick-tl { top: -3px; left: -3px; }
.frame-tick-tr { top: -3px; right: -3px; }
.frame-tick-bl { bottom: -3px; left: -3px; }
.frame-tick-br { bottom: -3px; right: -3px; }

.vignette {
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    bottom: 0;
    background: radial-gradient(ellipse at 70% 50%, transparent 0%, rgba(0,0,0,0.5) 90%);
    pointer-events: none;
}

.hero-right-glow {
    position: absolute;
    top: 0;
    left: 720px;
    right: 0;
    bottom: 0;
    background: radial-gradient(ellipse at 55% 45%, rgba(255,107,61,0.16) 0%, rgba(200,60,30,0.08) 40%, rgba(10,10,11,0) 75%);
    pointer-events: none;
}

.backed {
    position: absolute;
    left: 40px;
    right: 40px;
    bottom: 0;
    display: flex;
    flex-direction: row;
    align-items: center;
    justify-content: space-between;
    padding: 20px 0;
    border-top-width: 1px;
    border-color: rgba(255,255,255,0.08);
    background: linear-gradient(to top, #0a0a0b 60%, rgba(10,10,11,0) 100%);
}
"#;

    let tree_fn = || ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("viewport")
            .with_child(ElementDef::new(Tag::Div).with_class("frame")),
    };

    let h = TestHarness::new(css, tree_fn, 1440.0, 900.0);
    let Some(mut h) = try_with_gpu(h) else {
        eprintln!("Skipping: no GPU available");
        return;
    };
    h.step();
    let pixels = h.render();

    // With the full landing CSS, `.frame` is at (40, 60)-(1400, 840).
    // Left vertical border should sit around x=40..41. The bg is
    // `#0a0a0b` and the border color is rgba(255,255,255,0.08) which
    // composites to roughly (30, 30, 31) over the bg.
    // Sample x=40 y=400 and check it's at least slightly lighter than
    // the dark bg at x=20.
    let border = pixel_at(&pixels, 1440, 40, 400);
    let bg = pixel_at(&pixels, 1440, 20, 400);
    assert!(
        (border[0] as i32 + border[1] as i32 + border[2] as i32)
            > (bg[0] as i32 + bg[1] as i32 + bg[2] as i32) + 6,
        "expected .frame left border at x=40 to be brighter than bg at x=20 (border={border:?}, bg={bg:?})",
    );
}

#[test]
fn frame_rule_still_applies_with_all_frame_tick_variants() {
    // The landing CSS lists each `.frame-tick-<corner>` rule as a
    // single line with negative `top`/`left`/`right`/`bottom`
    // values. This test drops those rules into the mix in case one
    // of them disrupts parsing of an earlier rule via bad error
    // recovery.
    let css = r#"
.viewport {
    position: relative;
    display: flex;
    flex-direction: column;
    width: 200px;
    height: 200px;
    padding: 0 10px;
    background: #000000;
    overflow: hidden;
}

.frame {
    position: absolute;
    top: 20px;
    left: 10px;
    right: 10px;
    bottom: 20px;
    border-left-width: 2px;
    border-right-width: 2px;
    border-color: rgba(255,255,255,1.0);
    pointer-events: none;
}
.frame-edge-top {
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    height: 1px;
    background: rgba(255,255,255,0.08);
}
.frame-edge-bottom {
    position: absolute;
    bottom: 0;
    left: 0;
    right: 0;
    height: 1px;
    background: rgba(255,255,255,0.08);
}
.frame-tick {
    position: absolute;
    width: 6px;
    height: 6px;
    border-width: 1px;
    border-color: rgba(255,255,255,0.14);
}
.frame-tick-tl { top: -3px; left: -3px; }
.frame-tick-tr { top: -3px; right: -3px; }
.frame-tick-bl { bottom: -3px; left: -3px; }
.frame-tick-br { bottom: -3px; right: -3px; }
"#;

    let tree_fn = || ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("viewport")
            .with_child(ElementDef::new(Tag::Div).with_class("frame")),
    };

    let h = TestHarness::new(css, tree_fn, 200.0, 200.0);
    let Some(mut h) = try_with_gpu(h) else {
        eprintln!("Skipping: no GPU available");
        return;
    };
    h.step();
    let pixels = h.render();

    let left_border = pixel_at(&pixels, 200, 11, 100);
    assert!(
        left_border[0] > 200 && left_border[1] > 200 && left_border[2] > 200,
        "expected white left border at x=11 y=100, got {left_border:?}",
    );
}

#[test]
fn frame_rule_still_applies_with_vignette_rule_after() {
    // The landing CSS has `.vignette` and `.hero-right-glow` with
    // `background: radial-gradient(...)` declarations, followed by
    // many more rules. This test adds those rules to verify the
    // cascade doesn't get confused and wipe out `.frame`'s
    // `position: absolute`.
    let css = r#"
.viewport {
    position: relative;
    display: flex;
    flex-direction: column;
    width: 200px;
    height: 200px;
    padding: 0 10px;
    background: #000000;
    overflow: hidden;
}

.frame {
    position: absolute;
    top: 20px;
    left: 10px;
    right: 10px;
    bottom: 20px;
    border-left-width: 2px;
    border-right-width: 2px;
    border-color: rgba(255,255,255,1.0);
    pointer-events: none;
}

.vignette {
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    bottom: 0;
    background: radial-gradient(ellipse at 70% 50%, transparent 0%, rgba(0,0,0,0.5) 90%);
    pointer-events: none;
}

.hero-right-glow {
    position: absolute;
    top: 0;
    left: 720px;
    right: 0;
    bottom: 0;
    background: radial-gradient(ellipse at 55% 45%, rgba(255,107,61,0.16) 0%, rgba(200,60,30,0.08) 40%, rgba(10,10,11,0) 75%);
    pointer-events: none;
}
"#;

    let tree_fn = || ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("viewport")
            .with_child(ElementDef::new(Tag::Div).with_class("frame")),
    };

    let h = TestHarness::new(css, tree_fn, 200.0, 200.0);
    let Some(mut h) = try_with_gpu(h) else {
        eprintln!("Skipping: no GPU available");
        return;
    };
    h.step();
    let pixels = h.render();

    let left_border = pixel_at(&pixels, 200, 11, 100);
    assert!(
        left_border[0] > 200 && left_border[1] > 200 && left_border[2] > 200,
        "expected white left border at x=11 y=100, got {left_border:?}",
    );
}

#[test]
fn frame_rule_parses_inside_full_landing_css_shape() {
    // This is the actual shape used by the landing example: a
    // `:root` custom property block, then a `.viewport` with
    // `padding: 0 40px`, then `.frame` with all four insets, per
    // side border longhands, and `pointer-events`. The frame
    // previously ended up with `position: static` because one of
    // the earlier declarations poisoned the whole rule.
    let css = r#"
        :root {
            --line: rgba(255, 255, 255, 0.08);
        }
        .viewport {
            position: relative;
            display: flex;
            flex-direction: column;
            width: 100px;
            height: 100px;
            padding: 0 5px;
            background: #000000;
            overflow: hidden;
        }
        .frame {
            position: absolute;
            top: 10px;
            left: 10px;
            right: 10px;
            bottom: 10px;
            border-left-width: 2px;
            border-right-width: 2px;
            border-color: rgba(255, 255, 255, 1.0);
            pointer-events: none;
        }
    "#;

    let tree_fn = || ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("viewport")
            .with_child(ElementDef::new(Tag::Div).with_class("frame")),
    };

    let h = TestHarness::new(css, tree_fn, 100.0, 100.0);
    let Some(mut h) = try_with_gpu(h) else {
        eprintln!("Skipping: no GPU available");
        return;
    };
    h.step();
    let pixels = h.render();

    // Frame spans x=10..90, y=10..90. Left border at x=11 y=50.
    let left_border = pixel_at(&pixels, 100, 11, 50);
    assert!(
        left_border[0] > 200 && left_border[1] > 200 && left_border[2] > 200,
        "expected white left border of .frame at x=11 y=50, got {left_border:?}",
    );
}

#[test]
fn absolute_with_absolute_children_fills_inset_region() {
    // Replicates the `.frame` shape from the landing: an absolute
    // container with top/left/right/bottom set but no explicit size,
    // that itself contains absolute children (edges + ticks). The
    // container should still expand to fill the inset region.
    let css = r#"
        .viewport {
            position: relative;
            display: flex;
            flex-direction: column;
            width: 100px;
            height: 100px;
            padding: 0 5px;
            background: #000000;
            overflow: hidden;
        }
        .frame {
            position: absolute;
            top: 10px;
            left: 10px;
            right: 10px;
            bottom: 10px;
            border-left-width: 2px;
            border-right-width: 2px;
            border-color: rgb(255, 255, 255);
        }
        .edge-top {
            position: absolute;
            top: 0;
            left: 0;
            right: 0;
            height: 2px;
            background: rgb(255, 255, 255);
        }
        .edge-bottom {
            position: absolute;
            bottom: 0;
            left: 0;
            right: 0;
            height: 2px;
            background: rgb(255, 255, 255);
        }
    "#;

    let tree_fn = || ElementTree {
        root: ElementDef::new(Tag::Div).with_class("viewport").with_child(
            ElementDef::new(Tag::Div)
                .with_class("frame")
                .with_child(ElementDef::new(Tag::Div).with_class("edge-top"))
                .with_child(ElementDef::new(Tag::Div).with_class("edge-bottom")),
        ),
    };

    let h = TestHarness::new(css, tree_fn, 100.0, 100.0);
    let Some(mut h) = try_with_gpu(h) else {
        eprintln!("Skipping: no GPU available");
        return;
    };
    h.step();
    let pixels = h.render();

    // .frame spans from (10, 10) to (90, 90).
    // Left border: x=11 y=50 should be white.
    let left_border = pixel_at(&pixels, 100, 11, 50);
    assert!(
        left_border[0] > 200 && left_border[1] > 200 && left_border[2] > 200,
        "expected white left border of .frame at x=11, got {left_border:?}",
    );

    // Top edge: y=11 x=50 should be white (.edge-top).
    let top_edge = pixel_at(&pixels, 100, 50, 11);
    assert!(
        top_edge[0] > 200 && top_edge[1] > 200 && top_edge[2] > 200,
        "expected white top edge at y=11, got {top_edge:?}",
    );

    // Bottom edge: y=88 x=50 should be white (.edge-bottom).
    let bottom_edge = pixel_at(&pixels, 100, 50, 88);
    assert!(
        bottom_edge[0] > 200 && bottom_edge[1] > 200 && bottom_edge[2] > 200,
        "expected white bottom edge at y=88, got {bottom_edge:?}",
    );
}

#[test]
fn absolute_inside_padded_and_overflow_hidden_viewport() {
    // Mirrors the landing example's `.viewport` + `.frame` shape:
    // a flex-column viewport with horizontal padding and
    // `overflow: hidden`, containing an absolute child with
    // top/left/right/bottom but no explicit size. The child should
    // still fill its inset region rather than collapse to zero size
    // or lay out as a flex item.
    let css = r#"
        .viewport {
            position: relative;
            display: flex;
            flex-direction: column;
            width: 100px;
            height: 100px;
            padding: 0 5px;
            background: #000000;
            overflow: hidden;
        }
        .fill {
            position: absolute;
            top: 10px;
            left: 10px;
            right: 10px;
            bottom: 10px;
            background: rgb(200, 200, 200);
        }
    "#;

    let tree_fn = || ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("viewport")
            .with_child(ElementDef::new(Tag::Div).with_class("fill")),
    };

    let h = TestHarness::new(css, tree_fn, 100.0, 100.0);
    let Some(mut h) = try_with_gpu(h) else {
        eprintln!("Skipping: no GPU available");
        return;
    };
    h.step();
    let pixels = h.render();

    // Center of the fill rect should be gray.
    let center = pixel_at(&pixels, 100, 50, 50);
    assert!(
        center[0] > 150 && center[1] > 150 && center[2] > 150,
        "expected gray center of absolute fill, got {center:?}",
    );

    // Bottom margin should be black.
    let bottom = pixel_at(&pixels, 100, 50, 95);
    assert!(
        bottom[0] < 40 && bottom[1] < 40 && bottom[2] < 40,
        "expected black bottom margin, got {bottom:?}",
    );
}

#[test]
fn absolute_with_top_bottom_and_auto_height_fills_inset_region() {
    // An absolutely positioned box with `top`, `left`, `right`, and
    // `bottom` set but no explicit `width` or `height` should
    // stretch to fill the remaining region. This is the shape the
    // `.frame` element in the landing example uses: it's inset 60px
    // top/bottom and 40px left/right, with no size set.
    let css = r#"
        .viewport {
            position: relative;
            display: flex;
            flex-direction: column;
            width: 100px;
            height: 100px;
            background: #000000;
        }
        .fill {
            position: absolute;
            top: 10px;
            left: 10px;
            right: 10px;
            bottom: 10px;
            background: rgb(200, 200, 200);
        }
    "#;

    let tree_fn = || ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("viewport")
            .with_child(ElementDef::new(Tag::Div).with_class("fill")),
    };

    let h = TestHarness::new(css, tree_fn, 100.0, 100.0);
    let Some(mut h) = try_with_gpu(h) else {
        eprintln!("Skipping: no GPU available");
        return;
    };
    h.step();
    let pixels = h.render();

    // Center of the fill rect should be gray.
    let center = pixel_at(&pixels, 100, 50, 50);
    assert!(
        center[0] > 150 && center[1] > 150 && center[2] > 150,
        "expected gray center of absolute fill, got {center:?}",
    );

    // Pixel well outside the fill (margin region) should be black.
    let outside = pixel_at(&pixels, 100, 5, 50);
    assert!(
        outside[0] < 40 && outside[1] < 40 && outside[2] < 40,
        "expected black margin region, got {outside:?}",
    );

    // Pixel inside the top margin (y<10) should be black.
    let top_margin = pixel_at(&pixels, 100, 50, 5);
    assert!(
        top_margin[0] < 40 && top_margin[1] < 40 && top_margin[2] < 40,
        "expected black top margin, got {top_margin:?}",
    );

    // Pixel inside the bottom margin (y>90) should be black.
    let bottom_margin = pixel_at(&pixels, 100, 50, 95);
    assert!(
        bottom_margin[0] < 40 && bottom_margin[1] < 40 && bottom_margin[2] < 40,
        "expected black bottom margin, got {bottom_margin:?}",
    );
}

#[test]
fn border_top_width_longhand_renders() {
    // Separate test for the `border-top-width` longhand, which is
    // used by `.backed` in the landing example.
    let css = r#"
        .viewport {
            position: relative;
            width: 100px;
            height: 100px;
            background: #000000;
        }
        .strip {
            position: absolute;
            top: 40px;
            left: 10px;
            width: 80px;
            height: 20px;
            border-top-width: 2px;
            border-color: rgb(255, 255, 255);
        }
    "#;

    let tree_fn = || ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("viewport")
            .with_child(ElementDef::new(Tag::Div).with_class("strip")),
    };

    let h = TestHarness::new(css, tree_fn, 100.0, 100.0);
    let Some(mut h) = try_with_gpu(h) else {
        eprintln!("Skipping: no GPU available");
        return;
    };
    h.step();
    let pixels = h.render();

    // Top edge of .strip at y=40, 2px tall. Sample y=41.
    let top = pixel_at(&pixels, 100, 50, 41);
    assert!(
        top[0] > 200 && top[1] > 200 && top[2] > 200,
        "expected white top border at y=41, got {top:?}",
    );

    // Bottom of strip (y=59) should NOT have a border.
    let bottom = pixel_at(&pixels, 100, 50, 58);
    assert!(
        bottom[0] < 40 && bottom[1] < 40 && bottom[2] < 40,
        "expected no bottom border at y=58, got {bottom:?}",
    );
}
