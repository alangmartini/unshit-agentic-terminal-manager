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

/// Total red-channel energy of the glow (sum of the red channel over reddish
/// pixels). Used to check that the glow's INTENSITY tracks the shadow alpha —
/// a correct alpha-composited glow scales with alpha, whereas the premultiply
/// bug decouples brightness from alpha (rgb blows out regardless).
fn glow_red_energy(pixels: &[u8]) -> u64 {
    pixels
        .chunks_exact(4)
        .map(|p| {
            let (r, g, b) = (p[0] as i64, p[1] as i64, p[2] as i64);
            if r > g + 30 && r > b + 30 {
                r as u64
            } else {
                0
            }
        })
        .sum()
}

fn glow_css(alpha: f32) -> String {
    format!("{BASE}\n.label {{ text-shadow: 0 0 8px rgba(255, 0, 0, {alpha}); }}")
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
        glow_red > bare_red + 40,
        "an `0 0 8px` red text-shadow must paint a red halo beyond the bare \
         edge fringe (bare red fringe = {bare_red}, with-glow = {glow_red})"
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

#[test]
fn text_shadow_glow_intensity_scales_with_alpha() {
    // Regression guard for the subpixel-shader premultiply bug: a stronger
    // shadow alpha must produce a meaningfully brighter glow. The bug made the
    // glow blow out to a bright smear independent of alpha (rgb accumulated far
    // faster than alpha under the premultiplied blend), so a faint and a strong
    // glow looked the same. With correct premultiplied output the glow energy
    // scales ~linearly with alpha.
    let Some(faint) = render(&glow_css(0.15)) else {
        eprintln!("Skipping: no GPU available");
        return;
    };
    let Some(strong) = render(&glow_css(0.5)) else {
        return;
    };
    let faint_e = glow_red_energy(&faint);
    let strong_e = glow_red_energy(&strong);
    // Correct alpha compositing scales the glow with alpha (sub-linearly in the
    // sRGB framebuffer: a ~3.3x alpha ratio shows as ~1.6x energy). The
    // premultiply bug decoupled brightness from alpha (~1.0x). 1.3x cleanly
    // separates the two.
    assert!(
        strong_e * 10 > faint_e * 13,
        "glow intensity must scale with shadow alpha \
         (alpha 0.15 energy = {faint_e}, alpha 0.5 energy = {strong_e})"
    );
}
