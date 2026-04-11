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

#[cfg(test)]
mod tests {
    use super::*;

    // -- Helper functions -----------------------------------------------------

    #[test]
    fn path_d_parses_valid_path() {
        let node = path_d("M2 4l4 4l-4 4");
        assert!(matches!(node.primitive, SvgPrimitive::Path { .. }));
        assert!(node.children.is_empty());
    }

    #[test]
    fn circle_creates_circle_primitive() {
        let node = circle(8.0, 6.0, 2.5);
        assert!(matches!(
            node.primitive,
            SvgPrimitive::Circle { cx, cy, r }
            if (cx - 8.0).abs() < f32::EPSILON
            && (cy - 6.0).abs() < f32::EPSILON
            && (r - 2.5).abs() < f32::EPSILON
        ));
    }

    #[test]
    fn rect_creates_rect_primitive() {
        let node = rect(2.0, 3.0, 12.0, 10.0, 1.0);
        assert!(matches!(
            node.primitive,
            SvgPrimitive::Rect { x, y, width, height, rx, ry }
            if (x - 2.0).abs() < f32::EPSILON
            && (y - 3.0).abs() < f32::EPSILON
            && (width - 12.0).abs() < f32::EPSILON
            && (height - 10.0).abs() < f32::EPSILON
            && (rx - 1.0).abs() < f32::EPSILON
            && (ry - 1.0).abs() < f32::EPSILON
        ));
    }

    #[test]
    fn line_creates_line_primitive() {
        let node = line(6.0, 3.0, 6.0, 13.0);
        assert!(matches!(
            node.primitive,
            SvgPrimitive::Line { x1, y1, x2, y2 }
            if (x1 - 6.0).abs() < f32::EPSILON
            && (y1 - 3.0).abs() < f32::EPSILON
            && (x2 - 6.0).abs() < f32::EPSILON
            && (y2 - 13.0).abs() < f32::EPSILON
        ));
    }

    #[test]
    fn group_wraps_children() {
        let children = vec![path_d("M0 0l1 1"), circle(1.0, 1.0, 1.0)];
        let node = group(SvgAttrs::default(), children);
        assert!(matches!(node.primitive, SvgPrimitive::Group));
        assert_eq!(node.children.len(), 2);
    }

    #[test]
    fn root_attrs_sets_viewbox_and_stroke() {
        let attrs = root_attrs(1.6, StrokeLineCap::Round, StrokeLineJoin::Round);
        assert!(attrs.view_box.is_some());
        assert!(matches!(attrs.fill, Some(SvgPaint::None)));
        assert!(matches!(attrs.stroke, Some(SvgPaint::Current)));
        assert_eq!(attrs.stroke_width, Some(1.6));
        assert_eq!(attrs.stroke_linecap, Some(StrokeLineCap::Round));
        assert_eq!(attrs.stroke_linejoin, Some(StrokeLineJoin::Round));
    }

    #[test]
    fn svg_icon_wraps_in_element_def() {
        let node = icon_brand_chevron();
        // Should produce an ElementDef without panicking
        let _elem = svg_icon(node);
    }

    // -- Icon builder functions (smoke tests) ---------------------------------

    #[test]
    fn icon_brand_chevron_builds() {
        let node = icon_brand_chevron();
        assert!(matches!(node.primitive, SvgPrimitive::Group));
        assert_eq!(node.children.len(), 2);
    }

    #[test]
    fn icon_search_builds() {
        let node = icon_search();
        assert!(matches!(node.primitive, SvgPrimitive::Group));
        assert_eq!(node.children.len(), 2); // rect + path
    }

    #[test]
    fn icon_sidebar_toggle_builds() {
        let node = icon_sidebar_toggle();
        assert!(matches!(node.primitive, SvgPrimitive::Group));
        assert_eq!(node.children.len(), 2); // rect + line
    }

    #[test]
    fn icon_fullscreen_corners_builds() {
        let node = icon_fullscreen_corners();
        assert!(matches!(node.primitive, SvgPrimitive::Group));
        assert_eq!(node.children.len(), 1);
    }

    #[test]
    fn icon_plus_builds() {
        let node = icon_plus();
        assert!(matches!(node.primitive, SvgPrimitive::Group));
        assert_eq!(node.children.len(), 1);
    }

    #[test]
    fn icon_chevrons_builds() {
        let node = icon_chevrons();
        assert!(matches!(node.primitive, SvgPrimitive::Group));
        assert_eq!(node.children.len(), 1);
    }

    #[test]
    fn icon_terminal_builds() {
        let node = icon_terminal();
        assert!(matches!(node.primitive, SvgPrimitive::Group));
        assert_eq!(node.children.len(), 2); // rect + path
    }

    #[test]
    fn icon_user_builds() {
        let node = icon_user();
        assert!(matches!(node.primitive, SvgPrimitive::Group));
        assert_eq!(node.children.len(), 2); // circle + body_arc path
    }

    #[test]
    fn icon_user_body_arc_has_round_linecap() {
        let node = icon_user();
        // The second child (body_arc) should have stroke_linecap set to Round
        let body_arc = &node.children[1];
        assert_eq!(body_arc.attrs.stroke_linecap, Some(StrokeLineCap::Round));
    }

    #[test]
    fn icon_git_branch_builds() {
        let node = icon_git_branch();
        assert!(matches!(node.primitive, SvgPrimitive::Group));
        assert_eq!(node.children.len(), 4); // 3 circles + path
    }

    #[test]
    fn icon_folder_builds() {
        let node = icon_folder();
        assert!(matches!(node.primitive, SvgPrimitive::Group));
        assert_eq!(node.children.len(), 1);
    }

    #[test]
    fn icon_env_list_builds() {
        let node = icon_env_list();
        assert!(matches!(node.primitive, SvgPrimitive::Group));
        assert_eq!(node.children.len(), 4); // path + 3 dots
    }

    #[test]
    fn icon_env_list_dots_have_fill() {
        let node = icon_env_list();
        // Children 1, 2, 3 are the dots with fill = Current
        for i in 1..=3 {
            assert!(
                matches!(node.children[i].attrs.fill, Some(SvgPaint::Current)),
                "dot at index {} should have fill=Current",
                i,
            );
        }
    }

    #[test]
    fn icon_split_h_builds() {
        let node = icon_split_h();
        assert!(matches!(node.primitive, SvgPrimitive::Group));
        assert_eq!(node.children.len(), 2);
    }

    #[test]
    fn icon_split_v_builds() {
        let node = icon_split_v();
        assert!(matches!(node.primitive, SvgPrimitive::Group));
        assert_eq!(node.children.len(), 2);
    }

    #[test]
    fn icon_grid_builds() {
        let node = icon_grid();
        assert!(matches!(node.primitive, SvgPrimitive::Group));
        assert_eq!(node.children.len(), 3); // rect + 2 lines
    }

    #[test]
    fn icon_balance_builds() {
        let node = icon_balance();
        assert!(matches!(node.primitive, SvgPrimitive::Group));
        assert_eq!(node.children.len(), 1);
    }

    #[test]
    fn icon_settings_builds() {
        let node = icon_settings();
        assert!(matches!(node.primitive, SvgPrimitive::Group));
        assert_eq!(node.children.len(), 2); // circle + path
    }

    #[test]
    fn icon_close_builds() {
        let node = icon_close();
        assert!(matches!(node.primitive, SvgPrimitive::Group));
        assert_eq!(node.children.len(), 1);
    }

    // -- subtab_icon_for covers all variants ----------------------------------

    #[test]
    fn subtab_icon_for_terminal() {
        let node = subtab_icon_for(SubtabIcon::Terminal);
        assert!(matches!(node.primitive, SvgPrimitive::Group));
    }

    #[test]
    fn subtab_icon_for_user() {
        let node = subtab_icon_for(SubtabIcon::User);
        assert!(matches!(node.primitive, SvgPrimitive::Group));
    }

    #[test]
    fn subtab_icon_for_git_branch() {
        let node = subtab_icon_for(SubtabIcon::GitBranch);
        assert!(matches!(node.primitive, SvgPrimitive::Group));
    }

    #[test]
    fn subtab_icon_for_folder() {
        let node = subtab_icon_for(SubtabIcon::Folder);
        assert!(matches!(node.primitive, SvgPrimitive::Group));
    }

    #[test]
    fn subtab_icon_for_env_list() {
        let node = subtab_icon_for(SubtabIcon::EnvList);
        assert!(matches!(node.primitive, SvgPrimitive::Group));
    }
}
