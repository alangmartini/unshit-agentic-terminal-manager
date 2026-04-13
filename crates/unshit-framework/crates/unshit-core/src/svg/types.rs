//! Typed data structures for inline SVG primitives.
//!
//! Every struct here is plain data. Parsing lives in `path_parser` and
//! `attrs`. Tessellation lives in `unshit-renderer`.

use crate::style::types::Color;

/// A single SVG path command as produced by the mini language parser.
///
/// Coordinates are always stored in absolute form even if the source used a
/// relative command. Converting relative to absolute happens inside the
/// parser so downstream code only deals with one representation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PathCommand {
    /// Move the pen to (x, y). No stroke is emitted.
    MoveTo { x: f32, y: f32 },
    /// Draw a line from the previous pen position to (x, y).
    LineTo { x: f32, y: f32 },
    /// Cubic bezier with two control points.
    CubicTo { x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32 },
    /// Quadratic bezier with a single control point.
    QuadTo { x1: f32, y1: f32, x: f32, y: f32 },
    /// Close the current subpath back to its starting point.
    Close,
}

/// A paint value for `fill` or `stroke`.
///
/// `None` suppresses the pass entirely. `Current` defers color resolution to
/// the element computed style at render time so the CSS cascade still drives
/// `currentColor`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum SvgPaint {
    #[default]
    None,
    Current,
    Solid(Color),
}

/// Stroke line cap style. Mirrors SVG 1.1 names.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StrokeLineCap {
    #[default]
    Butt,
    Round,
    Square,
}

impl StrokeLineCap {
    pub fn as_u8(self) -> u8 {
        match self {
            StrokeLineCap::Butt => 0,
            StrokeLineCap::Round => 1,
            StrokeLineCap::Square => 2,
        }
    }
}

/// Stroke line join style. Mirrors SVG 1.1 names.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StrokeLineJoin {
    #[default]
    Miter,
    Round,
    Bevel,
}

impl StrokeLineJoin {
    pub fn as_u8(self) -> u8 {
        match self {
            StrokeLineJoin::Miter => 0,
            StrokeLineJoin::Round => 1,
            StrokeLineJoin::Bevel => 2,
        }
    }
}

/// SVG viewBox coordinates. Stored as (min_x, min_y, width, height).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewBox {
    pub min_x: f32,
    pub min_y: f32,
    pub width: f32,
    pub height: f32,
}

impl ViewBox {
    pub const fn new(min_x: f32, min_y: f32, width: f32, height: f32) -> Self {
        Self { min_x, min_y, width, height }
    }
}

impl Default for ViewBox {
    fn default() -> Self {
        Self { min_x: 0.0, min_y: 0.0, width: 1.0, height: 1.0 }
    }
}

/// A 3x2 affine transform in row major order.
///
/// Layout matches the standard SVG matrix notation:
/// ```text
/// [ a c e ]
/// [ b d f ]
/// [ 0 0 1 ]
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SvgTransform {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub e: f32,
    pub f: f32,
}

impl SvgTransform {
    pub const IDENTITY: SvgTransform =
        SvgTransform { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: 0.0, f: 0.0 };

    pub const fn new(a: f32, b: f32, c: f32, d: f32, e: f32, f: f32) -> Self {
        Self { a, b, c, d, e, f }
    }

    pub fn translate(tx: f32, ty: f32) -> Self {
        Self { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: tx, f: ty }
    }

    pub fn scale(sx: f32, sy: f32) -> Self {
        Self { a: sx, b: 0.0, c: 0.0, d: sy, e: 0.0, f: 0.0 }
    }

    pub fn rotate_deg(angle_deg: f32) -> Self {
        let r = angle_deg.to_radians();
        let (s, c) = r.sin_cos();
        Self { a: c, b: s, c: -s, d: c, e: 0.0, f: 0.0 }
    }

    /// Matrix multiply `self * other`. Use this to compose a chain in the
    /// order it was written in the source string (left to right application,
    /// right to left multiplication per SVG 1.1 2.5).
    pub fn multiply(self, other: SvgTransform) -> Self {
        Self {
            a: self.a * other.a + self.c * other.b,
            b: self.b * other.a + self.d * other.b,
            c: self.a * other.c + self.c * other.d,
            d: self.b * other.c + self.d * other.d,
            e: self.a * other.e + self.c * other.f + self.e,
            f: self.b * other.e + self.d * other.f + self.f,
        }
    }

    /// Transform a point.
    pub fn apply(self, x: f32, y: f32) -> (f32, f32) {
        (self.a * x + self.c * y + self.e, self.b * x + self.d * y + self.f)
    }
}

impl Default for SvgTransform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

/// Presentation attributes parsed from an SVG element.
///
/// Missing values fall through the SVG cascade to parent group attrs. A
/// default instance (no fill, no stroke, width 1) is the starting point so
/// that a `Some` on any field means the author explicitly set it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SvgAttrs {
    pub fill: Option<SvgPaint>,
    pub stroke: Option<SvgPaint>,
    pub stroke_width: Option<f32>,
    pub stroke_linecap: Option<StrokeLineCap>,
    pub stroke_linejoin: Option<StrokeLineJoin>,
    pub view_box: Option<ViewBox>,
    pub transform: Option<SvgTransform>,
    pub opacity: Option<f32>,
}

impl SvgAttrs {
    pub fn new() -> Self {
        Self::default()
    }

    /// Merge `self` over `parent`. Fields set on `self` win, missing fields
    /// inherit from the parent. `transform` always composes rather than
    /// replaces so nested groups accumulate correctly.
    pub fn cascade_over(&self, parent: &SvgAttrs) -> SvgAttrs {
        let transform = match (parent.transform, self.transform) {
            (Some(p), Some(c)) => Some(p.multiply(c)),
            (Some(p), None) => Some(p),
            (None, Some(c)) => Some(c),
            (None, None) => None,
        };

        SvgAttrs {
            fill: self.fill.or(parent.fill),
            stroke: self.stroke.or(parent.stroke),
            stroke_width: self.stroke_width.or(parent.stroke_width),
            stroke_linecap: self.stroke_linecap.or(parent.stroke_linecap),
            stroke_linejoin: self.stroke_linejoin.or(parent.stroke_linejoin),
            view_box: self.view_box.or(parent.view_box),
            transform,
            opacity: self.opacity.or(parent.opacity),
        }
    }
}

/// A single SVG primitive. Groups recurse into `SvgNode::children`.
#[derive(Clone, Debug, PartialEq)]
pub enum SvgPrimitive {
    Path {
        d: String,
        commands: Vec<PathCommand>,
    },
    Circle {
        cx: f32,
        cy: f32,
        r: f32,
    },
    Rect {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        rx: f32,
        ry: f32,
    },
    Line {
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
    },
    Polyline {
        points: Vec<(f32, f32)>,
    },
    Polygon {
        points: Vec<(f32, f32)>,
    },
    /// Root `<svg>` or inner `<g>` container. Has no geometry of its own.
    Group,
}

/// A node in an inline SVG subtree. The outer `ElementContent::Svg` always
/// points at a root node whose primitive is `Group`.
#[derive(Clone, Debug, PartialEq)]
pub struct SvgNode {
    pub primitive: SvgPrimitive,
    pub attrs: SvgAttrs,
    pub children: Vec<SvgNode>,
}

impl SvgNode {
    pub fn root() -> Self {
        Self { primitive: SvgPrimitive::Group, attrs: SvgAttrs::new(), children: Vec::new() }
    }

    pub fn with_attrs(mut self, attrs: SvgAttrs) -> Self {
        self.attrs = attrs;
        self
    }

    pub fn with_child(mut self, child: SvgNode) -> Self {
        self.children.push(child);
        self
    }
}
