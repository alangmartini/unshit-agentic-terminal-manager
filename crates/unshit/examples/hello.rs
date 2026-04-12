use unshit::app::{App, AppConfig};
use unshit::core::element::*;

fn main() {
    env_logger::init();

    let css = r#"
        .root {
            display: flex;
            flex-direction: column;
            width: 100%;
            height: 100%;
            background: rgba(13, 17, 23, 0.92);
            padding: 32px;
            gap: 24px;
        }

        .outer-card {
            display: flex;
            flex-direction: column;
            width: 100%;
            flex-grow: 1;
            background: rgba(13, 17, 23, 0.4);
            border-radius: 32px;
            border-width: 1px;
            border-color: rgba(16, 185, 129, 0.18);
            padding: 6px;
            box-shadow: 0px 25px 50px rgba(0, 0, 0, 0.3);
        }

        .inner-card {
            display: flex;
            flex-direction: column;
            flex-grow: 1;
            background: rgba(255, 255, 255, 0.03);
            border-radius: 26px;
            border-width: 1px;
            border-color: rgba(16, 185, 129, 0.12);
            padding: 32px;
            gap: 28px;
        }

        .badge-row {
            display: flex;
            gap: 8px;
            align-items: center;
        }

        .badge-primary {
            display: flex;
            align-items: center;
            flex-shrink: 0;
            padding: 6px 14px;
            background: rgba(16, 185, 129, 0.12);
            border-radius: 999px;
            border-width: 1px;
            border-color: rgba(16, 185, 129, 0.25);
            color: #6ee7b7;
            font-size: 13px;
        }

        .badge {
            display: flex;
            align-items: center;
            flex-shrink: 0;
            padding: 6px 14px;
            background: rgba(255, 255, 255, 0.04);
            border-radius: 999px;
            border-width: 1px;
            border-color: rgba(16, 185, 129, 0.1);
            color: #8b949e;
            font-size: 13px;
        }

        .content-row {
            display: flex;
            flex-grow: 1;
            gap: 40px;
        }

        .left-col {
            display: flex;
            flex-direction: column;
            flex-grow: 1;
            flex-shrink: 1;
            gap: 20px;
            justify-content: center;
        }

        .headline {
            color: #e6edf3;
            font-size: 44px;
            font-weight: bold;
            line-height: 1.15;
        }

        .accent-text {
            color: #10b981;
            font-size: 44px;
            font-weight: bold;
            line-height: 1.15;
        }

        .description {
            color: #8b949e;
            font-size: 17px;
            line-height: 1.6;
        }

        .button-row {
            display: flex;
            gap: 12px;
            align-items: center;
        }

        .btn-primary {
            display: flex;
            align-items: center;
            padding: 14px 28px;
            background: #10b981;
            border-radius: 14px;
            color: #ffffff;
            font-size: 15px;
            font-weight: bold;
            box-shadow: 0px 8px 24px rgba(16, 185, 129, 0.25);
        }

        .btn-outline {
            display: flex;
            align-items: center;
            padding: 14px 28px;
            background: rgba(239, 68, 68, 0.06);
            border-radius: 14px;
            border-width: 1px;
            border-color: rgba(248, 113, 113, 0.2);
            color: #fca5a5;
            font-size: 15px;
            font-weight: bold;
        }

        .right-col {
            display: flex;
            flex-direction: column;
            width: 360px;
            flex-shrink: 0;
        }

        .preview-card {
            display: flex;
            flex-direction: column;
            flex-grow: 1;
            background: rgba(13, 17, 23, 0.5);
            border-radius: 24px;
            border-width: 1px;
            border-color: rgba(16, 185, 129, 0.15);
            padding: 24px;
            gap: 16px;
            box-shadow: 0px 20px 40px rgba(0, 0, 0, 0.3);
        }

        .traffic-dots {
            display: flex;
            gap: 8px;
            align-items: center;
        }

        .dot-red {
            width: 10px;
            height: 10px;
            border-radius: 999px;
            background: rgba(251, 113, 133, 0.8);
        }

        .dot-amber {
            width: 10px;
            height: 10px;
            border-radius: 999px;
            background: rgba(251, 191, 36, 0.8);
        }

        .dot-green {
            width: 10px;
            height: 10px;
            border-radius: 999px;
            background: rgba(16, 185, 129, 0.8);
        }

        .preview-label {
            color: #484f58;
            font-size: 11px;
            font-weight: bold;
            letter-spacing: 3px;
        }

        .preview-inner {
            display: flex;
            flex-direction: column;
            flex-grow: 1;
            background: rgba(255, 255, 255, 0.025);
            border-radius: 20px;
            border-width: 1px;
            border-color: rgba(16, 185, 129, 0.08);
            padding: 20px;
            gap: 12px;
        }

        .preview-header {
            display: flex;
            justify-content: space-between;
            align-items: center;
        }

        .preview-header-left {
            display: flex;
            flex-direction: column;
            gap: 4px;
        }

        .preview-title-label {
            color: #484f58;
            font-size: 11px;
            font-weight: bold;
            letter-spacing: 3px;
        }

        .preview-title-value {
            color: #e6edf3;
            font-size: 22px;
            font-weight: bold;
        }

        .fps-badge {
            display: flex;
            align-items: center;
            flex-shrink: 0;
            padding: 8px 14px;
            background: rgba(16, 185, 129, 0.12);
            border-radius: 12px;
            border-width: 1px;
            border-color: rgba(16, 185, 129, 0.2);
            color: #34d399;
            font-size: 13px;
            font-weight: bold;
        }

        .stat-row {
            display: flex;
            justify-content: space-between;
            align-items: center;
            padding: 12px 16px;
            background: rgba(0, 0, 0, 0.2);
            border-radius: 14px;
            border-width: 1px;
            border-color: rgba(16, 185, 129, 0.06);
        }

        .stat-left {
            display: flex;
            align-items: center;
            gap: 12px;
        }

        .stat-dot {
            width: 8px;
            height: 8px;
            border-radius: 4px;
            background: #10b981;
            flex-shrink: 0;
        }

        .stat-label {
            color: #8b949e;
            font-size: 13px;
        }

        .stat-value {
            color: #e6edf3;
            font-size: 13px;
            font-weight: bold;
            flex-shrink: 0;
        }

        .features-row {
            display: flex;
            gap: 16px;
            width: 100%;
        }

        .feature-card {
            display: flex;
            flex-direction: column;
            flex-grow: 1;
            background: rgba(13, 17, 23, 0.4);
            border-radius: 20px;
            border-width: 1px;
            border-color: rgba(16, 185, 129, 0.12);
            padding: 24px;
            gap: 12px;
            box-shadow: 0px 10px 30px rgba(0, 0, 0, 0.25);
        }

        .feature-header {
            display: flex;
            justify-content: space-between;
            align-items: center;
        }

        .feature-info {
            display: flex;
            flex-direction: column;
            gap: 4px;
        }

        .feature-label {
            color: #484f58;
            font-size: 12px;
            font-weight: bold;
            letter-spacing: 2px;
        }

        .feature-value {
            color: #e6edf3;
            font-size: 20px;
            font-weight: bold;
        }

        .feature-dot {
            width: 12px;
            height: 12px;
            border-radius: 4px;
            background: #10b981;
            flex-shrink: 0;
        }

        .feature-hint {
            color: #8b949e;
            font-size: 13px;
            line-height: 1.5;
        }

        .brand-label {
            color: #484f58;
            font-size: 13px;
            font-weight: bold;
            letter-spacing: 2px;
        }

        .btn-primary {
            cursor: pointer;
        }
        .btn-primary:hover {
            background: #14d892;
            box-shadow: 0px 10px 30px rgba(16, 185, 129, 0.35);
        }
        .btn-primary:active {
            background: #0e9d6e;
            box-shadow: 0px 4px 12px rgba(16, 185, 129, 0.2);
        }

        .btn-outline {
            cursor: pointer;
        }
        .btn-outline:hover {
            background: rgba(239, 68, 68, 0.12);
            border-color: rgba(248, 113, 113, 0.35);
        }
        .btn-outline:active {
            background: rgba(239, 68, 68, 0.18);
        }

        .feature-card:hover {
            border-color: rgba(16, 185, 129, 0.25);
            background: rgba(13, 17, 23, 0.55);
        }

        .stat-row:hover {
            background: rgba(0, 0, 0, 0.3);
            border-color: rgba(16, 185, 129, 0.15);
        }

        .badge-primary:hover {
            background: rgba(16, 185, 129, 0.2);
            border-color: rgba(16, 185, 129, 0.4);
        }

        .badge:hover {
            background: rgba(255, 255, 255, 0.08);
            border-color: rgba(16, 185, 129, 0.2);
        }

        .preview-card:hover {
            border-color: rgba(16, 185, 129, 0.3);
        }
    "#;

    let app = App::new(
        AppConfig {
            title: "unshit framework".to_string(),
            width: 1280,
            height: 900,
            css: css.to_string(),
            ..Default::default()
        },
        || {
            ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("root")
                .with_child(
                    ElementDef::new(Tag::Div)
                        .with_class("outer-card")
                        .with_child(
                            ElementDef::new(Tag::Div)
                                .with_class("inner-card")
                                .with_child(
                                    ElementDef::new(Tag::Div)
                                        .with_class("badge-row")
                                        .with_child(
                                            ElementDef::new(Tag::Span)
                                                .with_class("badge-primary")
                                                .with_text("Experimental UI Runtime"),
                                        )
                                        .with_child(badge("Fast"))
                                        .with_child(badge("Composable"))
                                        .with_child(badge("Animated"))
                                        .with_child(badge("Typed"))
                                        .with_child(badge("Actually usable")),
                                )
                                .with_child(
                                    ElementDef::new(Tag::Div)
                                        .with_class("content-row")
                                        .with_child(
                                            ElementDef::new(Tag::Div)
                                                .with_class("left-col")
                                                .with_child(
                                                    ElementDef::new(Tag::Span)
                                                        .with_class("brand-label")
                                                        .with_text("unshit framework"),
                                                )
                                                .with_child(
                                                    ElementDef::new(Tag::Span)
                                                        .with_class("headline")
                                                        .with_text("Build products that feel"),
                                                )
                                                .with_child(
                                                    ElementDef::new(Tag::Span)
                                                        .with_class("accent-text")
                                                        .with_text("impossibly smooth."),
                                                )
                                                .with_child(
                                                    ElementDef::new(Tag::Span)
                                                        .with_class("description")
                                                        .with_text("unshit framework is a modern runtime-inspired interface concept with GPU-first rendering, polished motion, layered depth, and a design system that doesn't look like a developer placeholder."),
                                                )
                                                .with_child(
                                                    ElementDef::new(Tag::Div)
                                                        .with_class("button-row")
                                                        .with_child(
                                                            ElementDef::new(Tag::Div)
                                                                .with_class("btn-primary")
                                                                .with_child(
                                                                    ElementDef::new(Tag::Span)
                                                                        .with_text("Build something \u{2192}"),
                                                                ),
                                                        )
                                                        .with_child(
                                                            ElementDef::new(Tag::Div)
                                                                .with_class("btn-outline")
                                                                .with_child(
                                                                    ElementDef::new(Tag::Span)
                                                                        .with_text("Unshit the world"),
                                                                ),
                                                        ),
                                                ),
                                        )
                                        .with_child(
                                            ElementDef::new(Tag::Div)
                                                .with_class("right-col")
                                                .with_child(preview_card()),
                                        ),
                                ),
                        ),
                )
                .with_child(
                    ElementDef::new(Tag::Div)
                        .with_class("features-row")
                        .with_child(feature_card(
                            "Renderer",
                            "wgpu \u{00B7} GPU accelerated",
                            "Zero-jank motion, buttery transitions, native-feeling compositing.",
                        ))
                        .with_child(feature_card(
                            "Layout",
                            "Flexbox via taffy",
                            "Predictable layout primitives with modern spacing and adaptive grids.",
                        ))
                        .with_child(feature_card(
                            "Target",
                            "120fps desktop",
                            "Tuned for snappy interactions, micro-animations, and absurd responsiveness.",
                        )),
                ),
        }
        },
    );

    app.run();
}

fn badge(text: &str) -> ElementDef {
    ElementDef::new(Tag::Span).with_class("badge").with_text(text)
}

fn feature_card(label: &str, value: &str, hint: &str) -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("feature-card")
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("feature-header")
                .with_child(
                    ElementDef::new(Tag::Div)
                        .with_class("feature-info")
                        .with_child(
                            ElementDef::new(Tag::Span).with_class("feature-label").with_text(label),
                        )
                        .with_child(
                            ElementDef::new(Tag::Span).with_class("feature-value").with_text(value),
                        ),
                )
                .with_child(ElementDef::new(Tag::Div).with_class("feature-dot")),
        )
        .with_child(ElementDef::new(Tag::Span).with_class("feature-hint").with_text(hint))
}

fn preview_card() -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("preview-card")
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("traffic-dots")
                .with_child(ElementDef::new(Tag::Div).with_class("dot-red"))
                .with_child(ElementDef::new(Tag::Div).with_class("dot-amber"))
                .with_child(ElementDef::new(Tag::Div).with_class("dot-green"))
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("preview-label")
                        .with_text("LIVE PREVIEW"),
                ),
        )
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("preview-inner")
                .with_child(
                    ElementDef::new(Tag::Div)
                        .with_class("preview-header")
                        .with_child(
                            ElementDef::new(Tag::Div)
                                .with_class("preview-header-left")
                                .with_child(
                                    ElementDef::new(Tag::Span)
                                        .with_class("preview-title-label")
                                        .with_text("RUNTIME HEALTH"),
                                )
                                .with_child(
                                    ElementDef::new(Tag::Span)
                                        .with_class("preview-title-value")
                                        .with_text("Nominal"),
                                ),
                        )
                        .with_child(
                            ElementDef::new(Tag::Span).with_class("fps-badge").with_text("120 fps"),
                        ),
                )
                .with_child(stat_row("Composable primitives", "24 ready"))
                .with_child(stat_row("Motion presets", "11 active"))
                .with_child(stat_row("Type safety", "strict"))
                .with_child(stat_row("DX score", "pleasant")),
        )
}

fn stat_row(label: &str, value: &str) -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("stat-row")
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("stat-left")
                .with_child(ElementDef::new(Tag::Div).with_class("stat-dot"))
                .with_child(ElementDef::new(Tag::Span).with_class("stat-label").with_text(label)),
        )
        .with_child(ElementDef::new(Tag::Span).with_class("stat-value").with_text(value))
}
