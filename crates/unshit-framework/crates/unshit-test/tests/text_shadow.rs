//! GPU render test for CSS `text-shadow`: a blurred glow must paint colored
//! pixels around the text that bare text does not. The parser/cascade side is
//! covered by unit tests in `unshit-core::style::parse`; this is the end-to-end
//! proof that the stacked-tap glow actually reaches the framebuffer.

use unshit_core::element::*;
use unshit_test::TestHarness;

fn try_with_gpu(h: TestHarness) -> Option<TestHarness> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| h.with_gpu())).ok()
}

const W: f32 = 220.0;
const H: f32 = 90.0;

fn render(css: &str) -> Option<Vec<u8>> {
    let h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("page")
                .with_child(ElementDef::new(Tag::Span).with_class("label").with_text("Hi")),
        },
        W,
        H,
    );
    let mut h = try_with_gpu(h)?;
    h.step();
    Some(h.render())
}

/// Count clearly-red pixels (the glow color): red dominant over green/blue.
/// White text (255,255,255), black bg (0,0,0), and anti-aliased gray edges are
/// all excluded, so any hit is the colored shadow.
fn reddish(pixels: &[u8]) -> usize {
    pixels
        .chunks_exact(4)
        .filter(|p| {
            let (r, g, b) = (p[0] as i32, p[1] as i32, p[2] as i32);
            r > 80 && r >= g + 40 && r >= b + 40
        })
        .count()
}

const BASE: &str = r#"
    .page  { width: 100%; height: 100%; background: #000000; }
    .label { color: #ffffff; font-size: 48px; }
"#;

#[test]
fn text_shadow_paints_a_colored_glow_around_text() {
    // Subpixel text AA leaves a thin red/blue fringe on glyph edges even for
    // bare white text, so the signal is the DIFFERENCE: a red glow adds a broad
    // red halo of far more red pixels than the few edge-fringe ones.
    let Some(bare) = render(BASE) else {
        eprintln!("Skipping: no GPU available");
        return;
    };
    let shadow_css = format!("{BASE}\n.label {{ text-shadow: 0 0 8px rgb(255, 0, 0); }}");
    let Some(shadow) = render(&shadow_css) else {
        return;
    };
    let bare_red = reddish(&bare);
    let glow_red = reddish(&shadow);
    assert!(
        glow_red > bare_red + 150,
        "an `0 0 8px` red text-shadow must paint a broad red halo \
         (bare red fringe = {bare_red}, with-glow = {glow_red})"
    );
}

#[test]
fn text_shadow_none_matches_no_shadow() {
    // `text-shadow: none` must render like no shadow at all — no added glow.
    let Some(bare) = render(BASE) else {
        eprintln!("Skipping: no GPU available");
        return;
    };
    let Some(none) = render(&format!("{BASE}\n.label {{ text-shadow: none; }}")) else {
        return;
    };
    let diff = (reddish(&none) as i64 - reddish(&bare) as i64).abs();
    assert!(diff < 25, "text-shadow: none must add no glow (red delta = {diff})");
}
