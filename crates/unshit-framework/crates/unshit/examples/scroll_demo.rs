use unshit::app::{App, AppConfig};
use unshit::core::element::*;

fn main() {
    env_logger::Builder::from_env(
        env_logger::Env::default()
            .default_filter_or("info,wgpu_hal=error,wgpu_core=error,naga=error"),
    )
    .init();

    let css = r#"
        .root {
            display: flex;
            flex-direction: column;
            width: 100%;
            height: 100%;
            background: rgba(13, 17, 23, 0.95);
            padding: 32px;
            gap: 24px;
        }

        .header {
            display: flex;
            flex-direction: column;
            gap: 8px;
        }

        .title {
            color: #e6edf3;
            font-size: 36px;
            font-weight: bold;
        }

        .subtitle {
            color: #8b949e;
            font-size: 16px;
            line-height: 1.5;
        }

        .scroll-container {
            overflow: scroll;
            height: 500px;
            flex-direction: column;
            gap: 12px;
            padding: 16px;
            background: rgba(255, 255, 255, 0.03);
            border-radius: 16px;
            border-width: 1px;
            border-color: rgba(16, 185, 129, 0.15);
            flex-grow: 1;
        }

        .scroll-container-horizontal {
            overflow: scroll;
            height: 140px;
            flex-direction: row;
            gap: 12px;
            padding: 16px;
            background: rgba(255, 255, 255, 0.03);
            border-radius: 16px;
            border-width: 1px;
            border-color: rgba(16, 185, 129, 0.15);
            flex-shrink: 0;
        }

        .card {
            display: flex;
            flex-direction: column;
            padding: 16px 20px;
            background: rgba(13, 17, 23, 0.6);
            border-radius: 12px;
            border-width: 1px;
            border-color: rgba(16, 185, 129, 0.1);
            gap: 8px;
            flex-shrink: 0;
        }

        .card:hover {
            background: rgba(13, 17, 23, 0.85);
            border-color: rgba(16, 185, 129, 0.3);
        }

        .card-title {
            color: #e6edf3;
            font-size: 15px;
            font-weight: bold;
        }

        .card-description {
            color: #8b949e;
            font-size: 13px;
            line-height: 1.5;
        }

        .card-index {
            color: #10b981;
            font-size: 12px;
            font-weight: bold;
            letter-spacing: 2px;
        }

        .horizontal-card {
            display: flex;
            flex-direction: column;
            padding: 16px 24px;
            background: rgba(13, 17, 23, 0.6);
            border-radius: 12px;
            border-width: 1px;
            border-color: rgba(16, 185, 129, 0.1);
            gap: 6px;
            flex-shrink: 0;
            width: 200px;
            justify-content: center;
        }

        .horizontal-card:hover {
            background: rgba(13, 17, 23, 0.85);
            border-color: rgba(16, 185, 129, 0.3);
        }

        .hcard-title {
            color: #e6edf3;
            font-size: 14px;
            font-weight: bold;
        }

        .hcard-value {
            color: #10b981;
            font-size: 20px;
            font-weight: bold;
        }

        .section-label {
            color: #484f58;
            font-size: 12px;
            font-weight: bold;
            letter-spacing: 3px;
        }

        .badge-row {
            display: flex;
            gap: 8px;
            align-items: center;
        }

        .badge {
            display: flex;
            align-items: center;
            flex-shrink: 0;
            padding: 4px 12px;
            background: rgba(16, 185, 129, 0.1);
            border-radius: 999px;
            border-width: 1px;
            border-color: rgba(16, 185, 129, 0.2);
            color: #6ee7b7;
            font-size: 12px;
        }
    "#;

    let app = App::new(
        AppConfig {
            title: "Scroll Demo".to_string(),
            width: 1100,
            height: 750,
            css: css.to_string(),
            ..Default::default()
        },
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("root")
                .with_child(header())
                .with_child(vertical_scroll_container())
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("section-label")
                        .with_text("HORIZONTAL SCROLL"),
                )
                .with_child(horizontal_scroll_container()),
        },
    );

    app.run();
}

fn header() -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("header")
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("badge-row")
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("badge")
                        .with_text("overflow: scroll"),
                )
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("badge")
                        .with_text("vertical"),
                )
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("badge")
                        .with_text("horizontal"),
                ),
        )
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("title")
                .with_text("Scroll Demo"),
        )
        .with_child(
            ElementDef::new(Tag::Span)
                .with_class("subtitle")
                .with_text("A scrollable container with more content than fits visually. The cards below overflow the fixed-height container."),
        )
}

fn vertical_scroll_container() -> ElementDef {
    let items: Vec<(&str, &str)> = vec![
        (
            "Layout Engine",
            "Flexbox layout powered by taffy with full axis support and gap handling.",
        ),
        ("GPU Rendering", "All drawing happens on the GPU via wgpu for zero-jank compositing."),
        (
            "CSS Parsing",
            "Subset of CSS parsed at startup: colors, borders, padding, flex properties.",
        ),
        ("Border Radius", "Rounded corners on every element, from subtle 4px to full pill shapes."),
        ("Box Shadows", "Multi-layer shadows with blur, spread, and offset for depth."),
        ("Text Rendering", "Glyph rasterization with subpixel positioning using cosmic-text."),
        ("Color System", "RGBA everywhere: backgrounds, borders, text, and shadow colors."),
        ("Hover Effects", "Pseudo-class matching for :hover state changes on any property."),
        ("Active States", ":active pseudo-class tracks pointer-down for press feedback."),
        ("Cursor Icons", "CSS cursor property maps to system cursors: pointer, text, grab, etc."),
        ("Flex Grow", "Elements expand to fill available space with flex-grow ratios."),
        ("Flex Shrink", "Content shrinks proportionally when the container is too small."),
        ("Gap Property", "Consistent spacing between flex children without margin hacks."),
        ("Padding", "Per-side padding with shorthand: padding: 16px 20px."),
        ("Border Width", "Visible borders with configurable width and color per element."),
        ("Font Weight", "Bold and normal weights for typographic hierarchy."),
        ("Font Size", "Pixel-based font sizing from 11px labels to 44px headlines."),
        ("Letter Spacing", "Extra tracking for uppercase labels and badges."),
        ("Line Height", "Multiplier-based line height for readable paragraph text."),
        ("Overflow Scroll", "This container uses overflow: scroll to clip and scroll content."),
        ("Nested Layouts", "Flex containers inside flex containers for complex compositions."),
        ("Element Tree", "Declarative tree of ElementDef nodes built with a builder pattern."),
        ("Class Selectors", "CSS class selectors target elements by .class-name."),
        ("Display Flex", "All containers default to display: flex for predictable layout."),
        ("Width & Height", "Fixed dimensions in pixels or percentage of parent."),
    ];

    let mut container = ElementDef::new(Tag::Div).with_class("scroll-container");

    for (i, (title, description)) in items.iter().enumerate() {
        container = container.with_child(card(i + 1, title, description));
    }

    container
}

fn card(index: usize, title: &str, description: &str) -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("card")
        .with_child(
            ElementDef::new(Tag::Span).with_class("card-index").with_text(format!("{:02}", index)),
        )
        .with_child(ElementDef::new(Tag::Span).with_class("card-title").with_text(title))
        .with_child(
            ElementDef::new(Tag::Span).with_class("card-description").with_text(description),
        )
}

fn horizontal_scroll_container() -> ElementDef {
    let metrics: Vec<(&str, &str)> = vec![
        ("Elements", "128"),
        ("Redraws", "0.4ms"),
        ("Layout", "0.2ms"),
        ("GPU Calls", "34"),
        ("Textures", "12"),
        ("Vertices", "2.4k"),
        ("Fragments", "890k"),
        ("Memory", "18MB"),
        ("FPS", "120"),
        ("Vsync", "On"),
        ("Resolution", "2x"),
        ("Threads", "4"),
    ];

    let mut container = ElementDef::new(Tag::Div).with_class("scroll-container-horizontal");

    for (label, value) in &metrics {
        container = container.with_child(horizontal_card(label, value));
    }

    container
}

fn horizontal_card(label: &str, value: &str) -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("horizontal-card")
        .with_child(ElementDef::new(Tag::Span).with_class("hcard-title").with_text(label))
        .with_child(ElementDef::new(Tag::Span).with_class("hcard-value").with_text(value))
}
