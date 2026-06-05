//! STAGE 0 — Golden baseline / regression oracle for the CSS cascade.
//!
//! This test freezes the EXACT resolved-style output the engine produces today
//! for the app's real stylesheet (`assets/styles.css`), across the key themes
//! and selectors that theming touches. It is the oracle the var()/cascade
//! refactor (Stages 1-4) must keep byte-identical until the deliberate switch
//! that makes themed custom properties win.
//!
//! WHY THIS MATTERS (the gap that was captured, then closed in Stage 3):
//! Theming used to be faked with ~690 hand-authored concrete (non-`var()`)
//! declarations because `var()` was resolved at parse time by global textual
//! substitution seeded ONLY from `:root`. So `.app.theme-* { --token: ... }`
//! overrides (579 custom-property declarations) were dropped, and any
//! declaration that still referenced `var()` — e.g. the command palette `.cp`
//! (`background`, `border-color`) — resolved to the `:root` (amber) value
//! REGARDLESS of the active theme.
//!
//! STAGE 3 CLOSED THAT GAP: `var()` now resolves PER ELEMENT against the active
//! token scopes (self > active `.app.theme-*` root scope > `:root`). The frozen
//! blob below reflects the flip: the `.cp` rows for the themes that OVERRIDE the
//! palette's bg/border tokens — `dracula`, `nord`, `gruvbox`, `tokyo-night` —
//! now show the THEME color, not amber. The `amber`/`catppuccin`/`no-theme`
//! `.cp` rows stay amber on purpose: those themes do not redefine
//! `--bg-elevated`/`--bg-subtle`/`--border-default`, so the cascade correctly
//! falls back to `:root`. Every NON-`.cp` row is byte-identical to Stage 0 —
//! the concrete clone declarations still win by source order, so the safety net
//! holds: zero visual regression for clone-covered selectors, with the var()
//! flip landing exactly where authors used `var()` and the theme overrides it.
//!
//! TWO-LEVEL INDIRECTION FIX (post Stage 3): token values that are themselves a
//! `var()` reference are no longer eagerly concretized at parse time; they are
//! resolved lazily and multi-level at use time against the element's full scope
//! env. As a side effect the `theme-chip.dracula` `box-shadow` (authored
//! unconditionally on `.theme-chip` as `inset 0 0 0 1px var(--border-soft)`) now
//! RESOLVES to each theme's `--border-soft` instead of dropping to `[]`. That is
//! the only field that flips for these rows — `bg`/`color`/`bw`/`bc` stay
//! byte-identical — and it flips to the strictly-more-correct theme value.
//!
//! HOW THE GOLDEN IS BUILT:
//! `unshit_test::TestHarness` parses the real stylesheet and runs the real
//! per-node cascade (style resolution only — no GPU, no fonts beyond what the
//! harness already loads, fully deterministic). For each (theme, selector) we
//! build the smallest element tree that makes the theme's descendant selectors
//! match: a `.app[.theme-*]` root wrapping the queried leaf (some leaves are
//! additionally nested under `.settings-page`, matching how the app renders
//! the settings surface). We then snapshot the `ComputedStyle` fields theming
//! affects — `background`, `color`, `border_width`, `border_color`,
//! `box_shadow` — via their exact `Debug` form and compare against the frozen
//! `GOLDEN` blob below.
//!
//! REGENERATING (only when an intended behavior change lands): set
//! `UNSHIT_REGEN_GOLDEN=1` and run this test; it prints the new blob to stderr,
//! which can be pasted into `GOLDEN`.

use unshit::core::element::{ElementDef, ElementTree, Tag};
use unshit::core::style::parse::CompiledStylesheet;
use unshit::core::style::types::ComputedStyle;
use unshit_test::TestHarness;

const STYLES: &str = include_str!("../assets/styles.css");

/// Count of dropped custom-property declarations on non-`:root` selectors
/// (every `.app.theme-* { --token: ... }`, `.theme-chip.* { ... }`, etc.).
///
/// STAGE 3 DROVE THIS TO 0. The parse-time global `var()` substitution is gone;
/// every `--token` block — `:root` AND the per-theme/widget scopes — is now
/// collected into `token_scopes` and consumed by the cascade, so the definitions
/// are LIVE instead of dropped. Was 579 at Stage 0; the fall to 0 is the proof
/// that the 473 themed overrides became cascade-visible.
const GOLDEN_CUSTOM_PROPERTY_DROP_COUNT: usize = 0;

/// 6 themes + the default/no-theme root, x 8 key selectors. The `(name,
/// classes, wrap_settings)` triples below drive both the build and the row
/// labels in the golden blob.
fn themes() -> [(&'static str, Option<&'static str>); 7] {
    [
        ("no-theme", None),
        ("amber", Some("theme-amber")),
        ("dracula", Some("theme-dracula")),
        ("nord", Some("theme-nord")),
        ("tokyo-night", Some("theme-tokyo-night")),
        ("gruvbox", Some("theme-gruvbox")),
        ("catppuccin", Some("theme-catppuccin")),
    ]
}

/// (label, leaf classes, whether to nest under `.settings-page`).
fn selectors() -> [(&'static str, &'static [&'static str], bool); 8] {
    [
        ("sidebar", &["sidebar"], false),
        ("titlebar", &["titlebar"], false),
        ("tabbar", &["tabbar"], false),
        ("settings-page", &["settings-page"], false),
        ("set-page-header", &["set-page-header"], true),
        ("theme-chip.dracula", &["theme-chip", "dracula"], true),
        ("cp", &["cp"], false),
        ("sb-row", &["sb-row"], false),
    ]
}

/// The exact theme-affected fields, in their `Debug` form. Any cascade
/// regression that perturbs a resolved color, border, or shadow shows up here.
fn fmt(cs: &ComputedStyle) -> String {
    format!(
        "bg={:?} | color={:?} | bw={:?} | bc={:?} | shadow={:?}",
        cs.background, cs.color, cs.border_width, cs.border_color, cs.box_shadow
    )
}

/// Build the smallest tree whose root carries the theme class so the theme's
/// descendant selectors (`.app.theme-x .sidebar`) match the queried leaf.
fn build(theme: Option<&str>, leaf_classes: &[&str], wrap_settings: bool) -> ElementTree {
    let mut leaf = ElementDef::new(Tag::Div);
    for c in leaf_classes {
        leaf = leaf.with_class(*c);
    }
    let mut inner = ElementDef::new(Tag::Div)
        .with_class("layout")
        .with_child(leaf);
    if wrap_settings {
        inner = ElementDef::new(Tag::Div)
            .with_class("settings-page")
            .with_child(inner);
    }
    let mut root = ElementDef::new(Tag::Div).with_class("app");
    if let Some(t) = theme {
        root = root.with_class(t);
    }
    if wrap_settings {
        root = root.with_class("settings");
    }
    ElementTree {
        root: root.with_child(inner),
    }
}

/// Resolve one (theme, selector) pair to its `ComputedStyle` through the real
/// harness cascade.
fn resolve(theme: Option<&str>, leaf_classes: &[&str], wrap_settings: bool) -> ComputedStyle {
    let sel = format!(".{}", leaf_classes.join("."));
    let lc: Vec<&str> = leaf_classes.to_vec();
    let h = TestHarness::new(
        STYLES,
        move || build(theme, &lc, wrap_settings),
        1280.0,
        800.0,
    );
    h.query(&sel)
        .unwrap_or_else(|| panic!("selector {sel} not found"))
        .computed_style
}

/// Recompute the full golden blob from the live engine.
fn current_blob() -> String {
    let mut lines = Vec::new();
    for (tname, theme) in themes() {
        for (name, classes, wrap) in selectors() {
            let cs = resolve(theme, classes, wrap);
            lines.push(format!("{tname}|{name}|{}", fmt(&cs)));
        }
    }
    lines.join("\n")
}

#[test]
fn cascade_golden_baseline_is_unchanged() {
    let actual = current_blob();

    if std::env::var("UNSHIT_REGEN_GOLDEN").as_deref() == Ok("1") {
        eprintln!("\n===== REGENERATED GOLDEN (paste into GOLDEN) =====\n{actual}\n=====");
    }

    assert_eq!(
        actual, GOLDEN,
        "\nResolved styles changed for a (theme|selector) pair. If this is an \
         INTENTIONAL behavior change (e.g. the cascade now resolves themed \
         custom properties), re-run with UNSHIT_REGEN_GOLDEN=1 to print the new \
         blob and update GOLDEN. Otherwise this is a cascade regression — Stages \
         1-3 must keep these byte-identical.\n"
    );
}

#[test]
fn custom_property_drop_count_is_frozen() {
    let sheet = CompiledStylesheet::parse(STYLES);
    let custom = sheet
        .dropped
        .iter()
        .filter(|d| d.is_custom_property())
        .count();
    assert_eq!(
        custom, GOLDEN_CUSTOM_PROPERTY_DROP_COUNT,
        "Custom-property drop count changed (was {GOLDEN_CUSTOM_PROPERTY_DROP_COUNT}, now \
         {custom}). The cascade refactor (Stage 3/4) is expected to drive this to 0 as themed \
         custom properties become live; when it does, update this constant in the same change \
         and assert the new value (ultimately 0)."
    );
}

/// FROZEN golden blob: 7 themes x 8 selectors. Generated from the live engine
/// at Stage 0. Do not hand-edit — regenerate with `UNSHIT_REGEN_GOLDEN=1`.
const GOLDEN: &str = "\
no-theme|sidebar|bg=Color(Color { r: 34, g: 29, b: 22, a: 255 }) | color=Color { r: 0, g: 0, b: 0, a: 255 } | bw=Edges { top: 0.0, right: 1.0, bottom: 0.0, left: 0.0 } | bc=Color { r: 36, g: 30, b: 21, a: 255 } | shadow=[]
no-theme|titlebar|bg=Color(Color { r: 20, g: 17, b: 12, a: 255 }) | color=Color { r: 184, g: 162, b: 117, a: 255 } | bw=Edges { top: 0.0, right: 0.0, bottom: 1.0, left: 0.0 } | bc=Color { r: 36, g: 30, b: 21, a: 255 } | shadow=[]
no-theme|tabbar|bg=Color(Color { r: 34, g: 29, b: 22, a: 255 }) | color=Color { r: 235, g: 220, b: 182, a: 255 } | bw=Edges { top: 0.0, right: 0.0, bottom: 1.0, left: 0.0 } | bc=Color { r: 52, g: 43, b: 30, a: 255 } | shadow=[]
no-theme|settings-page|bg=Color(Color { r: 28, g: 24, b: 18, a: 255 }) | color=Color { r: 0, g: 0, b: 0, a: 255 } | bw=Edges { top: 0.0, right: 0.0, bottom: 0.0, left: 0.0 } | bc=Color { r: 0, g: 0, b: 0, a: 0 } | shadow=[]
no-theme|set-page-header|bg=LinearGradient(LinearGradient { angle_deg: 180.0, stops: [GradientStop { color: Color { r: 34, g: 29, b: 22, a: 255 }, position: Percent(0.0) }, GradientStop { color: Color { r: 28, g: 24, b: 18, a: 255 }, position: Percent(1.0) }], repeating: false }) | color=Color { r: 0, g: 0, b: 0, a: 255 } | bw=Edges { top: 0.0, right: 0.0, bottom: 1.0, left: 0.0 } | bc=Color { r: 36, g: 30, b: 21, a: 255 } | shadow=[]
no-theme|theme-chip.dracula|bg=Color(Color { r: 33, g: 34, b: 44, a: 255 }) | color=Color { r: 248, g: 248, b: 242, a: 255 } | bw=Edges { top: 1.0, right: 1.0, bottom: 1.0, left: 1.0 } | bc=Color { r: 68, g: 71, b: 90, a: 255 } | shadow=[BoxShadow { offset_x: 0.0, offset_y: 0.0, blur_radius: 0.0, spread_radius: 1.0, color: Color { r: 52, g: 43, b: 30, a: 255 }, inset: true }]
no-theme|cp|bg=LinearGradient(LinearGradient { angle_deg: 180.0, stops: [GradientStop { color: Color { r: 41, g: 35, b: 26, a: 255 }, position: Percent(0.0) }, GradientStop { color: Color { r: 34, g: 29, b: 22, a: 255 }, position: Percent(1.0) }], repeating: false }) | color=Color { r: 0, g: 0, b: 0, a: 255 } | bw=Edges { top: 1.0, right: 1.0, bottom: 1.0, left: 1.0 } | bc=Color { r: 74, g: 62, b: 42, a: 255 } | shadow=[BoxShadow { offset_x: 0.0, offset_y: 14.0, blur_radius: 40.0, spread_radius: 0.0, color: Color { r: 0, g: 0, b: 0, a: 165 }, inset: false }, BoxShadow { offset_x: 0.0, offset_y: 0.0, blur_radius: 0.0, spread_radius: 1.0, color: Color { r: 0, g: 0, b: 0, a: 76 }, inset: false }, BoxShadow { offset_x: 0.0, offset_y: 0.0, blur_radius: 40.0, spread_radius: 0.0, color: Color { r: 212, g: 163, b: 72, a: 13 }, inset: false }]
no-theme|sb-row|bg=Color(Color { r: 0, g: 0, b: 0, a: 0 }) | color=Color { r: 0, g: 0, b: 0, a: 255 } | bw=Edges { top: 0.0, right: 0.0, bottom: 0.0, left: 0.0 } | bc=Color { r: 0, g: 0, b: 0, a: 0 } | shadow=[]
amber|sidebar|bg=Color(Color { r: 34, g: 29, b: 22, a: 255 }) | color=Color { r: 235, g: 220, b: 182, a: 255 } | bw=Edges { top: 0.0, right: 1.0, bottom: 0.0, left: 0.0 } | bc=Color { r: 52, g: 43, b: 30, a: 255 } | shadow=[]
amber|titlebar|bg=Color(Color { r: 20, g: 17, b: 12, a: 255 }) | color=Color { r: 184, g: 162, b: 117, a: 255 } | bw=Edges { top: 0.0, right: 0.0, bottom: 1.0, left: 0.0 } | bc=Color { r: 36, g: 30, b: 21, a: 255 } | shadow=[]
amber|tabbar|bg=Color(Color { r: 34, g: 29, b: 22, a: 255 }) | color=Color { r: 235, g: 220, b: 182, a: 255 } | bw=Edges { top: 0.0, right: 0.0, bottom: 1.0, left: 0.0 } | bc=Color { r: 36, g: 30, b: 21, a: 255 } | shadow=[]
amber|settings-page|bg=Color(Color { r: 28, g: 24, b: 18, a: 255 }) | color=Color { r: 235, g: 220, b: 182, a: 255 } | bw=Edges { top: 0.0, right: 0.0, bottom: 0.0, left: 0.0 } | bc=Color { r: 0, g: 0, b: 0, a: 0 } | shadow=[]
amber|set-page-header|bg=LinearGradient(LinearGradient { angle_deg: 180.0, stops: [GradientStop { color: Color { r: 34, g: 29, b: 22, a: 158 }, position: Percent(0.0) }, GradientStop { color: Color { r: 34, g: 29, b: 22, a: 42 }, position: Percent(0.72) }, GradientStop { color: Color { r: 28, g: 24, b: 18, a: 20 }, position: Percent(1.0) }], repeating: false }) | color=Color { r: 235, g: 220, b: 182, a: 255 } | bw=Edges { top: 0.0, right: 0.0, bottom: 1.0, left: 0.0 } | bc=Color { r: 36, g: 30, b: 21, a: 255 } | shadow=[]
amber|theme-chip.dracula|bg=Color(Color { r: 33, g: 34, b: 44, a: 255 }) | color=Color { r: 248, g: 248, b: 242, a: 255 } | bw=Edges { top: 1.0, right: 1.0, bottom: 1.0, left: 1.0 } | bc=Color { r: 68, g: 71, b: 90, a: 255 } | shadow=[BoxShadow { offset_x: 0.0, offset_y: 0.0, blur_radius: 0.0, spread_radius: 1.0, color: Color { r: 52, g: 43, b: 30, a: 255 }, inset: true }]
amber|cp|bg=LinearGradient(LinearGradient { angle_deg: 180.0, stops: [GradientStop { color: Color { r: 41, g: 35, b: 26, a: 255 }, position: Percent(0.0) }, GradientStop { color: Color { r: 34, g: 29, b: 22, a: 255 }, position: Percent(1.0) }], repeating: false }) | color=Color { r: 235, g: 220, b: 182, a: 255 } | bw=Edges { top: 1.0, right: 1.0, bottom: 1.0, left: 1.0 } | bc=Color { r: 74, g: 62, b: 42, a: 255 } | shadow=[BoxShadow { offset_x: 0.0, offset_y: 14.0, blur_radius: 40.0, spread_radius: 0.0, color: Color { r: 0, g: 0, b: 0, a: 165 }, inset: false }, BoxShadow { offset_x: 0.0, offset_y: 0.0, blur_radius: 0.0, spread_radius: 1.0, color: Color { r: 0, g: 0, b: 0, a: 76 }, inset: false }, BoxShadow { offset_x: 0.0, offset_y: 0.0, blur_radius: 40.0, spread_radius: 0.0, color: Color { r: 212, g: 163, b: 72, a: 13 }, inset: false }]
amber|sb-row|bg=Color(Color { r: 0, g: 0, b: 0, a: 0 }) | color=Color { r: 235, g: 220, b: 182, a: 255 } | bw=Edges { top: 0.0, right: 0.0, bottom: 0.0, left: 0.0 } | bc=Color { r: 0, g: 0, b: 0, a: 0 } | shadow=[]
dracula|sidebar|bg=Color(Color { r: 33, g: 34, b: 44, a: 255 }) | color=Color { r: 248, g: 248, b: 242, a: 255 } | bw=Edges { top: 0.0, right: 1.0, bottom: 0.0, left: 0.0 } | bc=Color { r: 68, g: 71, b: 90, a: 255 } | shadow=[]
dracula|titlebar|bg=Color(Color { r: 33, g: 34, b: 44, a: 255 }) | color=Color { r: 226, g: 226, b: 220, a: 255 } | bw=Edges { top: 0.0, right: 0.0, bottom: 1.0, left: 0.0 } | bc=Color { r: 33, g: 34, b: 44, a: 255 } | shadow=[]
dracula|tabbar|bg=Color(Color { r: 45, g: 47, b: 58, a: 255 }) | color=Color { r: 248, g: 248, b: 242, a: 255 } | bw=Edges { top: 0.0, right: 0.0, bottom: 1.0, left: 0.0 } | bc=Color { r: 56, g: 58, b: 89, a: 255 } | shadow=[]
dracula|settings-page|bg=Color(Color { r: 40, g: 42, b: 54, a: 255 }) | color=Color { r: 248, g: 248, b: 242, a: 255 } | bw=Edges { top: 0.0, right: 0.0, bottom: 0.0, left: 0.0 } | bc=Color { r: 0, g: 0, b: 0, a: 0 } | shadow=[]
dracula|set-page-header|bg=LinearGradient(LinearGradient { angle_deg: 180.0, stops: [GradientStop { color: Color { r: 45, g: 47, b: 58, a: 153 }, position: Percent(0.0) }, GradientStop { color: Color { r: 40, g: 42, b: 54, a: 0 }, position: Percent(1.0) }], repeating: false }) | color=Color { r: 248, g: 248, b: 242, a: 255 } | bw=Edges { top: 0.0, right: 0.0, bottom: 1.0, left: 0.0 } | bc=Color { r: 33, g: 34, b: 44, a: 255 } | shadow=[]
dracula|theme-chip.dracula|bg=Color(Color { r: 33, g: 34, b: 44, a: 255 }) | color=Color { r: 248, g: 248, b: 242, a: 255 } | bw=Edges { top: 1.0, right: 1.0, bottom: 1.0, left: 1.0 } | bc=Color { r: 68, g: 71, b: 90, a: 255 } | shadow=[BoxShadow { offset_x: 0.0, offset_y: 0.0, blur_radius: 0.0, spread_radius: 1.0, color: Color { r: 52, g: 55, b: 70, a: 255 }, inset: true }]
dracula|cp|bg=LinearGradient(LinearGradient { angle_deg: 180.0, stops: [GradientStop { color: Color { r: 52, g: 55, b: 70, a: 255 }, position: Percent(0.0) }, GradientStop { color: Color { r: 33, g: 34, b: 44, a: 255 }, position: Percent(1.0) }], repeating: false }) | color=Color { r: 248, g: 248, b: 242, a: 255 } | bw=Edges { top: 1.0, right: 1.0, bottom: 1.0, left: 1.0 } | bc=Color { r: 68, g: 71, b: 90, a: 255 } | shadow=[BoxShadow { offset_x: 0.0, offset_y: 14.0, blur_radius: 40.0, spread_radius: 0.0, color: Color { r: 0, g: 0, b: 0, a: 165 }, inset: false }, BoxShadow { offset_x: 0.0, offset_y: 0.0, blur_radius: 0.0, spread_radius: 1.0, color: Color { r: 0, g: 0, b: 0, a: 76 }, inset: false }, BoxShadow { offset_x: 0.0, offset_y: 0.0, blur_radius: 40.0, spread_radius: 0.0, color: Color { r: 212, g: 163, b: 72, a: 13 }, inset: false }]
dracula|sb-row|bg=Color(Color { r: 0, g: 0, b: 0, a: 0 }) | color=Color { r: 248, g: 248, b: 242, a: 255 } | bw=Edges { top: 0.0, right: 0.0, bottom: 0.0, left: 0.0 } | bc=Color { r: 0, g: 0, b: 0, a: 0 } | shadow=[]
nord|sidebar|bg=Color(Color { r: 59, g: 66, b: 82, a: 255 }) | color=Color { r: 236, g: 239, b: 244, a: 255 } | bw=Edges { top: 0.0, right: 1.0, bottom: 0.0, left: 0.0 } | bc=Color { r: 76, g: 86, b: 106, a: 255 } | shadow=[]
nord|titlebar|bg=Color(Color { r: 36, g: 41, b: 51, a: 255 }) | color=Color { r: 216, g: 222, b: 233, a: 255 } | bw=Edges { top: 0.0, right: 0.0, bottom: 1.0, left: 0.0 } | bc=Color { r: 41, g: 46, b: 56, a: 255 } | shadow=[]
nord|tabbar|bg=Color(Color { r: 53, g: 59, b: 72, a: 255 }) | color=Color { r: 236, g: 239, b: 244, a: 255 } | bw=Edges { top: 0.0, right: 0.0, bottom: 1.0, left: 0.0 } | bc=Color { r: 59, g: 66, b: 82, a: 255 } | shadow=[]
nord|settings-page|bg=Color(Color { r: 46, g: 52, b: 64, a: 255 }) | color=Color { r: 236, g: 239, b: 244, a: 255 } | bw=Edges { top: 0.0, right: 0.0, bottom: 0.0, left: 0.0 } | bc=Color { r: 0, g: 0, b: 0, a: 0 } | shadow=[]
nord|set-page-header|bg=LinearGradient(LinearGradient { angle_deg: 180.0, stops: [GradientStop { color: Color { r: 53, g: 59, b: 72, a: 153 }, position: Percent(0.0) }, GradientStop { color: Color { r: 46, g: 52, b: 64, a: 0 }, position: Percent(1.0) }], repeating: false }) | color=Color { r: 236, g: 239, b: 244, a: 255 } | bw=Edges { top: 0.0, right: 0.0, bottom: 1.0, left: 0.0 } | bc=Color { r: 41, g: 46, b: 56, a: 255 } | shadow=[]
nord|theme-chip.dracula|bg=Color(Color { r: 33, g: 34, b: 44, a: 255 }) | color=Color { r: 248, g: 248, b: 242, a: 255 } | bw=Edges { top: 1.0, right: 1.0, bottom: 1.0, left: 1.0 } | bc=Color { r: 68, g: 71, b: 90, a: 255 } | shadow=[BoxShadow { offset_x: 0.0, offset_y: 0.0, blur_radius: 0.0, spread_radius: 1.0, color: Color { r: 59, g: 66, b: 82, a: 255 }, inset: true }]
nord|cp|bg=LinearGradient(LinearGradient { angle_deg: 180.0, stops: [GradientStop { color: Color { r: 67, g: 76, b: 94, a: 255 }, position: Percent(0.0) }, GradientStop { color: Color { r: 59, g: 66, b: 82, a: 255 }, position: Percent(1.0) }], repeating: false }) | color=Color { r: 236, g: 239, b: 244, a: 255 } | bw=Edges { top: 1.0, right: 1.0, bottom: 1.0, left: 1.0 } | bc=Color { r: 67, g: 76, b: 94, a: 255 } | shadow=[BoxShadow { offset_x: 0.0, offset_y: 14.0, blur_radius: 40.0, spread_radius: 0.0, color: Color { r: 0, g: 0, b: 0, a: 165 }, inset: false }, BoxShadow { offset_x: 0.0, offset_y: 0.0, blur_radius: 0.0, spread_radius: 1.0, color: Color { r: 0, g: 0, b: 0, a: 76 }, inset: false }, BoxShadow { offset_x: 0.0, offset_y: 0.0, blur_radius: 40.0, spread_radius: 0.0, color: Color { r: 212, g: 163, b: 72, a: 13 }, inset: false }]
nord|sb-row|bg=Color(Color { r: 0, g: 0, b: 0, a: 0 }) | color=Color { r: 236, g: 239, b: 244, a: 255 } | bw=Edges { top: 0.0, right: 0.0, bottom: 0.0, left: 0.0 } | bc=Color { r: 0, g: 0, b: 0, a: 0 } | shadow=[]
tokyo-night|sidebar|bg=Color(Color { r: 36, g: 40, b: 59, a: 255 }) | color=Color { r: 192, g: 202, b: 245, a: 255 } | bw=Edges { top: 0.0, right: 1.0, bottom: 0.0, left: 0.0 } | bc=Color { r: 65, g: 72, b: 104, a: 255 } | shadow=[]
tokyo-night|titlebar|bg=Color(Color { r: 22, g: 22, b: 30, a: 255 }) | color=Color { r: 169, g: 177, b: 214, a: 255 } | bw=Edges { top: 0.0, right: 0.0, bottom: 1.0, left: 0.0 } | bc=Color { r: 28, g: 30, b: 42, a: 255 } | shadow=[]
tokyo-night|tabbar|bg=Color(Color { r: 36, g: 40, b: 59, a: 255 }) | color=Color { r: 192, g: 202, b: 245, a: 255 } | bw=Edges { top: 0.0, right: 0.0, bottom: 1.0, left: 0.0 } | bc=Color { r: 41, g: 46, b: 66, a: 255 } | shadow=[]
tokyo-night|settings-page|bg=Color(Color { r: 26, g: 27, b: 38, a: 255 }) | color=Color { r: 192, g: 202, b: 245, a: 255 } | bw=Edges { top: 0.0, right: 0.0, bottom: 0.0, left: 0.0 } | bc=Color { r: 0, g: 0, b: 0, a: 0 } | shadow=[]
tokyo-night|set-page-header|bg=LinearGradient(LinearGradient { angle_deg: 180.0, stops: [GradientStop { color: Color { r: 31, g: 35, b: 53, a: 153 }, position: Percent(0.0) }, GradientStop { color: Color { r: 26, g: 27, b: 38, a: 0 }, position: Percent(1.0) }], repeating: false }) | color=Color { r: 192, g: 202, b: 245, a: 255 } | bw=Edges { top: 0.0, right: 0.0, bottom: 1.0, left: 0.0 } | bc=Color { r: 28, g: 30, b: 42, a: 255 } | shadow=[]
tokyo-night|theme-chip.dracula|bg=Color(Color { r: 33, g: 34, b: 44, a: 255 }) | color=Color { r: 248, g: 248, b: 242, a: 255 } | bw=Edges { top: 1.0, right: 1.0, bottom: 1.0, left: 1.0 } | bc=Color { r: 68, g: 71, b: 90, a: 255 } | shadow=[BoxShadow { offset_x: 0.0, offset_y: 0.0, blur_radius: 0.0, spread_radius: 1.0, color: Color { r: 41, g: 46, b: 66, a: 255 }, inset: true }]
tokyo-night|cp|bg=LinearGradient(LinearGradient { angle_deg: 180.0, stops: [GradientStop { color: Color { r: 41, g: 46, b: 66, a: 255 }, position: Percent(0.0) }, GradientStop { color: Color { r: 36, g: 40, b: 59, a: 255 }, position: Percent(1.0) }], repeating: false }) | color=Color { r: 192, g: 202, b: 245, a: 255 } | bw=Edges { top: 1.0, right: 1.0, bottom: 1.0, left: 1.0 } | bc=Color { r: 65, g: 72, b: 104, a: 255 } | shadow=[BoxShadow { offset_x: 0.0, offset_y: 14.0, blur_radius: 40.0, spread_radius: 0.0, color: Color { r: 0, g: 0, b: 0, a: 165 }, inset: false }, BoxShadow { offset_x: 0.0, offset_y: 0.0, blur_radius: 0.0, spread_radius: 1.0, color: Color { r: 0, g: 0, b: 0, a: 76 }, inset: false }, BoxShadow { offset_x: 0.0, offset_y: 0.0, blur_radius: 40.0, spread_radius: 0.0, color: Color { r: 212, g: 163, b: 72, a: 13 }, inset: false }]
tokyo-night|sb-row|bg=Color(Color { r: 0, g: 0, b: 0, a: 0 }) | color=Color { r: 192, g: 202, b: 245, a: 255 } | bw=Edges { top: 0.0, right: 0.0, bottom: 0.0, left: 0.0 } | bc=Color { r: 0, g: 0, b: 0, a: 0 } | shadow=[]
gruvbox|sidebar|bg=Color(Color { r: 50, g: 48, b: 47, a: 255 }) | color=Color { r: 235, g: 219, b: 178, a: 255 } | bw=Edges { top: 0.0, right: 1.0, bottom: 0.0, left: 0.0 } | bc=Color { r: 80, g: 73, b: 69, a: 255 } | shadow=[]
gruvbox|titlebar|bg=Color(Color { r: 29, g: 32, b: 33, a: 255 }) | color=Color { r: 213, g: 196, b: 161, a: 255 } | bw=Edges { top: 0.0, right: 0.0, bottom: 1.0, left: 0.0 } | bc=Color { r: 35, g: 37, b: 39, a: 255 } | shadow=[]
gruvbox|tabbar|bg=Color(Color { r: 50, g: 48, b: 47, a: 255 }) | color=Color { r: 235, g: 219, b: 178, a: 255 } | bw=Edges { top: 0.0, right: 0.0, bottom: 1.0, left: 0.0 } | bc=Color { r: 60, g: 56, b: 54, a: 255 } | shadow=[]
gruvbox|settings-page|bg=Color(Color { r: 40, g: 40, b: 40, a: 255 }) | color=Color { r: 235, g: 219, b: 178, a: 255 } | bw=Edges { top: 0.0, right: 0.0, bottom: 0.0, left: 0.0 } | bc=Color { r: 0, g: 0, b: 0, a: 0 } | shadow=[]
gruvbox|set-page-header|bg=LinearGradient(LinearGradient { angle_deg: 180.0, stops: [GradientStop { color: Color { r: 50, g: 48, b: 47, a: 153 }, position: Percent(0.0) }, GradientStop { color: Color { r: 40, g: 40, b: 40, a: 0 }, position: Percent(1.0) }], repeating: false }) | color=Color { r: 235, g: 219, b: 178, a: 255 } | bw=Edges { top: 0.0, right: 0.0, bottom: 1.0, left: 0.0 } | bc=Color { r: 35, g: 37, b: 39, a: 255 } | shadow=[]
gruvbox|theme-chip.dracula|bg=Color(Color { r: 33, g: 34, b: 44, a: 255 }) | color=Color { r: 248, g: 248, b: 242, a: 255 } | bw=Edges { top: 1.0, right: 1.0, bottom: 1.0, left: 1.0 } | bc=Color { r: 68, g: 71, b: 90, a: 255 } | shadow=[BoxShadow { offset_x: 0.0, offset_y: 0.0, blur_radius: 0.0, spread_radius: 1.0, color: Color { r: 60, g: 56, b: 54, a: 255 }, inset: true }]
gruvbox|cp|bg=LinearGradient(LinearGradient { angle_deg: 180.0, stops: [GradientStop { color: Color { r: 60, g: 56, b: 54, a: 255 }, position: Percent(0.0) }, GradientStop { color: Color { r: 50, g: 48, b: 47, a: 255 }, position: Percent(1.0) }], repeating: false }) | color=Color { r: 235, g: 219, b: 178, a: 255 } | bw=Edges { top: 1.0, right: 1.0, bottom: 1.0, left: 1.0 } | bc=Color { r: 80, g: 73, b: 69, a: 255 } | shadow=[BoxShadow { offset_x: 0.0, offset_y: 14.0, blur_radius: 40.0, spread_radius: 0.0, color: Color { r: 0, g: 0, b: 0, a: 165 }, inset: false }, BoxShadow { offset_x: 0.0, offset_y: 0.0, blur_radius: 0.0, spread_radius: 1.0, color: Color { r: 0, g: 0, b: 0, a: 76 }, inset: false }, BoxShadow { offset_x: 0.0, offset_y: 0.0, blur_radius: 40.0, spread_radius: 0.0, color: Color { r: 212, g: 163, b: 72, a: 13 }, inset: false }]
gruvbox|sb-row|bg=Color(Color { r: 0, g: 0, b: 0, a: 0 }) | color=Color { r: 235, g: 219, b: 178, a: 255 } | bw=Edges { top: 0.0, right: 0.0, bottom: 0.0, left: 0.0 } | bc=Color { r: 0, g: 0, b: 0, a: 0 } | shadow=[]
catppuccin|sidebar|bg=Color(Color { r: 24, g: 24, b: 37, a: 255 }) | color=Color { r: 205, g: 214, b: 244, a: 255 } | bw=Edges { top: 0.0, right: 1.0, bottom: 0.0, left: 0.0 } | bc=Color { r: 49, g: 50, b: 68, a: 255 } | shadow=[]
catppuccin|titlebar|bg=Color(Color { r: 17, g: 17, b: 27, a: 255 }) | color=Color { r: 186, g: 194, b: 222, a: 255 } | bw=Edges { top: 0.0, right: 0.0, bottom: 1.0, left: 0.0 } | bc=Color { r: 35, g: 34, b: 56, a: 255 } | shadow=[]
catppuccin|tabbar|bg=Color(Color { r: 24, g: 24, b: 37, a: 255 }) | color=Color { r: 205, g: 214, b: 244, a: 255 } | bw=Edges { top: 0.0, right: 0.0, bottom: 1.0, left: 0.0 } | bc=Color { r: 49, g: 50, b: 68, a: 255 } | shadow=[]
catppuccin|settings-page|bg=Color(Color { r: 30, g: 30, b: 46, a: 255 }) | color=Color { r: 205, g: 214, b: 244, a: 255 } | bw=Edges { top: 0.0, right: 0.0, bottom: 0.0, left: 0.0 } | bc=Color { r: 0, g: 0, b: 0, a: 0 } | shadow=[]
catppuccin|set-page-header|bg=LinearGradient(LinearGradient { angle_deg: 180.0, stops: [GradientStop { color: Color { r: 24, g: 24, b: 37, a: 153 }, position: Percent(0.0) }, GradientStop { color: Color { r: 30, g: 30, b: 46, a: 0 }, position: Percent(1.0) }], repeating: false }) | color=Color { r: 205, g: 214, b: 244, a: 255 } | bw=Edges { top: 0.0, right: 0.0, bottom: 1.0, left: 0.0 } | bc=Color { r: 35, g: 34, b: 56, a: 255 } | shadow=[]
catppuccin|theme-chip.dracula|bg=Color(Color { r: 33, g: 34, b: 44, a: 255 }) | color=Color { r: 248, g: 248, b: 242, a: 255 } | bw=Edges { top: 1.0, right: 1.0, bottom: 1.0, left: 1.0 } | bc=Color { r: 68, g: 71, b: 90, a: 255 } | shadow=[BoxShadow { offset_x: 0.0, offset_y: 0.0, blur_radius: 0.0, spread_radius: 1.0, color: Color { r: 52, g: 43, b: 30, a: 255 }, inset: true }]
catppuccin|cp|bg=LinearGradient(LinearGradient { angle_deg: 180.0, stops: [GradientStop { color: Color { r: 41, g: 35, b: 26, a: 255 }, position: Percent(0.0) }, GradientStop { color: Color { r: 34, g: 29, b: 22, a: 255 }, position: Percent(1.0) }], repeating: false }) | color=Color { r: 205, g: 214, b: 244, a: 255 } | bw=Edges { top: 1.0, right: 1.0, bottom: 1.0, left: 1.0 } | bc=Color { r: 74, g: 62, b: 42, a: 255 } | shadow=[BoxShadow { offset_x: 0.0, offset_y: 14.0, blur_radius: 40.0, spread_radius: 0.0, color: Color { r: 0, g: 0, b: 0, a: 165 }, inset: false }, BoxShadow { offset_x: 0.0, offset_y: 0.0, blur_radius: 0.0, spread_radius: 1.0, color: Color { r: 0, g: 0, b: 0, a: 76 }, inset: false }, BoxShadow { offset_x: 0.0, offset_y: 0.0, blur_radius: 40.0, spread_radius: 0.0, color: Color { r: 212, g: 163, b: 72, a: 13 }, inset: false }]
catppuccin|sb-row|bg=Color(Color { r: 0, g: 0, b: 0, a: 0 }) | color=Color { r: 205, g: 214, b: 244, a: 255 } | bw=Edges { top: 0.0, right: 0.0, bottom: 0.0, left: 0.0 } | bc=Color { r: 0, g: 0, b: 0, a: 0 } | shadow=[]";
