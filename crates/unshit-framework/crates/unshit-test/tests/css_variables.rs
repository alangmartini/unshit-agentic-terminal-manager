use unshit_core::element::*;
use unshit_core::style::parse::CompiledStylesheet;
use unshit_core::style::types::*;
use unshit_test::TestHarness;

#[test]
fn root_variable_resolves_to_background_color() {
    let css = r#"
        :root { --primary: #ff0000; }
        .box { background: var(--primary); width: 100px; height: 100px; }
    "#;
    let h = TestHarness::new(
        css,
        || ElementTree { root: ElementDef::new(Tag::Div).with_class("box") },
        800.0,
        600.0,
    );

    let snap = h.query(".box").unwrap();
    // #ff0000 = red
    assert_eq!(
        snap.computed_style.background,
        Background::Color(Color::rgb(255, 0, 0)),
        "var(--primary) should resolve to #ff0000 (red)"
    );
}

#[test]
fn fallback_used_when_variable_missing() {
    let css = r#"
        .box { background: var(--missing, blue); width: 100px; height: 100px; }
    "#;
    let h = TestHarness::new(
        css,
        || ElementTree { root: ElementDef::new(Tag::Div).with_class("box") },
        800.0,
        600.0,
    );

    let snap = h.query(".box").unwrap();
    assert_eq!(
        snap.computed_style.background,
        Background::Color(Color::rgb(0, 0, 255)),
        "var(--missing, blue) should fall back to blue"
    );
}

#[test]
fn variable_in_color_property() {
    let css = r#"
        :root { --text-color: #00ff00; }
        .label { color: var(--text-color); width: 100px; height: 100px; }
    "#;
    let h = TestHarness::new(
        css,
        || ElementTree { root: ElementDef::new(Tag::Div).with_class("label") },
        800.0,
        600.0,
    );

    let snap = h.query(".label").unwrap();
    assert_eq!(
        snap.computed_style.color,
        Color::rgb(0, 255, 0),
        "var(--text-color) should resolve to #00ff00 (green)"
    );
}

#[test]
fn variable_in_font_size() {
    let css = r#"
        :root { --heading-size: 24px; }
        .title { font-size: var(--heading-size); width: 100px; height: 100px; }
    "#;
    let h = TestHarness::new(
        css,
        || ElementTree { root: ElementDef::new(Tag::Div).with_class("title") },
        800.0,
        600.0,
    );

    let snap = h.query(".title").unwrap();
    assert!(
        (snap.computed_style.font_size - 24.0).abs() < 0.1,
        "var(--heading-size) should resolve to 24px, got {}",
        snap.computed_style.font_size
    );
}

#[test]
fn variable_in_padding() {
    let css = r#"
        :root { --spacing: 16px; }
        .card { padding: var(--spacing); width: 100px; height: 100px; }
    "#;
    let h = TestHarness::new(
        css,
        || ElementTree { root: ElementDef::new(Tag::Div).with_class("card") },
        800.0,
        600.0,
    );

    let snap = h.query(".card").unwrap();
    assert!(
        (snap.computed_style.padding.top - 16.0).abs() < 0.1,
        "padding-top should be 16px from var(--spacing), got {}",
        snap.computed_style.padding.top
    );
    assert!(
        (snap.computed_style.padding.left - 16.0).abs() < 0.1,
        "padding-left should be 16px from var(--spacing), got {}",
        snap.computed_style.padding.left
    );
}

#[test]
fn multiple_variables_in_same_stylesheet() {
    let css = r#"
        :root {
            --bg: #336699;
            --fg: white;
            --size: 20px;
        }
        .item {
            background: var(--bg);
            color: var(--fg);
            font-size: var(--size);
            width: 100px;
            height: 100px;
        }
    "#;
    let h = TestHarness::new(
        css,
        || ElementTree { root: ElementDef::new(Tag::Div).with_class("item") },
        800.0,
        600.0,
    );

    let snap = h.query(".item").unwrap();
    assert_eq!(snap.computed_style.background, Background::Color(Color::rgb(0x33, 0x66, 0x99)),);
    assert_eq!(snap.computed_style.color, Color::WHITE);
    assert!((snap.computed_style.font_size - 20.0).abs() < 0.1);
}

#[test]
fn star_selector_variables_also_work() {
    let css = r#"
        * { --accent: orange; }
        .box { background: var(--accent); width: 100px; height: 100px; }
    "#;
    let h = TestHarness::new(
        css,
        || ElementTree { root: ElementDef::new(Tag::Div).with_class("box") },
        800.0,
        600.0,
    );

    let snap = h.query(".box").unwrap();
    assert_eq!(
        snap.computed_style.background,
        Background::Color(Color::rgb(255, 165, 0)),
        "var(--accent) from * selector should resolve to orange"
    );
}

#[test]
fn custom_properties_stored_on_stylesheet() {
    let css = r#"
        :root {
            --primary: #ff0000;
            --secondary: blue;
        }
        .box { width: 100px; }
    "#;
    let stylesheet = CompiledStylesheet::parse(css);
    assert_eq!(stylesheet.custom_properties.get("--primary").unwrap(), "#ff0000");
    assert_eq!(stylesheet.custom_properties.get("--secondary").unwrap(), "blue");
}

#[test]
fn fallback_with_hex_color() {
    let css = r#"
        .box { color: var(--undefined, #abcdef); width: 100px; height: 100px; }
    "#;
    let h = TestHarness::new(
        css,
        || ElementTree { root: ElementDef::new(Tag::Div).with_class("box") },
        800.0,
        600.0,
    );

    let snap = h.query(".box").unwrap();
    assert_eq!(
        snap.computed_style.color,
        Color::rgb(0xab, 0xcd, 0xef),
        "fallback hex color should be used when variable is missing"
    );
}
