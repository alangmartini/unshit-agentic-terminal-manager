//! Lyon backed tessellation for the SVG primitive subset.
//!
//! Given an `SvgPrimitive` plus the cascaded `SvgAttrs` for the element, this
//! module produces a pair of CPU side vertex and index buffers ready to be
//! uploaded to the GPU. Fills and strokes are generated together when both
//! are requested so a single cached `SvgGeometry` drives both passes.
//!
//! Consumers should never call this directly on every frame. Routing happens
//! through the `SvgTessCache` which memoizes the `Arc<SvgGeometry>` output
//! keyed on path data plus stroke parameters.

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use lyon::math::{point, Point};
use lyon::path::Path;
use lyon::tessellation::geometry_builder::BuffersBuilder;
use lyon::tessellation::{
    FillOptions, FillTessellator, FillVertex, LineCap, LineJoin, Side, StrokeOptions,
    StrokeTessellator, StrokeVertex, VertexBuffers,
};
use unshit_core::style::types::Color;
use unshit_core::svg::types::{
    PathCommand, StrokeLineCap, StrokeLineJoin, SvgAttrs, SvgPaint, SvgPrimitive,
};

/// Default flattening tolerance. Smaller values give smoother curves at the
/// cost of more triangles. 0.02 gives noticeably smoother arcs on small (16px)
/// icons without a meaningful vertex count increase at that scale.
pub const DEFAULT_TOLERANCE: f32 = 0.02;

/// A tessellated SVG vertex, laid out for the GPU shader.
///
/// The `coverage` field implements Skia-style analytical anti-aliasing:
/// interior vertices get 1.0, stroke-edge vertices get a fractional value
/// based on distance from the geometric boundary. The GPU interpolates
/// coverage across triangles and the fragment shader uses it as an alpha
/// multiplier for sub-pixel edge softening.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct SvgVertex {
    pub position: [f32; 2],
    pub color: [f32; 4],
    pub coverage: f32,
}

/// Output of one tessellation run. Held behind an `Arc` so the cache can
/// hand out pointer equal references to hot entries.
#[derive(Clone, Debug, Default)]
pub struct SvgGeometry {
    pub vertices: Vec<SvgVertex>,
    pub indices: Vec<u32>,
}

impl SvgGeometry {
    pub fn is_empty(&self) -> bool {
        self.vertices.is_empty() || self.indices.is_empty()
    }

    pub fn vertex_count(&self) -> usize {
        self.vertices.len()
    }

    pub fn index_count(&self) -> usize {
        self.indices.len()
    }
}

/// Resolve `SvgPaint` into an effective RGBA color.
///
/// `SvgPaint::None` maps to fully transparent so callers know to skip the
/// pass. `SvgPaint::Current` resolves from the supplied `current_color` which
/// in turn comes from the element computed style at batch time.
pub fn resolve_paint(paint: SvgPaint, current_color: Color) -> Color {
    match paint {
        SvgPaint::None => Color::TRANSPARENT,
        SvgPaint::Current => current_color,
        SvgPaint::Solid(c) => c,
    }
}

fn color_to_f32(color: Color) -> [f32; 4] {
    color.to_linear_f32()
}

/// Tessellate a single primitive into an `SvgGeometry`.
///
/// `current_color` is the element computed `color` style. `tolerance` is the
/// lyon flattening tolerance, typically `DEFAULT_TOLERANCE`.
pub fn tessellate(
    primitive: &SvgPrimitive,
    attrs: &SvgAttrs,
    current_color: Color,
    tolerance: f32,
) -> Arc<SvgGeometry> {
    let mut buffers: VertexBuffers<SvgVertex, u32> = VertexBuffers::new();

    let fill_paint = attrs.fill.unwrap_or_default();
    let stroke_paint = attrs.stroke.unwrap_or_default();
    let stroke_width = attrs.stroke_width.unwrap_or(1.0);
    let linecap = attrs.stroke_linecap.unwrap_or_default();
    let linejoin = attrs.stroke_linejoin.unwrap_or_default();

    let fill_color = resolve_paint(fill_paint, current_color);
    let stroke_color = resolve_paint(stroke_paint, current_color);

    let want_fill = fill_color.a > 0 && !matches!(fill_paint, SvgPaint::None);
    let want_stroke =
        stroke_color.a > 0 && stroke_width > 0.0 && !matches!(stroke_paint, SvgPaint::None);

    if !want_fill && !want_stroke {
        return Arc::new(SvgGeometry::default());
    }

    let Some(path) = primitive_to_lyon_path(primitive) else {
        return Arc::new(SvgGeometry::default());
    };

    if want_fill {
        tessellate_fill(&path, fill_color, tolerance, &mut buffers);
    }
    if want_stroke {
        tessellate_stroke(
            &path,
            stroke_color,
            stroke_width,
            linecap,
            linejoin,
            tolerance,
            &mut buffers,
        );
    }

    Arc::new(SvgGeometry { vertices: buffers.vertices, indices: buffers.indices })
}

fn tessellate_fill(
    path: &Path,
    color: Color,
    tolerance: f32,
    buffers: &mut VertexBuffers<SvgVertex, u32>,
) {
    let options = FillOptions::tolerance(tolerance);
    let mut tessellator = FillTessellator::new();
    let color_arr = color_to_f32(color);
    let _ = tessellator.tessellate_path(
        path,
        &options,
        &mut BuffersBuilder::new(buffers, move |vertex: FillVertex| SvgVertex {
            position: vertex.position().to_array(),
            color: color_arr,
            coverage: 0.0, // uniform zero -> fwidth=0 -> shader skips AA
        }),
    );
}

#[allow(clippy::too_many_arguments)]
fn tessellate_stroke(
    path: &Path,
    color: Color,
    width: f32,
    linecap: StrokeLineCap,
    linejoin: StrokeLineJoin,
    tolerance: f32,
    buffers: &mut VertexBuffers<SvgVertex, u32>,
) {
    let cap = match linecap {
        StrokeLineCap::Butt => LineCap::Butt,
        StrokeLineCap::Round => LineCap::Round,
        StrokeLineCap::Square => LineCap::Square,
    };
    let join = match linejoin {
        StrokeLineJoin::Miter => LineJoin::Miter,
        StrokeLineJoin::Round => LineJoin::Round,
        StrokeLineJoin::Bevel => LineJoin::Bevel,
    };
    let options = StrokeOptions::tolerance(tolerance)
        .with_line_width(width)
        .with_start_cap(cap)
        .with_end_cap(cap)
        .with_line_join(join);

    let mut tessellator = StrokeTessellator::new();
    let color_arr = color_to_f32(color);
    let _ = tessellator.tessellate_path(
        path,
        &options,
        &mut BuffersBuilder::new(buffers, move |vertex: StrokeVertex| {
            // Signed side indicator: +1 for positive-side edge, -1 for
            // negative-side edge. The GPU interpolates this linearly across
            // each triangle, producing a smooth -1..+1 gradient across the
            // stroke width even though no vertex sits at the center.
            let coverage = match vertex.side() {
                Side::Positive => 1.0_f32,
                Side::Negative => -1.0_f32,
            };
            SvgVertex { position: vertex.position().to_array(), color: color_arr, coverage }
        }),
    );
}

/// Convert an `SvgPrimitive` into a lyon `Path`. Returns `None` for
/// degenerate shapes that have no area and no length.
fn primitive_to_lyon_path(primitive: &SvgPrimitive) -> Option<Path> {
    match primitive {
        SvgPrimitive::Path { commands, .. } => commands_to_lyon_path(commands),
        SvgPrimitive::Circle { cx, cy, r } => {
            if *r <= 0.0 {
                return None;
            }
            Some(build_circle(*cx, *cy, *r))
        }
        SvgPrimitive::Rect { x, y, width, height, rx, ry } => {
            if *width <= 0.0 || *height <= 0.0 {
                return None;
            }
            Some(build_rect(*x, *y, *width, *height, *rx, *ry))
        }
        SvgPrimitive::Line { x1, y1, x2, y2 } => {
            if (x1 - x2).abs() < 1e-6 && (y1 - y2).abs() < 1e-6 {
                return None;
            }
            let mut b = Path::builder();
            b.begin(point(*x1, *y1));
            b.line_to(point(*x2, *y2));
            b.end(false);
            Some(b.build())
        }
        SvgPrimitive::Polyline { points } => build_polyline(points, false),
        SvgPrimitive::Polygon { points } => build_polyline(points, true),
        SvgPrimitive::Group => None,
    }
}

fn commands_to_lyon_path(commands: &[PathCommand]) -> Option<Path> {
    if commands.is_empty() {
        return None;
    }
    let mut builder = Path::builder();
    let mut in_subpath = false;

    for cmd in commands {
        match *cmd {
            PathCommand::MoveTo { x, y } => {
                if in_subpath {
                    builder.end(false);
                }
                builder.begin(point(x, y));
                in_subpath = true;
            }
            PathCommand::LineTo { x, y } => {
                if !in_subpath {
                    builder.begin(point(x, y));
                    in_subpath = true;
                    continue;
                }
                builder.line_to(point(x, y));
            }
            PathCommand::CubicTo { x1, y1, x2, y2, x, y } => {
                if !in_subpath {
                    builder.begin(point(x, y));
                    in_subpath = true;
                    continue;
                }
                builder.cubic_bezier_to(point(x1, y1), point(x2, y2), point(x, y));
            }
            PathCommand::QuadTo { x1, y1, x, y } => {
                if !in_subpath {
                    builder.begin(point(x, y));
                    in_subpath = true;
                    continue;
                }
                builder.quadratic_bezier_to(point(x1, y1), point(x, y));
            }
            PathCommand::Close => {
                if in_subpath {
                    builder.end(true);
                    in_subpath = false;
                }
            }
        }
    }

    if in_subpath {
        builder.end(false);
    }
    Some(builder.build())
}

fn build_circle(cx: f32, cy: f32, r: f32) -> Path {
    // Approximate a circle with 4 cubic beziers. Control point offset is
    // 4/3 * tan(pi/8) times the radius.
    const K: f32 = 0.552_284_8;
    let mut b = Path::builder();
    let ox = r * K;
    let oy = r * K;
    b.begin(point(cx + r, cy));
    b.cubic_bezier_to(point(cx + r, cy + oy), point(cx + ox, cy + r), point(cx, cy + r));
    b.cubic_bezier_to(point(cx - ox, cy + r), point(cx - r, cy + oy), point(cx - r, cy));
    b.cubic_bezier_to(point(cx - r, cy - oy), point(cx - ox, cy - r), point(cx, cy - r));
    b.cubic_bezier_to(point(cx + ox, cy - r), point(cx + r, cy - oy), point(cx + r, cy));
    b.end(true);
    b.build()
}

fn build_rect(x: f32, y: f32, w: f32, h: f32, rx: f32, ry: f32) -> Path {
    let rx = rx.max(0.0).min(w * 0.5);
    let ry = ry.max(0.0).min(h * 0.5);
    let mut b = Path::builder();
    if rx <= 0.0 || ry <= 0.0 {
        b.begin(point(x, y));
        b.line_to(point(x + w, y));
        b.line_to(point(x + w, y + h));
        b.line_to(point(x, y + h));
        b.end(true);
        return b.build();
    }
    // Rounded rectangle via four quarter circle corners.
    const K: f32 = 0.552_284_8;
    let ox = rx * K;
    let oy = ry * K;
    let x0 = x;
    let y0 = y;
    let x1 = x + w;
    let y1 = y + h;

    b.begin(point(x0 + rx, y0));
    b.line_to(point(x1 - rx, y0));
    b.cubic_bezier_to(point(x1 - rx + ox, y0), point(x1, y0 + ry - oy), point(x1, y0 + ry));
    b.line_to(point(x1, y1 - ry));
    b.cubic_bezier_to(point(x1, y1 - ry + oy), point(x1 - rx + ox, y1), point(x1 - rx, y1));
    b.line_to(point(x0 + rx, y1));
    b.cubic_bezier_to(point(x0 + rx - ox, y1), point(x0, y1 - ry + oy), point(x0, y1 - ry));
    b.line_to(point(x0, y0 + ry));
    b.cubic_bezier_to(point(x0, y0 + ry - oy), point(x0 + rx - ox, y0), point(x0 + rx, y0));
    b.end(true);
    b.build()
}

fn build_polyline(points: &[(f32, f32)], closed: bool) -> Option<Path> {
    if points.len() < 2 {
        return None;
    }
    let mut b = Path::builder();
    let first: Point = point(points[0].0, points[0].1);
    b.begin(first);
    for p in &points[1..] {
        b.line_to(point(p.0, p.1));
    }
    b.end(closed);
    Some(b.build())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid_fill(color: Color) -> SvgAttrs {
        SvgAttrs { fill: Some(SvgPaint::Solid(color)), ..Default::default() }
    }

    fn solid_stroke(color: Color, width: f32) -> SvgAttrs {
        SvgAttrs {
            stroke: Some(SvgPaint::Solid(color)),
            stroke_width: Some(width),
            ..Default::default()
        }
    }

    #[test]
    fn fills_a_circle() {
        let p = SvgPrimitive::Circle { cx: 10.0, cy: 10.0, r: 5.0 };
        let g = tessellate(&p, &solid_fill(Color::BLACK), Color::BLACK, DEFAULT_TOLERANCE);
        assert!(g.vertex_count() > 0);
        assert!(g.index_count() > 0);
    }

    #[test]
    fn strokes_a_line_even_without_fill() {
        let p = SvgPrimitive::Line { x1: 0.0, y1: 0.0, x2: 20.0, y2: 0.0 };
        let g = tessellate(&p, &solid_stroke(Color::BLACK, 2.0), Color::BLACK, DEFAULT_TOLERANCE);
        assert!(g.vertex_count() > 0, "line stroke should produce vertices");
        assert!(g.index_count() > 0);
    }

    #[test]
    fn rect_with_no_fill_and_no_stroke_is_empty() {
        let p = SvgPrimitive::Rect { x: 0.0, y: 0.0, width: 10.0, height: 10.0, rx: 0.0, ry: 0.0 };
        let attrs = SvgAttrs::default();
        let g = tessellate(&p, &attrs, Color::BLACK, DEFAULT_TOLERANCE);
        assert!(g.is_empty());
    }

    #[test]
    fn zero_width_stroke_is_skipped() {
        let p = SvgPrimitive::Rect { x: 0.0, y: 0.0, width: 10.0, height: 10.0, rx: 0.0, ry: 0.0 };
        let attrs = solid_stroke(Color::BLACK, 0.0);
        let g = tessellate(&p, &attrs, Color::BLACK, DEFAULT_TOLERANCE);
        assert!(g.is_empty(), "zero width stroke should not generate triangles");
    }

    #[test]
    fn zero_radius_circle_is_empty() {
        let p = SvgPrimitive::Circle { cx: 0.0, cy: 0.0, r: 0.0 };
        let g = tessellate(&p, &solid_fill(Color::BLACK), Color::BLACK, DEFAULT_TOLERANCE);
        assert!(g.is_empty());
    }

    #[test]
    fn path_primitive_tessellates() {
        // Unit square via a short path.
        let commands = vec![
            PathCommand::MoveTo { x: 0.0, y: 0.0 },
            PathCommand::LineTo { x: 10.0, y: 0.0 },
            PathCommand::LineTo { x: 10.0, y: 10.0 },
            PathCommand::LineTo { x: 0.0, y: 10.0 },
            PathCommand::Close,
        ];
        let p = SvgPrimitive::Path { d: String::new(), commands };
        let g = tessellate(&p, &solid_fill(Color::BLACK), Color::BLACK, DEFAULT_TOLERANCE);
        assert!(g.vertex_count() >= 4);
    }

    #[test]
    fn polyline_strokes() {
        let p = SvgPrimitive::Polyline { points: vec![(0.0, 0.0), (5.0, 5.0), (10.0, 0.0)] };
        let g = tessellate(&p, &solid_stroke(Color::BLACK, 1.0), Color::BLACK, DEFAULT_TOLERANCE);
        assert!(g.vertex_count() > 0);
    }

    #[test]
    fn polygon_fills() {
        let p = SvgPrimitive::Polygon {
            points: vec![(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)],
        };
        let g = tessellate(&p, &solid_fill(Color::BLACK), Color::BLACK, DEFAULT_TOLERANCE);
        assert!(g.vertex_count() >= 4);
    }

    #[test]
    fn same_inputs_produce_byte_equal_output() {
        let p = SvgPrimitive::Circle { cx: 0.0, cy: 0.0, r: 5.0 };
        let a = tessellate(&p, &solid_fill(Color::BLACK), Color::BLACK, DEFAULT_TOLERANCE);
        let b = tessellate(&p, &solid_fill(Color::BLACK), Color::BLACK, DEFAULT_TOLERANCE);
        assert_eq!(a.vertices.len(), b.vertices.len());
        assert_eq!(a.indices.len(), b.indices.len());
        for (va, vb) in a.vertices.iter().zip(b.vertices.iter()) {
            assert_eq!(va.position, vb.position);
            assert_eq!(va.color, vb.color);
            assert_eq!(va.coverage, vb.coverage);
        }
        assert_eq!(a.indices, b.indices);
    }

    #[test]
    fn current_color_resolves_from_context() {
        let p = SvgPrimitive::Circle { cx: 0.0, cy: 0.0, r: 5.0 };
        let attrs = SvgAttrs { fill: Some(SvgPaint::Current), ..Default::default() };
        let red = Color::rgb(255, 0, 0);
        let g = tessellate(&p, &attrs, red, DEFAULT_TOLERANCE);
        assert!(g.vertex_count() > 0);
        // Every emitted vertex should carry the resolved red color.
        for v in &g.vertices {
            assert!((v.color[0] - 1.0).abs() < 1e-4);
            assert!(v.color[1] < 1e-4);
            assert!(v.color[2] < 1e-4);
        }
    }

    #[test]
    fn fill_and_stroke_together_stack_vertices() {
        let p = SvgPrimitive::Rect { x: 0.0, y: 0.0, width: 10.0, height: 10.0, rx: 0.0, ry: 0.0 };
        let mut attrs = solid_fill(Color::BLACK);
        attrs.stroke = Some(SvgPaint::Solid(Color::WHITE));
        attrs.stroke_width = Some(1.0);
        let g = tessellate(&p, &attrs, Color::BLACK, DEFAULT_TOLERANCE);
        let fill_only = tessellate(&p, &solid_fill(Color::BLACK), Color::BLACK, DEFAULT_TOLERANCE);
        assert!(g.vertex_count() > fill_only.vertex_count());
    }

    #[test]
    fn fill_vertices_have_zero_coverage() {
        let p = SvgPrimitive::Circle { cx: 10.0, cy: 10.0, r: 5.0 };
        let g = tessellate(&p, &solid_fill(Color::BLACK), Color::BLACK, DEFAULT_TOLERANCE);
        assert!(g.vertex_count() > 0);
        for v in &g.vertices {
            assert_eq!(
                v.coverage, 0.0,
                "fill vertices must have coverage 0.0 (uniform -> fwidth=0)"
            );
        }
    }

    #[test]
    fn stroke_vertices_have_signed_coverage() {
        let p = SvgPrimitive::Circle { cx: 10.0, cy: 10.0, r: 5.0 };
        let g = tessellate(&p, &solid_stroke(Color::BLACK, 2.0), Color::BLACK, DEFAULT_TOLERANCE);
        assert!(g.vertex_count() > 0);
        for v in &g.vertices {
            assert!(
                (-1.0..=1.0).contains(&v.coverage),
                "stroke coverage {} out of [-1, 1] range",
                v.coverage
            );
        }
        // Should have vertices on both sides of the stroke.
        let has_neg = g.vertices.iter().any(|v| v.coverage < -0.5);
        let has_pos = g.vertices.iter().any(|v| v.coverage > 0.5);
        assert!(has_neg, "stroke should have negative-side vertices");
        assert!(has_pos, "stroke should have positive-side vertices");
    }

    #[test]
    fn zero_width_stroke_coverage_is_safe() {
        let p = SvgPrimitive::Line { x1: 0.0, y1: 0.0, x2: 10.0, y2: 0.0 };
        let attrs = solid_stroke(Color::BLACK, 0.0);
        let g = tessellate(&p, &attrs, Color::BLACK, DEFAULT_TOLERANCE);
        // Zero width produces no geometry, but if it did, coverage must be in range.
        for v in &g.vertices {
            assert!(
                (-1.0..=1.0).contains(&v.coverage),
                "degenerate stroke coverage must be in [-1, 1]"
            );
        }
    }
}
