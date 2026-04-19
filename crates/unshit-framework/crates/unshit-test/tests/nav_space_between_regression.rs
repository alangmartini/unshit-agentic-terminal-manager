//! Regression: in the Organiza Nota landing example, `.nav` was rendered
//! with `justify_content: Start` and `width: Auto` despite the CSS rule
//! explicitly setting both. This test pins down the behaviour.

use unshit_core::element::*;
use unshit_core::style::types::{Dimension, JustifyContent};
use unshit_core::svg::{
    parse_svg_path, StrokeLineCap, StrokeLineJoin, SvgAttrs, SvgNode, SvgPaint, SvgPrimitive,
    ViewBox,
};
use unshit_test::TestHarness;

fn logo_mark_svg() -> SvgNode {
    let white = unshit_core::style::types::Color::WHITE;
    let commands = parse_svg_path("M4 3 H18 V19 L15 17 L12 19 L9 17 L6 19 L4 17 Z").unwrap_or_default();
    let mut body = SvgNode {
        primitive: SvgPrimitive::Path {
            d: "M4 3 H18 V19 L15 17 L12 19 L9 17 L6 19 L4 17 Z".to_string(),
            commands,
        },
        attrs: SvgAttrs::default(),
        children: Vec::new(),
    };
    body.attrs.stroke = Some(SvgPaint::Solid(white));
    body.attrs.stroke_width = Some(1.5);
    body.attrs.fill = Some(SvgPaint::None);
    SvgNode {
        primitive: SvgPrimitive::Group,
        attrs: SvgAttrs {
            view_box: Some(ViewBox::new(0.0, 0.0, 22.0, 22.0)),
            fill: Some(SvgPaint::None),
            stroke_linecap: Some(StrokeLineCap::Butt),
            stroke_linejoin: Some(StrokeLineJoin::Miter),
            ..Default::default()
        },
        children: vec![body],
    }
}

const ORGANIZA_CSS: &str = r#"
:root {
    --bg: #0a0a0b;
    --bg-2: #111113;
    --ink: #ffffff;
    --ink-2: #c7c7cc;
    --ink-3: #8e8e93;
    --ink-4: #48484a;
    --line: rgba(255,255,255,0.08);
    --line-2: rgba(255,255,255,0.14);
    --accent: #ff6b3d;
    --accent-glow: rgba(255,107,61,0.4);
}

.viewport {
    position: relative;
    display: flex;
    flex-direction: column;
    width: 1440px;
    height: 900px;
    padding: 0 40px;
    background: #0a0a0b;
    color: #ffffff;
    overflow: hidden;
}

.frame {
    position: absolute;
    top: 60px;
    left: 40px;
    right: 40px;
    bottom: 60px;
    border-left-width: 1px;
    border-right-width: 1px;
    border-color: rgba(255,255,255,0.08);
    pointer-events: none;
}
.frame-edge-top {
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    height: 1px;
    background: rgba(255,255,255,0.08);
}
.frame-edge-bottom {
    position: absolute;
    bottom: 0;
    left: 0;
    right: 0;
    height: 1px;
    background: rgba(255,255,255,0.08);
}
.frame-tick {
    position: absolute;
    width: 6px;
    height: 6px;
    border-width: 1px;
    border-color: rgba(255,255,255,0.14);
}
.frame-tick-tl { top: -3px; left: -3px; }
.frame-tick-tr { top: -3px; right: -3px; }
.frame-tick-bl { bottom: -3px; left: -3px; }
.frame-tick-br { bottom: -3px; right: -3px; }

.vignette {
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    bottom: 0;
    background: radial-gradient(ellipse at 70% 50%, transparent 0%, rgba(0,0,0,0.5) 90%);
    pointer-events: none;
}

.hero-right-glow {
    position: absolute;
    top: 0;
    left: 720px;
    right: 0;
    bottom: 0;
    background: radial-gradient(ellipse at 55% 45%, rgba(255,107,61,0.16) 0%, rgba(200,60,30,0.08) 40%, rgba(10,10,11,0) 75%);
    pointer-events: none;
}

.nav {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 22px 0;
    width: 1360px;
}
.logo {
    display: flex;
    align-items: center;
    gap: 10px;
}
.logo-mark {
    width: 22px;
    height: 22px;
}
.logo-text {
    color: #ffffff;
    font-size: 15px;
    font-weight: 600;
    letter-spacing: -0.15px;
}
.nav-center {
    display: flex;
    gap: 36px;
}
.nav-link {
    color: #c7c7cc;
    font-size: 13px;
    font-weight: 400;
}
.nav-link:hover {
    color: #ffffff;
}
.nav-right {
    display: flex;
    align-items: center;
    gap: 24px;
}
.help {
    color: #c7c7cc;
    font-size: 13px;
}
.btn-pill {
    height: 32px;
    padding: 0 16px;
    border-radius: 999px;
    background: #ffffff;
    color: #0a0a0b;
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
    border-width: 0;
    display: flex;
    align-items: center;
    justify-content: center;
}

.hero {
    display: flex;
    flex-direction: row;
    align-items: flex-start;
    padding-top: 40px;
    flex-grow: 1;
    min-height: 0;
    width: 1360px;
}
.hero-left {
    width: 680px;
    padding-right: 40px;
    display: flex;
    flex-direction: column;
}
.hero-right-spacer {
    width: 680px;
}

.badge {
    display: flex;
    align-items: center;
    align-self: flex-start;
    gap: 8px;
    padding: 6px 12px;
    border-width: 1px;
    border-color: rgba(255,255,255,0.14);
    border-radius: 999px;
    background: rgba(255,255,255,0.02);
    margin-bottom: 28px;
}
.badge-dot {
    width: 6px;
    height: 6px;
    background: #ff6b3d;
    border-radius: 50%;
    box-shadow: 0 0 8px rgba(255,107,61,0.6);
}
.badge-text {
    color: #c7c7cc;
    font-size: 12px;
}

.h1 {
    display: flex;
    flex-direction: column;
    width: 100%;
}
.h1-line {
    color: #ffffff;
    font-size: 56px;
    font-weight: 600;
    line-height: 1.05;
    letter-spacing: -1.7px;
    width: 100%;
}

.hero-sub {
    color: #8e8e93;
    font-size: 15px;
    line-height: 1.6;
    max-width: 440px;
    margin-top: 24px;
    margin-bottom: 36px;
}

.hero-ctas {
    display: flex;
    gap: 10px;
}
.btn-lg {
    height: 42px;
    padding: 0 20px;
    border-radius: 999px;
    font-size: 14px;
    font-weight: 500;
    cursor: pointer;
    border-width: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
}
.btn-lg-primary {
    background: #ffffff;
    color: #0a0a0b;
}
.btn-lg-secondary {
    background: rgba(255,255,255,0.05);
    color: #ffffff;
    border-width: 1px;
    border-color: rgba(255,255,255,0.14);
}

.bottom {
    display: flex;
    flex-direction: row;
    align-items: flex-end;
    justify-content: space-between;
    padding: 60px 0 100px 0;
    gap: 40px;
    width: 1360px;
}
.bottom-tag {
    display: flex;
    flex-direction: row;
    flex-wrap: wrap;
    max-width: 340px;
    color: #8e8e93;
    font-size: 13px;
    line-height: 1.6;
}
.bottom-tag-text {
    color: #8e8e93;
    font-size: 13px;
}
.bottom-tag-strong {
    color: #ffffff;
    font-weight: 600;
    font-size: 13px;
}
.stats {
    display: flex;
    flex-direction: row;
    gap: 44px;
}
.stat {
    display: flex;
    flex-direction: column;
    gap: 4px;
}
.stat-v {
    display: flex;
    flex-direction: row;
    align-items: baseline;
}
.stat-v-num {
    color: #ffffff;
    font-size: 28px;
    font-weight: 600;
    letter-spacing: -0.56px;
}
.stat-v-unit {
    color: #ff6b3d;
    font-size: 28px;
    font-weight: 500;
    letter-spacing: -0.56px;
}
.stat-k {
    color: #8e8e93;
    font-size: 11px;
    line-height: 1.3;
}

.backed {
    position: absolute;
    left: 40px;
    right: 40px;
    bottom: 0;
    display: flex;
    flex-direction: row;
    align-items: center;
    justify-content: space-between;
    padding: 20px 0;
    border-top-width: 1px;
    border-color: rgba(255,255,255,0.08);
    background: linear-gradient(to top, #0a0a0b 60%, rgba(10,10,11,0) 100%);
}
.backed-label {
    display: flex;
    flex-direction: column;
    font-size: 11px;
    line-height: 1.4;
}
.backed-label-strong {
    color: #c7c7cc;
    font-size: 11px;
    font-weight: 500;
}
.backed-label-text {
    color: #8e8e93;
    font-size: 11px;
}
.logos-window {
    width: 560px;
    overflow: hidden;
}
.logos-track {
    display: flex;
    flex-direction: row;
    gap: 56px;
}
.logo-item {
    display: flex;
    flex-direction: row;
    align-items: center;
    gap: 8px;
    color: #8e8e93;
    font-size: 15px;
    font-weight: 500;
    letter-spacing: -0.15px;
    opacity: 0.7;
}
.logo-item-icon {
    width: 18px;
    height: 18px;
}
.logo-item-text {
    color: #8e8e93;
    font-size: 15px;
    font-weight: 500;
}
"#;

fn try_with_gpu(h: TestHarness) -> Option<TestHarness> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| h.with_gpu())).ok()
}

#[test]
fn nav_receives_space_between_and_width_from_ancestors() {
    let tree_fn = || ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("viewport")
            .with_child(ElementDef::new(Tag::Div).with_class("hero-right-glow"))
            .with_child(
                ElementDef::new(Tag::Div)
                    .with_class("frame")
                    .with_child(ElementDef::new(Tag::Div).with_class("frame-edge-top"))
                    .with_child(ElementDef::new(Tag::Div).with_class("frame-edge-bottom"))
                    .with_child(ElementDef::new(Tag::Div).with_class("frame-tick frame-tick-tl"))
                    .with_child(ElementDef::new(Tag::Div).with_class("frame-tick frame-tick-tr"))
                    .with_child(ElementDef::new(Tag::Div).with_class("frame-tick frame-tick-bl"))
                    .with_child(ElementDef::new(Tag::Div).with_class("frame-tick frame-tick-br")),
            )
            .with_child(ElementDef::new(Tag::Div).with_class("vignette"))
            .with_child(
                ElementDef::new(Tag::Div)
                    .with_class("nav")
                    .with_child(
                        ElementDef::new(Tag::Div)
                            .with_class("logo")
                            .with_child(
                                ElementDef::new(Tag::Div)
                                    .with_class("logo-mark")
                                    .with_svg(logo_mark_svg()),
                            )
                            .with_child(
                                ElementDef::new(Tag::Span)
                                    .with_class("logo-text")
                                    .with_text("Organiza Nota"),
                            ),
                    )
                    .with_child(
                        ElementDef::new(Tag::Div)
                            .with_class("nav-center")
                            .with_child(
                                ElementDef::new(Tag::Span)
                                    .with_class("nav-link")
                                    .with_text("Como funciona"),
                            )
                            .with_child(
                                ElementDef::new(Tag::Span)
                                    .with_class("nav-link")
                                    .with_text("Segurança"),
                            )
                            .with_child(
                                ElementDef::new(Tag::Span)
                                    .with_class("nav-link")
                                    .with_text("Preços"),
                            )
                            .with_child(
                                ElementDef::new(Tag::Span)
                                    .with_class("nav-link")
                                    .with_text("Empresa"),
                            ),
                    )
                    .with_child(
                        ElementDef::new(Tag::Div)
                            .with_class("nav-right")
                            .with_child(
                                ElementDef::new(Tag::Span).with_class("help").with_text("Entrar"),
                            )
                            .with_child(
                                ElementDef::new(Tag::Button)
                                    .with_class("btn-pill")
                                    .with_child(
                                        ElementDef::new(Tag::Span).with_text("Começar"),
                                    ),
                            ),
                    ),
            )
            .with_child(
                ElementDef::new(Tag::Div)
                    .with_class("hero")
                    .with_child(
                        ElementDef::new(Tag::Div)
                            .with_class("hero-left")
                            .with_child(
                                ElementDef::new(Tag::Div)
                                    .with_class("badge")
                                    .with_child(ElementDef::new(Tag::Div).with_class("badge-dot"))
                                    .with_child(
                                        ElementDef::new(Tag::Span)
                                            .with_class("badge-text")
                                            .with_text("v2"),
                                    ),
                            )
                            .with_child(
                                ElementDef::new(Tag::Div)
                                    .with_class("h1")
                                    .with_child(
                                        ElementDef::new(Tag::Span)
                                            .with_class("h1-line")
                                            .with_text("Suas notas"),
                                    )
                                    .with_child(
                                        ElementDef::new(Tag::Span)
                                            .with_class("h1-line")
                                            .with_text("todas num um."),
                                    ),
                            )
                            .with_child(
                                ElementDef::new(Tag::Span)
                                    .with_class("hero-sub")
                                    .with_text("Conecte seu CPF."),
                            )
                            .with_child(
                                ElementDef::new(Tag::Div)
                                    .with_class("hero-ctas")
                                    .with_child(
                                        ElementDef::new(Tag::Button)
                                            .with_class("btn-lg btn-lg-primary")
                                            .with_child(
                                                ElementDef::new(Tag::Span)
                                                    .with_class("btn-lg-label")
                                                    .with_text("Conectar"),
                                            ),
                                    )
                                    .with_child(
                                        ElementDef::new(Tag::Button)
                                            .with_class("btn-lg btn-lg-secondary")
                                            .with_child(
                                                ElementDef::new(Tag::Span)
                                                    .with_class("btn-lg-label")
                                                    .with_text("Ver demo"),
                                            ),
                                    ),
                            ),
                    )
                    .with_child(ElementDef::new(Tag::Div).with_class("hero-right-spacer")),
            )
            .with_child(
                ElementDef::new(Tag::Div)
                    .with_class("bottom")
                    .with_child(
                        ElementDef::new(Tag::Div)
                            .with_class("bottom-tag")
                            .with_child(
                                ElementDef::new(Tag::Span)
                                    .with_class("bottom-tag-text")
                                    .with_text("Chega de "),
                            )
                            .with_child(
                                ElementDef::new(Tag::Span)
                                    .with_class("bottom-tag-strong")
                                    .with_text("caçar"),
                            ),
                    )
                    .with_child(
                        ElementDef::new(Tag::Div)
                            .with_class("stats")
                            .with_child(
                                ElementDef::new(Tag::Div)
                                    .with_class("stat")
                                    .with_child(
                                        ElementDef::new(Tag::Div)
                                            .with_class("stat-v")
                                            .with_child(
                                                ElementDef::new(Tag::Span)
                                                    .with_class("stat-v-num")
                                                    .with_text("26"),
                                            )
                                            .with_child(
                                                ElementDef::new(Tag::Span)
                                                    .with_class("stat-v-unit")
                                                    .with_text("/26"),
                                            ),
                                    )
                                    .with_child(
                                        ElementDef::new(Tag::Span)
                                            .with_class("stat-k")
                                            .with_text("Estados"),
                                    ),
                            ),
                    ),
            )
            .with_child(
                ElementDef::new(Tag::Div)
                    .with_class("backed")
                    .with_child(ElementDef::new(Tag::Div).with_class("backed-label"))
                    .with_child(
                        ElementDef::new(Tag::Div)
                            .with_class("logos-window")
                            .with_child(ElementDef::new(Tag::Div).with_class("logos-track")),
                    ),
            ),
    };

    let Some(mut h) = try_with_gpu(TestHarness::new(ORGANIZA_CSS, tree_fn, 1440.0, 900.0)) else {
        eprintln!("no GPU, skipping");
        return;
    };
    // Match what the example does: run a step after GPU init before reading state.
    h.step();

    let arena = h.arena();
    let mut found = false;
    for (_id, e) in arena.iter() {
        if e.classes.iter().any(|c| c == "nav") {
            eprintln!(
                "nav element: jc={:?} width={:?} padding={:?}",
                e.computed_style.justify_content,
                e.computed_style.width,
                e.computed_style.padding
            );
            assert_eq!(
                e.computed_style.justify_content,
                JustifyContent::SpaceBetween,
                ".nav should have justify_content SpaceBetween"
            );
            assert_eq!(
                e.computed_style.width,
                Dimension::Px(1360.0),
                ".nav should have width 1360px"
            );
            found = true;
        }
    }
    assert!(found, ".nav element not found in arena");
}
