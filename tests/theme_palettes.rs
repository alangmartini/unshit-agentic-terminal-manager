//! Every selectable palette must own its colors instead of falling back to Amber.
use unshit::core::element::{ElementDef, ElementTree, Tag};
use unshit::core::style::parse::CompiledStylesheet;
use unshit_test::TestHarness;

const STYLES: &str = include_str!("../assets/styles.css");

fn theme_ids() -> Vec<String> {
    let catalog: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("../assets/themes.json")).unwrap();
    let mut ids: Vec<_> = catalog
        .iter()
        .map(|theme| theme["id"].as_str().unwrap().to_owned())
        .collect();
    // Custom is selectable but is not a built-in preset in the catalog.
    ids.push("custom".into());
    ids
}

#[test]
fn all_themes_define_their_palette_tokens() {
    let sheet = CompiledStylesheet::parse(STYLES);
    let base = sheet.token_scopes.base_vars().unwrap();
    let mut missing = Vec::new();
    for theme in theme_ids() {
        let selector = format!(".app.theme-{theme}");
        let scope = sheet.token_scopes.by_selector(&selector).expect(&selector);
        if theme == "amber" {
            continue; // :root is deliberately the original Amber palette.
        }
        for (token, value) in base {
            // Neutral shadows, spacing and typography are shared. Aliases can
            // also be shared: var() resolves through the active theme at runtime.
            let is_palette = [
                "--bg-",
                "--fg-",
                "--border-",
                "--amber-",
                "--accent-",
                "--glow-",
                "--theme-chip-",
                "--cp-accent",
                "--ember",
                "--sage",
                "--rust",
                "--azure",
                "--violet",
            ]
            .iter()
            .any(|prefix| token.starts_with(prefix));
            if is_palette && !value.trim().starts_with("var(") && !scope.vars.contains_key(token) {
                missing.push(format!("{theme}: {token}"));
            }
        }
    }
    missing.sort();
    assert!(
        missing.is_empty(),
        "Palette tokens still inherited from Amber:\n{}",
        missing.join("\n")
    );
}

fn chrome_tree(theme: &str) -> ElementTree {
    let mut root = ElementDef::new(Tag::Div)
        .with_class("app")
        .with_class(format!("theme-{theme}"));
    for classes in [
        &["tab", "active"][..],
        &["pill-btn", "tm-search"],
        &["explorer-action", "active"],
        &["workspace", "active"],
        &["palette-base"],
        &["palette-elevated"],
        &["palette-highlight"],
    ] {
        let mut child = ElementDef::new(Tag::Div);
        for class in classes {
            child = child.with_class(*class);
        }
        if classes[0] == "workspace" {
            child = child.with_child(ElementDef::new(Tag::Div).with_class("workspace-name"));
        }
        root = root.with_child(child);
    }
    ElementTree { root }
}

#[test]
fn switching_every_theme_restyles_chrome_from_its_palette() {
    let styles = format!(
        "{STYLES}\n.palette-base {{ background: var(--bg-base); }}
        .palette-elevated {{ background: var(--bg-elevated); }}
        .palette-highlight {{ color: var(--amber-50); }}"
    );
    let mut harness = TestHarness::new(&styles, || chrome_tree("amber"), 1280.0, 800.0);
    for theme in theme_ids() {
        // Returning to Amber also catches stale values leaking from a theme.
        for next in [theme.as_str(), "amber"] {
            harness.rebuild(|| chrome_tree(next));
            for (selector, reference) in [
                (".tab.active", ".palette-base"),
                (".tm-search", ".palette-base"),
                (".explorer-action.active", ".palette-elevated"),
            ] {
                assert_eq!(
                    harness.query(selector).unwrap().computed_style.background,
                    harness.query(reference).unwrap().computed_style.background,
                    "{next}: {selector} must use the active palette"
                );
            }
            assert_eq!(
                harness
                    .query(".workspace-name")
                    .unwrap()
                    .computed_style
                    .color,
                harness
                    .query(".palette-highlight")
                    .unwrap()
                    .computed_style
                    .color,
                "{next}: active workspace must use the active palette"
            );
        }
    }
}
