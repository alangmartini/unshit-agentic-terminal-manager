//! Renderer checks for stacked `box-shadow` with the `inset` keyword.
//!
//! These tests render a centered rounded rect on a dark background and
//! probe specific pixels to confirm the shader emits outer shadows behind
//! the rect, the background fill inside, and inset shadows inside the
//! padding box.
//!
//! Each test is resilient to GPU absence: if a headless GPU context cannot
//! be created we skip the test rather than fail. Pixel probing is done on
//! small inner patches instead of full image snapshots so it survives
//! driver-level antialiasing jitter across machines.

use unshit_core::element::*;
use unshit_test::TestHarness;

fn try_with_gpu(h: TestHarness) -> Option<TestHarness> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| h.with_gpu())).ok()
}

fn pixel_at(pixels: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let idx = ((y * width + x) * 4) as usize;
    [pixels[idx], pixels[idx + 1], pixels[idx + 2], pixels[idx + 3]]
}

/// Renders a 100 by 100 centered rounded rect inside a 200 by 200 viewport
/// with a stacked outer plus inset box-shadow. The outer shadow should be
/// visible outside the rect and the inset shadow should darken the top
/// inner band of the rect.
#[test]
fn stacked_outer_and_inset_box_shadow() {
    // The rect is a bright blue fill so we can distinguish background,
    // inset shadow band, outer shadow, and backdrop fill by color alone.
    // The outer shadow is solid red (easy to detect) and the inset shadow
    // is solid green (also easy to detect).
    let css = r#"
        .root {
            display: flex;
            width: 200px;
            height: 200px;
            padding: 0;
            margin: 0;
            background: #000000;
            align-items: center;
            justify-content: center;
        }
        .box {
            width: 100px;
            height: 100px;
            border-radius: 16px;
            background: rgb(30, 60, 220);
            box-shadow:
                inset 0 0 12px rgba(0, 220, 60, 1.0),
                0 0 12px rgba(220, 40, 40, 1.0);
        }
    "#;

    let tree_fn = || ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("root")
            .with_child(ElementDef::new(Tag::Div).with_class("box")),
    };

    let h = TestHarness::new(css, tree_fn, 200.0, 200.0);
    let Some(mut h) = try_with_gpu(h) else {
        eprintln!("Skipping box_shadow_stacked: no GPU available");
        return;
    };
    h.step();
    let pixels = h.render();
    assert_eq!(pixels.len(), 200 * 200 * 4);

    // 1. Background well outside the rect and its outer shadow. At pixel
    //    (10, 10) we should be on the black root background.
    let corner = pixel_at(&pixels, 200, 10, 10);
    assert!(
        corner[0] < 30 && corner[1] < 30 && corner[2] < 30,
        "expected dark backdrop at (10,10), got {:?}",
        corner
    );

    // 2. Outer shadow band. The rect is centered at (100, 100) with half
    //    size 50 and 12px blur. At y = 100 and x just outside the right
    //    edge (x around 58 to 62 from the center, so absolute 158 to 162)
    //    the outer shadow should dominate red over green and blue.
    let outer = pixel_at(&pixels, 200, 158, 100);
    assert!(
        outer[0] > outer[1] && outer[0] > outer[2] && outer[0] > 40,
        "expected red dominant outer shadow band at (158,100), got {:?}",
        outer
    );

    // 3. Rect center. Deep inside the rect should be the blue fill with
    //    very little contribution from either shadow.
    let center = pixel_at(&pixels, 200, 100, 100);
    assert!(
        center[2] > 150,
        "expected blue fill dominant at rect center (100,100), got {:?}",
        center
    );
    assert!(
        center[2] > center[0] && center[2] > center[1],
        "expected blue channel dominant at rect center, got {:?}",
        center
    );

    // 4. Inset shadow band. Just inside the top edge of the rect the
    //    inset green shadow should mix heavily with the blue fill. Pick
    //    (100, 54) which is 4px in from the rect top at y = 50.
    let inset = pixel_at(&pixels, 200, 100, 54);
    assert!(
        inset[1] > 40,
        "expected visible green from inset shadow near top edge, got {:?}",
        inset
    );
    // The inset shadow must not bleed outside the padding box. Just above
    // the rect (y = 48) the green channel should be much lower than the
    // inset band.
    let above = pixel_at(&pixels, 200, 100, 48);
    assert!(
        above[1] <= inset[1],
        "inset shadow must not leak above the rect, got above={:?} inset={:?}",
        above,
        inset
    );
}

/// `box-shadow: none` must clear any prior shadow on the element.
#[test]
fn box_shadow_none_disables_shadow() {
    let css = r#"
        .root {
            display: flex;
            width: 120px;
            height: 120px;
            padding: 0;
            margin: 0;
            background: #000000;
            align-items: center;
            justify-content: center;
        }
        .box {
            width: 60px;
            height: 60px;
            border-radius: 8px;
            background: rgb(40, 40, 40);
            box-shadow: none;
        }
    "#;

    let tree_fn = || ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("root")
            .with_child(ElementDef::new(Tag::Div).with_class("box")),
    };

    let h = TestHarness::new(css, tree_fn, 120.0, 120.0);
    let Some(mut h) = try_with_gpu(h) else {
        eprintln!("Skipping box_shadow_none_disables_shadow: no GPU available");
        return;
    };
    h.step();
    let pixels = h.render();
    assert_eq!(pixels.len(), 120 * 120 * 4);

    // Pixel well outside the rect should still be the black backdrop (no
    // shadow bleed).
    let outside = pixel_at(&pixels, 120, 20, 60);
    assert!(
        outside[0] < 20 && outside[1] < 20 && outside[2] < 20,
        "expected backdrop pixel at (20,60), got {:?}",
        outside
    );
}
