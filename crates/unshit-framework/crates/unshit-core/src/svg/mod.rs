//! Inline SVG data model, path parser, and presentation attribute parser.
//!
//! This module contains no rendering logic. The renderer crate owns
//! tessellation, caching, and GPU upload. What lives here is the authoring
//! surface (tags, attributes, path commands) plus the small parsers that turn
//! user strings into typed data the renderer can consume.
//!
//! Scope. A deliberate subset of SVG 1.1 primitives: `svg`, `path`, `circle`,
//! `rect`, `line`, `polyline`, `polygon`, `g`. Filters, gradients, masks,
//! text, external images, animations, dasharray, and SVG 2 features are out
//! of scope on purpose.

pub mod attrs;
pub mod path_parser;
pub mod types;

pub use attrs::{
    parse_attribute_f32, parse_color_attr, parse_points, parse_stroke_linecap,
    parse_stroke_linejoin, parse_transform, parse_view_box, SvgAttrError,
};
pub use path_parser::{parse_svg_path, SvgPathError};
pub use types::{
    PathCommand, StrokeLineCap, StrokeLineJoin, SvgAttrs, SvgNode, SvgPaint, SvgPrimitive,
    SvgTransform, ViewBox,
};
