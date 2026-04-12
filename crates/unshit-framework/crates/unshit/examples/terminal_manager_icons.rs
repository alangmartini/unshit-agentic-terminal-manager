//! Terminal-manager icon showcase.
//!
//! Renders every unique inline SVG icon found in `../terminal-manager/` using
//! the framework's SVG primitive (landed in #120 / #124). Proves that:
//!
//! 1. All 17 icon designs from terminal-manager round trip through the
//!    hand rolled SVG path parser and tessellation cache.
//! 2. `stroke="currentColor"` inherits from the host element's CSS `color`
//!    property, so the same icon definition renders in multiple hues without
//!    duplicating geometry.
//! 3. The tessellation cache reuses entries for identical icons drawn at
//!    multiple sites on the page (the terminal icon appears four times in the
//!    original, here it appears three times under three colors).
//!
//! This example is the closing demo for epic #125 (full terminal-manager port
//! to unshit). The full port continues under its capillary issues.

use unshit::app::{App, AppConfig};
use unshit::core::element::*;
use unshit::core::svg::{
    parse_svg_path, StrokeLineCap, StrokeLineJoin, SvgAttrs, SvgNode, SvgPaint, SvgPrimitive,
    ViewBox,
};

fn main() {
    env_logger::init();

    let app = App::new(
        AppConfig {
            title: "terminal-manager icon showcase".to_string(),
            width: 1120,
            height: 780,
            css: CSS.to_string(),
            ..Default::default()
        },
        build_tree,
    );

    app.run();
}

// ---------------------------------------------------------------------------
// Tree
// ---------------------------------------------------------------------------

fn build_tree() -> ElementTree {
    ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("root")
            .with_child(
                ElementDef::new(Tag::Div)
                    .with_class("page")
                    .with_child(
                        ElementDef::new(Tag::Div)
                            .with_class("header")
                            .with_child(
                                ElementDef::new(Tag::Span)
                                    .with_class("kicker")
                                    .with_text("EPIC 125 CLOSING DEMO"),
                            )
                            .with_child(
                                ElementDef::new(Tag::Span)
                                    .with_class("title")
                                    .with_text("terminal-manager icon showcase"),
                            )
                            .with_child(
                                ElementDef::new(Tag::Span)
                                    .with_class("subtitle")
                                    .with_text(
                                        "Every inline SVG from ../terminal-manager/, rendered through the framework's SVG primitive. \
                                         Row color drives stroke via currentColor cascade.",
                                    ),
                            ),
                    )
                    .with_child(row("amber row", "row-amber", all_icons()))
                    .with_child(row("sage row", "row-sage", all_icons()))
                    .with_child(row("rust row", "row-rust", all_icons())),
            ),
    }
}

fn row(label: &str, color_class: &str, tiles: Vec<(&'static str, SvgNode)>) -> ElementDef {
    let mut container = ElementDef::new(Tag::Div)
        .with_class("row")
        .with_class(color_class)
        .with_child(ElementDef::new(Tag::Span).with_class("row-label").with_text(label));

    let mut grid = ElementDef::new(Tag::Div).with_class("grid");
    for (name, node) in tiles {
        grid = grid.with_child(icon_tile(name, node));
    }
    container = container.with_child(grid);
    container
}

fn icon_tile(label: &'static str, node: SvgNode) -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class("tile")
        .with_child(ElementDef::new(Tag::Div).with_class("icon-cell").with_svg(node))
        .with_child(ElementDef::new(Tag::Span).with_class("tile-label").with_text(label))
}

// ---------------------------------------------------------------------------
// Icon catalog
// ---------------------------------------------------------------------------

fn all_icons() -> Vec<(&'static str, SvgNode)> {
    vec![
        ("sidebar-toggle", icon_sidebar_toggle()),
        ("fullscreen-box", icon_fullscreen_box()),
        ("sidebar-split", icon_sidebar_split()),
        ("fullscreen-corners", icon_fullscreen_corners()),
        ("plus", icon_plus()),
        ("chevrons", icon_chevrons()),
        ("terminal", icon_terminal()),
        ("user", icon_user()),
        ("git-branch", icon_git_branch()),
        ("folder", icon_folder()),
        ("env-list", icon_env_list()),
        ("split-h", icon_split_h()),
        ("split-v", icon_split_v()),
        ("grid", icon_grid()),
        ("balance", icon_balance()),
        ("settings", icon_settings()),
        ("close", icon_close()),
    ]
}

// ---------------------------------------------------------------------------
// Icon builders. Each returns a root SVG Group with viewBox 0 0 16 16 and
// stroke="currentColor". Geometry matches ../terminal-manager/index.html.
// ---------------------------------------------------------------------------

fn icon_sidebar_toggle() -> SvgNode {
    // <svg ... stroke-width="1.6" linecap round linejoin round>
    //   <path d="M2 4l4 4l-4 4"/>
    //   <path d="M9 12h5"/>
    // </svg>
    group(
        root_attrs(1.6, StrokeLineCap::Round, StrokeLineJoin::Round),
        vec![path_d("M2 4l4 4l-4 4"), path_d("M9 12h5")],
    )
}

fn icon_fullscreen_box() -> SvgNode {
    // rect + path (prompt inside)
    group(
        root_attrs(1.5, StrokeLineCap::Round, StrokeLineJoin::Miter),
        vec![rect(2.0, 2.0, 12.0, 12.0, 1.0), path_d("M5 6l2 2l-2 2M8 10h3")],
    )
}

fn icon_sidebar_split() -> SvgNode {
    // rect + divider line at x=6
    group(
        root_attrs(1.4, StrokeLineCap::Butt, StrokeLineJoin::Miter),
        vec![rect(2.0, 3.0, 12.0, 10.0, 1.0), line(6.0, 3.0, 6.0, 13.0)],
    )
}

fn icon_fullscreen_corners() -> SvgNode {
    group(
        root_attrs(1.4, StrokeLineCap::Round, StrokeLineJoin::Miter),
        vec![path_d("M3 6V3h3M13 6V3h-3M3 10v3h3M13 10v3h-3")],
    )
}

fn icon_plus() -> SvgNode {
    group(
        root_attrs(1.6, StrokeLineCap::Round, StrokeLineJoin::Miter),
        vec![path_d("M8 3v10M3 8h10")],
    )
}

fn icon_chevrons() -> SvgNode {
    group(
        root_attrs(1.6, StrokeLineCap::Round, StrokeLineJoin::Miter),
        vec![path_d("M4 7l4-3l4 3M4 9l4 3l4-3")],
    )
}

fn icon_terminal() -> SvgNode {
    // rect for the screen, inner path shows `>_`
    group(
        root_attrs(1.5, StrokeLineCap::Round, StrokeLineJoin::Round),
        vec![rect(2.0, 3.0, 12.0, 10.0, 1.0), path_d("M5 7l2 1.5L5 10M8 10h3")],
    )
}

fn icon_user() -> SvgNode {
    // circle head + body arc
    let mut body_arc = path_d("M3 13c.8-2.5 2.8-4 5-4s4.2 1.5 5 4");
    // the body path overrides stroke-linecap to round (the svg root keeps
    // its default which in the source is unset)
    body_arc.attrs.stroke_linecap = Some(StrokeLineCap::Round);
    group(
        root_attrs(1.5, StrokeLineCap::Butt, StrokeLineJoin::Miter),
        vec![circle(8.0, 6.0, 2.5), body_arc],
    )
}

fn icon_git_branch() -> SvgNode {
    // 3 circles plus a path with elliptical arcs connecting them
    group(
        root_attrs(1.5, StrokeLineCap::Round, StrokeLineJoin::Miter),
        vec![
            circle(4.0, 4.0, 1.5),
            circle(4.0, 12.0, 1.5),
            circle(12.0, 8.0, 1.5),
            path_d("M4 5.5v5M5.5 4H9a2 2 0 012 2v.5M5.5 12H9a2 2 0 002-2v-.5"),
        ],
    )
}

fn icon_folder() -> SvgNode {
    // box body plus triangle lid
    group(
        root_attrs(1.5, StrokeLineCap::Round, StrokeLineJoin::Round),
        vec![path_d("M2 5h12v8H2zM2 5l6-3l6 3")],
    )
}

fn icon_env_list() -> SvgNode {
    // 3 lines plus 3 filled dots colored by currentColor
    let mut dot_a = circle(5.0, 4.0, 0.5);
    dot_a.attrs.fill = Some(SvgPaint::Current);
    let mut dot_b = circle(11.0, 8.0, 0.5);
    dot_b.attrs.fill = Some(SvgPaint::Current);
    let mut dot_c = circle(7.0, 12.0, 0.5);
    dot_c.attrs.fill = Some(SvgPaint::Current);

    group(
        root_attrs(1.5, StrokeLineCap::Round, StrokeLineJoin::Miter),
        vec![path_d("M3 4h10M3 8h10M3 12h10"), dot_a, dot_b, dot_c],
    )
}

fn icon_split_h() -> SvgNode {
    group(
        root_attrs(1.4, StrokeLineCap::Butt, StrokeLineJoin::Miter),
        vec![rect(2.0, 3.0, 12.0, 10.0, 1.0), line(8.0, 3.0, 8.0, 13.0)],
    )
}

fn icon_split_v() -> SvgNode {
    group(
        root_attrs(1.4, StrokeLineCap::Butt, StrokeLineJoin::Miter),
        vec![rect(2.0, 3.0, 12.0, 10.0, 1.0), line(2.0, 8.0, 14.0, 8.0)],
    )
}

fn icon_grid() -> SvgNode {
    group(
        root_attrs(1.4, StrokeLineCap::Butt, StrokeLineJoin::Miter),
        vec![rect(2.0, 3.0, 12.0, 10.0, 1.0), line(8.0, 3.0, 8.0, 13.0), line(2.0, 8.0, 14.0, 8.0)],
    )
}

fn icon_balance() -> SvgNode {
    group(
        root_attrs(1.4, StrokeLineCap::Round, StrokeLineJoin::Miter),
        vec![path_d("M4 8h8M6 5l-2 3l2 3M10 5l2 3l-2 3")],
    )
}

fn icon_settings() -> SvgNode {
    // center circle plus 8 tabs radiating outward
    group(
        root_attrs(1.4, StrokeLineCap::Round, StrokeLineJoin::Round),
        vec![
            circle(8.0, 8.0, 2.0),
            path_d(
                "M8 1.5v1.5M8 13v1.5M14.5 8H13M3 8H1.5M12.6 3.4l-1 1M4.4 11.6l-1 1M12.6 12.6l-1-1M4.4 4.4l-1-1",
            ),
        ],
    )
}

fn icon_close() -> SvgNode {
    group(
        root_attrs(1.8, StrokeLineCap::Round, StrokeLineJoin::Miter),
        vec![path_d("M4 4l8 8M12 4l-8 8")],
    )
}

// ---------------------------------------------------------------------------
// Builder helpers
// ---------------------------------------------------------------------------

fn group(attrs: SvgAttrs, children: Vec<SvgNode>) -> SvgNode {
    SvgNode { primitive: SvgPrimitive::Group, attrs, children }
}

fn path_d(d: &str) -> SvgNode {
    let commands = parse_svg_path(d).expect("icon path data must parse");
    SvgNode {
        primitive: SvgPrimitive::Path { d: d.to_string(), commands },
        attrs: SvgAttrs::default(),
        children: Vec::new(),
    }
}

fn circle(cx: f32, cy: f32, r: f32) -> SvgNode {
    SvgNode {
        primitive: SvgPrimitive::Circle { cx, cy, r },
        attrs: SvgAttrs::default(),
        children: Vec::new(),
    }
}

fn rect(x: f32, y: f32, width: f32, height: f32, rx: f32) -> SvgNode {
    SvgNode {
        primitive: SvgPrimitive::Rect { x, y, width, height, rx, ry: rx },
        attrs: SvgAttrs::default(),
        children: Vec::new(),
    }
}

fn line(x1: f32, y1: f32, x2: f32, y2: f32) -> SvgNode {
    SvgNode {
        primitive: SvgPrimitive::Line { x1, y1, x2, y2 },
        attrs: SvgAttrs::default(),
        children: Vec::new(),
    }
}

fn root_attrs(stroke_width: f32, cap: StrokeLineCap, join: StrokeLineJoin) -> SvgAttrs {
    SvgAttrs {
        view_box: Some(ViewBox::new(0.0, 0.0, 16.0, 16.0)),
        fill: Some(SvgPaint::None),
        stroke: Some(SvgPaint::Current),
        stroke_width: Some(stroke_width),
        stroke_linecap: Some(cap),
        stroke_linejoin: Some(join),
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// Style
// ---------------------------------------------------------------------------

const CSS: &str = r#"
.root {
    display: flex;
    width: 100%;
    height: 100%;
    background: #0a0806;
    padding: 28px;
}

.page {
    display: flex;
    flex-direction: column;
    width: 100%;
    flex-grow: 1;
    gap: 20px;
    background: rgba(20, 16, 12, 0.85);
    border-radius: 18px;
    border-width: 1px;
    border-color: rgba(212, 163, 72, 0.18);
    padding: 28px 32px;
}

.header {
    display: flex;
    flex-direction: column;
    gap: 4px;
}

.kicker {
    color: #8a6020;
    font-size: 11px;
    font-weight: bold;
    letter-spacing: 3px;
}

.title {
    color: #f0e6d2;
    font-size: 26px;
    font-weight: bold;
    letter-spacing: 0.5px;
}

.subtitle {
    color: #8b7355;
    font-size: 13px;
    line-height: 1.5;
    max-width: 860px;
}

.row {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 14px 16px;
    background: rgba(32, 24, 16, 0.6);
    border-radius: 12px;
    border-width: 1px;
    border-color: rgba(212, 163, 72, 0.08);
}

.row-label {
    color: #a0824c;
    font-size: 10px;
    font-weight: bold;
    letter-spacing: 2px;
}

.row-amber { color: #d4a348; }
.row-sage  { color: #8ea878; }
.row-rust  { color: #c87850; }

.grid {
    display: flex;
    flex-wrap: wrap;
    gap: 10px;
}

.tile {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 6px;
    width: 88px;
    padding: 12px 8px;
    background: rgba(18, 14, 10, 0.7);
    border-radius: 10px;
    border-width: 1px;
    border-color: rgba(212, 163, 72, 0.12);
}

.tile:hover {
    background: rgba(30, 22, 14, 0.85);
    border-color: rgba(212, 163, 72, 0.28);
}

.icon-cell {
    display: flex;
    width: 32px;
    height: 32px;
    align-items: center;
    justify-content: center;
}

.tile-label {
    color: #6b553a;
    font-size: 10px;
    font-weight: bold;
    letter-spacing: 0.5px;
}
"#;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_has_all_seventeen_unique_icons() {
        let icons = all_icons();
        assert_eq!(icons.len(), 17, "expected 17 unique icons, got {}", icons.len());
    }

    #[test]
    fn every_icon_root_is_a_group_with_view_box() {
        for (name, node) in all_icons() {
            assert!(
                matches!(node.primitive, SvgPrimitive::Group),
                "icon {name} root primitive should be a group"
            );
            let vb = node.attrs.view_box.expect("icon root should carry a viewBox");
            assert_eq!(vb.min_x, 0.0);
            assert_eq!(vb.min_y, 0.0);
            assert_eq!(vb.width, 16.0);
            assert_eq!(vb.height, 16.0);
        }
    }

    #[test]
    fn every_icon_strokes_via_current_color() {
        for (name, node) in all_icons() {
            assert_eq!(
                node.attrs.stroke,
                Some(SvgPaint::Current),
                "icon {name} should stroke via currentColor so CSS `color` drives it"
            );
        }
    }

    #[test]
    fn every_icon_has_at_least_one_child() {
        for (name, node) in all_icons() {
            assert!(
                !node.children.is_empty(),
                "icon {name} should contain at least one geometry child"
            );
        }
    }

    #[test]
    fn every_path_primitive_parses_to_commands() {
        // Walk every SvgNode recursively and assert that every path primitive
        // has non empty commands matching the `d` string.
        fn visit(name: &str, node: &SvgNode) {
            if let SvgPrimitive::Path { d, commands } = &node.primitive {
                assert!(
                    !commands.is_empty(),
                    "icon {name}: path `{d}` tessellated to zero commands"
                );
                // Reparse to prove the string is still valid after round trip.
                let reparsed = parse_svg_path(d)
                    .unwrap_or_else(|e| panic!("icon {name}: reparse of `{d}` failed: {e:?}"));
                assert_eq!(
                    reparsed.len(),
                    commands.len(),
                    "icon {name}: reparse of `{d}` produced different command count"
                );
            }
            for child in &node.children {
                visit(name, child);
            }
        }

        for (name, node) in all_icons() {
            visit(name, &node);
        }
    }

    #[test]
    fn tree_builds_without_panic() {
        let tree = build_tree();
        // Sanity check: the root is a Div with the `root` class.
        assert!(matches!(tree.root.tag, Tag::Div));
    }
}
