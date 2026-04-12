use unshit_core::element::*;
use unshit_core::style::types::{Color, TextDecoration};
use unshit_test::TestHarness;

#[test]
fn parse_text_decoration_underline() {
    let css = r#"
        .root { display: flex; width: 100%; height: 100%; }
        .underlined { text-decoration: underline; }
    "#;
    let h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("root")
                .with_child(ElementDef::new(Tag::Span).with_class("underlined").with_text("Hello")),
        },
        800.0,
        600.0,
    );

    let snap = h.query(".underlined").unwrap();
    assert_eq!(snap.computed_style.text_decoration, TextDecoration::Underline);
}

#[test]
fn parse_text_decoration_line_through() {
    let css = r#"
        .root { display: flex; width: 100%; height: 100%; }
        .struck { text-decoration: line-through; }
    "#;
    let h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("root")
                .with_child(ElementDef::new(Tag::Span).with_class("struck").with_text("Deleted")),
        },
        800.0,
        600.0,
    );

    let snap = h.query(".struck").unwrap();
    assert_eq!(snap.computed_style.text_decoration, TextDecoration::LineThrough);
}

#[test]
fn parse_text_decoration_overline() {
    let css = r#"
        .root { display: flex; width: 100%; height: 100%; }
        .over { text-decoration: overline; }
    "#;
    let h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("root")
                .with_child(ElementDef::new(Tag::Span).with_class("over").with_text("Over")),
        },
        800.0,
        600.0,
    );

    let snap = h.query(".over").unwrap();
    assert_eq!(snap.computed_style.text_decoration, TextDecoration::Overline);
}

#[test]
fn parse_text_decoration_none_resets() {
    let css = r#"
        .root { display: flex; width: 100%; height: 100%; text-decoration: underline; }
        .plain { text-decoration: none; }
    "#;
    let h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("root")
                .with_child(ElementDef::new(Tag::Span).with_class("plain").with_text("Plain")),
        },
        800.0,
        600.0,
    );

    // Parent has underline
    let root_snap = h.query(".root").unwrap();
    assert_eq!(root_snap.computed_style.text_decoration, TextDecoration::Underline);

    // Child overrides to none
    let snap = h.query(".plain").unwrap();
    assert_eq!(snap.computed_style.text_decoration, TextDecoration::None);
}

#[test]
fn text_decoration_inherits() {
    let css = r#"
        .root { display: flex; width: 100%; height: 100%; text-decoration: underline; }
    "#;
    let h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("root")
                .with_child(ElementDef::new(Tag::Span).with_class("child").with_text("Inherited")),
        },
        800.0,
        600.0,
    );

    // Child should inherit underline from parent
    let snap = h.query(".child").unwrap();
    assert_eq!(snap.computed_style.text_decoration, TextDecoration::Underline);
}

#[test]
fn parse_text_decoration_color() {
    let css = r#"
        .root { display: flex; width: 100%; height: 100%; }
        .colored {
            text-decoration: underline;
            text-decoration-color: red;
        }
    "#;
    let h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div).with_class("root").with_child(
                ElementDef::new(Tag::Span).with_class("colored").with_text("Colored underline"),
            ),
        },
        800.0,
        600.0,
    );

    let snap = h.query(".colored").unwrap();
    assert_eq!(snap.computed_style.text_decoration, TextDecoration::Underline);
    assert_eq!(snap.computed_style.text_decoration_color, Some(Color::rgb(255, 0, 0)));
}

#[test]
fn text_decoration_color_defaults_to_none() {
    let css = r#"
        .root { display: flex; width: 100%; height: 100%; }
        .simple { text-decoration: underline; }
    "#;
    let h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("root")
                .with_child(ElementDef::new(Tag::Span).with_class("simple").with_text("Simple")),
        },
        800.0,
        600.0,
    );

    let snap = h.query(".simple").unwrap();
    // When no text-decoration-color is set, it should be None (falls back to text color at render)
    assert_eq!(snap.computed_style.text_decoration_color, None);
}
