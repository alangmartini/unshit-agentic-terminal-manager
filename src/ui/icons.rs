use unshit::core::element::*;
use unshit::core::svg::{
    parse_svg_path, StrokeLineCap, StrokeLineJoin, SvgAttrs, SvgNode, SvgPaint, SvgPrimitive,
    ViewBox,
};

use crate::state::SubtabIcon;

pub fn svg_icon(node: SvgNode) -> ElementDef {
    ElementDef::new(Tag::Div).with_svg(node)
}

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

pub fn icon_brand_chevron() -> SvgNode {
    group(
        root_attrs(1.6, StrokeLineCap::Round, StrokeLineJoin::Round),
        vec![path_d("M2 4l4 4l-4 4"), path_d("M9 12h5")],
    )
}

pub fn icon_search() -> SvgNode {
    group(
        root_attrs(1.5, StrokeLineCap::Round, StrokeLineJoin::Miter),
        vec![rect(2.0, 2.0, 12.0, 12.0, 1.0), path_d("M5 6l2 2l-2 2M8 10h3")],
    )
}

pub fn icon_sidebar_toggle() -> SvgNode {
    group(
        root_attrs(1.4, StrokeLineCap::Butt, StrokeLineJoin::Miter),
        vec![rect(2.0, 3.0, 12.0, 10.0, 1.0), line(6.0, 3.0, 6.0, 13.0)],
    )
}

pub fn icon_fullscreen_corners() -> SvgNode {
    group(
        root_attrs(1.4, StrokeLineCap::Round, StrokeLineJoin::Miter),
        vec![path_d("M3 6V3h3M13 6V3h-3M3 10v3h3M13 10v3h-3")],
    )
}

pub fn icon_plus() -> SvgNode {
    group(
        root_attrs(1.6, StrokeLineCap::Round, StrokeLineJoin::Miter),
        vec![path_d("M8 3v10M3 8h10")],
    )
}

pub fn icon_chevrons() -> SvgNode {
    group(
        root_attrs(1.6, StrokeLineCap::Round, StrokeLineJoin::Miter),
        vec![path_d("M4 7l4-3l4 3M4 9l4 3l4-3")],
    )
}

pub fn icon_terminal() -> SvgNode {
    group(
        root_attrs(1.5, StrokeLineCap::Round, StrokeLineJoin::Round),
        vec![rect(2.0, 3.0, 12.0, 10.0, 1.0), path_d("M5 7l2 1.5L5 10M8 10h3")],
    )
}

pub fn icon_user() -> SvgNode {
    let mut body_arc = path_d("M3 13c.8-2.5 2.8-4 5-4s4.2 1.5 5 4");
    body_arc.attrs.stroke_linecap = Some(StrokeLineCap::Round);
    group(
        root_attrs(1.5, StrokeLineCap::Butt, StrokeLineJoin::Miter),
        vec![circle(8.0, 6.0, 2.5), body_arc],
    )
}

pub fn icon_git_branch() -> SvgNode {
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

pub fn icon_folder() -> SvgNode {
    group(
        root_attrs(1.5, StrokeLineCap::Round, StrokeLineJoin::Round),
        vec![path_d("M2 5h12v8H2zM2 5l6-3l6 3")],
    )
}

pub fn icon_env_list() -> SvgNode {
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

pub fn icon_split_h() -> SvgNode {
    group(
        root_attrs(1.4, StrokeLineCap::Butt, StrokeLineJoin::Miter),
        vec![rect(2.0, 3.0, 12.0, 10.0, 1.0), line(8.0, 3.0, 8.0, 13.0)],
    )
}

pub fn icon_split_v() -> SvgNode {
    group(
        root_attrs(1.4, StrokeLineCap::Butt, StrokeLineJoin::Miter),
        vec![rect(2.0, 3.0, 12.0, 10.0, 1.0), line(2.0, 8.0, 14.0, 8.0)],
    )
}

pub fn icon_grid() -> SvgNode {
    group(
        root_attrs(1.4, StrokeLineCap::Butt, StrokeLineJoin::Miter),
        vec![
            rect(2.0, 3.0, 12.0, 10.0, 1.0),
            line(8.0, 3.0, 8.0, 13.0),
            line(2.0, 8.0, 14.0, 8.0),
        ],
    )
}

pub fn icon_balance() -> SvgNode {
    group(
        root_attrs(1.4, StrokeLineCap::Round, StrokeLineJoin::Miter),
        vec![path_d("M4 8h8M6 5l-2 3l2 3M10 5l2 3l-2 3")],
    )
}

pub fn icon_settings() -> SvgNode {
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

pub fn icon_close() -> SvgNode {
    group(
        root_attrs(1.8, StrokeLineCap::Round, StrokeLineJoin::Miter),
        vec![path_d("M4 4l8 8M12 4l-8 8")],
    )
}

pub fn subtab_icon_for(kind: SubtabIcon) -> SvgNode {
    match kind {
        SubtabIcon::Terminal => icon_terminal(),
        SubtabIcon::User => icon_user(),
        SubtabIcon::GitBranch => icon_git_branch(),
        SubtabIcon::Folder => icon_folder(),
        SubtabIcon::EnvList => icon_env_list(),
    }
}
