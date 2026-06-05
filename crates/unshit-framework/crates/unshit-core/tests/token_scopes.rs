//! Stage 1 (cascade-aware token-scope collection) against the real app
//! stylesheet. These assertions pin the SHAPE of the collected scopes — they do
//! NOT exercise resolution (var() is still resolved globally this stage, so the
//! rendered output is unchanged). They guard that:
//!   - the per-theme `.app.theme-*` blocks are collected as their own scopes
//!     (today the engine drops every non-:root `--token`),
//!   - the `:root` base scope is collapsed into scope 0,
//!   - the per-scope pre-flatten layers a scope's overrides over the base
//!     (`--cp-accent: var(--amber-300)` picks up the theme's amber).

use unshit_core::style::parse::CompiledStylesheet;

const STYLES: &str = include_str!("../../../../../assets/styles.css");

#[test]
fn collects_theme_scopes_from_app_stylesheet() {
    let sheet = CompiledStylesheet::parse(STYLES);
    let scopes = &sheet.token_scopes;

    // Base scope is present and is scope 0.
    let base = scopes.base().expect(":root base scope");
    assert_eq!(base.key.0, 0, "base scope must be index 0");
    assert_eq!(base.selector_text, ":root");
    assert_eq!(base.vars.get("--bg-base").map(String::as_str), Some("#1c1812"), ":root --bg-base");

    // The dracula app theme is its own scope with the theme's --bg-base.
    let dracula = scopes
        .by_selector(".app.theme-dracula")
        .expect(".app.theme-dracula scope must be collected");
    assert_eq!(
        dracula.vars.get("--bg-base").map(String::as_str),
        Some("#282a36"),
        ".app.theme-dracula --bg-base"
    );

    // The dracula theme chip is also its own scope.
    let chip = scopes
        .by_selector(".theme-chip.dracula")
        .expect(".theme-chip.dracula scope must be collected");
    assert_eq!(
        chip.vars.get("--theme-chip-accent").map(String::as_str),
        Some("#bd93f9"),
        ".theme-chip.dracula --theme-chip-accent"
    );
}

#[test]
fn base_token_to_token_refs_are_stored_raw() {
    // `:root` declares `--cp-accent: var(--amber-300)`,
    // `--theme-chip-accent: var(--amber-300)`, and
    // `--theme-chip-preview-bg: var(--bg-elevated)`. These cross-token aliases
    // are stored RAW (NOT eagerly concretized to the base amber/elevated value),
    // so a theme override of the inner token propagates to consumers that reach
    // it through the alias. Resolution happens lazily at use time against the
    // element's `ScopeEnv`.
    let sheet = CompiledStylesheet::parse(STYLES);
    let base = sheet.token_scopes.base().unwrap();
    assert_eq!(
        base.vars.get("--cp-accent").map(String::as_str),
        Some("var(--amber-300)"),
        "base --cp-accent alias must be kept raw, not pre-flattened to the base amber",
    );
    assert_eq!(
        base.vars.get("--theme-chip-accent").map(String::as_str),
        Some("var(--amber-300)"),
        "base --theme-chip-accent alias must be kept raw",
    );
    assert_eq!(
        base.vars.get("--theme-chip-preview-bg").map(String::as_str),
        Some("var(--bg-elevated)"),
        "base --theme-chip-preview-bg alias must be kept raw",
    );
}

#[test]
fn two_level_alias_propagates_theme_override_at_use_time() {
    // A theme that overrides ONLY the inner token (--amber-300) must, for a
    // consumer reaching --cp-accent through the base alias, resolve to the theme
    // amber rather than the base amber — proving the two-level indirection fix.
    // The scope itself does NOT redeclare --cp-accent (unlike the old test); the
    // override must flow through the :root alias.
    use unshit_core::style::parse::ScopeEnv;
    let css = r#"
        :root { --amber-300: #d4a348; --cp-accent: var(--amber-300); }
        .app.theme-dracula { --amber-300: #bd93f9; }
    "#;
    let sheet = CompiledStylesheet::parse(css);
    let base = sheet.token_scopes.base().unwrap();
    let dracula = sheet.token_scopes.by_selector(".app.theme-dracula").unwrap();
    // Resolving var(--cp-accent) against [dracula, base] reaches dracula's amber.
    let env = ScopeEnv::new(None, Some(dracula.vars.as_ref()), Some(base.vars.as_ref()));
    assert_eq!(
        ScopeEnv::resolve_value("var(--cp-accent)", &env),
        "#bd93f9",
        "theme override of --amber-300 must propagate through the base --cp-accent alias",
    );
    // With no theme active, the same alias falls back to the base amber.
    let base_only = ScopeEnv::new(None, None, Some(base.vars.as_ref()));
    assert_eq!(ScopeEnv::resolve_value("var(--cp-accent)", &base_only), "#d4a348");
}

#[test]
fn scopes_are_source_ordered_and_base_first() {
    let sheet = CompiledStylesheet::parse(STYLES);
    let scopes = &sheet.token_scopes.scopes;
    assert!(!scopes.is_empty());
    // Scope 0 is the base.
    assert_eq!(scopes[0].selector_text, ":root");
    // Keys are dense indices 0..n in order.
    for (i, s) in scopes.iter().enumerate() {
        assert_eq!(s.key.0 as usize, i, "scope key must equal its index");
    }
    // source_order is non-decreasing across scopes (base first, themes after).
    for w in scopes.windows(2) {
        assert!(
            w[0].source_order <= w[1].source_order,
            "scopes must be in source order: {} then {}",
            w[0].source_order,
            w[1].source_order
        );
    }
    eprintln!("token scopes collected from styles.css: {}", scopes.len());
}
