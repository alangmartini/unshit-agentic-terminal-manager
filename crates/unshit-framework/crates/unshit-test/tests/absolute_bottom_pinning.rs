//! Regression for the Organiza Nota landing `.backed` bottom-pinning
//! bug. A `position: absolute` child with `left`, `right`, and `bottom: 0`
//! set should be pinned to the bottom of its positioned ancestor.
//!
//! The underlying bug was in the CSS parser: a block comment (e.g.
//! `/* backed bar */`) placed right before a rule would leak into the
//! selector, so `.backed` was parsed as `* / backed bar / .backed` and
//! never matched any element. This file tests the pinning behavior end
//! to end; `crates/unshit-core/tests/nav_parse_debug.rs` covers the
//! parser level regression.

use unshit_core::element::*;
use unshit_core::style::types::CssPosition;
use unshit_test::TestHarness;

fn try_with_gpu(h: TestHarness) -> Option<TestHarness> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| h.with_gpu())).ok()
}

fn pixel_at(pixels: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let idx = ((y * width + x) * 4) as usize;
    [pixels[idx], pixels[idx + 1], pixels[idx + 2], pixels[idx + 3]]
}

#[test]
fn absolute_child_with_left_right_bottom_pins_to_viewport_bottom() {
    // 200x200 viewport (flex column, relative, overflow hidden). Contains
    // a single absolute child with left/right/bottom set and an explicit
    // height. The red band should sit at y=180..200, flush with the bottom.
    let css = r#"
        .viewport {
            position: relative;
            display: flex;
            flex-direction: column;
            width: 200px;
            height: 200px;
            background: #000000;
            overflow: hidden;
        }
        .backed {
            position: absolute;
            left: 10px;
            right: 10px;
            bottom: 0;
            height: 20px;
            background: rgb(220, 40, 40);
        }
    "#;

    let tree_fn = || ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("viewport")
            .with_child(ElementDef::new(Tag::Div).with_class("backed")),
    };

    let h = TestHarness::new(css, tree_fn, 200.0, 200.0);
    let Some(mut h) = try_with_gpu(h) else {
        eprintln!("Skipping: no GPU available");
        return;
    };
    h.step();
    let pixels = h.render();

    // Center of the band (y=190) should be red, area just above (y=170)
    // should still be the black background, and the very top must not
    // show the band.
    let band = pixel_at(&pixels, 200, 100, 190);
    assert!(
        band[0] > 150 && band[1] < 80 && band[2] < 80,
        "expected red band pinned to viewport bottom at y=190, got {band:?}",
    );
    let above = pixel_at(&pixels, 200, 100, 170);
    assert!(
        above[0] < 40 && above[1] < 40 && above[2] < 40,
        "expected black above the band at y=170, got {above:?}",
    );
    let top = pixel_at(&pixels, 200, 100, 10);
    assert!(
        !(top[0] > 150 && top[1] < 80 && top[2] < 80),
        "expected black at top of viewport, got {top:?}",
    );
}

#[test]
fn absolute_backed_strip_pins_to_bottom_with_content_sized_height() {
    // Closer to the real `.backed` rule: left/right/bottom set, no
    // explicit `height`, but padding + child content + border-top +
    // linear-gradient background. The height is implied by content.
    let css = r#"
        .viewport {
            position: relative;
            display: flex;
            flex-direction: column;
            width: 200px;
            height: 200px;
            background: #000000;
            overflow: hidden;
        }
        .backed {
            position: absolute;
            left: 10px;
            right: 10px;
            bottom: 0;
            display: flex;
            flex-direction: row;
            align-items: center;
            justify-content: space-between;
            padding: 8px 0;
            border-top-width: 1px;
            border-color: rgba(255,255,255,0.08);
            background: linear-gradient(to top, rgb(220,40,40) 60%, rgba(220,40,40,0) 100%);
        }
        .cell {
            width: 20px;
            height: 14px;
            background: rgb(220, 40, 40);
        }
    "#;

    let tree_fn = || ElementTree {
        root: ElementDef::new(Tag::Div).with_class("viewport").with_child(
            ElementDef::new(Tag::Div)
                .with_class("backed")
                .with_child(ElementDef::new(Tag::Div).with_class("cell"))
                .with_child(ElementDef::new(Tag::Div).with_class("cell")),
        ),
    };

    let h = TestHarness::new(css, tree_fn, 200.0, 200.0);
    let Some(mut h) = try_with_gpu(h) else {
        eprintln!("Skipping: no GPU available");
        return;
    };
    h.step();
    let pixels = h.render();

    // 14px cell + 8+8 padding + 1px border = 31px tall. Sample inside the
    // left cell.
    let band = pixel_at(&pixels, 200, 15, 185);
    assert!(
        band[0] > 150 && band[1] < 80 && band[2] < 80,
        "expected red cell inside bottom-pinned strip at (15,185), got {band:?}",
    );
    let top = pixel_at(&pixels, 200, 100, 10);
    assert!(
        !(top[0] > 150 && top[1] < 80 && top[2] < 80),
        "expected black at top of viewport, got {top:?}",
    );
    let mid = pixel_at(&pixels, 200, 100, 100);
    assert!(
        !(mid[0] > 150 && mid[1] < 80 && mid[2] < 80),
        "expected black at middle of viewport, got {mid:?}",
    );
}

/// End to end regression for the original bug: when a rule is preceded
/// by a CSS block comment (common in the landing stylesheet), the
/// cascade should still apply the rule. Prior to the fix, the comment
/// leaked into the selector and `.backed` was never matched.
#[test]
fn rule_preceded_by_comment_still_applies() {
    let css = r#"
        .viewport {
            position: relative;
            display: flex;
            flex-direction: column;
            width: 200px;
            height: 200px;
            background: #000;
            overflow: hidden;
        }
        /* pinned bottom strip */
        .backed {
            position: absolute;
            left: 10px;
            right: 10px;
            bottom: 0;
            height: 20px;
            background: rgb(220, 40, 40);
        }
    "#;

    let tree_fn = || ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("viewport")
            .with_child(ElementDef::new(Tag::Div).with_class("backed")),
    };

    let h = TestHarness::new(css, tree_fn, 200.0, 200.0);
    let arena = h.arena();

    let mut backed_id = None;
    for (id, e) in arena.iter() {
        if e.classes.iter().any(|c| c == "backed") {
            backed_id = Some(id);
            break;
        }
    }
    let backed_id = backed_id.expect(".backed element missing");
    let elem = arena.get(backed_id).unwrap();
    let cs = &elem.computed_style;
    assert_eq!(
        cs.position,
        CssPosition::Absolute,
        ".backed should be position: absolute even when preceded by a comment",
    );
    assert!(cs.bottom.is_some(), ".backed should have `bottom` set");
    assert!(cs.left.is_some(), ".backed should have `left` set");
    assert!(cs.right.is_some(), ".backed should have `right` set");
}
