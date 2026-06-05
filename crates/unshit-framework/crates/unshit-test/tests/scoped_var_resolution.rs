//! Stage 3 POSITIVE tests: `var()` now resolves PER ELEMENT against the active
//! token scopes (self widget scope > active `.app.theme-*` root scope > `:root`
//! base), so a themed `--token` override wins where the author used `var()`.
//!
//! Unlike the Stage-0 golden (which is dominated by concrete clone declarations
//! that win by source order), these stylesheets are SYNTHETIC and var()-ONLY:
//! there is no concrete clone to fall back on, so the only way the assertion can
//! pass is if the cascade resolves the custom property against the element's
//! active scope. That isolates the new behavior the flip introduced.

use unshit_core::element::{ElementDef, ElementTree, Tag};
use unshit_core::style::parse::CompiledStylesheet;
use unshit_core::style::types::{Background, Color};
use unshit_test::TestHarness;

/// `.app.theme-dracula .sidebar { background: var(--bg-subtle) }` must resolve to
/// the DRACULA `--bg-subtle` (#21222c), not the `:root` amber value — proving the
/// active root theme scope wins over the base scope for a descendant element.
#[test]
fn theme_scope_overrides_root_for_descendant_var() {
    // var()-only: NO concrete `.app.theme-dracula .sidebar { background: ... }`
    // clone exists here, so a pass means the env resolved the token per scope.
    let css = r#"
        :root {
            --bg-base:   #1c1812;
            --bg-subtle: #29231a;
        }
        .app.theme-dracula {
            --bg-base:   #282a36;
            --bg-subtle: #21222c;
        }
        .sidebar { background: var(--bg-subtle); }
        .pane   { background: var(--bg-base); }
    "#;

    // Tree: .app.theme-dracula > .layout > { .sidebar, .pane }
    let build = || ElementTree {
        root: ElementDef::new(Tag::Div).with_class("app").with_class("theme-dracula").with_child(
            ElementDef::new(Tag::Div)
                .with_class("layout")
                .with_child(ElementDef::new(Tag::Div).with_class("sidebar"))
                .with_child(ElementDef::new(Tag::Div).with_class("pane")),
        ),
    };
    let h = TestHarness::new(css, build, 1280.0, 800.0);

    // --bg-subtle resolves to the DRACULA value (#21222c), not :root (#29231a).
    assert_eq!(
        h.query(".sidebar").unwrap().computed_style.background,
        Background::Color(Color::rgb(0x21, 0x22, 0x2c)),
        "var(--bg-subtle) under .app.theme-dracula must be the DRACULA value, not :root"
    );
    // --bg-base resolves to the DRACULA value (#282a36), not :root (#1c1812).
    assert_eq!(
        h.query(".pane").unwrap().computed_style.background,
        Background::Color(Color::rgb(0x28, 0x2a, 0x36)),
        "var(--bg-base) under .app.theme-dracula must be the DRACULA value, not :root"
    );
}

/// TWO-LEVEL INDIRECTION (the blocker fix): a base-scope token whose value is
/// itself a `var()` reference (`--cp-accent: var(--amber-300)`) must NOT be
/// eagerly concretized against `:root`. When a theme overrides only the INNER
/// token (`--amber-300`), a consumer of `var(--cp-accent)` — which reaches the
/// inner token only through the base alias — must resolve to the THEME value.
/// This is the exact `.cp-mode-pill .pfx { color: var(--cp-accent) }` site that
/// previously rendered the `:root` amber under every theme.
#[test]
fn two_level_base_alias_propagates_inner_token_theme_override() {
    // `:root` aliases --cp-accent to --amber-300; dracula overrides ONLY
    // --amber-300 (NOT --cp-accent). A descendant consumer of var(--cp-accent)
    // under .app.theme-dracula must resolve to dracula's amber (#bd93f9), not the
    // default (#d4a348). var()-ONLY: no concrete clone exists to fall back on.
    let css = r#"
        :root {
            --amber-300: #d4a348;
            --cp-accent: var(--amber-300);
        }
        .app.theme-dracula { --amber-300: #bd93f9; }
        .pfx { color: var(--cp-accent); }
    "#;
    // Tree: .app.theme-dracula > .cp-mode-pill > .pfx (a descendant, NOT the
    // root and NOT carrying the cp-accent scope itself — it reaches --cp-accent
    // purely through the :root alias).
    let build = || ElementTree {
        root: ElementDef::new(Tag::Div).with_class("app").with_class("theme-dracula").with_child(
            ElementDef::new(Tag::Div)
                .with_class("cp-mode-pill")
                .with_child(ElementDef::new(Tag::Div).with_class("pfx")),
        ),
    };
    let h = TestHarness::new(css, build, 1280.0, 800.0);
    assert_eq!(
        h.query(".pfx").unwrap().computed_style.color,
        Color::rgb(0xbd, 0x93, 0xf9),
        "theme override of the inner --amber-300 must propagate through the \
         base-scope --cp-accent alias to var(--cp-accent)"
    );

    // Control: with no theme active, the same alias resolves to the base amber.
    let build_no_theme = || ElementTree {
        root: ElementDef::new(Tag::Div).with_class("app").with_child(
            ElementDef::new(Tag::Div)
                .with_class("cp-mode-pill")
                .with_child(ElementDef::new(Tag::Div).with_class("pfx")),
        ),
    };
    let h2 = TestHarness::new(css, build_no_theme, 1280.0, 800.0);
    assert_eq!(
        h2.query(".pfx").unwrap().computed_style.color,
        Color::rgb(0xd4, 0xa3, 0x48),
        "with no theme active, var(--cp-accent) must fall back to the :root amber"
    );
}

/// The real app stylesheet exhibits the same two-level structure
/// (`:root { --cp-accent: var(--amber-300) }`, themes override `--amber-300`,
/// `.cp-mode-pill .pfx { color: var(--cp-accent) }`). Under `.app.theme-dracula`
/// the `.pfx` color must be dracula's amber, proving the fix on the live sheet.
#[test]
fn app_stylesheet_cp_mode_pill_pfx_uses_theme_amber() {
    const STYLES: &str = include_str!("../../../../../assets/styles.css");
    let build = || ElementTree {
        root: ElementDef::new(Tag::Div).with_class("app").with_class("theme-dracula").with_child(
            ElementDef::new(Tag::Div).with_class("cp-scrim").with_child(
                ElementDef::new(Tag::Div)
                    .with_class("cp-mode-pill")
                    .with_child(ElementDef::new(Tag::Div).with_class("pfx")),
            ),
        ),
    };
    let h = TestHarness::new(STYLES, build, 1280.0, 800.0);
    // dracula's --amber-300 is #bd93f9 (assets/styles.css). Before the two-level
    // fix this resolved to the :root amber #d4a348 regardless of theme.
    assert_eq!(
        h.query(".pfx").unwrap().computed_style.color,
        Color::rgb(0xbd, 0x93, 0xf9),
        ".cp-mode-pill .pfx under .app.theme-dracula must use dracula's amber via \
         the base-scope --cp-accent alias"
    );
}

/// With NO theme class on the root, the same var()-only stylesheet must resolve
/// against `:root` — the env's base scope is the only one active.
#[test]
fn base_scope_used_when_no_theme_active() {
    let css = r#"
        :root { --bg-subtle: #29231a; }
        .app.theme-dracula { --bg-subtle: #21222c; }
        .sidebar { background: var(--bg-subtle); }
    "#;
    let build = || ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("app")
            .with_child(ElementDef::new(Tag::Div).with_class("sidebar")),
    };
    let h = TestHarness::new(css, build, 1280.0, 800.0);
    assert_eq!(
        h.query(".sidebar").unwrap().computed_style.background,
        Background::Color(Color::rgb(0x29, 0x23, 0x1a)),
        "with no theme class, var(--bg-subtle) must resolve to :root"
    );
}

/// A widget SELF scope: `.theme-chip.dracula` carries its own `--token` class on
/// the element ITSELF (not the root). `var(--theme-chip-accent)` on that element
/// must resolve to the self-scope value, winning over `:root`, even when the root
/// is a different (or no) theme.
#[test]
fn self_scope_overrides_root_for_widget_element() {
    let css = r#"
        :root { --theme-chip-accent: #d4a348; }
        .theme-chip.dracula { --theme-chip-accent: #bd93f9; }
        .theme-chip { color: var(--theme-chip-accent); }
    "#;
    // Root is .app.theme-nord (a DIFFERENT theme that does not define the chip
    // accent) wrapping a .theme-chip.dracula element. The self scope must win.
    let build = || ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("app")
            .with_class("theme-nord")
            .with_child(ElementDef::new(Tag::Div).with_class("theme-chip").with_class("dracula")),
    };
    let h = TestHarness::new(css, build, 1280.0, 800.0);
    assert_eq!(
        h.query(".theme-chip").unwrap().computed_style.color,
        Color::rgb(0xbd, 0x93, 0xf9),
        "var(--theme-chip-accent) on .theme-chip.dracula must use the SELF scope value"
    );
}

/// The self scope wins over an active ROOT theme scope that also defines the same
/// token (self has higher specificity in the env order).
#[test]
fn self_scope_beats_active_root_theme_scope() {
    let css = r#"
        :root { --accent: #000000; }
        .app.theme-dracula { --accent: #111111; }
        .theme-chip.dracula { --accent: #bd93f9; }
        .theme-chip { color: var(--accent); }
    "#;
    let build = || ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("app")
            .with_class("theme-dracula")
            .with_child(ElementDef::new(Tag::Div).with_class("theme-chip").with_class("dracula")),
    };
    let h = TestHarness::new(css, build, 1280.0, 800.0);
    assert_eq!(
        h.query(".theme-chip").unwrap().computed_style.color,
        Color::rgb(0xbd, 0x93, 0xf9),
        "the widget self scope must win over the active root theme scope"
    );
}

/// The 473 themed `--token` overrides are now COLLECTED, not dropped: the
/// custom-property drop count from `parse()` of the real app stylesheet is ~0.
#[test]
fn custom_property_drops_fall_to_zero_for_app_stylesheet() {
    const STYLES: &str = include_str!("../../../../../assets/styles.css");
    let sheet = CompiledStylesheet::parse(STYLES);
    let custom_drops = sheet.dropped.iter().filter(|d| d.is_custom_property()).count();
    assert_eq!(
        custom_drops, 0,
        "themed custom-property overrides must be collected, not dropped (was 579 at Stage 0)"
    );
}

/// A malformed / unresolvable scoped `var()` (a token that no active scope
/// defines and that has no fallback) must NOT silently apply: the declaration is
/// dropped, leaving the property at its inherited/default value rather than a
/// garbage one. (The live cascade routes such a value to its drop sink; here we
/// observe that the property is simply not set to anything bogus.)
#[test]
fn malformed_scoped_var_does_not_apply() {
    let css = r#"
        :root { --known: #00ff00; }
        .app.theme-dracula { --known: #112233; }
        /* --missing is defined by NO scope and has no fallback */
        .widget { background: var(--missing); color: var(--known); }
    "#;
    let build = || ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("app")
            .with_class("theme-dracula")
            .with_child(ElementDef::new(Tag::Div).with_class("widget")),
    };
    let h = TestHarness::new(css, build, 1280.0, 800.0);
    let cs = h.query(".widget").unwrap().computed_style;
    // The resolvable sibling declaration still applies (theme value), proving the
    // bad one did not poison the rest of the block.
    assert_eq!(
        cs.color,
        Color::rgb(0x11, 0x22, 0x33),
        "the resolvable var(--known) must still apply the theme value"
    );
    // The malformed var(--missing) must NOT have set a real background — it stays
    // at the default (transparent), not some accidental concrete color.
    assert_eq!(
        cs.background,
        Background::Color(Color::TRANSPARENT),
        "an unresolvable scoped var() must be dropped, not applied"
    );
}
