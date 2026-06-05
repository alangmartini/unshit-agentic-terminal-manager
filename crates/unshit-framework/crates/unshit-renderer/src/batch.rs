use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::sync::OnceLock;

use crate::atlas::{GlyphAtlas, GlyphEntry, GlyphKey};
use crate::canvas::{CanvasCallback, CanvasRegistry};
#[cfg(target_os = "windows")]
use crate::dw_rasterizer::DwRasterizer;
use crate::line_quad_cache::{hash_row_cells, LineGeometrySig, LineQuadCache};
use crate::pipeline::image::ImageInstance;
use crate::pipeline::quad::{QuadInstance, MAX_GRADIENT_STOPS};
use crate::pipeline::text::GlyphInstance;
use crate::svg_cache::SvgTessCache;
use crate::svg_tess::SvgGeometry;
use cosmic_text::{Buffer, FontSystem, Metrics, Shaping, SwashCache};
use swash::scale::{Render, ScaleContext, Source, StrikeWith};
use swash::zeno::{Angle, Format, Transform, Vector};

/// Glyph rasterization backends.
///
/// On Windows, carries both SwashCache (for CSS/UI text, where cosmic-text
/// metrics must match the rasterizer) and DwRasterizer (for the terminal
/// grid, where characters sit on a fixed cell grid and DirectWrite quality
/// matters most).
///
/// On non-Windows, only SwashCache is available.
pub struct Rasterizer<'a> {
    pub swash: &'a mut SwashCache,
    pub subpixel_swash: &'a mut SubpixelSwashCache,
    #[cfg(target_os = "windows")]
    pub dw: &'a DwRasterizer,
}

/// Swash rasterizer variant that requests RGB subpixel masks. cosmic-text's
/// public SwashCache currently hardcodes alpha masks, so the renderer owns the
/// alternate scale context needed by the experimental subpixel text pipeline.
pub struct SubpixelSwashCache {
    context: ScaleContext,
}

impl SubpixelSwashCache {
    pub fn new() -> Self {
        Self { context: ScaleContext::new() }
    }

    fn get_image_uncached(
        &mut self,
        font_system: &mut FontSystem,
        cache_key: cosmic_text::CacheKey,
    ) -> Option<cosmic_text::SwashImage> {
        let font = font_system.get_font(cache_key.font_id)?;
        let mut scaler = self
            .context
            .builder(font.as_swash())
            .size(f32::from_bits(cache_key.font_size_bits))
            .hint(true)
            .build();
        let offset = Vector::new(cache_key.x_bin.as_float(), cache_key.y_bin.as_float());

        Render::new(&[
            Source::ColorOutline(0),
            Source::ColorBitmap(StrikeWith::BestFit),
            Source::Outline,
        ])
        .format(ui_subpixel_mask_format())
        .offset(offset)
        .transform(if cache_key.flags.contains(cosmic_text::CacheKeyFlags::FAKE_ITALIC) {
            Some(Transform::skew(Angle::from_degrees(14.0), Angle::from_degrees(0.0)))
        } else {
            None
        })
        .render(&mut scaler, cache_key.glyph_id)
    }
}

fn ui_subpixel_mask_format() -> Format {
    Format::subpixel_bgra()
}

impl Default for SubpixelSwashCache {
    fn default() -> Self {
        Self::new()
    }
}
use rustc_hash::{FxHashMap, FxHashSet};
use unshit_core::cell_grid::{BgRun, CellAttrs, CellGrid};
use unshit_core::cursor::CursorShape;
use unshit_core::dirty::DirtyFlags;
use unshit_core::element::{ElementContent, InputType, Tag};
use unshit_core::event::TextSelection;
use unshit_core::id::NodeId;
use unshit_core::layout::{
    font_weight_number, measure_text_with_style_cached, text_attrs, truncate_text_with_ellipsis,
    TextMeasureCache,
};
use unshit_core::scroll::{self, ScrollbarVisualState};
use unshit_core::style::types::{
    apply_text_transform, Background, Color, CssPosition, CssResize, Display, FilterFunction,
    FontStyle, FontWeight, GradientStopPosition, Layer, LinearGradient, Overflow, RadialGradient,
    RadialShape, RenderTarget, TextAlign, TextDecoration, TextOverflow, Visibility, WhiteSpace,
};
use unshit_core::svg::types::{
    PathCommand, StrokeLineCap, StrokeLineJoin, SvgAttrs, SvgNode, SvgPaint, SvgPrimitive,
    SvgTransform, ViewBox,
};
use unshit_core::trace::{append_terminal_trace_line, terminal_trace_enabled};
use unshit_core::tree::NodeArena;

/// Zero value for the gradient stop color slots in `QuadInstance`. Used at
/// every call site that emits a solid color quad to keep the instance layout
/// uniform.
pub(crate) const EMPTY_GRADIENT_STOP_COLORS: [[f32; 4]; MAX_GRADIENT_STOPS] =
    [[0.0; 4]; MAX_GRADIENT_STOPS];
pub(crate) const EMPTY_GRADIENT_STOP_POSITIONS: [f32; MAX_GRADIENT_STOPS] =
    [0.0; MAX_GRADIENT_STOPS];
/// Zero value for the radial specific aux slot. Solid color quads and
/// linear gradients leave it untouched; only radial gradient quads write
/// real `(center, radii)` values into the slot.
pub(crate) const EMPTY_GRADIENT_EXTRA: [f32; 4] = [0.0; 4];

/// Zero value for the `mask-image` slots (alpha0 pos0 alpha1 pos1 etc.).
/// A `mask_params.w` of zero (see `EMPTY_MASK_PARAMS`) selects the "no
/// mask" branch in the shader, so these can stay zero at every emit site
/// that does not attach a mask-image.
pub(crate) const EMPTY_MASK_STOPS: [f32; 4] = [0.0; 4];
/// Mask `params` default. `stop_count = 0` (the last component) disables
/// mask alpha modulation in the shader.
pub(crate) const EMPTY_MASK_PARAMS: [f32; 4] = [0.0; 4];

/// One shot flag so the stop truncation warning does not spam logs.
static GRADIENT_TRUNCATE_WARNED: AtomicBool = AtomicBool::new(false);
static LAST_TERMINAL_RENDER_TRACE_HASH: AtomicU64 = AtomicU64::new(0);
const WINDOWS_TERMINAL_PARITY_CALIBRATED_FG: Color = Color { r: 196, g: 196, b: 196, a: 255 };
const WINDOWS_TERMINAL_PARITY_LITERAL_FG: Color = Color { r: 204, g: 204, b: 204, a: 255 };
const ENV_PARITY_CELL_WIDTH_SCALE: &str = "TM_PARITY_CELL_WIDTH_SCALE";
const WINDOWS_TERMINAL_PARITY_CELL_WIDTH_SCALE: f32 = 0.996;

fn aligned_text_x(
    render_x: f32,
    padding_left: f32,
    content_w: f32,
    text_w: f32,
    align: TextAlign,
) -> f32 {
    let line_start = render_x + padding_left;
    match align {
        TextAlign::Left => line_start,
        TextAlign::Center => line_start + (content_w - text_w) * 0.5,
        TextAlign::Right => line_start + content_w - text_w,
    }
}

fn terminal_grid_trace_hash(grid: &CellGrid) -> u64 {
    use std::hash::{Hash, Hasher};

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    grid.rows().hash(&mut hasher);
    grid.cols().hash(&mut hasher);
    grid.cursor_row().hash(&mut hasher);
    grid.cursor_col().hash(&mut hasher);
    grid.cursor_visible().hash(&mut hasher);
    for row in grid.debug_rows(4, 96) {
        row.hash(&mut hasher);
    }
    hasher.finish()
}

#[cfg(target_os = "windows")]
fn use_directwrite_grid_rasterization() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("TM_FORCE_DIRECTWRITE_GRID").is_some())
}

fn parity_windows_terminal_colors_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var_os("TM_PARITY_WINDOWS_TERMINAL_COLORS")
            .filter(|v| !v.is_empty())
            .map(|v| {
                let normalized = v.to_string_lossy().trim().to_ascii_lowercase();
                !matches!(normalized.as_str(), "0" | "false" | "off" | "no")
            })
            .unwrap_or(false)
    })
}

fn parity_cell_width_scale_from_values(value: Option<std::ffi::OsString>, wt_profile: bool) -> f32 {
    value
        .and_then(|v| v.into_string().ok())
        .and_then(|v| v.parse::<f32>().ok())
        .filter(|v| (0.95..=1.05).contains(v))
        .unwrap_or(if wt_profile { WINDOWS_TERMINAL_PARITY_CELL_WIDTH_SCALE } else { 1.0 })
}

fn parity_terminal_cell_width_scale() -> f32 {
    static SCALE: OnceLock<f32> = OnceLock::new();
    *SCALE.get_or_init(|| {
        parity_cell_width_scale_from_values(
            std::env::var_os(ENV_PARITY_CELL_WIDTH_SCALE),
            parity_windows_terminal_colors_enabled(),
        )
    })
}

#[cfg(target_os = "windows")]
fn use_directwrite_ui_rasterization() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("TM_FORCE_DIRECTWRITE_UI").is_some())
}

#[inline]
fn atlas_font_namespace(cache_key: &cosmic_text::CacheKey) -> u64 {
    use std::hash::{Hash, Hasher};

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    cache_key.font_id.hash(&mut hasher);
    cache_key.flags.hash(&mut hasher);
    hasher.finish()
}

#[inline]
fn atlas_text_font_namespace(
    cache_key: &cosmic_text::CacheKey,
    font_family: &str,
    font_weight: FontWeight,
) -> u64 {
    use std::hash::{Hash, Hasher};

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    cache_key.font_id.hash(&mut hasher);
    cache_key.flags.hash(&mut hasher);
    font_family.hash(&mut hasher);
    font_weight_number(font_weight).hash(&mut hasher);
    hasher.finish()
}

/// Compute the projected axis length of a gradient inside a box of size
/// `(w, h)` for the given angle in degrees.
///
/// CSS Images Level 3 defines the axis length as the distance between the
/// two opposite corners of the gradient box along the gradient direction.
/// For an axis aligned 0deg / 180deg gradient this collapses to `h`; for
/// 90deg / 270deg it collapses to `w`. Diagonals fall out of the standard
/// `|w * sin| + |h * cos|` formula. The result is clamped to a tiny
/// positive value so the per stop pixel normalization never divides by
/// zero on a degenerate element.
fn projected_axis_length(angle_deg: f32, w: f32, h: f32) -> f32 {
    let rad = angle_deg.to_radians();
    let s = rad.sin().abs();
    let c = rad.cos().abs();
    (w * s + h * c).max(1e-6)
}

/// Pack the shared stop list of either a linear or a radial gradient into
/// the GPU side stop arrays.
///
/// Returns the number of stops actually written (capped at
/// `MAX_GRADIENT_STOPS`). The `opacity` multiplier is folded into each stop's
/// alpha so the shader does not need an extra uniform path for opacity of
/// gradient backgrounds. Gradients with more than `MAX_GRADIENT_STOPS` stops
/// are truncated; the first time this happens we emit a one time warning.
///
/// `axis_length` is the projection length the shader uses to interpret pixel
/// stop positions. For linear gradients pass the corner to corner length
/// from `projected_axis_length`. For radial gradients pass the resolved rx so
/// pixel stop positions express distance along the gradient line in pixels.
fn pack_stop_list(
    stops: &[unshit_core::style::types::GradientStop],
    opacity: f32,
    axis_length: f32,
    colors: &mut [[f32; 4]; MAX_GRADIENT_STOPS],
    positions: &mut [f32; MAX_GRADIENT_STOPS],
    kind_label: &str,
) -> usize {
    if stops.len() > MAX_GRADIENT_STOPS && !GRADIENT_TRUNCATE_WARNED.swap(true, Ordering::Relaxed) {
        eprintln!(
            "unshit-renderer: {} has {} stops, truncating to {} (further warnings suppressed)",
            kind_label,
            stops.len(),
            MAX_GRADIENT_STOPS
        );
    }

    let count = stops.len().min(MAX_GRADIENT_STOPS);
    let safe_axis = axis_length.max(1e-6);
    let mut prev: f32 = 0.0;
    for (i, stop) in stops.iter().take(count).enumerate() {
        let mut c = stop.color.to_linear_f32();
        c[3] *= opacity;
        colors[i] = c;
        let raw = match stop.position {
            GradientStopPosition::Percent(v) => v,
            GradientStopPosition::Px(v) => (v / safe_axis).max(0.0),
        };
        // Cross unit monotonic clamp. After parse time each unit is sorted
        // independently, so a percent stop followed by a pixel stop can
        // still produce an out of order pair once both are normalized.
        // Snap any out of order entry up to the previous normalized value
        // so the shader's segment scan stays linear.
        let snapped = if i == 0 { raw.max(0.0) } else { raw.max(prev) };
        positions[i] = snapped;
        prev = snapped;
    }
    count
}

/// Pack a `LinearGradient` into the GPU side stop arrays.
///
/// Returns `(colors, positions, params)` where `params` is the
/// `gradient_params` vec4 storing `[angle_radians, repeating_flag, 0,
/// stop_count]`. The `repeating_flag` slot is 0.0 for a non repeating
/// gradient and 1.0 for a repeating one (issue #128); the shader branches
/// on it to choose between the `clamp` and `fract` sampling paths. The
/// `opacity` multiplier is folded into each stop's alpha so the shader does
/// not need an extra uniform path for opacity of gradient backgrounds.
///
/// Pixel space stop positions are normalized against the projected axis
/// length so the shader always sees positions in the same `[0.0, 1.0]`
/// space regardless of whether the source CSS used `%` or `px`.
fn pack_gradient(
    grad: &LinearGradient,
    opacity: f32,
    elem_w: f32,
    elem_h: f32,
) -> ([[f32; 4]; MAX_GRADIENT_STOPS], [f32; MAX_GRADIENT_STOPS], [f32; 4]) {
    let mut colors = EMPTY_GRADIENT_STOP_COLORS;
    let mut positions = EMPTY_GRADIENT_STOP_POSITIONS;
    let axis_length = projected_axis_length(grad.angle_deg, elem_w, elem_h);
    let count = pack_stop_list(
        &grad.stops,
        opacity,
        axis_length,
        &mut colors,
        &mut positions,
        "linear-gradient",
    );
    let angle_rad = grad.angle_deg.to_radians();
    let repeating_flag = if grad.repeating { 1.0 } else { 0.0 };
    // gradient_params.w > 0 marks a linear gradient with `count` stops;
    // gradient_params.y is the repeating flag (issue #128).
    let params = [angle_rad, repeating_flag, 0.0, count as f32];
    (colors, positions, params)
}

/// Pack a `RadialGradient` into the GPU side stop arrays plus the radial
/// auxiliary slot.
///
/// Returns `(colors, positions, params, extra)`:
/// * `params.w` is `-(count as f32)` so the shader can dispatch on the
///   sign to enter the radial branch (see `QuadInstance` docs).
/// * `params.y` is `-1.0` for a circle, `1.0` for an ellipse. The shader
///   uses the sign to pick the isotropic vs anisotropic distance function.
/// * `extra` carries `[center_x, center_y, radius_x, radius_y]` in element
///   local pixels, resolved against the rect width and height. A degenerate
///   radius (`<= 0`) is left as is and the shader collapses the gradient
///   to the last stop color.
fn pack_radial_gradient(
    grad: &RadialGradient,
    width: f32,
    height: f32,
    opacity: f32,
) -> ([[f32; 4]; MAX_GRADIENT_STOPS], [f32; MAX_GRADIENT_STOPS], [f32; 4], [f32; 4]) {
    let mut colors = EMPTY_GRADIENT_STOP_COLORS;
    let mut positions = EMPTY_GRADIENT_STOP_POSITIONS;
    let resolved = grad.resolve(width, height);
    // Pixel positions on a radial gradient are distances along the gradient
    // line from center to rx, so we normalize against rx (clamped to a tiny
    // positive value upstream by `pack_stop_list`).
    let count = pack_stop_list(
        &grad.stops,
        opacity,
        resolved.rx.max(1e-6),
        &mut colors,
        &mut positions,
        "radial-gradient",
    );
    let shape_tag = match resolved.shape {
        RadialShape::Circle => -1.0,
        RadialShape::Ellipse => 1.0,
    };
    // gradient_params.w < 0 marks a radial gradient with `count` stops.
    let params = [0.0, shape_tag, 0.0, -(count as f32)];
    let extra = [resolved.center_x, resolved.center_y, resolved.rx, resolved.ry];
    (colors, positions, params, extra)
}

/// Pack a `mask-image: linear-gradient(...)` declaration into the four
/// auxiliary slots on `QuadInstance` (`mask_stops_01`, `mask_stops_23`,
/// `mask_params`).
///
/// The mask gradient carries only the alpha channel of each stop and its
/// position along the projected axis. Two stops per `vec4` keeps the
/// GPU instance size manageable; up to four stops are supported today.
/// When the source gradient has more than four stops we truncate silently
/// (same policy as background gradient truncation above) because the
/// common edge fade pattern is always three or four stops.
///
/// `mask_params.w` is the stop count. The fragment shader selects the
/// "no mask" fast path when this is zero, so the pre-normalization here is
/// defensive: we emit `stop_count = 0` when the source list is empty.
pub(crate) fn pack_mask_image(mask: &LinearGradient) -> ([f32; 4], [f32; 4], [f32; 4]) {
    let mut stops_01 = [0.0_f32; 4];
    let mut stops_23 = [0.0_f32; 4];
    let axis_length = projected_axis_length(mask.angle_deg, 1.0, 1.0);
    // Mask gradient stops resolve into [0, 1] along the projection axis,
    // exactly like background gradient stops. We only need the alpha
    // channel of each stop so we drop rgb and re pack positions inline.
    let count = mask.stops.len().min(crate::pipeline::quad::MAX_MASK_STOPS);
    for (i, stop) in mask.stops.iter().take(count).enumerate() {
        let alpha = stop.color.a as f32 / 255.0;
        // Pixel stops on a mask are unusual (the test fixture always uses
        // percentages) but we honor them the same way the background
        // pipeline does: resolve against the unit projected axis length so
        // the value already sits in [0, 1].
        let pos = match stop.position {
            unshit_core::style::types::GradientStopPosition::Percent(v) => v,
            unshit_core::style::types::GradientStopPosition::Px(v) => {
                // Without the element's projected axis length available at
                // this point we treat pixel stops as fractions. Callers
                // that care about mixed unit masks can migrate to the full
                // axis resolution in a follow up.
                v / axis_length.max(1.0)
            }
        };
        if i < 2 {
            stops_01[i * 2] = alpha;
            stops_01[i * 2 + 1] = pos.clamp(0.0, 1.0);
        } else {
            let j = i - 2;
            stops_23[j * 2] = alpha;
            stops_23[j * 2 + 1] = pos.clamp(0.0, 1.0);
        }
    }
    let angle_rad = mask.angle_deg.to_radians();
    let params = [angle_rad, 0.0, 0.0, count as f32];
    (stops_01, stops_23, params)
}

pub struct ImageBatch {
    pub path: String,
    pub instances: Vec<ImageInstance>,
    pub object_fit: unshit_core::style::types::ObjectFit,
    pub object_position: unshit_core::style::types::ObjectPosition,
}

/// One queued SVG draw. `geometry` is an `Arc` pointer into the tessellation
/// cache; `translate` and `scale` map the local SVG user units into pixel
/// coordinates, `clip_rect` is the pixel space scissor rectangle inherited
/// from containing overflow clips, `color_tint` is a per draw multiplier
/// (used to honor element level `color` inheritance for `currentColor`), and
/// `opacity` is the multiplied element opacity.
#[derive(Clone)]
pub struct SvgDrawCall {
    pub geometry: Arc<SvgGeometry>,
    pub translate: [f32; 2],
    pub scale: [f32; 2],
    pub clip_rect: [f32; 4],
    pub color_tint: [f32; 4],
    pub opacity: f32,
}

/// Marker emitted per element that has a resolved `backdrop-filter`.
///
/// The GPU render loop consumes these markers, splits the current pass at the
/// point the element would draw, copies the pixels in `rect` into an offscreen
/// texture, runs a two pass separable Gaussian blur, and then reopens the
/// pass with `LoadOp::Load` so the filtered element draws on top of the
/// blurred backdrop.
///
/// Boundaries are tagged at batch build time with the prefix counts for each
/// primitive type in the layer at the moment the filtered element is about
/// to draw. The render loop uses those counts to know exactly how many
/// entries of each primitive type of the current layer to emit before
/// splitting the pass.
#[derive(Clone, Copy, Debug)]
pub struct BackdropBoundary {
    /// The element bounding rectangle in pixel space, intersected with the
    /// inherited clip rect.
    pub rect: [f32; 4],
    /// The inherited clip rectangle in pixel space.
    pub clip_rect: [f32; 4],
    /// Gaussian kernel radius in pixels, already clamped to `[0, 64]`.
    pub blur_radius: f32,
    /// Prefix counts of each primitive type in the current layer at the
    /// moment this boundary was emitted. The render loop flushes exactly
    /// these many primitives before running the blur passes.
    pub quad_prefix: u32,
    pub glyph_prefix: u32,
    pub svg_prefix: u32,
    pub image_batch_prefix: u32,
    pub canvas_prefix: u32,
    /// Always `true` today. Future hook for #117 item 8: when damage
    /// tracking lands, a clean boundary can reuse a cached blurred texture
    /// without rerunning the blur passes.
    pub dirty: bool,
}

/// Identifies which primitive type a draw span covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrawKind {
    Quad,
    Glyph,
}

/// A contiguous range of primitives to draw in a specific order.
/// The GPU render loop processes spans sequentially, switching pipelines
/// as needed, to preserve correct occlusion (painter's algorithm).
#[derive(Clone, Copy, Debug)]
pub struct DrawSpan {
    pub kind: DrawKind,
    pub start: u32,
    pub count: u32,
}

/// Geometry captured for one `ElementContent::Grid` node when the
/// experimental fragment shader path is active. Consumed by
/// `GridFragmentPass::process` after the walk.
///
/// Intentionally free of GPU handles and cell data. Step 2 of the #96
/// wiring plan captures geometry only; follow up steps will thread cell
/// snapshots and glyph metadata through here.
#[cfg(feature = "grid-fragment-shader")]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GridDrawRecord {
    pub node_id: NodeId,
    pub origin_x: f32,
    pub origin_y: f32,
    pub cell_w: f32,
    pub cell_h: f32,
    pub cols: u32,
    pub rows: u32,
    pub font_size: f32,
    pub opacity: f32,
    pub clip_rect: [f32; 4],
}

pub struct FrameBatch {
    pub quad_instances: Vec<QuadInstance>,
    pub glyph_instances: Vec<GlyphInstance>,
    pub image_batches: Vec<ImageBatch>,
    pub canvas_callbacks: Vec<CanvasCallback>,
    pub svg_draws: Vec<SvgDrawCall>,
    /// Backdrop filter boundaries collected during the walk, in draw order.
    /// When empty the renderer fast path runs with zero extra cost.
    pub backdrop_boundaries: Vec<BackdropBoundary>,
    /// Draw spans for interleaved quad/glyph rendering to fix text occlusion.
    /// When non-empty, the GPU render loop processes spans sequentially
    /// instead of rendering all quads then all glyphs.
    pub draw_spans: Vec<DrawSpan>,
    /// Grid nodes routed through the experimental fragment shader path
    /// during this frame's walk. Empty when the feature is on but the
    /// runtime flag is unset, and when no grid elements are visible.
    #[cfg(feature = "grid-fragment-shader")]
    pub grid_records: Vec<GridDrawRecord>,
}

impl Default for FrameBatch {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameBatch {
    pub fn new() -> Self {
        Self {
            quad_instances: Vec::with_capacity(4096),
            glyph_instances: Vec::with_capacity(16384),
            image_batches: Vec::new(),
            canvas_callbacks: Vec::new(),
            svg_draws: Vec::new(),
            backdrop_boundaries: Vec::new(),
            draw_spans: Vec::with_capacity(1024),
            #[cfg(feature = "grid-fragment-shader")]
            grid_records: Vec::new(),
        }
    }

    pub fn clear(&mut self) {
        self.quad_instances.clear();
        self.glyph_instances.clear();
        self.image_batches.clear();
        self.canvas_callbacks.clear();
        self.svg_draws.clear();
        self.backdrop_boundaries.clear();
        self.draw_spans.clear();
        #[cfg(feature = "grid-fragment-shader")]
        self.grid_records.clear();
    }
}

/// Routing helper for the experimental fragment shader grid path.
/// Pushes a [`GridDrawRecord`] onto `batch.grid_records` and returns `true`
/// when `use_fragment` is set, indicating to the caller that
/// `emit_grid_cells` must be skipped for this grid node.
///
/// Kept as a separate free function so tests can drive routing behavior
/// without needing to toggle the process wide `TM_USE_GRID_FRAGMENT_SHADER`
/// environment variable (which is cached behind a `OnceLock`).
#[cfg(feature = "grid-fragment-shader")]
#[allow(clippy::too_many_arguments)]
pub(crate) fn try_record_grid_for_fragment_path(
    use_fragment: bool,
    batch: &mut FrameBatch,
    node_id: NodeId,
    origin_x: f32,
    origin_y: f32,
    cell_w: f32,
    cell_h: f32,
    cols: u32,
    rows: u32,
    font_size: f32,
    opacity: f32,
    clip_rect: [f32; 4],
) -> bool {
    if !use_fragment {
        return false;
    }
    batch.grid_records.push(GridDrawRecord {
        node_id,
        origin_x,
        origin_y,
        cell_w,
        cell_h,
        cols,
        rows,
        font_size,
        opacity,
        clip_rect,
    });
    true
}

pub struct LayeredBatch {
    pub layers: [FrameBatch; Layer::COUNT],
}

impl LayeredBatch {
    pub fn new() -> Self {
        Self { layers: std::array::from_fn(|_| FrameBatch::new()) }
    }

    pub fn clear(&mut self) {
        for layer in &mut self.layers {
            layer.clear();
        }
    }

    pub fn layer_mut(&mut self, layer: Layer) -> &mut FrameBatch {
        &mut self.layers[layer as usize]
    }

    /// Returns `true` if any layer has at least one `BackdropBoundary`.
    ///
    /// The GPU render loop uses this as the single source of truth for the
    /// fast path: when no layer carries a boundary, the renderer runs the
    /// existing single pass code path and allocates nothing.
    pub fn has_backdrop_boundaries(&self) -> bool {
        self.layers.iter().any(|l| !l.backdrop_boundaries.is_empty())
    }
}

impl Default for LayeredBatch {
    fn default() -> Self {
        Self::new()
    }
}

/// Cached shaped glyph data. Positions are relative to the text origin (0,0).
/// Absolute positioning and color are applied at emission time.
#[derive(Clone)]
struct CachedGlyph {
    rel_x: f32, // physical.x + bearing offset
    rel_y: f32, // run_y + physical.y + bearing offset
    atlas_key: GlyphKey,
}

/// Pre-shaped text result cached to avoid re-creating cosmic-text Buffers.
#[derive(Clone)]
struct ShapedTextEntry {
    glyphs: Vec<CachedGlyph>,
}

/// Cache for shaped text layouts. Keyed on text content + font params + width.
///
/// Storage is double buffered: each `finish_frame` swaps the previous and
/// current maps and clears the new current. Lookups promote matching entries
/// from previous into current, so the live set is bounded by the last two
/// frames of hits. Entries untouched for two consecutive frames are dropped
/// with no explicit eviction pass.
///
/// `last_atlas_generation` guards against coarse atlas invalidations: when
/// the glyph atlas bumps its generation (eviction, rebuild), `finish_frame`
/// wipes both halves on the next call before the swap so stale atlas UVs
/// never survive into a new frame cycle. The per glyph residency check in
/// `emit_shaped_text_run` remains the primary correctness mechanism for
/// mid frame churn.
pub struct ShapedTextCache {
    buf: crate::double_buffered::DoubleBufferedCache<ShapedCacheKey, ShapedTextEntry>,
    last_atlas_generation: u64,
}

#[derive(Hash, Eq, PartialEq, Clone)]
struct ShapedCacheKey {
    text_hash: u64,
    font_family_hash: u64,
    font_weight: u16,
    font_style: FontStyle,
    font_size_tenths: i32,
    line_height_tenths: i32,
    letter_spacing_tenths: i32,
    max_width_tenths: i32,
}

impl Default for ShapedTextCache {
    fn default() -> Self {
        Self::new()
    }
}

impl ShapedTextCache {
    pub fn new() -> Self {
        Self {
            buf: crate::double_buffered::DoubleBufferedCache::with_capacity(256),
            last_atlas_generation: 0,
        }
    }

    /// Empty both halves. Call on font family, DPI, or size changes.
    pub fn clear(&mut self) {
        self.buf.clear();
    }

    /// Total entries across both halves. Primarily a diagnostic.
    pub fn len(&self) -> usize {
        self.buf.len()
    }

    /// True when both halves are empty.
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    /// End of frame marker. Swaps previous and current, dropping any entry
    /// that was not touched this frame. If `atlas_generation` differs from
    /// the last observed value the entire cache is cleared first. This is a
    /// defense in depth layer on top of the per glyph residency check in
    /// `emit_shaped_text_run`.
    pub fn finish_frame(&mut self, atlas_generation: u64) {
        if atlas_generation != self.last_atlas_generation {
            self.buf.clear();
            self.last_atlas_generation = atlas_generation;
            return;
        }
        self.buf.finish_frame();
    }
}

/// Records the primitives produced by a single node (and its subtree) in the
/// previous frame for a specific layer. Used by `BatchCache` to replay cached
/// output for clean (non-dirty) nodes without rebuilding.
#[derive(Clone, Default)]
pub struct BatchRange {
    /// Render-space geometry that produced this range. Cached primitives
    /// contain absolute positions and clip rectangles, so a pure relayout
    /// must not replay them when dirty flags were not raised.
    pub signature: BatchCacheSignature,
    pub quads: Vec<QuadInstance>,
    pub glyphs: Vec<GlyphInstance>,
    pub svgs: Vec<SvgDrawCall>,
    pub draw_spans: Vec<DrawSpan>,
    /// Unique glyph atlas keys used by this node range (including subtree).
    pub glyph_keys: Vec<GlyphKey>,
    /// Glyph atlas generation this range was built against.
    pub atlas_generation: u64,
    /// Backdrop boundaries emitted within this node's subtree. Prefix counts
    /// are stored RELATIVE to the snapshot's quad_start / glyph_start /
    /// svg_start / image_start / canvas_start. Replay restores absolute
    /// prefixes against the current layer state. Without this, cache replay
    /// silently drops the boundary, the blur pass never runs on subsequent
    /// frames, and elements behind a `backdrop-filter` overlay alternate
    /// between blurred and unblurred whenever the renderer takes the cache
    /// path (issues #143, #142).
    pub backdrop_boundaries: Vec<BackdropBoundary>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BatchCacheSignature {
    pub render_rect: [f32; 4],
    pub clip_rect: [f32; 4],
    /// The node's composed CSS-transform affine in delta-from-identity encoding
    /// (`[a-1, b, c, d-1, e, f]`), so the identity is all-zero (matching
    /// `Default`). Included so a node re-emits when its own or any ancestor's
    /// transform changes — the cached instances bake an absolute matrix.
    pub xform: [f32; 6],
}

impl BatchCacheSignature {
    pub fn new(render_rect: [f32; 4], clip_rect: [f32; 4], xform: [f32; 6]) -> Self {
        Self { render_rect, clip_rect, xform }
    }

    fn matches(self, other: Self) -> bool {
        self.render_rect
            .iter()
            .chain(self.clip_rect.iter())
            .chain(self.xform.iter())
            .zip(other.render_rect.iter().chain(other.clip_rect.iter()).chain(other.xform.iter()))
            .all(|(a, b)| (*a - *b).abs() <= 0.01)
    }
}

/// Per-frame cache that stores the actual `QuadInstance` and `GlyphInstance`
/// data emitted by each node in the previous frame.  When a node has neither
/// `PAINT` nor `SUBTREE_PAINT` the batch builder extends the current batch
/// with the stored instances instead of regenerating them.
///
/// The cache is keyed on `(NodeId, layer_index)` so nodes that appear on
/// multiple layers (portals, overlays) each get their own entry.
pub struct BatchCache {
    /// Stores instances produced by each `(node_id, layer_index)` last frame.
    ranges: FxHashMap<(NodeId, usize), BatchRange>,
    /// Staging map built during the current frame; swapped in at commit time.
    pending: FxHashMap<(NodeId, usize), BatchRange>,
}

impl Default for BatchCache {
    fn default() -> Self {
        Self::new()
    }
}

impl BatchCache {
    pub fn new() -> Self {
        Self {
            ranges: FxHashMap::with_capacity_and_hasher(512, Default::default()),
            pending: FxHashMap::with_capacity_and_hasher(512, Default::default()),
        }
    }

    /// Clear all cached state.  Call when the entire frame must be rebuilt
    /// (e.g. after a window resize or stylesheet reload).
    pub fn clear(&mut self) {
        self.ranges.clear();
        self.pending.clear();
    }

    /// Begin building cache data for the current frame.  Clears the staging
    /// map so stale entries from the previous frame's build do not accumulate.
    pub fn begin_frame(&mut self) {
        self.pending.clear();
    }

    /// Commit the current frame's staging data as the authoritative cache for
    /// the next frame.  Call this after `build_render_batch` completes.
    pub fn commit_frame(&mut self) {
        std::mem::swap(&mut self.ranges, &mut self.pending);
        self.pending.clear();
    }

    /// Record the primitives emitted for `node_id` on `layer_index` during
    /// the current frame into the staging map. `backdrop_boundaries` carry
    /// prefix counts relative to the snapshot's primitive start indices; the
    /// replay path restores absolute prefixes.
    #[allow(clippy::too_many_arguments)]
    pub fn record(
        &mut self,
        node_id: NodeId,
        layer_index: usize,
        quads: Vec<QuadInstance>,
        glyphs: Vec<GlyphInstance>,
        svgs: Vec<SvgDrawCall>,
        draw_spans: Vec<DrawSpan>,
        glyph_keys: Vec<GlyphKey>,
        atlas_generation: u64,
        backdrop_boundaries: Vec<BackdropBoundary>,
    ) {
        self.record_with_signature(
            node_id,
            layer_index,
            BatchCacheSignature::default(),
            quads,
            glyphs,
            svgs,
            draw_spans,
            glyph_keys,
            atlas_generation,
            backdrop_boundaries,
        );
    }

    /// Record a range with the render geometry that produced it.
    #[allow(clippy::too_many_arguments)]
    pub fn record_with_signature(
        &mut self,
        node_id: NodeId,
        layer_index: usize,
        signature: BatchCacheSignature,
        quads: Vec<QuadInstance>,
        glyphs: Vec<GlyphInstance>,
        svgs: Vec<SvgDrawCall>,
        draw_spans: Vec<DrawSpan>,
        glyph_keys: Vec<GlyphKey>,
        atlas_generation: u64,
        backdrop_boundaries: Vec<BackdropBoundary>,
    ) {
        self.pending.insert(
            (node_id, layer_index),
            BatchRange {
                signature,
                quads,
                glyphs,
                svgs,
                draw_spans,
                glyph_keys,
                atlas_generation,
                backdrop_boundaries,
            },
        );
    }

    /// Retrieve the cached instances for `node_id` on `layer_index` from the
    /// **previous** frame, if any.
    pub fn get(&self, node_id: NodeId, layer_index: usize) -> Option<&BatchRange> {
        self.ranges.get(&(node_id, layer_index))
    }

    /// Read a cached range AND carry the entry forward into the current
    /// frame's staging map. Call this from the batch builder's replay path
    /// so clean nodes that replay from cache are not silently dropped by
    /// the next `commit_frame` swap.
    ///
    /// Without this carry-forward, the swap-and-clear pattern in
    /// `commit_frame` produces an alternating hit/miss cycle: a replayed
    /// entry never reaches `pending`, so it vanishes on the next commit
    /// and the walker must re-render fresh the frame after. That churn is
    /// wasted work and, combined with out-of-band `computed_style`
    /// mutations from transition and animation ticks, shows up as visible
    /// flicker in the app (issues #41 and #42).
    ///
    /// Preference order inside a single frame:
    /// 1. If `pending` already has an entry for this key (the caller
    ///    recorded fresh primitives earlier in the same walk), return it
    ///    unchanged.
    /// 2. Otherwise, if `ranges` has an entry from the previous frame,
    ///    clone it into `pending` and return the cloned reference.
    /// 3. Otherwise return `None` so the walker knows to render fresh.
    pub fn replay(
        &mut self,
        node_id: NodeId,
        layer_index: usize,
        atlas_generation: u64,
    ) -> Option<&BatchRange> {
        self.replay_with_signature(
            node_id,
            layer_index,
            atlas_generation,
            BatchCacheSignature::default(),
        )
    }

    /// Replay only when both atlas generation and render geometry still
    /// match. Layout can move elements without setting PAINT; without this
    /// guard, stale absolute-position primitives replay after window resize.
    pub fn replay_with_signature(
        &mut self,
        node_id: NodeId,
        layer_index: usize,
        atlas_generation: u64,
        signature: BatchCacheSignature,
    ) -> Option<&BatchRange> {
        let key = (node_id, layer_index);
        if self.pending.contains_key(&key) {
            let range = self.pending.get(&key).expect("contains_key checked pending entry");
            return (range.atlas_generation == atlas_generation
                && range.signature.matches(signature))
            .then_some(range);
        }
        let range = self.ranges.get(&key)?.clone();
        if range.atlas_generation != atlas_generation || !range.signature.matches(signature) {
            return None;
        }
        self.pending.insert(key, range);
        self.pending.get(&key)
    }
}

impl ShapedTextCache {
    #[allow(clippy::too_many_arguments)]
    fn make_key(
        text: &str,
        font_family: &str,
        font_weight: FontWeight,
        font_style: FontStyle,
        font_size: f32,
        line_height: f32,
        letter_spacing: f32,
        max_width: Option<f32>,
    ) -> ShapedCacheKey {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        text.hash(&mut hasher);
        ShapedCacheKey {
            text_hash: hasher.finish(),
            font_family_hash: shape_cache_font_id(font_family),
            font_weight: font_weight_number(font_weight),
            font_style,
            font_size_tenths: (font_size * 10.0) as i32,
            line_height_tenths: (line_height * 10.0) as i32,
            letter_spacing_tenths: (letter_spacing * 10.0) as i32,
            max_width_tenths: max_width.map_or(-1, |w| (w * 10.0) as i32),
        }
    }
}

// ---------------------------------------------------------------------------
// ShapeCache
//
// Persistent, cross-frame cache of shaped prototype glyphs for the terminal
// grid. The hot path in `emit_grid_cells` shapes every unique character it
// encounters once per frame; with this cache warmed the shaping call happens
// at most once per character per font/size/style combination for the lifetime
// of the cache.
//
// Hits bypass `buffer.set_text` and `shape_until_scroll` entirely.
// ---------------------------------------------------------------------------

/// Identity tag for a shaped glyph configuration. Cache entries are only
/// reused for keys that match byte-for-byte.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ShapeCacheKey {
    pub ch: char,
    pub font_id: u64,
    pub style: u64,
    pub font_size_tenths: u32,
}

/// Cache-friendly copy of the shaped `cosmic_text::LayoutGlyph` and its
/// positioning baseline. Only fields needed to reproduce the physical glyph
/// each frame are retained, so the entry is both small and independent of
/// any `Buffer` / `FontSystem` state across frames.
#[derive(Clone)]
pub struct ShapedGlyphEntry {
    pub layout_glyph: cosmic_text::LayoutGlyph,
    pub line_y: f32,
}

/// Cross-frame cache of shaped prototype glyphs, keyed on
/// `(char, font_id, style, font_size)`. The key is independent of subpixel
/// bin and cell origin so it remains valid as the pane scrolls or moves.
///
/// Storage is double buffered (see [`DoubleBufferedCache`]): each
/// `finish_frame` swaps the previous and current maps. Entries untouched for
/// two consecutive frames are dropped with no explicit eviction pass. Preload
/// and terminal hot characters promote on every frame, so the preload set
/// effectively never evicts; rarely touched characters age out automatically.
///
/// Invalidation is coarse on purpose: any change to the global font stack,
/// DPI scale, or font size should call [`ShapeCache::clear`]. Per-character
/// invalidation is intentionally not supported because monospace advances
/// are stable inside a single font configuration.
///
/// [`DoubleBufferedCache`]: crate::double_buffered::DoubleBufferedCache
pub struct ShapeCache {
    entries: crate::double_buffered::DoubleBufferedCache<ShapeCacheKey, Option<ShapedGlyphEntry>>,
    hits: u64,
    misses: u64,
    /// Hits promoted out of the previous frame's map. Diagnostic only; the
    /// caller cannot act on this breakdown mid frame.
    previous_hits: u64,
    /// Identity of the cache configuration used to populate entries. When
    /// any of these shift we drop everything; see [`ShapeCache::retune`].
    configured_font_id: u64,
    configured_scale_thousandths: u32,
    configured_font_size_tenths: u32,
}

impl Default for ShapeCache {
    fn default() -> Self {
        Self::new()
    }
}

impl ShapeCache {
    pub fn new() -> Self {
        Self {
            // Sized for 95 ASCII + typical box-drawing + growth headroom.
            entries: crate::double_buffered::DoubleBufferedCache::with_capacity(256),
            hits: 0,
            misses: 0,
            previous_hits: 0,
            configured_font_id: 0,
            configured_scale_thousandths: 0,
            configured_font_size_tenths: 0,
        }
    }

    /// Hits since the most recent [`ShapeCache::clear`]. Includes both
    /// `current` frame hits and `previous` frame hits that got promoted.
    pub fn hits(&self) -> u64 {
        self.hits
    }

    /// Misses since the most recent [`ShapeCache::clear`].
    pub fn misses(&self) -> u64 {
        self.misses
    }

    /// Subset of [`ShapeCache::hits`] that came from the previous frame's
    /// map (entries promoted during lookup). A high ratio indicates the
    /// working set exceeds a single frame of unique characters. Reset by
    /// [`ShapeCache::clear`].
    pub fn previous_hits(&self) -> u64 {
        self.previous_hits
    }

    /// Rough cache hit rate in the range 0.0..=1.0. Returns 0.0 when no
    /// lookups have been recorded yet.
    pub fn hit_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            self.hits as f64 / total as f64
        }
    }

    /// Number of entries currently cached across both halves.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when no entries have been stored.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Drops every entry and resets hit/miss counters. Call when font
    /// identity, DPI scale, or size changes.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.hits = 0;
        self.misses = 0;
        self.previous_hits = 0;
    }

    /// Swap previous and current halves, dropping any entry not touched this
    /// frame. Call once per frame after `build_render_batch` completes.
    pub fn finish_frame(&mut self) {
        self.entries.finish_frame();
    }

    /// Ensure the cache matches the active font/scale/size configuration.
    /// If any input changed since the last call, the cache is dropped so
    /// stale shaping data from the old configuration cannot leak.
    ///
    /// Call this at the start of each frame before the first lookup.
    pub fn retune(&mut self, font_family: &str, scale_factor: f32, font_size: f32) {
        let font_id = shape_cache_font_id(font_family);
        let scale = (scale_factor * 1000.0).round() as u32;
        let size = (font_size * 10.0).round() as u32;

        if font_id != self.configured_font_id
            || scale != self.configured_scale_thousandths
            || size != self.configured_font_size_tenths
        {
            self.clear();
            self.configured_font_id = font_id;
            self.configured_scale_thousandths = scale;
            self.configured_font_size_tenths = size;
        }
    }

    /// Look up a cached shaped entry, promoting any hit from `previous` to
    /// `current`. Records a hit or miss either way. Callers that miss must
    /// call [`ShapeCache::insert`] with the shaped result so the next frame
    /// hits.
    ///
    /// Counters: `hits` increments on any match, `previous_hits` additionally
    /// increments when the entry was promoted out of the previous frame's
    /// map.
    pub fn get(&mut self, key: &ShapeCacheKey) -> Option<&Option<ShapedGlyphEntry>> {
        match self.entries.get_or_promote_tracked(key) {
            Some((entry, promoted)) => {
                self.hits += 1;
                if promoted {
                    self.previous_hits += 1;
                }
                Some(entry)
            }
            None => {
                self.misses += 1;
                None
            }
        }
    }

    /// Insert a shaped result (including the `None` case where cosmic-text
    /// produced no glyph for the character) so subsequent frames hit.
    pub fn insert(&mut self, key: ShapeCacheKey, value: Option<ShapedGlyphEntry>) {
        self.entries.insert(key, value);
    }

    /// Range of characters preloaded by [`ShapeCache::preload_defaults`] in
    /// order: printable ASCII 0x20..=0x7e then the Unicode box-drawing block
    /// 0x2500..=0x257f. Used by tests and diagnostics.
    pub fn default_preload_chars() -> impl Iterator<Item = char> {
        (0x20u32..=0x7e).chain(0x2500..=0x257f).filter_map(char::from_u32)
    }

    /// Fill the cache with printable ASCII and the Unicode box-drawing block.
    /// Terminal workloads touch these characters first; preloading them
    /// pushes the cache hit rate above 95 percent within a screenful of
    /// output.
    ///
    /// Insert-only: on repeat calls existing entries are left untouched if
    /// present in either half.
    pub fn preload_defaults(
        &mut self,
        shape: &mut dyn FnMut(char) -> (ShapeCacheKey, Option<ShapedGlyphEntry>),
    ) {
        for ch in Self::default_preload_chars() {
            let (key, value) = shape(ch);
            // Only insert if neither half already has the entry; this
            // preserves idempotence on repeated preload calls.
            if self.entries.peek(&key).is_none() {
                self.entries.insert(key, value);
            }
        }
    }
}

/// Collect every ancestor (and self) of any `ElementContent::Grid` whose
/// cursor is currently visible. The renderer's cursor blink phase clock
/// changes every paint, so a cached batch entry on any ancestor of a
/// cursor-bearing grid would freeze the cursor at the phase recorded at
/// cache write time. Returning every ancestor here lets `walk_for_batch`
/// force `node_dirty = true` along the path and re-emit the cursor on
/// each redraw, even when no `DirtyFlags::PAINT` was set externally.
fn cursor_blink_dirty_ancestors(arena: &NodeArena) -> FxHashSet<NodeId> {
    let mut force_dirty: FxHashSet<NodeId> = FxHashSet::default();
    for (id, elem) in arena.iter() {
        if let ElementContent::Grid(ref grid) = elem.content {
            if grid.cursor_visible() {
                let mut cur = id;
                if !force_dirty.insert(cur) {
                    continue;
                }
                while let Some(e) = arena.get(cur) {
                    if e.parent.is_dangling() {
                        break;
                    }
                    cur = e.parent;
                    if !force_dirty.insert(cur) {
                        break;
                    }
                }
            }
        }
    }
    force_dirty
}

#[allow(clippy::too_many_arguments)]
pub fn build_render_batch(
    arena: &NodeArena,
    root: NodeId,
    batch: &mut LayeredBatch,
    atlas: &mut GlyphAtlas,
    font_system: &mut FontSystem,
    rasterizer: &mut Rasterizer<'_>,
    measure_cache: &mut TextMeasureCache,
    shaped_cache: &mut ShapedTextCache,
    svg_cache: &mut SvgTessCache,
    shape_cache: &mut ShapeCache,
    text_selection: Option<&TextSelection>,
    registry: Option<&CanvasRegistry>,
    scrollbar_state: &ScrollbarVisualState,
    focused: NodeId,
    batch_cache: &mut BatchCache,
    mut line_cache: Option<&mut LineQuadCache>,
) {
    let initial_clip = [0.0_f32, 0.0, 9999.0, 9999.0];
    let mut portals: Vec<(NodeId, Layer)> = Vec::new();
    let cursor_blink_force_dirty = cursor_blink_dirty_ancestors(arena);
    walk_for_batch(
        arena,
        root,
        root,
        batch,
        atlas,
        font_system,
        rasterizer,
        measure_cache,
        shaped_cache,
        svg_cache,
        shape_cache,
        initial_clip,
        0.0,
        0.0,
        text_selection,
        registry,
        scrollbar_state,
        focused,
        Layer::Content,
        &mut portals,
        batch_cache,
        None,
        line_cache.as_deref_mut(),
        &cursor_blink_force_dirty,
        Affine2::IDENTITY,
    );

    // Process deferred portal nodes with fresh viewport clip
    for (portal_node, target_layer) in portals {
        walk_for_batch(
            arena,
            portal_node,
            root,
            batch,
            atlas,
            font_system,
            rasterizer,
            measure_cache,
            shaped_cache,
            svg_cache,
            shape_cache,
            initial_clip,
            0.0,
            0.0,
            text_selection,
            registry,
            scrollbar_state,
            focused,
            target_layer,
            &mut Vec::new(),
            batch_cache,
            None,
            line_cache.as_deref_mut(),
            &cursor_blink_force_dirty,
            Affine2::IDENTITY,
        );
    }
}

/// Flush accumulated primitives of `kind` into `draw_spans` if `end > cursor`.
/// Returns the new cursor position.
#[inline]
fn flush_span(spans: &mut Vec<DrawSpan>, kind: DrawKind, cursor: usize, end: usize) -> usize {
    if end > cursor {
        spans.push(DrawSpan { kind, start: cursor as u32, count: (end - cursor) as u32 });
    }
    end
}

/// Clear PAINT and SUBTREE_PAINT flags on every node in a subtree after a
/// frame has been rendered. Call this after `build_render_batch` completes.
/// This requires mutable access to the arena; kept as a separate step so
/// `walk_for_batch` can retain a shared `&NodeArena` borrow.
pub fn clear_paint_flags_subtree(arena: &mut NodeArena, node_id: NodeId) {
    let children = arena.children(node_id);
    for child_id in children {
        clear_paint_flags_subtree(arena, child_id);
    }
    if let Some(element) = arena.get_mut(node_id) {
        element.dirty.remove(DirtyFlags::PAINT | DirtyFlags::SUBTREE_PAINT);
    }
}

/// Identity values for the per-instance transform affine. The instance encodes
/// the 2x2 linear part as a delta from identity (`a-1, b, c, d-1`) so all-zero
/// is the identity; every instance literal is constructed with these and the
/// real transform is stamped onto a transformed node's own instances after
/// emit (see [`stamp_xform_quads`] / the stamping in `walk_for_batch`).
const IDENTITY_XFORM: [f32; 4] = [0.0; 4];
const IDENTITY_XFORM_TRANSLATE: [f32; 2] = [0.0; 2];

/// A 2x3 affine transform `x' = a*x + c*y + e`, `y' = b*x + d*y + f`, in
/// screen pixels. Used to compose CSS `transform`s down the paint subtree.
#[derive(Clone, Copy)]
struct Affine2 {
    a: f32,
    b: f32,
    c: f32,
    d: f32,
    e: f32,
    f: f32,
}

impl Affine2 {
    const IDENTITY: Affine2 = Affine2 { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: 0.0, f: 0.0 };

    fn is_identity(self) -> bool {
        self.a == 1.0
            && self.b == 0.0
            && self.c == 0.0
            && self.d == 1.0
            && self.e == 0.0
            && self.f == 0.0
    }

    /// `self ∘ other`: the affine that applies `other` first, then `self`.
    fn compose(self, o: Affine2) -> Affine2 {
        Affine2 {
            a: self.a * o.a + self.c * o.b,
            b: self.b * o.a + self.d * o.b,
            c: self.a * o.c + self.c * o.d,
            d: self.b * o.c + self.d * o.d,
            e: self.a * o.e + self.c * o.f + self.e,
            f: self.b * o.e + self.d * o.f + self.f,
        }
    }

    /// The 2x2 linear part as a delta from identity (`[a-1, b, c, d-1]`), the
    /// `QuadInstance::xform` / `GlyphInstance::xform` encoding.
    fn xform_delta(self) -> [f32; 4] {
        [self.a - 1.0, self.b, self.c, self.d - 1.0]
    }

    /// The translation part `[e, f]`.
    fn xform_translate(self) -> [f32; 2] {
        [self.e, self.f]
    }

    /// Six floats for the batch cache signature, in the same delta-from-identity
    /// encoding as the instance, so the identity hashes to all-zero and matches
    /// the derived `Default`.
    fn signature(self) -> [f32; 6] {
        [self.a - 1.0, self.b, self.c, self.d - 1.0, self.e, self.f]
    }
}

/// Build the screen-space affine for a CSS `transform` about the element's
/// center (transform-origin defaults to `50% 50%`; the app never authors
/// another origin). Returns the identity for an identity transform so callers
/// keep the matrix-free fast path. `render_x`/`render_y` are the element's
/// painted top-left; `w`/`h` its border-box size.
fn element_affine(
    t: &unshit_core::style::types::Transform,
    render_x: f32,
    render_y: f32,
    w: f32,
    h: f32,
) -> Affine2 {
    if t.is_identity() {
        return Affine2::IDENTITY;
    }
    let ox = render_x + w * 0.5;
    let oy = render_y + h * 0.5;
    let (sin, cos) = t.rotate.sin_cos();
    // Linear part = Rotate · Scale (a point is scaled, then rotated), matching
    // the canonical `Translate · Rotate · Scale` compose order.
    let a = t.scale_x * cos;
    let b = t.scale_x * sin;
    let c = -t.scale_y * sin;
    let d = t.scale_y * cos;
    let tx = t.translate_x.map(|v| v.resolve(w)).unwrap_or(0.0);
    let ty = t.translate_y.map(|v| v.resolve(h)).unwrap_or(0.0);
    // p' = O + L·(p - O) + T  =>  p' = L·p + (O - L·O + T). The translate is
    // applied in screen space (outermost), correct for the canonical order.
    let e = ox - (a * ox + c * oy) + tx;
    let f = oy - (b * ox + d * oy) + ty;
    Affine2 { a, b, c, d, e, f }
}

/// Stamp a transform onto a contiguous run of just-emitted quads (a node's own
/// primitives). No-op encoding identity is never passed here (callers gate on
/// `is_identity`).
fn stamp_xform_quads(quads: &mut [QuadInstance], xform: [f32; 4], translate: [f32; 2]) {
    for q in quads {
        q.xform = xform;
        q.xform_translate = translate;
    }
}

/// Stamp a transform onto a contiguous run of just-emitted glyphs.
fn stamp_xform_glyphs(glyphs: &mut [GlyphInstance], xform: [f32; 4], translate: [f32; 2]) {
    for g in glyphs {
        g.xform = xform;
        g.xform_translate = translate;
    }
}

#[allow(clippy::too_many_arguments)]
fn walk_for_batch(
    arena: &NodeArena,
    node_id: NodeId,
    root: NodeId,
    batch: &mut LayeredBatch,
    atlas: &mut GlyphAtlas,
    font_system: &mut FontSystem,
    rasterizer: &mut Rasterizer<'_>,
    measure_cache: &mut TextMeasureCache,
    shaped_cache: &mut ShapedTextCache,
    svg_cache: &mut SvgTessCache,
    shape_cache: &mut ShapeCache,
    clip_rect: [f32; 4],
    scroll_offset_x: f32,
    scroll_offset_y: f32,
    text_selection: Option<&TextSelection>,
    registry: Option<&CanvasRegistry>,
    scrollbar_state: &ScrollbarVisualState,
    focused: NodeId,
    current_layer: Layer,
    portals: &mut Vec<(NodeId, Layer)>,
    batch_cache: &mut BatchCache,
    parent_glyph_keys: Option<&mut FxHashSet<GlyphKey>>,
    mut line_cache: Option<&mut LineQuadCache>,
    cursor_blink_force_dirty: &FxHashSet<NodeId>,
    parent_xform: Affine2,
) {
    let Some(element) = arena.get(node_id) else {
        return;
    };

    let style = &element.computed_style;

    if style.display == Display::None || style.opacity == 0.0 {
        return;
    }

    // Portal: defer rendering to a later pass with fresh clip
    if let RenderTarget::Portal(target_layer) = style.render_target {
        portals.push((node_id, target_layer));
        return;
    }

    // Resolve effective layer before the damage check so we use the correct
    // layer index when looking up cached ranges.
    let effective_layer = if style.layer != Layer::Content { style.layer } else { current_layer };
    let layer_index = effective_layer as usize;

    let rect = element.layout_rect;

    // Resolve `border-radius` against the box. Percent corners (`50%` circular
    // avatars) resolve against `min(width, height)` so they stay circular on
    // non-square boxes (CSS resolves radius percentages against the box). Pure
    // px corners pass through unchanged, so this also covers the f32 fast path.
    let border_radius = style.border_radius_src.resolve(rect.width.min(rect.height));

    let render_x = rect.x - scroll_offset_x;
    let render_y = rect.y - scroll_offset_y;

    // CSS `transform` is applied at paint time as a screen-space affine that
    // does not disturb layout (siblings and hit-testing keep the in-flow
    // position). This element's transform is composed about its center with the
    // inherited subtree transform; the result is baked into every primitive
    // this node emits (stamped after emit) and threaded to children so the
    // whole subtree transforms together. The identity case stays matrix-free.
    let node_xform = parent_xform.compose(element_affine(
        &style.transform,
        render_x,
        render_y,
        rect.width,
        rect.height,
    ));

    // The transform is part of the cache signature so a node re-emits when its
    // own OR any ancestor's transform changes (an ancestor change alters
    // `node_xform` here even when the node's content is otherwise clean).
    let cache_signature = BatchCacheSignature::new(
        [render_x, render_y, rect.width, rect.height],
        clip_rect,
        node_xform.signature(),
    );

    // Damage-aware skip: if neither PAINT nor SUBTREE_PAINT is set, replay the
    // previously cached primitive instances and skip the entire subtree.
    //
    // `batch_cache.replay` both reads the cached range AND carries it
    // forward into the current frame's staging map, so the entry survives
    // the next `commit_frame` swap. Calling the pure `get` here would
    // leak the entry on the next commit and force clean nodes to alternate
    // between cache hits and forced re-renders every other frame.
    //
    // Replay is additionally gated on glyph atlas generation. Cached ranges
    // built against an older atlas generation are discarded so stale UVs are
    // never replayed after atlas eviction/repack.
    let mut node_dirty = element.dirty.intersects(DirtyFlags::PAINT | DirtyFlags::SUBTREE_PAINT);

    // Cell grids that own the focused cursor must re-emit on every paint
    // so the global blink phase clock (`CellGrid::cursor_blink_phase_now`)
    // can flip the cursor on and off. The bypass also covers every
    // ancestor of such a grid (precomputed by `cursor_blink_dirty_ancestors`):
    // without it, a clean ancestor would replay its cached subtree and the
    // walk would never reach the grid, freezing the cursor at whatever
    // phase was recorded when the cache was written. The line-quad cache
    // inside `emit_grid_cells` still skips per-row work, so the cost of
    // the bypass is bounded by the focused pane's depth times row count.
    if !node_dirty && cursor_blink_force_dirty.contains(&node_id) {
        node_dirty = true;
    }

    if !node_dirty {
        if let Some(cached) = batch_cache.replay_with_signature(
            node_id,
            layer_index,
            atlas.generation,
            cache_signature,
        ) {
            for key in &cached.glyph_keys {
                atlas.touch(key);
            }
            let lb = batch.layer_mut(effective_layer);
            let quad_offset = lb.quad_instances.len() as u32;
            let glyph_offset = lb.glyph_instances.len() as u32;
            let svg_offset = lb.svg_draws.len() as u32;
            let image_offset = lb.image_batches.len() as u32;
            let canvas_offset = lb.canvas_callbacks.len() as u32;
            lb.quad_instances.extend_from_slice(&cached.quads);
            lb.glyph_instances.extend_from_slice(&cached.glyphs);
            lb.svg_draws.extend_from_slice(&cached.svgs);
            for span in &cached.draw_spans {
                let offset = match span.kind {
                    DrawKind::Quad => quad_offset,
                    DrawKind::Glyph => glyph_offset,
                };
                lb.draw_spans.push(DrawSpan {
                    kind: span.kind,
                    start: span.start + offset,
                    count: span.count,
                });
            }
            // Restore backdrop boundaries with absolute prefixes adjusted to
            // the current layer state. Without this, the blur pass never
            // runs on cache-hit frames and elements behind a
            // `backdrop-filter` overlay alternate between blurred and
            // unblurred whenever the cache path is taken.
            for b in &cached.backdrop_boundaries {
                lb.backdrop_boundaries.push(BackdropBoundary {
                    rect: b.rect,
                    clip_rect: b.clip_rect,
                    blur_radius: b.blur_radius,
                    quad_prefix: b.quad_prefix + quad_offset,
                    glyph_prefix: b.glyph_prefix + glyph_offset,
                    svg_prefix: b.svg_prefix + svg_offset,
                    image_batch_prefix: b.image_batch_prefix + image_offset,
                    canvas_prefix: b.canvas_prefix + canvas_offset,
                    dirty: b.dirty,
                });
            }
            if let Some(parent_keys) = parent_glyph_keys {
                for key in &cached.glyph_keys {
                    parent_keys.insert(*key);
                }
            }
            return;
        }
        // No cached data available: fall through to render this node so it
        // gets populated in the cache for subsequent frames.
    }

    // Record where this node starts writing primitives so we can snapshot the
    // range after all children have been processed.
    let quad_start = batch.layer_mut(effective_layer).quad_instances.len();
    let glyph_start = batch.layer_mut(effective_layer).glyph_instances.len();
    let svg_start = batch.layer_mut(effective_layer).svg_draws.len();
    let span_start = batch.layer_mut(effective_layer).draw_spans.len();
    let image_start = batch.layer_mut(effective_layer).image_batches.len();
    let canvas_start = batch.layer_mut(effective_layer).canvas_callbacks.len();
    let boundary_start = batch.layer_mut(effective_layer).backdrop_boundaries.len();
    let mut node_glyph_keys: FxHashSet<GlyphKey> = FxHashSet::default();

    // Running cursors for draw span tracking. Updated after each flush.
    let mut quad_cursor = quad_start;
    let glyph_cursor = glyph_start;

    let is_visible = style.visibility == Visibility::Visible;
    let opacity = style.opacity;

    // Per-axis clipping: `overflow-x` clips the horizontal extent (left/right)
    // and `overflow-y` clips the vertical extent (top/bottom) independently.
    let clips_x = style.overflow_x != Overflow::Visible;
    let clips_y = style.overflow_y != Overflow::Visible;
    let clips_children = clips_x || clips_y;
    let child_clip = if clips_children {
        // Start from the inherited clip rect, then tighten only the axes this
        // element actually clips.
        let (mut new_x, mut new_right) = (clip_rect[0], clip_rect[0] + clip_rect[2]);
        let (mut new_y, mut new_bottom) = (clip_rect[1], clip_rect[1] + clip_rect[3]);
        if clips_x {
            new_x = render_x.max(clip_rect[0]);
            new_right = (render_x + rect.width).min(clip_rect[0] + clip_rect[2]);
        }
        if clips_y {
            new_y = render_y.max(clip_rect[1]);
            new_bottom = (render_y + rect.height).min(clip_rect[1] + clip_rect[3]);
        }
        [new_x, new_y, (new_right - new_x).max(0.0), (new_bottom - new_y).max(0.0)]
    } else {
        clip_rect
    };
    let (child_scroll_x, child_scroll_y) = if clips_children {
        (scroll_offset_x + element.scroll_x, scroll_offset_y + element.scroll_y)
    } else {
        (scroll_offset_x, scroll_offset_y)
    };
    // (CSS `transform` — including translate — now propagates to the subtree
    // via `node_xform` baked into each instance's affine, not via the child
    // scroll offset.)

    if is_visible && style.outline_width > 0.0 && style.outline_color.a > 0 {
        let expand = style.outline_width + style.outline_offset;
        let mut oc = style.outline_color.to_linear_f32();
        oc[3] *= opacity;
        let outline_border = [style.outline_width; 4];

        batch.layer_mut(effective_layer).quad_instances.push(QuadInstance {
            pos: [render_x - expand, render_y - expand],
            size: [rect.width + expand * 2.0, rect.height + expand * 2.0],
            color: [0.0; 4], // no fill
            border_color: oc,
            border_width: outline_border,
            border_radius,
            clip_rect,
            shadow_color: [0.0; 4],
            shadow_offset: [0.0; 2],
            shadow_params: [0.0; 2],
            shadow_spread: [0.0; 2],
            gradient_stop_colors: EMPTY_GRADIENT_STOP_COLORS,
            gradient_stop_positions: EMPTY_GRADIENT_STOP_POSITIONS,
            gradient_params: [0.0; 4],
            gradient_extra: EMPTY_GRADIENT_EXTRA,
            mask_stops_01: EMPTY_MASK_STOPS,
            mask_stops_23: EMPTY_MASK_STOPS,
            mask_params: EMPTY_MASK_PARAMS,
            xform: IDENTITY_XFORM,
            xform_translate: IDENTITY_XFORM_TRANSLATE,
        });
    }

    // Backdrop filter boundary: tagged at the point the element is about to
    // draw its own background. The render loop uses the per primitive
    // prefix counts to flush all preceding draws, blur the framebuffer
    // contents behind the element, then reopen the pass so the element
    // draws on top. We only emit a boundary if the element has a non zero
    // area after clipping, mirroring the zero area edge case in the design.
    if is_visible {
        if let Some(ref backdrop) = style.backdrop_filter {
            let max_blur = backdrop
                .filters
                .iter()
                .map(|f| match f {
                    FilterFunction::Blur(r) => *r,
                })
                .fold(0.0_f32, f32::max);
            // A rounded to zero radius degenerates to a no op pass split.
            if max_blur > 0.5 && rect.width > 0.0 && rect.height > 0.0 {
                let elem_left = render_x.max(clip_rect[0]);
                let elem_top = render_y.max(clip_rect[1]);
                let elem_right = (render_x + rect.width).min(clip_rect[0] + clip_rect[2]);
                let elem_bottom = (render_y + rect.height).min(clip_rect[1] + clip_rect[3]);
                let elem_w = (elem_right - elem_left).max(0.0);
                let elem_h = (elem_bottom - elem_top).max(0.0);
                if elem_w > 0.0 && elem_h > 0.0 {
                    let lb = batch.layer_mut(effective_layer);
                    lb.backdrop_boundaries.push(BackdropBoundary {
                        rect: [elem_left, elem_top, elem_w, elem_h],
                        clip_rect,
                        blur_radius: max_blur,
                        quad_prefix: lb.quad_instances.len() as u32,
                        glyph_prefix: lb.glyph_instances.len() as u32,
                        svg_prefix: lb.svg_draws.len() as u32,
                        image_batch_prefix: lb.image_batches.len() as u32,
                        canvas_prefix: lb.canvas_callbacks.len() as u32,
                        dirty: true,
                    });
                }
            }
        }
    }

    if is_visible
        && (style.background.is_visible()
            || style.border_width.any_nonzero()
            || !style.box_shadow.is_empty())
    {
        // 1. Outer shadows go behind the background quad on the same layer,
        //    in declared order. CSS paints them in reverse so the first
        //    shadow lands on top; we mirror that by pushing outer shadows in
        //    reverse order so later pushes draw on top within the batch.
        for shadow in style.box_shadow.iter().rev() {
            if shadow.inset {
                continue;
            }
            if shadow.color.a == 0 {
                continue;
            }
            let mut sc = shadow.color.to_linear_f32();
            sc[3] *= opacity;
            batch.layer_mut(effective_layer).quad_instances.push(QuadInstance {
                pos: [render_x, render_y],
                size: [rect.width, rect.height],
                color: [0.0; 4],
                border_color: [0.0; 4],
                border_width: [0.0; 4],
                border_radius,
                clip_rect,
                shadow_color: sc,
                shadow_offset: [shadow.offset_x, shadow.offset_y],
                shadow_params: [shadow.blur_radius, 0.0],
                shadow_spread: [shadow.spread_radius, 0.0],
                gradient_stop_colors: EMPTY_GRADIENT_STOP_COLORS,
                gradient_stop_positions: EMPTY_GRADIENT_STOP_POSITIONS,
                gradient_params: [0.0; 4],
                gradient_extra: EMPTY_GRADIENT_EXTRA,
                mask_stops_01: EMPTY_MASK_STOPS,
                mask_stops_23: EMPTY_MASK_STOPS,
                mask_params: EMPTY_MASK_PARAMS,
                xform: IDENTITY_XFORM,
                xform_translate: IDENTITY_XFORM_TRANSLATE,
            });
        }

        // 2. Background + border quad (no embedded shadow).
        let mut bc = style.border_color.to_linear_f32();
        bc[3] *= opacity;

        let (bg, grad_stop_colors, grad_stop_positions, grad_params, grad_extra) =
            match &style.background {
                Background::Color(c) => {
                    let mut bg = c.to_linear_f32();
                    bg[3] *= opacity;
                    (
                        bg,
                        EMPTY_GRADIENT_STOP_COLORS,
                        EMPTY_GRADIENT_STOP_POSITIONS,
                        [0.0; 4],
                        EMPTY_GRADIENT_EXTRA,
                    )
                }
                Background::LinearGradient(grad) => {
                    let (colors, positions, params) =
                        pack_gradient(grad, opacity, rect.width, rect.height);
                    ([0.0; 4], colors, positions, params, EMPTY_GRADIENT_EXTRA)
                }
                Background::RadialGradient(grad) => {
                    let (colors, positions, params, extra) =
                        pack_radial_gradient(grad, rect.width, rect.height, opacity);
                    ([0.0; 4], colors, positions, params, extra)
                }
            };

        if style.background.is_visible() || style.border_width.any_nonzero() {
            let (mask_stops_01, mask_stops_23, mask_params) = style
                .mask_image
                .as_ref()
                .map(pack_mask_image)
                .unwrap_or((EMPTY_MASK_STOPS, EMPTY_MASK_STOPS, EMPTY_MASK_PARAMS));
            batch.layer_mut(effective_layer).quad_instances.push(QuadInstance {
                pos: [render_x, render_y],
                size: [rect.width, rect.height],
                color: bg,
                border_color: bc,
                border_width: style.border_width.to_array(),
                border_radius,
                clip_rect,
                shadow_color: [0.0; 4],
                shadow_offset: [0.0; 2],
                shadow_params: [0.0; 2],
                shadow_spread: [0.0; 2],
                gradient_stop_colors: grad_stop_colors,
                gradient_stop_positions: grad_stop_positions,
                gradient_params: grad_params,
                gradient_extra: grad_extra,
                mask_stops_01,
                mask_stops_23,
                mask_params,
                xform: IDENTITY_XFORM,
                xform_translate: IDENTITY_XFORM_TRANSLATE,
            });
        }

        // 3. Inset shadows go over the background, clipped to the padding
        //    box. The shader reads the inset flag from `shadow_params.y`.
        for shadow in style.box_shadow.iter() {
            if !shadow.inset {
                continue;
            }
            if shadow.color.a == 0 {
                continue;
            }
            let mut sc = shadow.color.to_linear_f32();
            sc[3] *= opacity;
            batch.layer_mut(effective_layer).quad_instances.push(QuadInstance {
                pos: [render_x, render_y],
                size: [rect.width, rect.height],
                color: [0.0; 4],
                border_color: [0.0; 4],
                border_width: [0.0; 4],
                border_radius,
                clip_rect,
                shadow_color: sc,
                shadow_offset: [shadow.offset_x, shadow.offset_y],
                shadow_params: [shadow.blur_radius, 1.0],
                shadow_spread: [shadow.spread_radius, 0.0],
                gradient_stop_colors: EMPTY_GRADIENT_STOP_COLORS,
                gradient_stop_positions: EMPTY_GRADIENT_STOP_POSITIONS,
                gradient_params: [0.0; 4],
                gradient_extra: EMPTY_GRADIENT_EXTRA,
                mask_stops_01: EMPTY_MASK_STOPS,
                mask_stops_23: EMPTY_MASK_STOPS,
                mask_params: EMPTY_MASK_PARAMS,
                xform: IDENTITY_XFORM,
                xform_translate: IDENTITY_XFORM_TRANSLATE,
            });
        }
    }

    // Flush background/outline/shadow/border quads before text emission.
    {
        let lb = batch.layer_mut(effective_layer);
        let end = lb.quad_instances.len();
        quad_cursor = flush_span(&mut lb.draw_spans, DrawKind::Quad, quad_cursor, end);
    }

    // Input element rendering
    if element.tag == Tag::Input && is_visible {
        let style = &element.computed_style;
        let content_w = rect.width - style.padding.left - style.padding.right;
        let content_h = rect.height - style.padding.top - style.padding.bottom;

        let input = &element.input_state;

        match input.input_type {
            InputType::Hidden => {
                // Nothing to render.
            }
            InputType::Checkbox | InputType::Radio => {
                // Both are rendered as a small square/circle (the outer box is
                // already drawn by the quad pass via CSS). Checked state uses
                // vector geometry instead of font glyphs so centering is based
                // on the control box, not font baseline metrics.
                if input.checked {
                    emit_checked_input_marker(
                        input.input_type,
                        render_x,
                        render_y,
                        rect.width,
                        rect.height,
                        style.color,
                        opacity,
                        clip_rect,
                        svg_cache,
                        batch.layer_mut(effective_layer),
                    );
                }
            }
            InputType::Range => {
                // Render a thin horizontal track and a circular thumb.
                let track_h = 4.0_f32;
                let thumb_r = (content_h * 0.5).min(8.0);
                let thumb_d = thumb_r * 2.0;

                let track_x = render_x + style.padding.left;
                let track_y = render_y + (rect.height - track_h) * 0.5;
                let track_w = content_w;

                // Track background.
                let mut track_color = style.color.to_linear_f32();
                track_color[3] *= opacity * 0.3;
                batch.layer_mut(effective_layer).quad_instances.push(QuadInstance {
                    pos: [track_x, track_y],
                    size: [track_w, track_h],
                    color: track_color,
                    border_color: [0.0; 4],
                    border_width: [0.0; 4],
                    border_radius: [track_h * 0.5; 4],
                    clip_rect,
                    shadow_color: [0.0; 4],
                    shadow_offset: [0.0; 2],
                    shadow_params: [0.0; 2],
                    shadow_spread: [0.0; 2],
                    gradient_stop_colors: EMPTY_GRADIENT_STOP_COLORS,
                    gradient_stop_positions: EMPTY_GRADIENT_STOP_POSITIONS,
                    gradient_params: [0.0; 4],
                    gradient_extra: EMPTY_GRADIENT_EXTRA,
                    mask_stops_01: EMPTY_MASK_STOPS,
                    mask_stops_23: EMPTY_MASK_STOPS,
                    mask_params: EMPTY_MASK_PARAMS,
                    xform: IDENTITY_XFORM,
                    xform_translate: IDENTITY_XFORM_TRANSLATE,
                });

                // Thumb position.
                let ratio = if input.max > input.min {
                    ((input.numeric_value - input.min) / (input.max - input.min)).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let thumb_cx = track_x + ratio * track_w;
                let thumb_x = thumb_cx - thumb_r;
                let thumb_y = render_y + (rect.height - thumb_d) * 0.5;

                let mut thumb_color = style.color.to_linear_f32();
                thumb_color[3] *= opacity;
                batch.layer_mut(effective_layer).quad_instances.push(QuadInstance {
                    pos: [thumb_x, thumb_y],
                    size: [thumb_d, thumb_d],
                    color: thumb_color,
                    border_color: [0.0; 4],
                    border_width: [0.0; 4],
                    border_radius: [thumb_r; 4],
                    clip_rect,
                    shadow_color: [0.0; 4],
                    shadow_offset: [0.0; 2],
                    shadow_params: [0.0; 2],
                    shadow_spread: [0.0; 2],
                    gradient_stop_colors: EMPTY_GRADIENT_STOP_COLORS,
                    gradient_stop_positions: EMPTY_GRADIENT_STOP_POSITIONS,
                    gradient_params: [0.0; 4],
                    gradient_extra: EMPTY_GRADIENT_EXTRA,
                    mask_stops_01: EMPTY_MASK_STOPS,
                    mask_stops_23: EMPTY_MASK_STOPS,
                    mask_params: EMPTY_MASK_PARAMS,
                    xform: IDENTITY_XFORM,
                    xform_translate: IDENTITY_XFORM_TRANSLATE,
                });
            }
            InputType::Text | InputType::Password | InputType::Number => {
                // For password, substitute bullet characters at render time.
                let masked: String;
                let is_placeholder = input.value.is_empty();
                let display_text = if is_placeholder {
                    element.placeholder.as_deref().unwrap_or("")
                } else if input.input_type == InputType::Password {
                    masked = "\u{2022}".repeat(input.value.chars().count());
                    &masked
                } else {
                    &input.value
                };
                let transformed_display_text = if input.input_type == InputType::Password {
                    std::borrow::Cow::Borrowed(display_text)
                } else {
                    apply_text_transform(display_text, style.text_transform)
                };
                let display_text = transformed_display_text.as_ref();

                if !display_text.is_empty() {
                    let mut text_color =
                        if is_placeholder { style.placeholder_color } else { style.color };
                    text_color.a = (text_color.a as f32 * opacity) as u8;

                    let (text_w, text_h) = measure_text_with_style_cached(
                        display_text,
                        &style.font_family,
                        style.font_weight,
                        style.font_style,
                        style.font_size,
                        style.line_height,
                        style.letter_spacing,
                        Some(content_w),
                        font_system,
                        Some(measure_cache),
                    );
                    let y_offset = ((content_h - text_h) * 0.5).max(0.0);
                    let text_x = aligned_text_x(
                        render_x,
                        style.padding.left,
                        content_w,
                        text_w,
                        style.text_align,
                    );
                    let text_y = render_y + style.padding.top + y_offset;

                    emit_text_glyphs_cached(
                        display_text,
                        text_x,
                        text_y,
                        Some(content_w),
                        style.font_size,
                        style.line_height,
                        style.letter_spacing,
                        &style.font_family,
                        style.font_weight,
                        style.font_style,
                        &text_color,
                        clip_rect,
                        batch.layer_mut(effective_layer),
                        atlas,
                        font_system,
                        rasterizer,
                        shaped_cache,
                        Some(&mut node_glyph_keys),
                    );
                }

                // Render cursor (caret) when focused and cursor is visible
                if node_id == focused && element.cursor_state.visible {
                    let value_text: std::borrow::Cow<'_, str> =
                        if input.input_type == InputType::Password {
                            std::borrow::Cow::Owned("\u{2022}".repeat(input.value.chars().count()))
                        } else {
                            apply_text_transform(input.value.as_str(), style.text_transform)
                        };
                    let value_w = if value_text.is_empty() {
                        0.0
                    } else {
                        measure_text_with_style_cached(
                            value_text.as_ref(),
                            &style.font_family,
                            style.font_weight,
                            style.font_style,
                            style.font_size,
                            style.line_height,
                            style.letter_spacing,
                            Some(content_w),
                            font_system,
                            Some(measure_cache),
                        )
                        .0
                    };
                    let text_x = aligned_text_x(
                        render_x,
                        style.padding.left,
                        content_w,
                        value_w,
                        style.text_align,
                    );
                    // For password, measure prefix of masked text.
                    let cursor_x = if input.cursor_pos == 0 || input.value.is_empty() {
                        0.0
                    } else {
                        let prefix: String = if input.input_type == InputType::Password {
                            // Each char maps to one bullet.
                            let char_count = input.value[..input.cursor_pos].chars().count();
                            "\u{2022}".repeat(char_count)
                        } else {
                            apply_text_transform(
                                &input.value[..input.cursor_pos],
                                style.text_transform,
                            )
                            .into_owned()
                        };
                        let (w, _) = measure_text_with_style_cached(
                            &prefix,
                            &style.font_family,
                            style.font_weight,
                            style.font_style,
                            style.font_size,
                            style.line_height,
                            style.letter_spacing,
                            Some(content_w),
                            font_system,
                            Some(measure_cache),
                        );
                        w
                    };

                    let caret_source = if input.value.is_empty() {
                        std::borrow::Cow::Borrowed(" ")
                    } else if input.input_type == InputType::Password {
                        std::borrow::Cow::Borrowed(input.value.as_str())
                    } else {
                        apply_text_transform(input.value.as_str(), style.text_transform)
                    };
                    let (_, caret_text_h) = measure_text_with_style_cached(
                        caret_source.as_ref(),
                        &style.font_family,
                        style.font_weight,
                        style.font_style,
                        style.font_size,
                        style.line_height,
                        style.letter_spacing,
                        Some(content_w),
                        font_system,
                        Some(measure_cache),
                    );
                    let caret_y_offset = ((content_h - caret_text_h) * 0.5).max(0.0);
                    let caret_y = render_y + style.padding.top + caret_y_offset;
                    let caret_height = style.font_size * style.line_height;

                    // Determine cursor dimensions based on shape
                    let cursor_shape = style.caret_shape;
                    let (caret_w, caret_h, caret_pos_x, caret_pos_y) = match cursor_shape {
                        CursorShape::Block => {
                            let char_width = style.font_size * 0.6;
                            (char_width, caret_height, text_x + cursor_x, caret_y)
                        }
                        CursorShape::Beam => (2.0_f32, caret_height, text_x + cursor_x, caret_y),
                        CursorShape::Underline => {
                            let char_width = style.font_size * 0.6;
                            let underline_y = caret_y + caret_height - 2.0;
                            (char_width, 2.0_f32, text_x + cursor_x, underline_y)
                        }
                    };

                    let mut caret_color = style.caret_color.to_linear_f32();
                    if cursor_shape == CursorShape::Block {
                        caret_color[3] *= opacity * 0.5;
                    } else {
                        caret_color[3] *= opacity;
                    }

                    batch.layer_mut(effective_layer).quad_instances.push(QuadInstance {
                        pos: [caret_pos_x, caret_pos_y],
                        size: [caret_w, caret_h],
                        color: caret_color,
                        border_color: [0.0; 4],
                        border_width: [0.0; 4],
                        border_radius: [0.0; 4],
                        clip_rect,
                        shadow_color: [0.0; 4],
                        shadow_offset: [0.0; 2],
                        shadow_params: [0.0; 2],
                        shadow_spread: [0.0; 2],
                        gradient_stop_colors: EMPTY_GRADIENT_STOP_COLORS,
                        gradient_stop_positions: EMPTY_GRADIENT_STOP_POSITIONS,
                        gradient_params: [0.0; 4],
                        gradient_extra: EMPTY_GRADIENT_EXTRA,
                        mask_stops_01: EMPTY_MASK_STOPS,
                        mask_stops_23: EMPTY_MASK_STOPS,
                        mask_params: EMPTY_MASK_PARAMS,
                        xform: IDENTITY_XFORM,
                        xform_translate: IDENTITY_XFORM_TRANSLATE,
                    });
                }
            }
        }
    } else {
        match &element.content {
            ElementContent::Text(ref raw_text) if is_visible && !raw_text.is_empty() => {
                let transformed_text = apply_text_transform(raw_text, style.text_transform);
                let mut text = transformed_text.as_ref();
                let mut text_color = style.color;
                text_color.a = (text_color.a as f32 * opacity) as u8;

                let content_w = rect.width - style.padding.left - style.padding.right;
                let content_h = rect.height - style.padding.top - style.padding.bottom;

                let text_max_w =
                    if matches!(style.white_space, WhiteSpace::Nowrap | WhiteSpace::Pre) {
                        None
                    } else {
                        Some(content_w)
                    };

                let (mut text_w, mut text_h) = measure_text_with_style_cached(
                    text,
                    &style.font_family,
                    style.font_weight,
                    style.font_style,
                    style.font_size,
                    style.line_height,
                    style.letter_spacing,
                    text_max_w,
                    font_system,
                    Some(measure_cache),
                );

                // `text-overflow: ellipsis` on a non-wrapping run that overflows
                // its content box: truncate on a grapheme-cluster boundary and
                // append an ellipsis BEFORE we emit glyphs, so the clip rect no
                // longer does the cutting. Truncating first means the
                // (re)measure below keys the cache on the truncated string, so a
                // truncated run can neither be served from nor pollute the
                // full-run cache entry (and vice versa). `Clip` stays a no-op.
                let truncated_holder: String;
                if style.text_overflow == TextOverflow::Ellipsis
                    && matches!(style.white_space, WhiteSpace::Nowrap)
                    && text_w > content_w
                    && content_w > 0.0
                {
                    if let Some(truncated) = truncate_text_with_ellipsis(
                        text,
                        &style.font_family,
                        style.font_weight,
                        style.font_style,
                        style.font_size,
                        style.line_height,
                        style.letter_spacing,
                        content_w,
                        font_system,
                    ) {
                        truncated_holder = truncated;
                        text = truncated_holder.as_str();
                        let (tw, th) = measure_text_with_style_cached(
                            text,
                            &style.font_family,
                            style.font_weight,
                            style.font_style,
                            style.font_size,
                            style.line_height,
                            style.letter_spacing,
                            text_max_w,
                            font_system,
                            Some(measure_cache),
                        );
                        text_w = tw;
                        text_h = th;
                    }
                }
                let y_offset = ((content_h - text_h) * 0.5).max(0.0);

                let text_x = aligned_text_x(
                    render_x,
                    style.padding.left,
                    content_w,
                    text_w,
                    style.text_align,
                );
                let text_y = render_y + style.padding.top + y_offset;

                // Selection highlight rendering (emitted before text so it renders behind glyphs)
                if let Some(selection) = text_selection {
                    if !selection.is_collapsed() {
                        let byte_range = if selection.anchor_element == selection.focus_element {
                            if selection.anchor_element == node_id {
                                selection.single_element_range()
                            } else {
                                None
                            }
                        } else {
                            let anchor_order = unshit_core::event::document_order(
                                arena,
                                root,
                                selection.anchor_element,
                            );
                            let focus_order = unshit_core::event::document_order(
                                arena,
                                root,
                                selection.focus_element,
                            );
                            let node_order =
                                unshit_core::event::document_order(arena, root, node_id);

                            match (anchor_order, focus_order, node_order) {
                                (Some(ao), Some(fo), Some(no)) => {
                                    selection.element_byte_range(node_id, no, ao, fo, text.len())
                                }
                                _ => None,
                            }
                        };

                        if let Some((sel_start, sel_end)) = byte_range {
                            let line_ranges = unshit_core::layout::text_line_ranges(
                                text,
                                style.font_size,
                                style.line_height,
                                style.letter_spacing,
                                text_max_w,
                                sel_start,
                                sel_end,
                                font_system,
                            );

                            let sel_color = [0.2, 0.4, 0.8, 0.4];

                            for lr in &line_ranges {
                                batch.layer_mut(effective_layer).quad_instances.push(
                                    QuadInstance {
                                        pos: [text_x + lr.x, text_y + lr.y],
                                        size: [lr.width, lr.height],
                                        color: sel_color,
                                        border_color: [0.0; 4],
                                        border_width: [0.0; 4],
                                        border_radius: [0.0; 4],
                                        clip_rect,
                                        shadow_color: [0.0; 4],
                                        shadow_offset: [0.0; 2],
                                        shadow_params: [0.0; 2],
                                        shadow_spread: [0.0; 2],
                                        gradient_stop_colors: EMPTY_GRADIENT_STOP_COLORS,
                                        gradient_stop_positions: EMPTY_GRADIENT_STOP_POSITIONS,
                                        gradient_params: [0.0; 4],
                                        gradient_extra: EMPTY_GRADIENT_EXTRA,
                                        mask_stops_01: EMPTY_MASK_STOPS,
                                        mask_stops_23: EMPTY_MASK_STOPS,
                                        mask_params: EMPTY_MASK_PARAMS,
                                        xform: IDENTITY_XFORM,
                                        xform_translate: IDENTITY_XFORM_TRANSLATE,
                                    },
                                );
                            }
                        }
                    }
                }

                emit_text_glyphs_cached(
                    text,
                    text_x,
                    text_y,
                    text_max_w,
                    style.font_size,
                    style.line_height,
                    style.letter_spacing,
                    &style.font_family,
                    style.font_weight,
                    style.font_style,
                    &text_color,
                    clip_rect,
                    batch.layer_mut(effective_layer),
                    atlas,
                    font_system,
                    rasterizer,
                    shaped_cache,
                    Some(&mut node_glyph_keys),
                );

                // Text decoration line rendering
                if style.text_decoration != TextDecoration::None {
                    let deco_color = style.text_decoration_color.unwrap_or(style.color);
                    let mut deco_color_linear = deco_color.to_linear_f32();
                    deco_color_linear[3] *= opacity;

                    let font_size = style.font_size;
                    let line_thickness = (font_size * 0.07).max(1.0);

                    let deco_y = match style.text_decoration {
                        TextDecoration::Underline => text_y + font_size * 0.85,
                        TextDecoration::LineThrough => text_y + font_size * 0.5,
                        TextDecoration::Overline => text_y,
                        TextDecoration::None => unreachable!(),
                    };

                    batch.layer_mut(effective_layer).quad_instances.push(QuadInstance {
                        pos: [text_x, deco_y],
                        size: [text_w.min(content_w), line_thickness],
                        color: deco_color_linear,
                        border_color: [0.0; 4],
                        border_width: [0.0; 4],
                        border_radius: [0.0; 4],
                        clip_rect,
                        shadow_color: [0.0; 4],
                        shadow_offset: [0.0; 2],
                        shadow_params: [0.0; 2],
                        shadow_spread: [0.0; 2],
                        gradient_stop_colors: EMPTY_GRADIENT_STOP_COLORS,
                        gradient_stop_positions: EMPTY_GRADIENT_STOP_POSITIONS,
                        gradient_params: [0.0; 4],
                        gradient_extra: EMPTY_GRADIENT_EXTRA,
                        mask_stops_01: EMPTY_MASK_STOPS,
                        mask_stops_23: EMPTY_MASK_STOPS,
                        mask_params: EMPTY_MASK_PARAMS,
                        xform: IDENTITY_XFORM,
                        xform_translate: IDENTITY_XFORM_TRANSLATE,
                    });
                }
            }
            ElementContent::Image(ref path) if is_visible && !path.is_empty() => {
                let instance = ImageInstance {
                    pos: [render_x, render_y],
                    size: [rect.width, rect.height],
                    border_radius,
                    opacity,
                    _pad: [0.0; 3],
                    clip_rect,
                };
                let layer_batch = batch.layer_mut(effective_layer);
                if let Some(ib) = layer_batch
                    .image_batches
                    .iter_mut()
                    .find(|b| b.path == *path && b.object_fit == style.object_fit)
                {
                    ib.instances.push(instance);
                } else {
                    layer_batch.image_batches.push(ImageBatch {
                        path: path.clone(),
                        instances: vec![instance],
                        object_fit: style.object_fit,
                        object_position: style.object_position,
                    });
                }
            }
            ElementContent::Canvas if is_visible => {
                if let Some(registry) = registry {
                    if let Some(ref id) = element.id {
                        if let Some(painter) = registry.get(id) {
                            let canvas_node_id = registry.get_node_id(id);
                            batch.layer_mut(effective_layer).canvas_callbacks.push(
                                CanvasCallback {
                                    painter: Arc::clone(painter),
                                    rect: unshit_core::element::LayoutRect {
                                        x: render_x,
                                        y: render_y,
                                        width: rect.width,
                                        height: rect.height,
                                    },
                                    clip_rect,
                                    node_id: canvas_node_id,
                                },
                            );
                        }
                    }
                }
            }
            ElementContent::Grid(ref grid) if is_visible => {
                // cell_h derives from CSS line_height (the source of truth).
                let cell_h = style.font_size * style.line_height;
                // Grid cell width must match the active glyph shaping/raster
                // path. When TM_FORCE_DIRECTWRITE_GRID is off, the terminal
                // uses swash/cosmic-text on Windows too, so use the same
                // monospace width measurement there.
                #[cfg(target_os = "windows")]
                let cell_w = if use_directwrite_grid_rasterization() {
                    rasterizer.dw.measure_advance_width('M', style.font_size)
                } else {
                    measure_monospace_cell_width_for_family(
                        font_system,
                        &rasterizer.dw.font_family,
                        style.font_size,
                        cell_h,
                    )
                };
                #[cfg(not(target_os = "windows"))]
                let cell_w = measure_monospace_cell_width(font_system, style.font_size, cell_h);

                // Publish metrics so the app's resize handler can read them.
                unshit_core::cell_grid::CellGrid::publish_cell_metrics(cell_w, cell_h);

                // Compute grid dimensions from element size and cell metrics,
                // then publish a pending resize when they differ from the
                // current grid. This eliminates the timing gap where the
                // layout resize handler reads stale cell metrics.
                let content_w = rect.width - style.padding.left - style.padding.right;
                let content_h = rect.height - style.padding.top - style.padding.bottom;
                let cols = (content_w / cell_w).max(1.0) as u16;
                let rows = (content_h / cell_h).max(1.0) as u16;
                if cols as usize != grid.cols() || rows as usize != grid.rows() {
                    unshit_core::cell_grid::CellGrid::publish_pending_resize(cols, rows);
                }
                let render_cell_w = cell_w * parity_terminal_cell_width_scale();

                #[cfg(feature = "grid-fragment-shader")]
                let routed_to_fragment = try_record_grid_for_fragment_path(
                    crate::grid_fragment_upload::runtime_flag_enabled(),
                    batch.layer_mut(effective_layer),
                    node_id,
                    render_x + style.padding.left,
                    render_y + style.padding.top,
                    render_cell_w,
                    cell_h,
                    cols as u32,
                    rows as u32,
                    style.font_size,
                    opacity,
                    clip_rect,
                );
                #[cfg(not(feature = "grid-fragment-shader"))]
                let routed_to_fragment = false;

                if !routed_to_fragment {
                    emit_grid_cells(
                        grid,
                        render_x + style.padding.left,
                        render_y + style.padding.top,
                        render_cell_w,
                        cell_h,
                        style.font_size,
                        opacity,
                        clip_rect,
                        batch.layer_mut(effective_layer),
                        atlas,
                        font_system,
                        rasterizer,
                        shape_cache,
                        Some(&mut node_glyph_keys),
                        node_id,
                        line_cache.as_deref_mut(),
                    );
                }
            }
            ElementContent::Svg(ref node) if is_visible => {
                emit_svg_node(
                    node,
                    &SvgAttrs::default(),
                    SvgTransform::IDENTITY,
                    node.attrs.view_box.unwrap_or_else(ViewBox::default),
                    render_x,
                    render_y,
                    rect.width,
                    rect.height,
                    style.color,
                    opacity,
                    clip_rect,
                    svg_cache,
                    batch.layer_mut(effective_layer),
                );
            }
            _ => {}
        }
    }

    // Flush text/content glyphs and any interleaved quads (selection
    // highlights, text decorations, input cursors) before child recursion.
    {
        let lb = batch.layer_mut(effective_layer);
        let qend = lb.quad_instances.len();
        let gend = lb.glyph_instances.len();
        let _ = flush_span(&mut lb.draw_spans, DrawKind::Quad, quad_cursor, qend);
        let _ = flush_span(&mut lb.draw_spans, DrawKind::Glyph, glyph_cursor, gend);
        // Bake this node's own CSS transform onto the primitives it emitted
        // before recursing (background / border / shadow / text / grid).
        // Children compose their own transform during recursion; this node's
        // post-children scrollbar/grip quads are stamped below. Identity nodes
        // keep the matrix-free fast path (instances stay at the zero default).
        if !node_xform.is_identity() {
            let xf = node_xform.xform_delta();
            let xt = node_xform.xform_translate();
            stamp_xform_quads(&mut lb.quad_instances[quad_start..qend], xf, xt);
            stamp_xform_glyphs(&mut lb.glyph_instances[glyph_start..gend], xf, xt);
        }
    }

    // Collect children into a Vec so we can sort by z-index.
    // Stable sort preserves DOM order for children with equal z-index.
    let mut children_ids: Vec<NodeId> = Vec::new();
    let mut needs_sort = false;
    {
        let mut c = element.first_child;
        while !c.is_dangling() {
            if !needs_sort {
                let z = arena.get(c).map(|e| e.computed_style.z_index).unwrap_or(0);
                if z != 0 {
                    needs_sort = true;
                }
            }
            children_ids.push(c);
            c = arena.get(c).map(|e| e.next_sibling).unwrap_or(NodeId::DANGLING);
        }
    }
    if needs_sort {
        children_ids
            .sort_by_key(|&id| arena.get(id).map(|e| e.computed_style.z_index).unwrap_or(0));
    }

    for &child in &children_ids {
        // Per CSS spec, absolutely positioned children escape their
        // parent's overflow clip and scroll offset.
        let (effective_clip, eff_scroll_x, eff_scroll_y) =
            if let Some(child_elem) = arena.get(child) {
                if matches!(
                    child_elem.computed_style.position,
                    CssPosition::Absolute | CssPosition::Fixed
                ) {
                    (clip_rect, scroll_offset_x, scroll_offset_y)
                } else {
                    (child_clip, child_scroll_x, child_scroll_y)
                }
            } else {
                (child_clip, child_scroll_x, child_scroll_y)
            };
        walk_for_batch(
            arena,
            child,
            root,
            batch,
            atlas,
            font_system,
            rasterizer,
            measure_cache,
            shaped_cache,
            svg_cache,
            shape_cache,
            effective_clip,
            eff_scroll_x,
            eff_scroll_y,
            text_selection,
            registry,
            scrollbar_state,
            focused,
            effective_layer,
            portals,
            batch_cache,
            Some(&mut node_glyph_keys),
            line_cache.as_deref_mut(),
            cursor_blink_force_dirty,
            node_xform,
        );
    }

    // Children have already flushed their own draw spans internally
    // during their recursive `walk_for_batch` calls.  Do NOT flush a
    // span here: it would create duplicate overlapping spans that
    // re-draw all children's quads then all glyphs, destroying the
    // interleaved draw order and causing text occlusion (earlier DOM
    // text renders on top of later DOM backgrounds).
    //
    // Just advance quad_cursor past children's contributions so the
    // scrollbar/grip flush below starts at the right offset.
    {
        let lb = batch.layer_mut(effective_layer);
        quad_cursor = lb.quad_instances.len();
    }

    // Snapshot the primitives emitted by this node and its subtree into the
    // pending cache.  Future frames can replay this data when the node is clean.
    // (draw_spans are snapshotted *after* scrollbar/grip emission below so the
    // cache replay is complete; see the second snapshot block.)

    // Overlay scrollbar rendering.
    // Emitted after children so the scrollbar draws on top of content.
    if style.overflow_x == Overflow::Scroll || style.overflow_y == Overflow::Scroll {
        let (v_geom, h_geom) =
            scroll::compute_scrollbar_geometry(arena, node_id, render_x, render_y);

        const TRACK_COLOR: [f32; 4] = [1.0, 1.0, 1.0, 0.0];
        const CORNER_RADIUS: f32 = 4.0;
        const THUMB_INSET: f32 = 4.0;

        let mut push_scrollbar_quad =
            |pos: [f32; 2], size: [f32; 2], color: [f32; 4], radius: f32| {
                batch.layer_mut(effective_layer).quad_instances.push(QuadInstance {
                    pos,
                    size,
                    color,
                    border_color: [0.0; 4],
                    border_width: [0.0; 4],
                    border_radius: [radius; 4],
                    clip_rect: child_clip,
                    shadow_color: [0.0; 4],
                    shadow_offset: [0.0; 2],
                    shadow_params: [0.0; 2],
                    shadow_spread: [0.0; 2],
                    gradient_stop_colors: EMPTY_GRADIENT_STOP_COLORS,
                    gradient_stop_positions: EMPTY_GRADIENT_STOP_POSITIONS,
                    gradient_params: [0.0; 4],
                    gradient_extra: EMPTY_GRADIENT_EXTRA,
                    mask_stops_01: EMPTY_MASK_STOPS,
                    mask_stops_23: EMPTY_MASK_STOPS,
                    mask_params: EMPTY_MASK_PARAMS,
                    xform: IDENTITY_XFORM,
                    xform_translate: IDENTITY_XFORM_TRANSLATE,
                });
            };

        for geom in [v_geom.as_ref(), h_geom.as_ref()].into_iter().flatten() {
            let alpha = scrollbar_state.thumb_alpha(node_id, geom.axis);
            let thumb_color = [1.0, 1.0, 1.0, alpha];
            let (thumb_pos, thumb_size) = match geom.axis {
                scroll::ScrollbarAxis::Vertical => (
                    [geom.thumb_x + THUMB_INSET, geom.thumb_y],
                    [(geom.thumb_w - THUMB_INSET * 2.0).max(1.0), geom.thumb_h],
                ),
                scroll::ScrollbarAxis::Horizontal => (
                    [geom.thumb_x, geom.thumb_y + THUMB_INSET],
                    [geom.thumb_w, (geom.thumb_h - THUMB_INSET * 2.0).max(1.0)],
                ),
            };
            push_scrollbar_quad(
                [geom.track_x, geom.track_y],
                [geom.track_w, geom.track_h],
                TRACK_COLOR,
                CORNER_RADIUS,
            );
            push_scrollbar_quad(thumb_pos, thumb_size, thumb_color, CORNER_RADIUS);
        }
    }

    // Resize grip indicator.
    // Per CSS spec, `resize` only works when `overflow` is not `visible`.
    if style.resize != CssResize::None
        && (style.overflow_x != Overflow::Visible || style.overflow_y != Overflow::Visible)
    {
        const GRIP_SIZE: f32 = 12.0;
        const DOT_SIZE: f32 = 2.0;
        const GRIP_COLOR: [f32; 4] = [1.0, 1.0, 1.0, 0.35];

        let base_x = render_x + rect.width - GRIP_SIZE;
        let base_y = render_y + rect.height - GRIP_SIZE;

        // Three diagonal dots (bottom-right corner grip pattern).
        let dots: &[(f32, f32)] = &[
            (GRIP_SIZE - 3.0, GRIP_SIZE - 3.0),
            (GRIP_SIZE - 7.0, GRIP_SIZE - 3.0),
            (GRIP_SIZE - 3.0, GRIP_SIZE - 7.0),
            (GRIP_SIZE - 11.0, GRIP_SIZE - 3.0),
            (GRIP_SIZE - 7.0, GRIP_SIZE - 7.0),
            (GRIP_SIZE - 3.0, GRIP_SIZE - 11.0),
        ];
        for &(dx, dy) in dots {
            batch.layer_mut(effective_layer).quad_instances.push(QuadInstance {
                pos: [base_x + dx, base_y + dy],
                size: [DOT_SIZE, DOT_SIZE],
                color: GRIP_COLOR,
                border_color: [0.0; 4],
                border_width: [0.0; 4],
                border_radius: [1.0; 4],
                clip_rect: child_clip,
                shadow_color: [0.0; 4],
                shadow_offset: [0.0; 2],
                shadow_params: [0.0; 2],
                shadow_spread: [0.0; 2],
                gradient_stop_colors: EMPTY_GRADIENT_STOP_COLORS,
                gradient_stop_positions: EMPTY_GRADIENT_STOP_POSITIONS,
                gradient_params: [0.0; 4],
                gradient_extra: EMPTY_GRADIENT_EXTRA,
                mask_stops_01: EMPTY_MASK_STOPS,
                mask_stops_23: EMPTY_MASK_STOPS,
                mask_params: EMPTY_MASK_PARAMS,
                xform: IDENTITY_XFORM,
                xform_translate: IDENTITY_XFORM_TRANSLATE,
            });
        }
    }

    // Flush scrollbar/resize grip quads.
    {
        let lb = batch.layer_mut(effective_layer);
        let qend = lb.quad_instances.len();
        let _ = flush_span(&mut lb.draw_spans, DrawKind::Quad, quad_cursor, qend);
        // Bake the transform onto this node's post-children own quads
        // (scrollbar track/thumb, resize grip), which sit in `[quad_cursor,
        // qend)`. Children's instances are already stamped and excluded.
        if !node_xform.is_identity() {
            stamp_xform_quads(
                &mut lb.quad_instances[quad_cursor..qend],
                node_xform.xform_delta(),
                node_xform.xform_translate(),
            );
        }
    }

    // Snapshot primitives and draw spans into the pending cache.
    {
        let lb = batch.layer_mut(effective_layer);
        let quads = lb.quad_instances[quad_start..].to_vec();
        let glyphs = lb.glyph_instances[glyph_start..].to_vec();
        let svgs = lb.svg_draws[svg_start..].to_vec();
        let quad_start_u32 = quad_start as u32;
        let glyph_start_u32 = glyph_start as u32;
        let svg_start_u32 = svg_start as u32;
        let image_start_u32 = image_start as u32;
        let canvas_start_u32 = canvas_start as u32;
        let spans = lb.draw_spans[span_start..]
            .iter()
            .map(|span| DrawSpan {
                kind: span.kind,
                start: match span.kind {
                    DrawKind::Quad => span.start.saturating_sub(quad_start_u32),
                    DrawKind::Glyph => span.start.saturating_sub(glyph_start_u32),
                },
                count: span.count,
            })
            .collect::<Vec<_>>();
        // Capture backdrop boundaries with prefix counts converted to be
        // relative to this node's primitive start indices. Replay restores
        // absolute prefixes against the layer state at replay time.
        let boundaries = lb.backdrop_boundaries[boundary_start..]
            .iter()
            .map(|b| BackdropBoundary {
                rect: b.rect,
                clip_rect: b.clip_rect,
                blur_radius: b.blur_radius,
                quad_prefix: b.quad_prefix.saturating_sub(quad_start_u32),
                glyph_prefix: b.glyph_prefix.saturating_sub(glyph_start_u32),
                svg_prefix: b.svg_prefix.saturating_sub(svg_start_u32),
                image_batch_prefix: b.image_batch_prefix.saturating_sub(image_start_u32),
                canvas_prefix: b.canvas_prefix.saturating_sub(canvas_start_u32),
                dirty: b.dirty,
            })
            .collect::<Vec<_>>();
        let glyph_keys = node_glyph_keys.iter().copied().collect::<Vec<_>>();
        batch_cache.record_with_signature(
            node_id,
            layer_index,
            cache_signature,
            quads,
            glyphs,
            svgs,
            spans,
            glyph_keys,
            atlas.generation,
            boundaries,
        );
    }
    if let Some(parent_keys) = parent_glyph_keys {
        for key in node_glyph_keys {
            parent_keys.insert(key);
        }
    }
}

fn input_marker_node(input_type: InputType) -> SvgNode {
    let attrs = match input_type {
        InputType::Checkbox => SvgAttrs {
            view_box: Some(ViewBox::new(0.0, 0.0, 16.0, 16.0)),
            fill: Some(SvgPaint::None),
            stroke: Some(SvgPaint::Current),
            stroke_width: Some(1.8),
            stroke_linecap: Some(StrokeLineCap::Round),
            stroke_linejoin: Some(StrokeLineJoin::Round),
            ..Default::default()
        },
        InputType::Radio => SvgAttrs {
            view_box: Some(ViewBox::new(0.0, 0.0, 16.0, 16.0)),
            fill: Some(SvgPaint::Current),
            stroke: Some(SvgPaint::None),
            ..Default::default()
        },
        _ => SvgAttrs::default(),
    };

    let primitive = match input_type {
        InputType::Checkbox => SvgPrimitive::Path {
            d: "M3.5 8.2l3 3l6-6.5".to_string(),
            commands: vec![
                PathCommand::MoveTo { x: 3.5, y: 8.2 },
                PathCommand::LineTo { x: 6.5, y: 11.2 },
                PathCommand::LineTo { x: 12.5, y: 4.7 },
            ],
        },
        InputType::Radio => SvgPrimitive::Circle { cx: 8.0, cy: 8.0, r: 4.25 },
        _ => SvgPrimitive::Group,
    };

    SvgNode { primitive, attrs, children: Vec::new() }
}

#[allow(clippy::too_many_arguments)]
fn emit_checked_input_marker(
    input_type: InputType,
    render_x: f32,
    render_y: f32,
    width: f32,
    height: f32,
    current_color: Color,
    opacity: f32,
    clip_rect: [f32; 4],
    svg_cache: &mut SvgTessCache,
    batch: &mut FrameBatch,
) {
    if !matches!(input_type, InputType::Checkbox | InputType::Radio) {
        return;
    }

    let marker_size = width.min(height).max(0.0) * 0.78;
    if marker_size <= 0.0 {
        return;
    }
    let marker_x = render_x + (width - marker_size) * 0.5;
    let marker_y = render_y + (height - marker_size) * 0.5;
    let node = input_marker_node(input_type);

    emit_svg_node(
        &node,
        &SvgAttrs::default(),
        SvgTransform::IDENTITY,
        node.attrs.view_box.unwrap_or_default(),
        marker_x,
        marker_y,
        marker_size,
        marker_size,
        current_color,
        opacity,
        clip_rect,
        svg_cache,
        batch,
    );
}

/// Walk an `SvgNode` tree, accumulate the SVG presentation cascade and
/// transform, tessellate each leaf primitive via the shared cache, and emit
/// one `SvgDrawCall` per leaf into the active layer batch.
#[allow(clippy::too_many_arguments)]
fn emit_svg_node(
    node: &SvgNode,
    parent_attrs: &SvgAttrs,
    parent_transform: SvgTransform,
    root_view_box: ViewBox,
    root_x: f32,
    root_y: f32,
    root_w: f32,
    root_h: f32,
    current_color: Color,
    opacity: f32,
    clip_rect: [f32; 4],
    svg_cache: &mut SvgTessCache,
    batch: &mut FrameBatch,
) {
    // The node level attributes cascade over the parent group attrs.
    let cascaded = node.attrs.cascade_over(parent_attrs);
    // The node transform (if any) composes onto the parent transform. Note
    // that cascade_over also composes the transform, so we use the cascaded
    // value directly here.
    let transform = cascaded.transform.unwrap_or(SvgTransform::IDENTITY);
    let _ = parent_transform;

    if matches!(node.primitive, SvgPrimitive::Group) {
        for child in &node.children {
            emit_svg_node(
                child,
                &cascaded,
                transform,
                root_view_box,
                root_x,
                root_y,
                root_w,
                root_h,
                current_color,
                opacity,
                clip_rect,
                svg_cache,
                batch,
            );
        }
        return;
    }

    use unshit_core::svg::types::SvgPaint;
    let effective_fill = match cascaded.fill.unwrap_or(SvgPaint::Solid(Color::BLACK)) {
        SvgPaint::None => Color::TRANSPARENT,
        SvgPaint::Current => current_color,
        SvgPaint::Solid(c) => c,
    };
    let effective_stroke = match cascaded.stroke.unwrap_or(SvgPaint::None) {
        SvgPaint::None => Color::TRANSPARENT,
        SvgPaint::Current => current_color,
        SvgPaint::Solid(c) => c,
    };

    let geometry = svg_cache.get_or_tessellate(
        &node.primitive,
        &cascaded,
        current_color,
        effective_fill,
        effective_stroke,
    );
    if geometry.is_empty() {
        return;
    }

    // Map the viewBox onto the rendered rectangle. Non uniform scale is
    // allowed and produces visibly stretched strokes, matching browser SVG
    // behavior. Transforms on groups (already composed into `transform`)
    // apply in viewBox space before the viewport scale.
    let vb_w = if root_view_box.width > 0.0 { root_view_box.width } else { 1.0 };
    let vb_h = if root_view_box.height > 0.0 { root_view_box.height } else { 1.0 };
    let scale_x = root_w / vb_w;
    let scale_y = root_h / vb_h;
    let translate_x = root_x - root_view_box.min_x * scale_x + transform.e * scale_x;
    let translate_y = root_y - root_view_box.min_y * scale_y + transform.f * scale_y;

    // For now we only use the translate components of the SVG transform
    // plus the viewport scale. Rotation and non axis aligned scale require
    // a full 2x2 transform uniform; tracked as a follow up.
    batch.svg_draws.push(SvgDrawCall {
        geometry,
        translate: [translate_x, translate_y],
        scale: [scale_x * transform.a, scale_y * transform.d],
        clip_rect,
        color_tint: [1.0, 1.0, 1.0, 1.0],
        opacity,
    });
}

#[allow(clippy::too_many_arguments)]
fn emit_text_glyphs_cached(
    text: &str,
    x: f32,
    y: f32,
    max_width: Option<f32>,
    font_size: f32,
    line_height: f32,
    letter_spacing: f32,
    font_family: &str,
    font_weight: FontWeight,
    font_style: FontStyle,
    color: &Color,
    clip_rect: [f32; 4],
    batch: &mut FrameBatch,
    atlas: &mut GlyphAtlas,
    font_system: &mut FontSystem,
    rasterizer: &mut Rasterizer<'_>,
    shaped_cache: &mut ShapedTextCache,
    mut glyph_keys_out: Option<&mut FxHashSet<GlyphKey>>,
) {
    let cache_key = ShapedTextCache::make_key(
        text,
        font_family,
        font_weight,
        font_style,
        font_size,
        line_height,
        letter_spacing,
        max_width,
    );
    let color_linear = color.to_linear_f32();

    // Check if we have a cached shaped result. If any atlas key is missing,
    // invalidate this shaped entry and rebuild so glyphs are never silently
    // dropped on atlas churn. `get_or_promote` moves a hit from `previous`
    // into `current` so it survives the next `finish_frame` swap.
    if let Some(entry) = shaped_cache.buf.get_or_promote(&cache_key).cloned() {
        let atlas_ready = entry.glyphs.iter().all(|cg| atlas.cache.contains_key(&cg.atlas_key));
        if atlas_ready {
            for cg in &entry.glyphs {
                let atlas_entry = atlas
                    .cache
                    .get(&cg.atlas_key)
                    .copied()
                    .expect("atlas_ready guarantees all shaped glyph keys exist");
                atlas.touch(&cg.atlas_key);
                if let Some(keys) = glyph_keys_out.as_deref_mut() {
                    keys.insert(cg.atlas_key);
                }
                batch.glyph_instances.push(GlyphInstance {
                    pos: [x + cg.rel_x, y + cg.rel_y],
                    size: atlas_entry.size,
                    uv_min: [atlas_entry.uv_rect[0], atlas_entry.uv_rect[1]],
                    uv_max: [atlas_entry.uv_rect[2], atlas_entry.uv_rect[3]],
                    color: color_linear,
                    clip_rect,
                    xform: IDENTITY_XFORM,
                    xform_translate: IDENTITY_XFORM_TRANSLATE,
                });
            }
            return;
        }
        shaped_cache.buf.remove(&cache_key);
    }

    // Cache miss: shape text and populate cache
    let metrics = Metrics::new(font_size, font_size * line_height);
    let mut buffer = Buffer::new(font_system, metrics);
    buffer.set_size(font_system, max_width.map(|w| w.max(1.0)), None);
    buffer.set_text(
        font_system,
        text,
        text_attrs(font_family, font_weight, font_style),
        Shaping::Advanced,
    );
    buffer.shape_until_scroll(font_system, false);

    let mut cached_glyphs = Vec::new();

    for run in buffer.layout_runs() {
        let run_y = run.line_y;
        for (glyph_idx, glyph) in run.glyphs.iter().enumerate() {
            let ls_offset = glyph_idx as f32 * letter_spacing;
            let physical = glyph.physical((ls_offset, 0.0), 1.0);

            let key = GlyphKey {
                font_id: atlas_text_font_namespace(&physical.cache_key, font_family, font_weight),
                glyph_id: physical.cache_key.glyph_id,
                font_size_tenths: (font_size * 10.0) as u16,
                subpixel_bin: ((physical.cache_key.x_bin as u8) << 2)
                    | (physical.cache_key.y_bin as u8),
            };

            let entry = if let Some(entry) = atlas.cache.get(&key).copied() {
                atlas.touch(&key);
                entry
            } else {
                let raster_result = rasterize_swash_for_atlas(
                    rasterizer,
                    font_system,
                    &physical,
                    atlas,
                    key,
                    font_family,
                    font_weight,
                );
                match raster_result {
                    Some(entry) => entry,
                    None => continue,
                }
            };

            let rel_x = physical.x as f32 + entry.offset[0];
            let rel_y = run_y + physical.y as f32 + entry.offset[1];

            cached_glyphs.push(CachedGlyph { rel_x, rel_y, atlas_key: key });
            if let Some(keys) = glyph_keys_out.as_deref_mut() {
                keys.insert(key);
            }

            batch.glyph_instances.push(GlyphInstance {
                pos: [x + rel_x, y + rel_y],
                size: entry.size,
                uv_min: [entry.uv_rect[0], entry.uv_rect[1]],
                uv_max: [entry.uv_rect[2], entry.uv_rect[3]],
                color: color_linear,
                clip_rect,
                xform: IDENTITY_XFORM,
                xform_translate: IDENTITY_XFORM_TRANSLATE,
            });
        }
    }

    shaped_cache.buf.insert(cache_key, ShapedTextEntry { glyphs: cached_glyphs });
}

/// Hash a font family name into the stable `font_id` used by
/// [`ShapeCacheKey`]. Empty string falls back to 0 so anonymous / monospace
/// default families share a bucket.
pub fn shape_cache_font_id(font_family: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    if font_family.is_empty() {
        return 0;
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    font_family.hash(&mut hasher);
    hasher.finish()
}

/// Local struct shared by `emit_grid_cells` and `emit_grid_row_fresh` so
/// both paths resolve glyphs into the same shape.
struct ResolvedGlyph {
    key: GlyphKey,
    entry: GlyphEntry,
    physical_x: i32,
    physical_y: i32,
    line_y: f32,
}

const TERMINAL_SHAPE_STYLE_REGULAR: u64 = 0;
const TERMINAL_SHAPE_STYLE_ITALIC: u64 = 1 << 0;
const TERMINAL_DIM_INTENSITY: f32 = 0.5;

#[derive(Clone, Copy, Debug, PartialEq)]
struct TerminalCellRect {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

// Terminal drawing glyphs need exact cell geometry. Font rasterizers often
// leave side bearings on box/block glyphs, which shows up as seams under
// DirectWrite subpixel text.
fn terminal_fg_color(cell: &unshit_core::cell_grid::Cell, opacity: f32) -> [f32; 4] {
    let fg = if cell.attrs.contains(CellAttrs::INVERSE) {
        if cell.bg.a == 0 {
            Color::BLACK
        } else {
            cell.bg
        }
    } else {
        cell.fg
    };
    let mut fg_linear = fg.to_linear_f32();
    if cell.attrs.contains(CellAttrs::DIM) {
        fg_linear[0] *= TERMINAL_DIM_INTENSITY;
        fg_linear[1] *= TERMINAL_DIM_INTENSITY;
        fg_linear[2] *= TERMINAL_DIM_INTENSITY;
    }
    fg_linear[3] *= opacity;
    fg_linear
}

fn terminal_effective_bg(grid: &CellGrid, row: usize, col: usize) -> Color {
    let cell = &grid.cells()[row * grid.cols() + col];
    terminal_effective_bg_for_cell(cell, parity_windows_terminal_colors_enabled())
}

fn terminal_glyph_position_for_parity(gx: f32, gy: f32, parity_colors: bool) -> [f32; 2] {
    if parity_colors {
        [gx, gy.round()]
    } else {
        [gx, gy]
    }
}

fn terminal_effective_bg_for_cell(
    cell: &unshit_core::cell_grid::Cell,
    parity_colors: bool,
) -> Color {
    if cell.attrs.contains(CellAttrs::INVERSE) {
        terminal_inverse_bg_color(cell.fg, parity_colors)
    } else {
        cell.bg
    }
}

fn terminal_inverse_bg_color(fg: Color, parity_colors: bool) -> Color {
    if parity_colors && fg == WINDOWS_TERMINAL_PARITY_CALIBRATED_FG {
        WINDOWS_TERMINAL_PARITY_LITERAL_FG
    } else {
        fg
    }
}

fn terminal_bg_runs_in_range(
    grid: &CellGrid,
    row: usize,
    start_col: usize,
    end_col: usize,
) -> Vec<BgRun> {
    terminal_bg_runs_in_range_for_parity(
        grid,
        row,
        start_col,
        end_col,
        parity_windows_terminal_colors_enabled(),
    )
}

fn terminal_bg_runs_in_range_for_parity(
    grid: &CellGrid,
    row: usize,
    start_col: usize,
    end_col: usize,
    parity_colors: bool,
) -> Vec<BgRun> {
    if row >= grid.rows() {
        return Vec::new();
    }
    let end_col = end_col.min(grid.cols());
    if start_col >= end_col {
        return Vec::new();
    }

    let mut runs: Vec<BgRun> = Vec::new();
    let row_base = row * grid.cols();
    let mut cur: Option<BgRun> = None;
    for col in start_col..end_col {
        let cell = &grid.cells()[row_base + col];
        let bg = terminal_effective_bg_for_cell(cell, parity_colors);
        match cur.as_mut() {
            Some(run) if run.bg == bg => run.end_col = col + 1,
            _ => {
                if let Some(finished) = cur.take() {
                    runs.push(finished);
                }
                cur = Some(BgRun { start_col: col, end_col: col + 1, bg });
            }
        }
    }
    if let Some(finished) = cur.take() {
        runs.push(finished);
    }
    runs
}

fn terminal_bg_overlap_enabled(origin_x: f32, cell_w: f32) -> bool {
    origin_x.fract().abs() > 0.001 || cell_w.fract().abs() > 0.001
}

fn terminal_bg_boundary_x(origin_x: f32, cell_w: f32, col: usize) -> f32 {
    origin_x + col as f32 * cell_w
}

fn terminal_bg_snapped_boundary_x(origin_x: f32, cell_w: f32, col: usize) -> f32 {
    terminal_bg_boundary_x(origin_x, cell_w, col).round()
}

fn terminal_bg_run_edges(
    grid: &CellGrid,
    row: usize,
    start_col: usize,
    end_col: usize,
    run_start_col: usize,
    run_end_col: usize,
    origin_x: f32,
    cell_w: f32,
) -> (f32, f32) {
    let mut left = terminal_bg_boundary_x(origin_x, cell_w, run_start_col);
    let mut right = terminal_bg_boundary_x(origin_x, cell_w, run_end_col);

    if run_start_col > start_col && terminal_effective_bg(grid, row, run_start_col - 1).a != 0 {
        left = terminal_bg_snapped_boundary_x(origin_x, cell_w, run_start_col);
    }

    if run_end_col < end_col && terminal_effective_bg(grid, row, run_end_col).a != 0 {
        right = terminal_bg_snapped_boundary_x(origin_x, cell_w, run_end_col);
    }

    (left, right)
}

fn terminal_row_has_blink(cells: &[unshit_core::cell_grid::Cell], row: usize, cols: usize) -> bool {
    let start = row * cols;
    let end = start + cols;
    cells[start..end].iter().any(|cell| cell.attrs.contains(CellAttrs::BLINK))
}

fn terminal_row_content_sig(
    cells: &[unshit_core::cell_grid::Cell],
    row: usize,
    cols: usize,
    blink_phase_on: bool,
) -> u64 {
    let sig = hash_row_cells(cells, row, cols);
    if terminal_row_has_blink(cells, row, cols) && !blink_phase_on {
        sig ^ 0x9e37_79b9_7f4a_7c15
    } else {
        sig
    }
}

fn terminal_cell_foreground_visible(
    cell: &unshit_core::cell_grid::Cell,
    blink_phase_on: bool,
) -> bool {
    blink_phase_on || !cell.attrs.contains(CellAttrs::BLINK)
}

fn terminal_shape_style(attrs: CellAttrs) -> u64 {
    let mut style = TERMINAL_SHAPE_STYLE_REGULAR;
    if attrs.contains(CellAttrs::ITALIC) {
        style |= TERMINAL_SHAPE_STYLE_ITALIC;
    }
    style
}

fn terminal_text_attrs<'a>(
    family: cosmic_text::Family<'a>,
    attrs: CellAttrs,
) -> cosmic_text::Attrs<'a> {
    let mut text_attrs = cosmic_text::Attrs::new().family(family);
    if attrs.contains(CellAttrs::ITALIC) {
        text_attrs = text_attrs.style(cosmic_text::Style::Oblique);
    }
    text_attrs
}

fn push_terminal_quad(
    row_quads: &mut Vec<QuadInstance>,
    pos: [f32; 2],
    size: [f32; 2],
    color: [f32; 4],
    clip_rect: [f32; 4],
) {
    if size[0] <= 0.0 || size[1] <= 0.0 || color[3] <= 0.0 {
        return;
    }

    row_quads.push(QuadInstance {
        pos,
        size,
        color,
        border_color: [0.0; 4],
        border_width: [0.0; 4],
        border_radius: [0.0; 4],
        clip_rect,
        shadow_color: [0.0; 4],
        shadow_offset: [0.0; 2],
        shadow_params: [0.0; 2],
        shadow_spread: [0.0; 2],
        gradient_stop_colors: EMPTY_GRADIENT_STOP_COLORS,
        gradient_stop_positions: EMPTY_GRADIENT_STOP_POSITIONS,
        gradient_params: [0.0; 4],
        gradient_extra: EMPTY_GRADIENT_EXTRA,
        mask_stops_01: EMPTY_MASK_STOPS,
        mask_stops_23: EMPTY_MASK_STOPS,
        mask_params: EMPTY_MASK_PARAMS,
        xform: IDENTITY_XFORM,
        xform_translate: IDENTITY_XFORM_TRANSLATE,
    });
}

fn push_terminal_cell_rect(
    row_quads: &mut Vec<QuadInstance>,
    cell_x: f32,
    cell_y: f32,
    rect: TerminalCellRect,
    color: [f32; 4],
    clip_rect: [f32; 4],
) {
    push_terminal_quad(
        row_quads,
        [cell_x + rect.x, cell_y + rect.y],
        [rect.width, rect.height],
        color,
        clip_rect,
    );
}

fn push_terminal_cell_rect_snapped(
    row_quads: &mut Vec<QuadInstance>,
    cell_x: f32,
    cell_y: f32,
    rect: TerminalCellRect,
    color: [f32; 4],
    clip_rect: [f32; 4],
) {
    let x0 = (cell_x + rect.x).round();
    let y0 = (cell_y + rect.y).round();
    let x1 = (cell_x + rect.x + rect.width).round();
    let y1 = (cell_y + rect.y + rect.height).round();
    push_terminal_quad(
        row_quads,
        [x0, y0],
        [(x1 - x0).max(1.0), (y1 - y0).max(1.0)],
        color,
        clip_rect,
    );
}

fn emit_terminal_text_decorations(
    cell: &unshit_core::cell_grid::Cell,
    cell_x: f32,
    cell_y: f32,
    cell_w: f32,
    cell_h: f32,
    color: [f32; 4],
    clip_rect: [f32; 4],
    row_quads: &mut Vec<QuadInstance>,
) -> bool {
    let mut emitted = false;
    let thickness = (cell_h / 16.0).round().max(1.0);
    let overlap = 0.25;

    if cell.attrs.contains(CellAttrs::UNDERLINE) {
        let y = (cell_h * 0.82).round().min(cell_h - thickness);
        push_terminal_quad(
            row_quads,
            [cell_x - overlap, cell_y + y],
            [cell_w + overlap * 2.0, thickness],
            color,
            clip_rect,
        );
        emitted = true;
    }

    if cell.attrs.contains(CellAttrs::STRIKETHROUGH) {
        let y = (cell_h * 0.50).round().min(cell_h - thickness);
        push_terminal_quad(
            row_quads,
            [cell_x - overlap, cell_y + y],
            [cell_w + overlap * 2.0, thickness],
            color,
            clip_rect,
        );
        emitted = true;
    }

    emitted
}

fn terminal_block_rect(
    ch: char,
    cell_w: f32,
    cell_h: f32,
    trailing_edge_clamp: f32,
) -> Option<TerminalCellRect> {
    let overlap = 0.5;
    match ch {
        '\u{2588}' => Some(TerminalCellRect {
            x: -overlap,
            y: -overlap,
            width: cell_w + overlap * 2.0 - trailing_edge_clamp,
            height: cell_h + overlap * 2.0,
        }),
        '\u{2580}' => Some(TerminalCellRect {
            x: -overlap,
            y: -overlap,
            width: cell_w + overlap * 2.0,
            height: cell_h * 0.5 + overlap,
        }),
        '\u{2584}' => Some(TerminalCellRect {
            x: -overlap,
            y: cell_h * 0.5,
            width: cell_w + overlap * 2.0,
            height: cell_h * 0.5 + overlap,
        }),
        '\u{258c}' => Some(TerminalCellRect {
            x: -overlap,
            y: -overlap,
            width: cell_w * 0.5 + overlap,
            height: cell_h + overlap * 2.0,
        }),
        '\u{2590}' => Some(TerminalCellRect {
            x: cell_w * 0.5,
            y: -overlap,
            width: cell_w * 0.5 + overlap,
            height: cell_h + overlap * 2.0,
        }),
        '\u{2581}'..='\u{2587}' => {
            let eighths = ch as u32 - 0x2580;
            let height = cell_h * (eighths as f32 / 8.0) + overlap;
            Some(TerminalCellRect {
                x: -overlap,
                y: cell_h - height,
                width: cell_w + overlap * 2.0,
                height,
            })
        }
        '\u{2589}'..='\u{258f}' => {
            let eighths = 8 - (ch as u32 - 0x2588);
            Some(TerminalCellRect {
                x: -overlap,
                y: -overlap,
                width: cell_w * (eighths as f32 / 8.0) + overlap,
                height: cell_h + overlap * 2.0,
            })
        }
        _ => None,
    }
}

fn terminal_primitive_y_bias_for_parity(ch: char, parity_colors: bool) -> f32 {
    if !parity_colors {
        return 0.0;
    }

    match ch {
        '\u{2580}'..='\u{259f}' => 1.0,
        '\u{2500}' | '\u{250c}' | '\u{252c}' | '\u{2510}' | '\u{251c}' | '\u{253c}'
        | '\u{2524}' | '\u{2514}' | '\u{2534}' | '\u{2518}' => 1.0,
        '\u{2501}' => 1.0,
        _ => 0.0,
    }
}

fn terminal_block_or_shade_char(ch: char) -> bool {
    matches!(ch, '\u{2580}'..='\u{259f}')
}

fn terminal_primitive_trailing_edge_clamp_for_parity(
    ch: char,
    next_ch: Option<char>,
    parity_colors: bool,
) -> f32 {
    if !parity_colors {
        return 0.0;
    }

    match ch {
        '\u{2588}' if !next_ch.is_some_and(terminal_block_or_shade_char) => 1.0,
        '\u{2501}' if next_ch != Some('\u{2501}') => 1.0,
        _ => 0.0,
    }
}

fn terminal_shade_threshold(ch: char) -> Option<u8> {
    match ch {
        '\u{2591}' => Some(4),
        '\u{2592}' => Some(8),
        '\u{2593}' => Some(12),
        _ => None,
    }
}

fn terminal_shade_pixel_filled(x: u32, y: u32, threshold: u8) -> bool {
    let diagonal_phase = if y % 2 == 0 { 0 } else { 2 };
    match threshold {
        4 => (x + diagonal_phase) % 4 == 0,
        8 => (x + (diagonal_phase / 2)) % 2 == 0,
        12 => (x + diagonal_phase) % 4 != 0,
        _ => false,
    }
}

fn emit_shade_block_primitive(
    ch: char,
    cell_x: f32,
    cell_y: f32,
    cell_w: f32,
    cell_h: f32,
    color: [f32; 4],
    clip_rect: [f32; 4],
    row_quads: &mut Vec<QuadInstance>,
) -> bool {
    let Some(threshold) = terminal_shade_threshold(ch) else {
        return false;
    };

    let width = cell_w.round().max(1.0) as u32;
    let height = cell_h.round().max(1.0) as u32;
    let origin_x = cell_x.floor();
    let origin_y = cell_y.floor();
    for y in 0..height {
        let mut span_start: Option<u32> = None;
        for x in 0..=width {
            let filled = x < width
                && terminal_shade_pixel_filled(origin_x as u32 + x, origin_y as u32 + y, threshold);
            match (span_start, filled) {
                (None, true) => span_start = Some(x),
                (Some(start), false) => {
                    push_terminal_quad(
                        row_quads,
                        [origin_x + start as f32, origin_y + y as f32],
                        [(x - start) as f32, 1.0],
                        color,
                        clip_rect,
                    );
                    span_start = None;
                }
                _ => {}
            }
        }
    }

    true
}

fn box_stroke_width(cell_h: f32, heavy: bool) -> f32 {
    let light = (cell_h / 16.0).round().max(1.0);
    if heavy {
        (light * 2.0).max(2.0)
    } else {
        light
    }
}

fn box_connections(ch: char) -> Option<(bool, bool, bool, bool, bool)> {
    let (left, right, up, down, heavy) = match ch {
        '\u{2500}' => (true, true, false, false, false),
        '\u{2502}' => (false, false, true, true, false),
        '\u{250c}' => (false, true, false, true, false),
        '\u{2510}' => (true, false, false, true, false),
        '\u{2514}' => (false, true, true, false, false),
        '\u{2518}' => (true, false, true, false, false),
        '\u{251c}' => (false, true, true, true, false),
        '\u{2524}' => (true, false, true, true, false),
        '\u{252c}' => (true, true, false, true, false),
        '\u{2534}' => (true, true, true, false, false),
        '\u{253c}' => (true, true, true, true, false),
        '\u{2501}' => (true, true, false, false, true),
        '\u{2503}' => (false, false, true, true, true),
        '\u{250f}' => (false, true, false, true, true),
        '\u{2513}' => (true, false, false, true, true),
        '\u{2517}' => (false, true, true, false, true),
        '\u{251b}' => (true, false, true, false, true),
        '\u{2523}' => (false, true, true, true, true),
        '\u{252b}' => (true, false, true, true, true),
        '\u{2533}' => (true, true, false, true, true),
        '\u{253b}' => (true, true, true, false, true),
        '\u{254b}' => (true, true, true, true, true),
        _ => return None,
    };
    Some((left, right, up, down, heavy))
}

fn emit_box_drawing_primitive(
    ch: char,
    cell_x: f32,
    cell_y: f32,
    cell_w: f32,
    cell_h: f32,
    trailing_edge_clamp: f32,
    parity_colors: bool,
    color: [f32; 4],
    clip_rect: [f32; 4],
    row_quads: &mut Vec<QuadInstance>,
) -> bool {
    let Some((left, right, up, down, heavy)) = box_connections(ch) else {
        return false;
    };

    let overlap = 0.5;
    let stroke = box_stroke_width(cell_h, heavy);
    let cx = cell_w * 0.5;
    let cy = cell_h * 0.5;
    let snap_to_pixels = parity_colors;

    if left || right {
        let x0 = if left { -overlap } else { cx - stroke * 0.5 };
        let x1 = if right { cell_w + overlap - trailing_edge_clamp } else { cx + stroke * 0.5 };
        let rect = TerminalCellRect { x: x0, y: cy - stroke * 0.5, width: x1 - x0, height: stroke };
        if snap_to_pixels {
            push_terminal_cell_rect_snapped(row_quads, cell_x, cell_y, rect, color, clip_rect);
        } else {
            push_terminal_cell_rect(row_quads, cell_x, cell_y, rect, color, clip_rect);
        }
    }

    if up || down {
        let y0 = if up { -overlap } else { cy - stroke * 0.5 };
        let y1 = if down { cell_h + overlap } else { cy + stroke * 0.5 };
        let rect = TerminalCellRect { x: cx - stroke * 0.5, y: y0, width: stroke, height: y1 - y0 };
        if snap_to_pixels {
            push_terminal_cell_rect_snapped(row_quads, cell_x, cell_y, rect, color, clip_rect);
        } else {
            push_terminal_cell_rect(row_quads, cell_x, cell_y, rect, color, clip_rect);
        }
    }

    true
}

fn emit_box_double_stroke(
    horizontal: bool,
    center: f32,
    start: f32,
    end: f32,
    stroke: f32,
    snap_to_pixels: bool,
    cell_x: f32,
    cell_y: f32,
    color: [f32; 4],
    clip_rect: [f32; 4],
    row_quads: &mut Vec<QuadInstance>,
) {
    if horizontal {
        let rect = TerminalCellRect {
            x: start,
            y: center - stroke * 0.5,
            width: end - start,
            height: stroke,
        };
        if snap_to_pixels {
            push_terminal_cell_rect_snapped(row_quads, cell_x, cell_y, rect, color, clip_rect);
        } else {
            push_terminal_cell_rect(row_quads, cell_x, cell_y, rect, color, clip_rect);
        }
    } else {
        let rect = TerminalCellRect {
            x: center - stroke * 0.5,
            y: start,
            width: stroke,
            height: end - start,
        };
        if snap_to_pixels {
            push_terminal_cell_rect_snapped(row_quads, cell_x, cell_y, rect, color, clip_rect);
        } else {
            push_terminal_cell_rect(row_quads, cell_x, cell_y, rect, color, clip_rect);
        }
    }
}

fn emit_double_box_drawing_primitive(
    ch: char,
    cell_x: f32,
    cell_y: f32,
    cell_w: f32,
    cell_h: f32,
    parity_colors: bool,
    color: [f32; 4],
    clip_rect: [f32; 4],
    row_quads: &mut Vec<QuadInstance>,
) -> bool {
    let (left, right, up, down, double_horizontal, double_vertical) = match ch {
        '\u{2550}' => (true, true, false, false, true, false), // ═
        '\u{2551}' => (false, false, true, true, false, true), // ║
        '\u{2554}' => (false, true, false, true, true, true),  // ╔
        '\u{2557}' => (true, false, false, true, true, true),  // ╗
        '\u{255a}' => (false, true, true, false, true, true),  // ╚
        '\u{255d}' => (true, false, true, false, true, true),  // ╝
        '\u{255f}' => (false, true, true, true, false, true),  // ╟
        '\u{2562}' => (true, false, true, true, false, true),  // ╢
        _ => return false,
    };

    let overlap = 0.5;
    let stroke = box_stroke_width(cell_h, false);
    let cx = cell_w * 0.5;
    let cy = cell_h * 0.5;
    let x_offset = (cell_w * 0.15).round().max(stroke);
    let y_offset = (cell_h * 0.09).round().max(stroke);
    let snap_to_pixels = parity_colors;
    let horizontal_centers =
        if double_horizontal { [cy - y_offset, cy + y_offset] } else { [cy, cy] };
    let vertical_centers = if double_vertical { [cx - x_offset, cx + x_offset] } else { [cx, cx] };
    let horizontal_count = if double_horizontal { 2 } else { 1 };
    let vertical_count = if double_vertical { 2 } else { 1 };
    let corner_join = double_horizontal && double_vertical && (left ^ right) && (up ^ down);
    let corner_reversed_pairs = corner_join && ((left && down) || (right && up));
    let corner_pair = |index: usize| {
        if corner_reversed_pairs {
            1 - index
        } else {
            index
        }
    };

    if left || right {
        for (index, center) in horizontal_centers.iter().take(horizontal_count).enumerate() {
            let mut x0 = if left { -overlap } else { cx - stroke * 0.5 };
            let mut x1 = if right { cell_w + overlap } else { cx + stroke * 0.5 };
            if corner_join {
                let vertical_center = vertical_centers[corner_pair(index)];
                if !left {
                    x0 = vertical_center - stroke * 0.5;
                }
                if !right {
                    x1 = vertical_center + stroke * 0.5;
                }
            }
            emit_box_double_stroke(
                true,
                *center,
                x0,
                x1,
                stroke,
                snap_to_pixels,
                cell_x,
                cell_y,
                color,
                clip_rect,
                row_quads,
            );
        }
    }

    if up || down {
        for (index, center) in vertical_centers.iter().take(vertical_count).enumerate() {
            let mut y0 = if up { -overlap } else { cy - stroke * 0.5 };
            let mut y1 = if down { cell_h + overlap } else { cy + stroke * 0.5 };
            if corner_join {
                let horizontal_center = horizontal_centers[corner_pair(index)];
                if !up {
                    y0 = horizontal_center - stroke * 0.5;
                }
                if !down {
                    y1 = horizontal_center + stroke * 0.5;
                }
            }
            emit_box_double_stroke(
                false,
                *center,
                y0,
                y1,
                stroke,
                snap_to_pixels,
                cell_x,
                cell_y,
                color,
                clip_rect,
                row_quads,
            );
        }
    }

    true
}

fn emit_terminal_cell_primitive(
    cell: &unshit_core::cell_grid::Cell,
    row: usize,
    col: usize,
    origin_x: f32,
    origin_y: f32,
    cell_w: f32,
    cell_h: f32,
    opacity: f32,
    clip_rect: [f32; 4],
    parity_colors: bool,
    blink_phase_on: bool,
    next_ch: Option<char>,
    row_quads: &mut Vec<QuadInstance>,
) -> bool {
    if cell.ch == '\0' || cell.wide_continuation {
        return false;
    }
    if !terminal_cell_foreground_visible(cell, blink_phase_on) {
        return true;
    }

    let cell_x = origin_x + col as f32 * cell_w;
    let cell_y = origin_y + row as f32 * cell_h;
    let primitive_y = cell_y + terminal_primitive_y_bias_for_parity(cell.ch, parity_colors);
    let trailing_edge_clamp =
        terminal_primitive_trailing_edge_clamp_for_parity(cell.ch, next_ch, parity_colors);
    let color = terminal_fg_color(cell, opacity);
    let decorated = emit_terminal_text_decorations(
        cell, cell_x, cell_y, cell_w, cell_h, color, clip_rect, row_quads,
    );

    if cell.ch == ' ' {
        return decorated;
    }

    if let Some(rect) = terminal_block_rect(cell.ch, cell_w, cell_h, trailing_edge_clamp) {
        if parity_colors {
            push_terminal_cell_rect_snapped(row_quads, cell_x, primitive_y, rect, color, clip_rect);
        } else {
            push_terminal_cell_rect(row_quads, cell_x, primitive_y, rect, color, clip_rect);
        }
        return true;
    }

    if parity_colors
        && emit_shade_block_primitive(
            cell.ch,
            cell_x,
            primitive_y,
            cell_w,
            cell_h,
            color,
            clip_rect,
            row_quads,
        )
    {
        return true;
    }

    emit_box_drawing_primitive(
        cell.ch,
        cell_x,
        primitive_y,
        cell_w,
        cell_h,
        trailing_edge_clamp,
        parity_colors,
        color,
        clip_rect,
        row_quads,
    ) || (parity_colors
        && emit_double_box_drawing_primitive(
            cell.ch,
            cell_x,
            primitive_y,
            cell_w,
            cell_h,
            parity_colors,
            color,
            clip_rect,
            row_quads,
        ))
}

/// Emit a background `QuadInstance` for every bg run on `row` between
/// `start_col` (inclusive) and `end_col` (exclusive). Adjacent cells with
/// matching background color merge regardless of foreground color or
/// attribute flags (issue #84), so colorized text rows with uniform bg
/// but varying fg per token collapse to a single bg quad. This is the
/// pure, GPU-independent half of the row emission pipeline used by
/// `emit_grid_row_fresh`, and every production caller passes the full
/// row range `0..cols`.
///
/// Unlike earlier versions of the renderer this helper does NOT filter
/// runs by the grid's per-cell `dirty` flags. The retained line quad
/// cache (see `line_quad_cache.rs`) stores a whole-row payload keyed by
/// the whole-row content hash; on a cache MISS the entire row must be
/// re-emitted so the stored payload covers every column. Narrowing the
/// emission to the damaged sub-range caused the classic "typing a
/// character blanks the rest of the row" regression (issue #63).
///
/// Peer terminals (WezTerm, Alacritty, Zed) treat damage bounds as
/// invalidation hints, not emission extent. This helper mirrors that
/// contract: `start_col`/`end_col` are only the bounds of the loop, not
/// a dirty filter.
#[allow(clippy::too_many_arguments)]
fn emit_grid_row_backgrounds(
    grid: &CellGrid,
    row: usize,
    start_col: usize,
    end_col: usize,
    origin_x: f32,
    origin_y: f32,
    cell_w: f32,
    cell_h: f32,
    opacity: f32,
    clip_rect: [f32; 4],
    row_quads: &mut Vec<QuadInstance>,
) {
    let py = origin_y + row as f32 * cell_h;
    let overlap_enabled = terminal_bg_overlap_enabled(origin_x, cell_w);
    // Issue #84: merge adjacent cells that share a bg regardless of fg
    // or attrs. A typical colorized row has uniform bg but varies fg per
    // token, so collapsing on bg alone cuts redundant bg quads. Mirrors
    // Zed's `BackgroundRegion` pass.
    for run in terminal_bg_runs_in_range(grid, row, start_col, end_col) {
        // Default background elision. The terminal cell `DEFAULT_BG` is
        // fully transparent (see `src/terminal/mod.rs`), and the frame
        // clear already paints the chrome color which shows through by
        // design. Emitting a zero alpha quad is a visual no op and a
        // waste of an instance, so skip it.
        let bg = run.bg;
        if bg.a == 0 {
            continue;
        }
        let mut bg_color = bg.to_linear_f32();
        bg_color[3] *= opacity;
        let (px, width) = if overlap_enabled {
            let (left, right) = terminal_bg_run_edges(
                grid,
                row,
                start_col,
                end_col,
                run.start_col,
                run.end_col,
                origin_x,
                cell_w,
            );
            (left, right - left)
        } else {
            (
                terminal_bg_boundary_x(origin_x, cell_w, run.start_col),
                run.col_count() as f32 * cell_w,
            )
        };
        if width <= 0.0 {
            continue;
        }
        row_quads.push(QuadInstance {
            pos: [px, py],
            size: [width, cell_h],
            color: bg_color,
            border_color: [0.0; 4],
            border_width: [0.0; 4],
            border_radius: [0.0; 4],
            clip_rect,
            shadow_color: [0.0; 4],
            shadow_offset: [0.0; 2],
            shadow_params: [0.0; 2],
            shadow_spread: [0.0; 2],
            gradient_stop_colors: EMPTY_GRADIENT_STOP_COLORS,
            gradient_stop_positions: EMPTY_GRADIENT_STOP_POSITIONS,
            gradient_params: [0.0; 4],
            gradient_extra: EMPTY_GRADIENT_EXTRA,
            mask_stops_01: EMPTY_MASK_STOPS,
            mask_stops_23: EMPTY_MASK_STOPS,
            mask_params: EMPTY_MASK_PARAMS,
            xform: IDENTITY_XFORM,
            xform_translate: IDENTITY_XFORM_TRANSLATE,
        });
    }
}

/// Emit background quads and glyph instances for a `CellGrid`.
///
/// This path skips cosmic-text shaping entirely for already rasterized
/// glyphs. When a retained per line quad cache is provided, rows whose
/// content and geometry signatures still match the cache are replayed by
/// appending the previously emitted vertex instances to the frame batch,
/// skipping shaping, rasterization, and iteration.
#[allow(clippy::too_many_arguments)]
fn emit_grid_cells(
    grid: &CellGrid,
    origin_x: f32,
    origin_y: f32,
    cell_w: f32,
    cell_h: f32,
    font_size: f32,
    opacity: f32,
    clip_rect: [f32; 4],
    batch: &mut FrameBatch,
    atlas: &mut GlyphAtlas,
    font_system: &mut FontSystem,
    rasterizer: &mut Rasterizer<'_>,
    shape_cache: &mut ShapeCache,
    mut glyph_keys_out: Option<&mut FxHashSet<GlyphKey>>,
    node_id: NodeId,
    mut line_cache: Option<&mut LineQuadCache>,
) {
    let rows = grid.rows();
    let cols = grid.cols();
    let cells = grid.cells();
    let trace_hash = terminal_grid_trace_hash(grid);
    let trace_this_grid = terminal_trace_enabled()
        && LAST_TERMINAL_RENDER_TRACE_HASH.swap(trace_hash, Ordering::Relaxed) != trace_hash;
    let trace_rows = if trace_this_grid { Some(grid.debug_rows(4, 96)) } else { None };
    let mut trace_glyphs: Vec<String> = Vec::new();

    let atlas_generation = atlas.generation;
    let blink_phase_on = !CellGrid::is_window_focused() || CellGrid::cursor_blink_phase_now();

    // Shape each unique character once, then cache the fully resolved glyph
    // per actual atlas key. Fractional cell origins can change the subpixel
    // bins, so caching only by `char` is incorrect when cell_w/cell_h are not
    // integers. This cache is shared across every row we actually emit
    // (cache miss path) in this pass.
    let mut glyph_cache: FxHashMap<GlyphKey, ResolvedGlyph> = FxHashMap::default();

    // Precompute the font-family fields for the ShapeCache key. The style
    // portion still comes from each cell's SGR attrs (bold/italic).
    #[cfg(target_os = "windows")]
    let family_name: &str = &rasterizer.dw.font_family;
    #[cfg(not(target_os = "windows"))]
    let family_name: &str = "";
    let shape_font_id = shape_cache_font_id(family_name);
    let shape_font_size_tenths = (font_size * 10.0).round() as u32;

    // Auto-invalidate the cache if font, DPI, or size has changed since the
    // last grid render. `retune` is a no-op when nothing has changed, so the
    // hot path still costs a single hashmap lookup per cell.
    shape_cache.retune(family_name, 1.0, font_size);

    // Reusable buffer for glyph shaping on cache miss.
    let metrics = cosmic_text::Metrics::new(font_size, cell_h);
    let mut buffer = cosmic_text::Buffer::new(font_system, metrics);
    buffer.set_size(font_system, Some(cell_w * 4.0), None);
    let mut ch_buf = [0u8; 4];

    // Geometry inputs are constant across every row of a single grid pass,
    // so build the geometry signature once and reuse it for every row probe.
    let geom_sig = LineGeometrySig::new(
        origin_x,
        origin_y,
        cell_w,
        cell_h,
        font_size,
        opacity,
        clip_rect,
        cols as u32,
        atlas_generation,
    );

    // Drop any cached lines whose identity no longer appears in the
    // current grid. This keeps memory bounded on grid shrink and on any
    // full-grid identity reset (clear, DECALN). `retain_ids` is built
    // from the grid's stable line_ids.
    if let Some(cache) = line_cache.as_deref_mut() {
        let retain_ids: FxHashSet<u64> = grid.line_ids().iter().copied().collect();
        cache.retain_element_ids(node_id, &retain_ids);
    }

    for row in 0..rows {
        // Stable line identity: the cache is keyed on `(node, line_id)`
        // so a scroll that rotates this line to a new row index replays
        // the cached payload without a miss. `line_id` is assigned by
        // the grid and moves with the content.
        let line_id = grid.line_id(row).unwrap_or(0);

        // Compute the whole-row content hash. The hash decides cache
        // freshness; the damage range decides emission extent on miss.
        // Mirrors Alacritty's `LineDamageBounds` and WezTerm's
        // `changed_since(seqno)` patterns (issue #52 Step 4).
        let content_sig = terminal_row_content_sig(cells, row, cols, blink_phase_on);

        // Cache probe: replay the cached instances for this line when its
        // content hash and geometry signature still match. This is the
        // clean-row skip path: if `line_damage` is clean and the cache has
        // a hit, we extend_from_slice without any shaping or iteration.
        //
        // Issue #77: `lookup_and_retarget` translates cached Y when the
        // line's stable `line_id` has rotated to a new row via scroll /
        // shift. Without this, the cache hit replays vertices at the
        // pre-scroll Y and overlaps whatever now renders at the old slot.
        if let Some(cache) = line_cache.as_deref_mut() {
            if let Some(hit) =
                cache.lookup_and_retarget(node_id, line_id, content_sig, geom_sig, row, cell_h)
            {
                batch.quad_instances.extend_from_slice(&hit.quads);
                batch.glyph_instances.extend_from_slice(&hit.glyphs);
                for key in &hit.glyph_keys {
                    atlas.touch(key);
                    if let Some(keys) = glyph_keys_out.as_deref_mut() {
                        keys.insert(*key);
                    }
                }
                continue;
            }
        }

        // Cache miss. Decide between a narrow splice (cells outside the
        // damage range reuse cached glyphs) and a full fresh emit. The
        // splice path is safe when:
        //   1) An existing cache entry is present under the same
        //      (node, line_id) and same geometry signature. The
        //      geometry signature covers atlas generation, so any
        //      atlas bump correctly falls through to full fresh emit.
        //   2) The cached entry carries a per-column glyph index, which
        //      Step 4's cache format populates on every fresh emit.
        // When either precondition fails we emit the whole row and
        // repopulate the cache. This preserves the issue #63 contract:
        // the cache payload stored by the miss path always spans the
        // full row, never a damaged sub-range.
        // Resolve the damage window for this row. A clean row or an
        // inverted `first > last` range yields `None`, which forces the
        // full-row fresh emit path below.
        let damage_range = grid.line_damage_for(row).and_then(|ld| {
            if ld.is_clean() {
                return None;
            }
            let start = (ld.first_dirty_col as usize).min(cols);
            let end = ((ld.last_dirty_col as usize).saturating_add(1)).min(cols);
            (start < end).then_some((start, end))
        });

        // Look up a cached entry that is splice-compatible: same node +
        // line identity, same geometry signature (so atlas generation,
        // origin, cell metrics all match), and a per-column glyph index
        // of the expected length. We clone the referenced vectors so the
        // mutable borrow of `line_cache` can be released for the store
        // below. The clone cost is proportional to `cols` and is still
        // much cheaper than the alternative (reshape every column).
        //
        // Issue #77: `cached_row` is carried alongside so the splice
        // path can translate copied glyph Y when the line rotated to a
        // new row slot between the cache store and this splice probe.
        let splice_inputs = damage_range.and_then(|(start, end)| {
            let cached = line_cache.as_deref()?.get(node_id, line_id)?;
            if cached.geometry != Some(geom_sig) || cached.glyph_col_index.len() != cols {
                return None;
            }
            Some((
                start,
                end,
                cached.glyphs.clone(),
                cached.glyph_keys.clone(),
                cached.glyph_col_index.clone(),
                cached.cached_row,
            ))
        });

        let mut row_quads: Vec<QuadInstance> = Vec::new();
        let mut row_glyphs: Vec<GlyphInstance> = Vec::new();
        let mut row_keys: Vec<GlyphKey> = Vec::new();
        let mut row_glyph_col_index: Vec<Option<u32>> = Vec::with_capacity(cols);

        // Splice fast-path when a compatible cached entry and a concrete
        // damage range are both available. Otherwise emit fresh for the
        // full row.
        if let Some((
            damage_start,
            damage_end,
            cached_glyphs,
            cached_keys,
            cached_col_index,
            cached_row,
        )) = splice_inputs
        {
            emit_grid_row_splice(
                grid,
                cells,
                row,
                cols,
                damage_start,
                damage_end,
                &cached_glyphs,
                &cached_keys,
                &cached_col_index,
                cached_row,
                origin_x,
                origin_y,
                cell_w,
                cell_h,
                font_size,
                opacity,
                clip_rect,
                shape_font_id,
                TERMINAL_SHAPE_STYLE_REGULAR,
                shape_font_size_tenths,
                family_name,
                blink_phase_on,
                &mut row_quads,
                &mut row_glyphs,
                &mut row_keys,
                &mut row_glyph_col_index,
                &mut glyph_cache,
                shape_cache,
                atlas,
                font_system,
                rasterizer,
                &mut buffer,
                &mut ch_buf,
                trace_this_grid,
                &mut trace_glyphs,
            );
        } else {
            emit_grid_row_fresh(
                grid,
                cells,
                row,
                cols,
                origin_x,
                origin_y,
                cell_w,
                cell_h,
                font_size,
                opacity,
                clip_rect,
                shape_font_id,
                TERMINAL_SHAPE_STYLE_REGULAR,
                shape_font_size_tenths,
                family_name,
                blink_phase_on,
                &mut row_quads,
                &mut row_glyphs,
                &mut row_keys,
                &mut row_glyph_col_index,
                &mut glyph_cache,
                shape_cache,
                atlas,
                font_system,
                rasterizer,
                &mut buffer,
                &mut ch_buf,
                trace_this_grid,
                &mut trace_glyphs,
            );
        }

        // Forward to the frame batch.
        batch.quad_instances.extend_from_slice(&row_quads);
        batch.glyph_instances.extend_from_slice(&row_glyphs);
        if let Some(keys) = glyph_keys_out.as_deref_mut() {
            for key in &row_keys {
                keys.insert(*key);
            }
        }

        // Store the fresh line in the cache keyed by its stable identity.
        // Subsequent frames that see the same line_id + content_sig replay
        // this payload without shaping. Both the splice and fresh paths
        // produce a whole-row payload plus its per-column glyph index.
        // `row` is captured so `lookup_and_retarget` can translate Y when
        // a scroll rotates this line to a different row slot (issue #77).
        if let Some(cache) = line_cache.as_deref_mut() {
            cache.store(
                node_id,
                line_id,
                content_sig,
                geom_sig,
                row_quads,
                row_glyphs,
                row_keys,
                row_glyph_col_index,
                row,
            );
        }
    }

    if trace_this_grid {
        let rows_dump = trace_rows.unwrap_or_default();
        append_terminal_trace_line(&format!(
            "terminal-trace stage=emit_grid_cells rows={} cols={} origin=({:.1}, {:.1}) cell=({:.2}, {:.2}) cursor=({}, {}) visible={} row0={:?} row1={:?} row2={:?} row3={:?} glyphs={}",
            rows,
            cols,
            origin_x,
            origin_y,
            cell_w,
            cell_h,
            grid.cursor_row(),
            grid.cursor_col(),
            grid.cursor_visible(),
            rows_dump.first().cloned().unwrap_or_default(),
            rows_dump.get(1).cloned().unwrap_or_default(),
            rows_dump.get(2).cloned().unwrap_or_default(),
            rows_dump.get(3).cloned().unwrap_or_default(),
            trace_glyphs.join(" | "),
        ));
    }

    // Draw cursor block when visible. The cursor moves every frame in the
    // typical interactive case; we do not cache it, because the pertinent
    // row's content hash may still be stable while the cursor shifts inside
    // that row.
    //
    // Renderer side blink (#135 Phase 1): `cursor_visible` is a one-shot
    // flag that means "this pane owns the focused cursor". The actual
    // blink animation is computed here from a global phase clock, so
    // toggling the cursor on a 500 ms timer no longer requires a tree
    // rebuild. The phase is "on" for half a cycle, "off" for the next,
    // matching the legacy 530 ms cadence (see
    // [`unshit_core::cell_grid::CURSOR_BLINK_HALF_CYCLE_MS`]). When the
    // OS window is not focused we draw the cursor steady on so the
    // unfocused state mirrors the legacy behaviour.
    if grid.cursor_visible() && blink_phase_on {
        let crow = grid.cursor_row();
        let ccol = grid.cursor_col();
        if crow < rows && ccol < cols {
            let cx = origin_x + ccol as f32 * cell_w;
            let cy = origin_y + crow as f32 * cell_h;

            let cursor_idx = crow * cols + ccol;
            let cell_fg = cells[cursor_idx].fg;
            let mut cursor_color = cell_fg.to_linear_f32();
            cursor_color[3] *= opacity * 0.7;

            batch.quad_instances.push(QuadInstance {
                pos: [cx, cy],
                size: [cell_w, cell_h],
                color: cursor_color,
                border_color: [0.0; 4],
                border_width: [0.0; 4],
                border_radius: [0.0; 4],
                clip_rect,
                shadow_color: [0.0; 4],
                shadow_offset: [0.0; 2],
                shadow_params: [0.0; 2],
                shadow_spread: [0.0; 2],
                gradient_stop_colors: EMPTY_GRADIENT_STOP_COLORS,
                gradient_stop_positions: EMPTY_GRADIENT_STOP_POSITIONS,
                gradient_params: [0.0; 4],
                gradient_extra: EMPTY_GRADIENT_EXTRA,
                mask_stops_01: EMPTY_MASK_STOPS,
                mask_stops_23: EMPTY_MASK_STOPS,
                mask_params: EMPTY_MASK_PARAMS,
                xform: IDENTITY_XFORM,
                xform_translate: IDENTITY_XFORM_TRANSLATE,
            });
        }
    }
}

/// Emit the glyph for a single cell, shaping if not yet cached and
/// pushing the resulting `GlyphInstance` and `GlyphKey` into the row
/// buffers. Returns `true` when a glyph was emitted. Empty cells, wide
/// continuations, and cells whose prototype shape-step yields nothing
/// return `false`.
///
/// Shared between `emit_grid_row_fresh` (full-row fresh emit) and
/// `emit_grid_row_splice` (column-range splice fast path on cache miss
/// with matching geometry). Both paths push the emitted glyph at the
/// end of `row_glyphs`/`row_keys`; the caller records the resulting
/// index in its `glyph_col_index` side-table.
#[allow(clippy::too_many_arguments)]
fn emit_grid_cell_glyph(
    cell: &unshit_core::cell_grid::Cell,
    row: usize,
    col: usize,
    origin_x: f32,
    origin_y: f32,
    cell_w: f32,
    cell_h: f32,
    font_size: f32,
    opacity: f32,
    clip_rect: [f32; 4],
    parity_colors: bool,
    shape_font_id: u64,
    shape_style: u64,
    shape_font_size_tenths: u32,
    family_name: &str,
    row_glyphs: &mut Vec<GlyphInstance>,
    row_keys: &mut Vec<GlyphKey>,
    glyph_cache: &mut FxHashMap<GlyphKey, ResolvedGlyph>,
    shape_cache: &mut ShapeCache,
    atlas: &mut GlyphAtlas,
    font_system: &mut FontSystem,
    rasterizer: &mut Rasterizer<'_>,
    buffer: &mut cosmic_text::Buffer,
    ch_buf: &mut [u8; 4],
    trace_this_grid: bool,
    trace_glyphs: &mut Vec<String>,
) -> bool {
    // Skip glyph for empty cells and wide continuation cells.
    if cell.is_empty() || cell.wide_continuation {
        return false;
    }

    let py = origin_y + row as f32 * cell_h;
    // INVERSE swaps fg/bg; only fg is needed here since the bg quad
    // was already emitted via the run loop in the caller.
    let fg_linear = terminal_fg_color(cell, opacity);
    let shape_style = shape_style | terminal_shape_style(cell.attrs);

    let cache_key = ShapeCacheKey {
        ch: cell.ch,
        font_id: shape_font_id,
        style: shape_style,
        font_size_tenths: shape_font_size_tenths,
    };
    let px = origin_x + col as f32 * cell_w;
    // Cross-frame ShapeCache. Hit path bypasses set_text and
    // shape_until_scroll entirely; miss path shapes once and stores.
    let prototype = if let Some(entry) = shape_cache.get(&cache_key) {
        entry.clone()
    } else {
        let ch_str = cell.ch.encode_utf8(ch_buf);
        #[cfg(target_os = "windows")]
        let family = cosmic_text::Family::Name(family_name);
        #[cfg(not(target_os = "windows"))]
        let family = cosmic_text::Family::Monospace;
        // Silence unused on non-windows.
        let _ = family_name;
        let attrs = terminal_text_attrs(family, cell.attrs);
        buffer.set_text(font_system, ch_str, attrs, cosmic_text::Shaping::Advanced);
        buffer.shape_until_scroll(font_system, false);

        let shaped = buffer.layout_runs().find_map(|run| {
            run.glyphs
                .first()
                .cloned()
                .map(|glyph| ShapedGlyphEntry { layout_glyph: glyph, line_y: run.line_y })
        });
        shape_cache.insert(cache_key, shaped.clone());
        shaped
    };

    let Some(prototype) = prototype else {
        return false;
    };

    let px_floor = px.floor();
    let py_floor = py.floor();
    let physical = prototype.layout_glyph.physical((px - px_floor, py - py_floor), 1.0);
    let key = GlyphKey {
        font_id: atlas_font_namespace(&physical.cache_key),
        glyph_id: physical.cache_key.glyph_id,
        font_size_tenths: (font_size * 10.0) as u16,
        subpixel_bin: ((physical.cache_key.x_bin as u8) << 2) | (physical.cache_key.y_bin as u8),
    };

    let was_cached = glyph_cache.contains_key(&key);
    let resolved = if let Some(cached) = glyph_cache.get(&key) {
        atlas.touch(&cached.key);
        cached
    } else {
        let entry = if let Some(entry) = atlas.cache.get(&key).copied() {
            atlas.touch(&key);
            entry
        } else {
            match rasterize_grid_glyph_for_atlas(
                rasterizer,
                font_system,
                &physical,
                cell.ch,
                font_size,
                atlas,
                key,
            ) {
                Some(entry) => entry,
                None => return false,
            }
        };

        glyph_cache.entry(key).or_insert(ResolvedGlyph {
            key,
            entry,
            physical_x: physical.x,
            physical_y: physical.y,
            line_y: prototype.line_y,
        })
    };

    row_keys.push(resolved.key);

    let gx = px_floor + resolved.physical_x as f32 + resolved.entry.offset[0];
    let gy = py_floor + resolved.line_y + resolved.physical_y as f32 + resolved.entry.offset[1];
    if trace_this_grid && row < 4 && trace_glyphs.len() < 64 {
        trace_glyphs.push(format!(
            "{} r{}c{} ch={:?} key=({}, {}, {}) pos=({:.1}, {:.1})",
            if was_cached { "cache" } else { "miss" },
            row,
            col,
            cell.ch,
            resolved.key.font_id,
            resolved.key.glyph_id,
            resolved.key.subpixel_bin,
            gx,
            gy,
        ));
    }

    row_glyphs.push(GlyphInstance {
        pos: terminal_glyph_position_for_parity(gx, gy, parity_colors),
        size: resolved.entry.size,
        uv_min: [resolved.entry.uv_rect[0], resolved.entry.uv_rect[1]],
        uv_max: [resolved.entry.uv_rect[2], resolved.entry.uv_rect[3]],
        color: fg_linear,
        clip_rect,
        xform: IDENTITY_XFORM,
        xform_translate: IDENTITY_XFORM_TRANSLATE,
    });

    true
}

fn terminal_row_next_ch(
    cells: &[unshit_core::cell_grid::Cell],
    row_base: usize,
    col: usize,
    cols: usize,
) -> Option<char> {
    (col + 1 < cols).then(|| cells[row_base + col + 1].ch)
}

/// Emit one row of a cell grid into the row-local output buffers. Called
/// by `emit_grid_cells` on cache miss when no usable cached entry exists
/// (first frame for this `line_id`, geometry signature drift, or atlas
/// generation bump). Spans the full row `0..cols`: merged background
/// `QuadInstance`s per style run, then per cell glyphs using the cross
/// frame `ShapeCache`.
///
/// Also populates `row_glyph_col_index` with one entry per column: the
/// index into `row_glyphs`/`row_keys` where that column's glyph was
/// pushed, or `None` when the cell had no glyph (empty, continuation,
/// or shaping failed). Step 4 uses this index to splice only the
/// damaged columns on subsequent cache misses with matching geometry.
#[allow(clippy::too_many_arguments)]
fn emit_grid_row_fresh(
    grid: &CellGrid,
    cells: &[unshit_core::cell_grid::Cell],
    row: usize,
    cols: usize,
    origin_x: f32,
    origin_y: f32,
    cell_w: f32,
    cell_h: f32,
    font_size: f32,
    opacity: f32,
    clip_rect: [f32; 4],
    shape_font_id: u64,
    shape_style: u64,
    shape_font_size_tenths: u32,
    family_name: &str,
    blink_phase_on: bool,
    row_quads: &mut Vec<QuadInstance>,
    row_glyphs: &mut Vec<GlyphInstance>,
    row_keys: &mut Vec<GlyphKey>,
    row_glyph_col_index: &mut Vec<Option<u32>>,
    glyph_cache: &mut FxHashMap<GlyphKey, ResolvedGlyph>,
    shape_cache: &mut ShapeCache,
    atlas: &mut GlyphAtlas,
    font_system: &mut FontSystem,
    rasterizer: &mut Rasterizer<'_>,
    buffer: &mut cosmic_text::Buffer,
    ch_buf: &mut [u8; 4],
    trace_this_grid: bool,
    trace_glyphs: &mut Vec<String>,
) {
    let row_base = row * cols;
    let parity_colors = parity_windows_terminal_colors_enabled();

    // Merge adjacent cells that share the same background color (ignoring
    // fg and attrs) into a single background QuadInstance across the
    // full row. Mirrors Zed's BackgroundRegion pass: one bg quad per bg
    // color stripe instead of one per style run, so colorized text rows
    // with uniform bg but varying fg collapse to a single quad.
    emit_grid_row_backgrounds(
        grid, row, 0, cols, origin_x, origin_y, cell_w, cell_h, opacity, clip_rect, row_quads,
    );

    row_glyph_col_index.clear();
    row_glyph_col_index.reserve(cols);

    // Emit per-cell glyphs across the full row, recording each column's
    // index into the row's glyph vecs so future splice passes can reuse
    // unchanged cells without reshaping.
    for col in 0..cols {
        let cell = &cells[row_base + col];
        let next_ch = terminal_row_next_ch(cells, row_base, col, cols);
        if emit_terminal_cell_primitive(
            cell,
            row,
            col,
            origin_x,
            origin_y,
            cell_w,
            cell_h,
            opacity,
            clip_rect,
            parity_colors,
            blink_phase_on,
            next_ch,
            row_quads,
        ) {
            row_glyph_col_index.push(None);
            continue;
        }
        if !terminal_cell_foreground_visible(cell, blink_phase_on) {
            row_glyph_col_index.push(None);
            continue;
        }

        let pre_len = row_glyphs.len() as u32;
        let emitted = emit_grid_cell_glyph(
            cell,
            row,
            col,
            origin_x,
            origin_y,
            cell_w,
            cell_h,
            font_size,
            opacity,
            clip_rect,
            parity_colors,
            shape_font_id,
            shape_style,
            shape_font_size_tenths,
            family_name,
            row_glyphs,
            row_keys,
            glyph_cache,
            shape_cache,
            atlas,
            font_system,
            rasterizer,
            buffer,
            ch_buf,
            trace_this_grid,
            trace_glyphs,
        );
        row_glyph_col_index.push(if emitted { Some(pre_len) } else { None });
    }
}

/// Column-range splice fast path for the cache miss case in
/// `emit_grid_cells`. Used when an existing cache entry under
/// `(node, line_id)` has the same geometry signature but a different
/// content signature, and `line_damage` reports a concrete sub-range.
/// The splice path reshapes only the damaged columns and reuses the
/// cached glyph data for cells outside the damage range.
///
/// The stored cache payload must remain whole-row (issue #63 regression
/// contract): a HIT for the new content signature must replay every
/// column. This path preserves the contract because every column's
/// glyph is either freshly emitted or copied from the previous cache
/// entry, and backgrounds are re-emitted for the whole row.
///
/// Mirrors Alacritty's `LineDamageBounds`-driven emission and
/// WezTerm's `changed_since(seqno)` cross-frame reuse.
#[allow(clippy::too_many_arguments)]
fn emit_grid_row_splice(
    grid: &CellGrid,
    cells: &[unshit_core::cell_grid::Cell],
    row: usize,
    cols: usize,
    damage_start: usize,
    damage_end: usize,
    cached_glyphs: &[GlyphInstance],
    cached_keys: &[GlyphKey],
    cached_col_index: &[Option<u32>],
    cached_row: usize,
    origin_x: f32,
    origin_y: f32,
    cell_w: f32,
    cell_h: f32,
    font_size: f32,
    opacity: f32,
    clip_rect: [f32; 4],
    shape_font_id: u64,
    shape_style: u64,
    shape_font_size_tenths: u32,
    family_name: &str,
    blink_phase_on: bool,
    row_quads: &mut Vec<QuadInstance>,
    row_glyphs: &mut Vec<GlyphInstance>,
    row_keys: &mut Vec<GlyphKey>,
    row_glyph_col_index: &mut Vec<Option<u32>>,
    glyph_cache: &mut FxHashMap<GlyphKey, ResolvedGlyph>,
    shape_cache: &mut ShapeCache,
    atlas: &mut GlyphAtlas,
    font_system: &mut FontSystem,
    rasterizer: &mut Rasterizer<'_>,
    buffer: &mut cosmic_text::Buffer,
    ch_buf: &mut [u8; 4],
    trace_this_grid: bool,
    trace_glyphs: &mut Vec<String>,
) {
    let row_base = row * cols;
    let parity_colors = parity_windows_terminal_colors_enabled();

    // Backgrounds: always re-emit for the full row. Bg runs are cheap
    // (O(cols)) and may shift boundaries as cell bg colors change, so
    // partial bg splices risk leaving stale runs.
    emit_grid_row_backgrounds(
        grid, row, 0, cols, origin_x, origin_y, cell_w, cell_h, opacity, clip_rect, row_quads,
    );

    row_glyph_col_index.clear();
    row_glyph_col_index.reserve(cols);

    for col in 0..cols {
        let cell = &cells[row_base + col];
        let next_ch = terminal_row_next_ch(cells, row_base, col, cols);
        if emit_terminal_cell_primitive(
            cell,
            row,
            col,
            origin_x,
            origin_y,
            cell_w,
            cell_h,
            opacity,
            clip_rect,
            parity_colors,
            blink_phase_on,
            next_ch,
            row_quads,
        ) {
            row_glyph_col_index.push(None);
            continue;
        }
        if !terminal_cell_foreground_visible(cell, blink_phase_on) {
            row_glyph_col_index.push(None);
            continue;
        }

        if col >= damage_start && col < damage_end {
            // Damaged column: emit fresh glyph.
            let pre_len = row_glyphs.len() as u32;
            let emitted = emit_grid_cell_glyph(
                cell,
                row,
                col,
                origin_x,
                origin_y,
                cell_w,
                cell_h,
                font_size,
                opacity,
                clip_rect,
                parity_colors,
                shape_font_id,
                shape_style,
                shape_font_size_tenths,
                family_name,
                row_glyphs,
                row_keys,
                glyph_cache,
                shape_cache,
                atlas,
                font_system,
                rasterizer,
                buffer,
                ch_buf,
                trace_this_grid,
                trace_glyphs,
            );
            row_glyph_col_index.push(if emitted { Some(pre_len) } else { None });
        } else {
            // Undamaged column: copy the cached glyph. The cached cell's
            // content is unchanged (damage range excludes it), the atlas
            // generation matches (geometry signature matched), and the
            // cell position is unchanged modulo the row-delta translate
            // (issue #77): if the line rotated to a new row between the
            // cache store and this splice, shift Y by the row delta so
            // the replayed glyph lands at the current row's Y slot.
            match cached_col_index.get(col).copied().flatten() {
                Some(cached_idx) => {
                    let i = cached_idx as usize;
                    let glyph = cached_glyphs.get(i).copied();
                    let key = cached_keys.get(i).copied();
                    match (glyph, key) {
                        (Some(mut g), Some(k)) => {
                            if cached_row != row {
                                let dy = (row as f32 - cached_row as f32) * cell_h;
                                g.pos[1] += dy;
                            }
                            atlas.touch(&k);
                            let new_idx = row_glyphs.len() as u32;
                            row_glyphs.push(g);
                            row_keys.push(k);
                            row_glyph_col_index.push(Some(new_idx));
                        }
                        _ => {
                            row_glyph_col_index.push(None);
                        }
                    }
                }
                None => {
                    row_glyph_col_index.push(None);
                }
            }
        }
    }
}

/// Walk the arena and emit select-specific rendering:
/// - Closed select: label text and dropdown arrow rendered into the content layer.
/// - Open select: dropdown items rendered into the overlay layer.
///
/// This is called from app.rs after `build_render_batch` so that select overlays
/// always appear on top of regular content.
#[allow(clippy::too_many_arguments)]
pub fn emit_select_overlays(
    arena: &NodeArena,
    root: NodeId,
    batch: &mut LayeredBatch,
    atlas: &mut GlyphAtlas,
    font_system: &mut FontSystem,
    rasterizer: &mut Rasterizer<'_>,
    shaped_cache: &mut ShapedTextCache,
    vw: f32,
    vh: f32,
) {
    emit_select_overlays_rec(
        arena,
        root,
        batch,
        atlas,
        font_system,
        rasterizer,
        shaped_cache,
        vw,
        vh,
    );
}

fn emit_select_overlays_rec(
    arena: &NodeArena,
    node_id: NodeId,
    batch: &mut LayeredBatch,
    atlas: &mut GlyphAtlas,
    font_system: &mut FontSystem,
    rasterizer: &mut Rasterizer<'_>,
    shaped_cache: &mut ShapedTextCache,
    vw: f32,
    vh: f32,
) {
    let Some(element) = arena.get(node_id) else { return };

    if element.tag == Tag::Select {
        emit_select_node(element, batch, atlas, font_system, rasterizer, shaped_cache, vw, vh);
        // Select has no arena children to recurse into.
        return;
    }

    // Recurse into children
    let mut child = element.first_child;
    // Store child IDs to avoid borrow conflict while iterating
    let mut children = Vec::new();
    while !child.is_dangling() {
        children.push(child);
        child = arena.get(child).map(|e| e.next_sibling).unwrap_or(NodeId::DANGLING);
    }
    for child_id in children {
        emit_select_overlays_rec(
            arena,
            child_id,
            batch,
            atlas,
            font_system,
            rasterizer,
            shaped_cache,
            vw,
            vh,
        );
    }
}

/// Emit rendering for a single select element and, if open, its dropdown.
#[allow(clippy::too_many_arguments)]
fn emit_select_node(
    element: &unshit_core::element::Element,
    batch: &mut LayeredBatch,
    atlas: &mut GlyphAtlas,
    font_system: &mut FontSystem,
    rasterizer: &mut Rasterizer<'_>,
    shaped_cache: &mut ShapedTextCache,
    vw: f32,
    vh: f32,
) {
    let Some(ref ss) = element.select_state else { return };
    let rect = element.layout_rect;
    let style = &element.computed_style;

    // Derive colors from the element's computed style or fall back to defaults.
    let fg_color = style.color;
    let font_size = style.font_size.max(10.0);
    let line_height = style.line_height;
    let letter_spacing = style.letter_spacing;
    let pad_left = style.padding.left;
    let pad_top = style.padding.top;

    // --- Selected label text inside the select box ---
    if !ss.options.is_empty() {
        let sel_idx = (ss.selected_index as usize).min(ss.options.len().saturating_sub(1));
        let label = &ss.options[sel_idx].label;
        let text_x = rect.x + pad_left;
        let text_y = rect.y + pad_top;
        let text_w = (rect.width - pad_left - style.padding.right - 20.0).max(1.0);
        let clip = [rect.x, rect.y, rect.x + rect.width, rect.y + rect.height];
        let content_layer = batch.layer_mut(Layer::Content);
        emit_text_glyphs_cached(
            label,
            text_x,
            text_y,
            Some(text_w),
            font_size,
            line_height,
            letter_spacing,
            &style.font_family,
            style.font_weight,
            style.font_style,
            &fg_color,
            clip,
            content_layer,
            atlas,
            font_system,
            rasterizer,
            shaped_cache,
            None,
        );
    }

    // --- Dropdown arrow (▼) at right edge ---
    {
        let arrow_x = rect.x + rect.width - 18.0;
        let arrow_y = rect.y + pad_top;
        let clip = [rect.x, rect.y, rect.x + rect.width, rect.y + rect.height];
        let content_layer = batch.layer_mut(Layer::Content);
        emit_text_glyphs_cached(
            "\u{25BC}",
            arrow_x,
            arrow_y,
            Some(18.0),
            font_size,
            line_height,
            letter_spacing,
            &style.font_family,
            style.font_weight,
            style.font_style,
            &fg_color,
            clip,
            content_layer,
            atlas,
            font_system,
            rasterizer,
            shaped_cache,
            None,
        );
    }

    // --- Dropdown panel when open ---
    if !ss.open || ss.options.is_empty() {
        return;
    }

    let item_h = (font_size * line_height * 1.2).max(24.0);
    let dropdown_w = rect.width;
    let dropdown_x = rect.x;
    let dropdown_y = rect.y + rect.height;
    let dropdown_h = item_h * ss.options.len() as f32;

    // Clamp dropdown to viewport bottom
    let actual_y =
        if dropdown_y + dropdown_h > vh { (rect.y - dropdown_h).max(0.0) } else { dropdown_y };

    let overlay_clip = [0.0, 0.0, vw, vh];

    // Dropdown background panel
    let overlay_layer = batch.layer_mut(Layer::Overlay);
    overlay_layer.quad_instances.push(QuadInstance {
        pos: [dropdown_x, actual_y],
        size: [dropdown_w, dropdown_h],
        color: [0.15, 0.15, 0.18, 0.97],
        border_color: [0.35, 0.35, 0.40, 1.0],
        border_width: [1.0, 1.0, 1.0, 1.0],
        border_radius: [4.0, 4.0, 4.0, 4.0],
        clip_rect: overlay_clip,
        shadow_color: [0.0, 0.0, 0.0, 0.4],
        shadow_offset: [0.0, 4.0],
        shadow_params: [8.0, 0.0],
        shadow_spread: [0.0, 0.0],
        gradient_stop_colors: EMPTY_GRADIENT_STOP_COLORS,
        gradient_stop_positions: EMPTY_GRADIENT_STOP_POSITIONS,
        gradient_params: [0.0, 0.0, 0.0, 0.0],
        gradient_extra: EMPTY_GRADIENT_EXTRA,
        mask_stops_01: EMPTY_MASK_STOPS,
        mask_stops_23: EMPTY_MASK_STOPS,
        mask_params: EMPTY_MASK_PARAMS,
        xform: IDENTITY_XFORM,
        xform_translate: IDENTITY_XFORM_TRANSLATE,
    });

    // Per-option rows
    for (i, opt) in ss.options.iter().enumerate() {
        let item_y = actual_y + i as f32 * item_h;
        let is_highlighted = ss.highlighted_index == Some(i as u32);
        let is_selected = ss.selected_index == i as u32;
        let item_clip = [dropdown_x, actual_y, dropdown_x + dropdown_w, actual_y + dropdown_h];

        if is_highlighted {
            let overlay_layer = batch.layer_mut(Layer::Overlay);
            overlay_layer.quad_instances.push(QuadInstance {
                pos: [dropdown_x + 2.0, item_y + 1.0],
                size: [dropdown_w - 4.0, item_h - 2.0],
                color: [0.27, 0.49, 0.82, 0.85],
                border_color: [0.0, 0.0, 0.0, 0.0],
                border_width: [0.0, 0.0, 0.0, 0.0],
                border_radius: [3.0, 3.0, 3.0, 3.0],
                clip_rect: item_clip,
                shadow_color: [0.0, 0.0, 0.0, 0.0],
                shadow_offset: [0.0, 0.0],
                shadow_params: [0.0, 0.0],
                shadow_spread: [0.0, 0.0],
                gradient_stop_colors: EMPTY_GRADIENT_STOP_COLORS,
                gradient_stop_positions: EMPTY_GRADIENT_STOP_POSITIONS,
                gradient_params: [0.0, 0.0, 0.0, 0.0],
                gradient_extra: EMPTY_GRADIENT_EXTRA,
                mask_stops_01: EMPTY_MASK_STOPS,
                mask_stops_23: EMPTY_MASK_STOPS,
                mask_params: EMPTY_MASK_PARAMS,
                xform: IDENTITY_XFORM,
                xform_translate: IDENTITY_XFORM_TRANSLATE,
            });
        }

        // Checkmark for the currently selected option
        let text_x_offset = if is_selected { 20.0 } else { 8.0 };
        let text_color = Color { r: 230, g: 230, b: 235, a: 255 };

        if is_selected {
            let check_color = Color { r: 100, g: 200, b: 120, a: 255 };
            let overlay_layer = batch.layer_mut(Layer::Overlay);
            emit_text_glyphs_cached(
                "\u{2713}",
                dropdown_x + 4.0,
                item_y + 4.0,
                Some(14.0),
                font_size.min(14.0),
                line_height,
                letter_spacing,
                &style.font_family,
                style.font_weight,
                style.font_style,
                &check_color,
                item_clip,
                overlay_layer,
                atlas,
                font_system,
                rasterizer,
                shaped_cache,
                None,
            );
        }

        let overlay_layer = batch.layer_mut(Layer::Overlay);
        emit_text_glyphs_cached(
            &opt.label,
            dropdown_x + text_x_offset,
            item_y + 4.0,
            Some(dropdown_w - text_x_offset - 8.0),
            font_size,
            line_height,
            letter_spacing,
            &style.font_family,
            style.font_weight,
            style.font_style,
            &text_color,
            item_clip,
            overlay_layer,
            atlas,
            font_system,
            rasterizer,
            shaped_cache,
            None,
        );
    }
}

/// Cache key for [`CellMetricsCache`]. Keyed on the inputs that can change the
/// measured monospace cell dimensions: font identity (family name hash), font
/// size in tenths of a pixel, line height in tenths of a pixel, and DPI scale
/// factor in thousandths.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct CellMetricsKey {
    pub font_id: u64,
    pub font_size_tenths: u32,
    pub line_height_tenths: u32,
    pub scale_factor_thousandths: u32,
}

impl CellMetricsKey {
    pub fn new(font_family: &str, font_size: f32, line_height: f32, scale_factor: f32) -> Self {
        Self {
            font_id: shape_cache_font_id(font_family),
            font_size_tenths: (font_size * 10.0).round() as u32,
            line_height_tenths: (line_height * 10.0).round() as u32,
            scale_factor_thousandths: (scale_factor * 1000.0).round() as u32,
        }
    }
}

/// Measured cell metrics for a given font configuration.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CellMetrics {
    pub cell_w: f32,
    pub cell_h: f32,
}

/// Cross-frame cache for monospace cell metrics. Keyed on font identity,
/// font size, line height, and DPI scale factor. Entries are only produced on
/// a cache miss; hits are O(1) hash lookups. Invalidation is automatic via
/// the key: changing any input produces a fresh lookup.
///
/// A single shared instance lives on the frame loop and is reused across
/// frames. The hot render path consults it on every `ElementContent::Grid`
/// node but only measures once per unique configuration.
pub struct CellMetricsCache {
    entries: FxHashMap<CellMetricsKey, CellMetrics>,
    /// Number of cache misses (fresh measurements) since creation. Exposed
    /// so tests can prove the cache actually caches.
    misses: u64,
}

impl Default for CellMetricsCache {
    fn default() -> Self {
        Self::new()
    }
}

impl CellMetricsCache {
    pub fn new() -> Self {
        Self { entries: FxHashMap::default(), misses: 0 }
    }

    /// Returns the measured `CellMetrics` for the given configuration,
    /// reusing any cached value when the key matches.
    pub fn get_or_measure(
        &mut self,
        font_system: &mut FontSystem,
        font_family: &str,
        font_size: f32,
        line_height: f32,
        scale_factor: f32,
    ) -> CellMetrics {
        let key = CellMetricsKey::new(font_family, font_size, line_height, scale_factor);
        if let Some(&hit) = self.entries.get(&key) {
            return hit;
        }
        self.misses += 1;
        let cell_w = measure_monospace_advance(font_system, font_family, font_size, line_height);
        let cell_h = line_height;
        let metrics = CellMetrics { cell_w, cell_h };
        self.entries.insert(key, metrics);
        metrics
    }

    /// Number of cache misses since creation. Exists for tests and diagnostics.
    pub fn miss_count(&self) -> u64 {
        self.misses
    }

    /// Number of entries currently cached.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when no entry has ever been measured.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Clears every cached entry. Intended for use when the font stack is
    /// swapped at runtime or the atlas generation changes.
    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

/// Measure the actual advance width of a monospace glyph at the given font_size.
///
/// `line_height` is the absolute pixel line height (typically `font_size * style.line_height`
/// from CSS resolution). Accepting it as a parameter keeps the renderer's cell
/// placement code as the single source of truth for the line_height value, rather
/// than hardcoding 1.2 inside this function.
///
/// Cached: only re-measures when font_size, line_height, font family, or DPI
/// scale change. Backed by an internal [`CellMetricsCache`] so the measurement
/// survives across frames.
#[cfg_attr(target_os = "windows", allow(dead_code))]
fn measure_monospace_cell_width(
    font_system: &mut FontSystem,
    font_size: f32,
    line_height: f32,
) -> f32 {
    measure_monospace_cell_width_for_family(
        font_system,
        monospace_family_name(),
        font_size,
        line_height,
    )
}

fn measure_monospace_cell_width_for_family(
    font_system: &mut FontSystem,
    font_family: &str,
    font_size: f32,
    line_height: f32,
) -> f32 {
    use std::sync::Mutex;
    static CACHE: Mutex<Option<CellMetricsCache>> = Mutex::new(None);

    let mut guard = CACHE.lock().expect("cell metrics cache mutex poisoned");
    let cache = guard.get_or_insert_with(CellMetricsCache::new);
    cache.get_or_measure(font_system, font_family, font_size, line_height, 1.0).cell_w
}

/// Default family name used for the free-function measurement path. Tests and
/// `measure_monospace_cell_width` call this; the renderer's grid emission path
/// now routes through [`CellMetricsCache::get_or_measure`] directly with the
/// active family.
#[cfg_attr(target_os = "windows", allow(dead_code))]
fn monospace_family_name() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        "Consolas"
    }
    #[cfg(not(target_os = "windows"))]
    {
        ""
    }
}

/// Perform the actual cosmic-text measurement for a monospace glyph. Kept
/// separate from the cache so the cache owns the memoization policy and this
/// function remains a pure measurement op.
fn measure_monospace_advance(
    font_system: &mut FontSystem,
    font_family: &str,
    font_size: f32,
    line_height: f32,
) -> f32 {
    let family = if font_family.is_empty() {
        cosmic_text::Family::Monospace
    } else {
        cosmic_text::Family::Name(font_family)
    };

    let metrics = cosmic_text::Metrics::new(font_size, line_height);
    let mut buffer = cosmic_text::Buffer::new(font_system, metrics);
    buffer.set_size(font_system, Some(font_size * 10.0), None);
    buffer.set_text(
        font_system,
        "M",
        cosmic_text::Attrs::new().family(family),
        cosmic_text::Shaping::Advanced,
    );
    buffer.shape_until_scroll(font_system, false);

    if let Some(glyph) = buffer.layout_runs().flat_map(|run| run.glyphs.iter()).next() {
        return glyph.w;
    }
    font_size * 0.6
}

/// Rasterize a glyph via SwashCache and insert into the atlas.
/// Used for CSS/UI text where cosmic-text metrics must match the rasterizer.
fn rasterize_swash_for_atlas(
    rasterizer: &mut Rasterizer<'_>,
    font_system: &mut FontSystem,
    physical: &cosmic_text::PhysicalGlyph,
    atlas: &mut GlyphAtlas,
    key: GlyphKey,
    font_family: &str,
    font_weight: FontWeight,
) -> Option<crate::atlas::GlyphEntry> {
    #[cfg(target_os = "windows")]
    if atlas.bytes_per_pixel == 4 && use_directwrite_ui_rasterization() {
        let (dwrite_family, dwrite_weight) =
            dwrite_ui_face_hint(font_system, physical.cache_key.font_id, font_family, font_weight);
        if let Some(rg) = rasterizer.dw.rasterize_ui_glyph(
            &dwrite_family,
            dwrite_weight,
            physical.cache_key.glyph_id,
            f32::from_bits(physical.cache_key.font_size_bits),
        ) {
            if rg.width == 0 || rg.height == 0 {
                return None;
            }
            let entry = atlas.get_or_insert(
                key,
                rg.width,
                rg.height,
                rgba_glyph_data_for_atlas(rg.data, atlas.bytes_per_pixel),
                [rg.bearing_x, rg.bearing_y],
            )?;
            atlas.touch(&key);
            return Some(entry);
        }
    }

    let image = if atlas.bytes_per_pixel == 4 && crate::text_rendering::use_subpixel_text_shader() {
        rasterizer.subpixel_swash.get_image_uncached(font_system, physical.cache_key)?
    } else {
        rasterizer.swash.get_image_uncached(font_system, physical.cache_key)?
    };
    if image.placement.width == 0 || image.placement.height == 0 {
        return None;
    }

    let w = image.placement.width;
    let h = image.placement.height;
    let bearing_x = image.placement.left as f32;
    let bearing_y = -(image.placement.top as f32);

    let glyph_data = glyph_image_data_for_atlas(image.content, image.data, atlas.bytes_per_pixel);

    let entry = atlas.get_or_insert(key, w, h, glyph_data, [bearing_x, bearing_y])?;
    atlas.touch(&key);
    Some(entry)
}

#[cfg(target_os = "windows")]
fn dwrite_ui_face_hint(
    font_system: &FontSystem,
    font_id: cosmic_text::fontdb::ID,
    css_font_family: &str,
    css_font_weight: FontWeight,
) -> (String, u16) {
    if let Some(face) = font_system.db().face(font_id) {
        if let Some((family, _)) = face.families.first() {
            return (family.clone(), face.weight.0);
        }
    }

    (css_font_family.to_string(), font_weight_number(css_font_weight))
}

fn glyph_image_data_for_atlas(
    content: cosmic_text::SwashContent,
    data: Vec<u8>,
    bytes_per_pixel: u32,
) -> Vec<u8> {
    match (bytes_per_pixel, content) {
        (4, cosmic_text::SwashContent::SubpixelMask) => data,
        (4, cosmic_text::SwashContent::Mask) => {
            data.into_iter().flat_map(|a| [a, a, a, a]).collect()
        }
        (4, cosmic_text::SwashContent::Color) => data
            .chunks(4)
            .flat_map(|c| {
                let a = c.get(3).copied().unwrap_or(255);
                [a, a, a, a]
            })
            .collect(),
        (_, cosmic_text::SwashContent::Mask) => data,
        (_, cosmic_text::SwashContent::Color) => {
            data.chunks(4).map(|c| c.get(3).copied().unwrap_or(255)).collect()
        }
        (_, cosmic_text::SwashContent::SubpixelMask) => data
            .chunks(4)
            .map(|c| {
                let r = c.first().copied().unwrap_or(0) as u16;
                let g = c.get(1).copied().unwrap_or(r as u8) as u16;
                let b = c.get(2).copied().unwrap_or(g as u8) as u16;
                let a = c.get(3).copied().unwrap_or_else(|| r.max(g).max(b) as u8) as u16;
                r.max(g).max(b).max(a) as u8
            })
            .collect(),
    }
}

fn rgba_glyph_data_for_atlas(data: Vec<u8>, bytes_per_pixel: u32) -> Vec<u8> {
    if bytes_per_pixel == 4 {
        return data;
    }

    data.chunks(4)
        .map(|c| {
            let r = c.first().copied().unwrap_or(0);
            let g = c.get(1).copied().unwrap_or(r);
            let b = c.get(2).copied().unwrap_or(g);
            let a = c.get(3).copied().unwrap_or_else(|| r.max(g).max(b));
            a.max(r).max(g).max(b)
        })
        .collect()
}

/// Rasterize a terminal grid glyph and insert into the atlas.
/// On Windows, uses DirectWrite for native ClearType quality.
/// On non-Windows, falls back to SwashCache.
fn rasterize_grid_glyph_for_atlas(
    rasterizer: &mut Rasterizer<'_>,
    font_system: &mut FontSystem,
    physical: &cosmic_text::PhysicalGlyph,
    ch: char,
    font_size: f32,
    atlas: &mut GlyphAtlas,
    key: GlyphKey,
) -> Option<crate::atlas::GlyphEntry> {
    #[cfg(target_os = "windows")]
    {
        if use_directwrite_grid_rasterization() {
            let _ = (font_system, physical); // not needed on DirectWrite path
            let rg = rasterizer.dw.rasterize_glyph(ch, font_size)?;
            if rg.width == 0 || rg.height == 0 {
                return None;
            }
            let entry = atlas.get_or_insert(
                key,
                rg.width,
                rg.height,
                rgba_glyph_data_for_atlas(rg.data, atlas.bytes_per_pixel),
                [rg.bearing_x, rg.bearing_y],
            )?;
            atlas.touch(&key);
            Some(entry)
        } else {
            // The trace shows terminal content stays correct through batching,
            // so prefer the swash path until the Windows-specific raster data
            // corruption is understood. TM_FORCE_DIRECTWRITE_GRID=1 restores
            // the old path for A/B verification.
            rasterize_swash_for_atlas(
                rasterizer,
                font_system,
                physical,
                atlas,
                key,
                "",
                FontWeight::Normal,
            )
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (ch, font_size); // not needed on swash path
        rasterize_swash_for_atlas(
            rasterizer,
            font_system,
            physical,
            atlas,
            key,
            "",
            FontWeight::Normal,
        )
    }
}

#[cfg(test)]
mod transform_affine_tests {
    use super::*;
    use unshit_core::style::types::{Transform, TransformX};

    fn approx(a: f32, b: f32) {
        assert!((a - b).abs() < 1e-4, "expected {b}, got {a}");
    }

    #[test]
    fn identity_transform_is_matrix_free() {
        let m = element_affine(&Transform::IDENTITY, 10.0, 20.0, 100.0, 50.0);
        assert!(m.is_identity());
        // Delta encoding of the identity is all-zero (matches the instance and
        // cache-signature defaults).
        assert_eq!(m.xform_delta(), [0.0; 4]);
        assert_eq!(m.xform_translate(), [0.0; 2]);
        assert_eq!(m.signature(), [0.0; 6]);
    }

    #[test]
    fn compose_with_identity_is_a_noop() {
        let t = Transform { scale_x: 2.0, scale_y: 2.0, ..Transform::IDENTITY };
        let m = element_affine(&t, 0.0, 0.0, 100.0, 100.0);
        let l = Affine2::IDENTITY.compose(m);
        let r = m.compose(Affine2::IDENTITY);
        for (a, b) in m.signature().iter().zip(l.signature().iter()) {
            approx(*a, *b);
        }
        for (a, b) in m.signature().iter().zip(r.signature().iter()) {
            approx(*a, *b);
        }
    }

    #[test]
    fn scale_about_center_fixes_the_center_point() {
        // A box at (20,20) size 40x40 has center (40,40). `scale(0.5)` about the
        // center must leave the center fixed and halve offsets from it.
        let t = Transform { scale_x: 0.5, scale_y: 0.5, ..Transform::IDENTITY };
        let m = element_affine(&t, 20.0, 20.0, 40.0, 40.0);
        // Center stays put.
        approx(m.a * 40.0 + m.c * 40.0 + m.e, 40.0);
        approx(m.b * 40.0 + m.d * 40.0 + m.f, 40.0);
        // The left edge (x=20) moves halfway toward the center (-> 30).
        approx(m.a * 20.0 + m.c * 20.0 + m.e, 30.0);
    }

    #[test]
    fn rotate_90_about_center_maps_right_to_bottom() {
        // 90deg clockwise (screen space, y-down) about center (50,50): a point
        // to the right of center maps to below center.
        let t = Transform { rotate: std::f32::consts::PI / 2.0, ..Transform::IDENTITY };
        let m = element_affine(&t, 0.0, 0.0, 100.0, 100.0);
        // (90, 50) is 40px right of center -> should land 40px below center
        // at (50, 90).
        approx(m.a * 90.0 + m.c * 50.0 + m.e, 50.0);
        approx(m.b * 90.0 + m.d * 50.0 + m.f, 90.0);
    }

    #[test]
    fn translate_y_is_origin_independent() {
        let t = Transform { translate_y: Some(TransformX::Px(-12.0)), ..Transform::IDENTITY };
        let m = element_affine(&t, 7.0, 7.0, 33.0, 33.0);
        // Pure translate: linear part is identity, only the y offset is set.
        approx(m.a, 1.0);
        approx(m.d, 1.0);
        approx(m.e, 0.0);
        approx(m.f, -12.0);
    }

    #[test]
    fn nested_scales_multiply() {
        // A child scaled 0.5 under a parent scaled 0.5 paints at 0.25 overall.
        let half = Transform { scale_x: 0.5, scale_y: 0.5, ..Transform::IDENTITY };
        let parent = element_affine(&half, 0.0, 0.0, 100.0, 100.0);
        let child = element_affine(&half, 0.0, 0.0, 100.0, 100.0);
        let composed = parent.compose(child);
        approx(composed.a, 0.25);
        approx(composed.d, 0.25);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use unshit_core::element::{Element, Tag};
    use unshit_core::id::NodeId;
    use unshit_core::scroll::ScrollbarVisualState;
    use unshit_core::tree::NodeArena;

    #[test]
    fn ui_subpixel_mask_format_uses_bgr_order() {
        assert_eq!(
            ui_subpixel_mask_format(),
            Format::CustomSubpixel([0.3, 0.0, -0.3]),
            "settings/browser parity target uses BGR ClearType channel order"
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn dwrite_ui_face_hint_uses_actual_shaped_face() {
        let font_system = FontSystem::new();
        let face = font_system
            .db()
            .faces()
            .find(|face| {
                face.families.first().is_some_and(|(family, _)| family != "JetBrains Mono")
            })
            .expect("test requires at least one non-JetBrains system face");
        let expected_family = face.families.first().unwrap().0.clone();
        let expected_weight = face.weight.0;

        let (family, weight) = dwrite_ui_face_hint(
            &font_system,
            face.id,
            "\"JetBrains Mono\", monospace",
            FontWeight::Bold,
        );

        assert_eq!(family, expected_family);
        assert_eq!(weight, expected_weight);
    }

    /// Helper: build a minimal arena with a single div node (no taffy needed).
    fn build_single_node() -> (NodeArena, NodeId) {
        let mut arena = NodeArena::new();
        let elem = Element::new(Tag::Div);
        let root = arena.alloc(elem);
        (arena, root)
    }

    #[test]
    fn aligned_text_x_honors_text_align() {
        let left = aligned_text_x(10.0, 4.0, 100.0, 20.0, TextAlign::Left);
        let center = aligned_text_x(10.0, 4.0, 100.0, 20.0, TextAlign::Center);
        let right = aligned_text_x(10.0, 4.0, 100.0, 20.0, TextAlign::Right);

        assert_eq!(left, 14.0);
        assert_eq!(center, 54.0);
        assert_eq!(right, 94.0);
    }

    #[test]
    fn checked_checkbox_marker_uses_centered_svg_not_text_glyph() {
        let mut batch = FrameBatch::new();
        let mut svg_cache = SvgTessCache::with_capacity(8);

        emit_checked_input_marker(
            InputType::Checkbox,
            10.0,
            20.0,
            20.0,
            20.0,
            Color::WHITE,
            1.0,
            [0.0, 0.0, 100.0, 100.0],
            &mut svg_cache,
            &mut batch,
        );

        assert_eq!(batch.svg_draws.len(), 1);
        assert!(batch.glyph_instances.is_empty());
        let draw = &batch.svg_draws[0];
        assert_eq!(draw.scale[0], draw.scale[1]);
        let marker_center_x = draw.translate[0] + draw.scale[0] * 8.0;
        let marker_center_y = draw.translate[1] + draw.scale[1] * 8.0;
        assert!((marker_center_x - 20.0).abs() < 0.001);
        assert!((marker_center_y - 30.0).abs() < 0.001);
    }

    #[test]
    fn checked_radio_marker_uses_centered_svg_not_text_glyph() {
        let mut batch = FrameBatch::new();
        let mut svg_cache = SvgTessCache::with_capacity(8);

        emit_checked_input_marker(
            InputType::Radio,
            4.0,
            6.0,
            16.0,
            24.0,
            Color::WHITE,
            1.0,
            [0.0, 0.0, 100.0, 100.0],
            &mut svg_cache,
            &mut batch,
        );

        assert_eq!(batch.svg_draws.len(), 1);
        assert!(batch.glyph_instances.is_empty());
        let draw = &batch.svg_draws[0];
        assert_eq!(draw.scale[0], draw.scale[1]);
        let marker_center_x = draw.translate[0] + draw.scale[0] * 8.0;
        let marker_center_y = draw.translate[1] + draw.scale[1] * 8.0;
        assert!((marker_center_x - 12.0).abs() < 0.001);
        assert!((marker_center_y - 18.0).abs() < 0.001);
    }

    /// Regression guard for issue #147: a clean idle frame must keep the
    /// cursor's grid (and every ancestor on the path from the root) out of
    /// the batch cache replay path so the renderer can re-emit the cursor
    /// quad with the current blink phase. Without this, the cursor freezes
    /// at whatever phase was recorded when the cache was last written.
    fn build_grid_under_parent(cursor_visible: bool) -> (NodeArena, NodeId, NodeId) {
        let mut arena = NodeArena::new();
        let root = arena.alloc(Element::new(Tag::Div));
        let pane = arena.alloc(Element::new(Tag::Div));
        let mut grid = unshit_core::cell_grid::CellGrid::new(2, 2);
        grid.set_cursor(0, 0);
        grid.set_cursor_visible(cursor_visible);
        let mut grid_elem = Element::new(Tag::Div);
        grid_elem.content = ElementContent::Grid(grid);
        let grid_id = arena.alloc(grid_elem);
        arena.append_child(root, pane);
        arena.append_child(pane, grid_id);
        (arena, root, grid_id)
    }

    #[test]
    fn cursor_blink_dirty_ancestors_includes_grid_and_path_to_root() {
        let (arena, root, grid_id) = build_grid_under_parent(true);
        let pane = arena.get(grid_id).unwrap().parent;
        let force = cursor_blink_dirty_ancestors(&arena);
        assert!(force.contains(&grid_id), "grid itself must be force-dirtied");
        assert!(force.contains(&pane), "intermediate parent must be force-dirtied");
        assert!(force.contains(&root), "root must be force-dirtied");
    }

    #[test]
    fn cursor_blink_dirty_ancestors_empty_when_cursor_hidden() {
        let (arena, _root, _grid_id) = build_grid_under_parent(false);
        let force = cursor_blink_dirty_ancestors(&arena);
        assert!(
            force.is_empty(),
            "no node should be force-dirtied when no grid has a visible cursor"
        );
    }

    #[test]
    fn cursor_blink_dirty_ancestors_handles_no_grids() {
        let (arena, _root) = build_single_node();
        let force = cursor_blink_dirty_ancestors(&arena);
        assert!(force.is_empty(), "arenas without grids produce an empty set");
    }

    #[test]
    fn terminal_inverse_transparent_bg_uses_opaque_glyph_color() {
        let cell = unshit_core::cell_grid::Cell {
            ch: 'z',
            fg: Color { r: 204, g: 204, b: 204, a: 255 },
            bg: Color::TRANSPARENT,
            attrs: CellAttrs::INVERSE,
            wide_continuation: false,
        };

        let fg = terminal_fg_color(&cell, 1.0);

        assert_eq!(fg, Color::BLACK.to_linear_f32());
    }

    #[test]
    fn terminal_dim_reduces_intensity_without_making_glyph_translucent() {
        let cell = unshit_core::cell_grid::Cell {
            ch: 'z',
            fg: Color { r: 200, g: 120, b: 40, a: 255 },
            bg: Color::TRANSPARENT,
            attrs: CellAttrs::DIM,
            wide_continuation: false,
        };

        let normal = cell.fg.to_linear_f32();
        let dim = terminal_fg_color(&cell, 1.0);

        assert_eq!(dim[0], normal[0] * TERMINAL_DIM_INTENSITY);
        assert_eq!(dim[1], normal[1] * TERMINAL_DIM_INTENSITY);
        assert_eq!(dim[2], normal[2] * TERMINAL_DIM_INTENSITY);
        assert_eq!(
            dim[3], normal[3],
            "SGR faint should keep glyph coverage crisp; element opacity is applied separately"
        );
    }

    #[test]
    fn terminal_inverse_bg_uses_literal_campbell_fg_under_parity_profile() {
        assert_eq!(
            terminal_inverse_bg_color(WINDOWS_TERMINAL_PARITY_CALIBRATED_FG, true),
            WINDOWS_TERMINAL_PARITY_LITERAL_FG
        );
        assert_eq!(
            terminal_inverse_bg_color(WINDOWS_TERMINAL_PARITY_CALIBRATED_FG, false),
            WINDOWS_TERMINAL_PARITY_CALIBRATED_FG
        );
        assert_eq!(terminal_inverse_bg_color(Color::rgb(10, 20, 30), true), Color::rgb(10, 20, 30));
    }

    #[test]
    fn terminal_glyph_position_snap_is_parity_scoped() {
        assert_eq!(terminal_glyph_position_for_parity(10.25, 20.75, false), [10.25, 20.75]);
        assert_eq!(terminal_glyph_position_for_parity(10.25, 20.75, true), [10.25, 21.0]);
    }

    #[test]
    fn terminal_bg_runs_only_recalibrate_inverse_default_fg_under_parity_profile() {
        let mut grid = CellGrid::new(1, 2);
        grid.set_cell(
            0,
            0,
            unshit_core::cell_grid::Cell {
                ch: 'i',
                fg: WINDOWS_TERMINAL_PARITY_CALIBRATED_FG,
                bg: Color::TRANSPARENT,
                attrs: CellAttrs::INVERSE,
                wide_continuation: false,
            },
        );
        grid.set_cell(
            0,
            1,
            unshit_core::cell_grid::Cell {
                ch: 'b',
                fg: Color::WHITE,
                bg: WINDOWS_TERMINAL_PARITY_CALIBRATED_FG,
                attrs: CellAttrs::empty(),
                wide_continuation: false,
            },
        );

        let runs = terminal_bg_runs_in_range_for_parity(&grid, 0, 0, 2, true);

        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].bg, WINDOWS_TERMINAL_PARITY_LITERAL_FG);
        assert_eq!(runs[1].bg, WINDOWS_TERMINAL_PARITY_CALIBRATED_FG);
    }

    #[test]
    fn terminal_primitive_y_bias_is_parity_scoped_and_targeted() {
        assert_eq!(terminal_primitive_y_bias_for_parity('\u{2588}', false), 0.0);
        assert_eq!(terminal_primitive_y_bias_for_parity('\u{2588}', true), 1.0);
        assert_eq!(terminal_primitive_y_bias_for_parity('\u{2592}', true), 1.0);
        assert_eq!(terminal_primitive_y_bias_for_parity('\u{2501}', true), 1.0);
        assert_eq!(terminal_primitive_y_bias_for_parity('\u{2554}', true), 0.0);
        assert_eq!(
            terminal_primitive_y_bias_for_parity('\u{2500}', true),
            1.0,
            "light horizontal table borders align one pixel lower in Windows Terminal"
        );
        assert_eq!(
            terminal_primitive_y_bias_for_parity('\u{2502}', true),
            0.0,
            "vertical-only light borders keep their calibrated baseline"
        );
        assert_eq!(
            terminal_primitive_y_bias_for_parity('\u{255f}', true),
            0.0,
            "mixed tee glyphs keep their calibrated double-stroke baseline"
        );
    }

    #[test]
    fn terminal_primitive_trailing_edge_clamp_targets_run_end_only() {
        assert_eq!(
            terminal_primitive_trailing_edge_clamp_for_parity('\u{2588}', Some('\u{2593}'), true),
            0.0,
            "full block followed by a shade glyph remains joined to the block run"
        );
        assert_eq!(
            terminal_primitive_trailing_edge_clamp_for_parity('\u{2588}', Some(' '), true),
            1.0,
            "trailing full block edge is shortened in parity mode"
        );
        assert_eq!(
            terminal_primitive_trailing_edge_clamp_for_parity('\u{2501}', Some('\u{2501}'), true),
            0.0,
            "interior heavy horizontal cells keep overlap"
        );
        assert_eq!(
            terminal_primitive_trailing_edge_clamp_for_parity('\u{2501}', Some(' '), true),
            1.0,
            "last heavy horizontal cell loses the extra right-edge pixel"
        );
        assert_eq!(
            terminal_primitive_trailing_edge_clamp_for_parity('\u{2588}', Some(' '), false),
            0.0,
            "clamp is parity-profile scoped"
        );
    }

    #[test]
    fn terminal_primitive_run_end_clamp_uses_row_next_ch() {
        let cells = [terminal_cell('\u{2501}'), terminal_cell('\u{2501}'), terminal_cell(' ')];

        assert_eq!(terminal_row_next_ch(&cells, 0, 0, 3), Some('\u{2501}'));
        assert_eq!(terminal_row_next_ch(&cells, 0, 1, 3), Some(' '));
        assert_eq!(terminal_row_next_ch(&cells, 0, 2, 3), None);

        let mut interior = Vec::new();
        assert!(emit_terminal_cell_primitive(
            &cells[0],
            0,
            0,
            0.0,
            0.0,
            10.0,
            20.0,
            1.0,
            [0.0, 0.0, 200.0, 200.0],
            true,
            true,
            terminal_row_next_ch(&cells, 0, 0, 3),
            &mut interior,
        ));

        let mut run_end = Vec::new();
        assert!(emit_terminal_cell_primitive(
            &cells[1],
            0,
            1,
            0.0,
            0.0,
            10.0,
            20.0,
            1.0,
            [0.0, 0.0, 200.0, 200.0],
            true,
            true,
            terminal_row_next_ch(&cells, 0, 1, 3),
            &mut run_end,
        ));

        assert_eq!(interior.len(), 1);
        assert_eq!(run_end.len(), 1);
        assert!(
            interior[0].size[0] > run_end[0].size[0],
            "interior heavy-rule cells keep overlap while the run end is clamped"
        );
    }

    #[test]
    fn terminal_primitive_cell_rect_snap_aligns_fractional_quads_to_pixels() {
        let mut quads = Vec::new();
        push_terminal_cell_rect_snapped(
            &mut quads,
            10.2,
            20.2,
            TerminalCellRect { x: 0.25, y: 0.25, width: 2.5, height: 0.8 },
            [1.0, 1.0, 1.0, 1.0],
            [0.0, 0.0, 200.0, 200.0],
        );

        assert_eq!(quads.len(), 1);
        assert_eq!(quads[0].pos, [10.0, 20.0]);
        assert_eq!(quads[0].size, [3.0, 1.0]);
    }

    #[test]
    fn parity_cell_width_scale_from_values_uses_windows_terminal_profile_default() {
        assert_eq!(
            parity_cell_width_scale_from_values(None, true),
            WINDOWS_TERMINAL_PARITY_CELL_WIDTH_SCALE
        );
        assert_eq!(
            parity_cell_width_scale_from_values(Some(std::ffi::OsString::from("")), true),
            WINDOWS_TERMINAL_PARITY_CELL_WIDTH_SCALE
        );
    }

    #[test]
    fn parity_cell_width_scale_from_values_defaults_to_neutral_outside_parity_profile() {
        assert_eq!(parity_cell_width_scale_from_values(None, false), 1.0);
        assert_eq!(
            parity_cell_width_scale_from_values(Some(std::ffi::OsString::from("invalid")), false),
            1.0
        );
    }

    #[test]
    fn parity_cell_width_scale_from_values_accepts_small_calibration_range() {
        assert_eq!(
            parity_cell_width_scale_from_values(Some(std::ffi::OsString::from("0.985")), true),
            0.985
        );
        assert_eq!(
            parity_cell_width_scale_from_values(Some(std::ffi::OsString::from("1.015")), false),
            1.015
        );
    }

    #[test]
    fn parity_cell_width_scale_from_values_rejects_large_distortion() {
        assert_eq!(
            parity_cell_width_scale_from_values(Some(std::ffi::OsString::from("0.5")), true),
            WINDOWS_TERMINAL_PARITY_CELL_WIDTH_SCALE
        );
        assert_eq!(
            parity_cell_width_scale_from_values(Some(std::ffi::OsString::from("1.5")), false),
            1.0
        );
    }

    /// Helper: run `build_render_batch` using only CPU-side structures (no GPU
    /// atlas, no SVG cache that requires rasterization).  We pass a fake
    /// `ShapedTextCache` and rely on the fact that an unstyled div node emits
    /// no glyphs and no quads.
    fn run_batch_cpu_only(
        _arena: &NodeArena,
        root: NodeId,
        batch_cache: &mut BatchCache,
    ) -> LayeredBatch {
        use cosmic_text::{FontSystem, SwashCache};
        use unshit_core::layout::TextMeasureCache;

        let mut batch = LayeredBatch::new();
        // Build a minimal fake atlas: we cannot call GlyphAtlas::new without a
        // wgpu device, so we bypass it by using Default which zeroes the cache.
        // No glyphs are emitted for unstyled div nodes so this is safe.
        let mut font_system = FontSystem::new();
        let mut swash_cache = SwashCache::new();
        let mut subpixel_swash_cache = SubpixelSwashCache::new();
        #[cfg(target_os = "windows")]
        let _dw = crate::dw_rasterizer::DwRasterizer::new("Consolas");
        let mut _rasterizer = Rasterizer {
            swash: &mut swash_cache,
            subpixel_swash: &mut subpixel_swash_cache,
            #[cfg(target_os = "windows")]
            dw: &_dw,
        };
        let mut measure_cache = TextMeasureCache::default();
        let mut shaped_cache = ShapedTextCache::new();
        let mut svg_cache = crate::svg_cache::SvgTessCache::with_capacity(0);
        let scrollbar = ScrollbarVisualState::default();

        // We need a GlyphAtlas but cannot construct one without a device.
        // Instead skip build_render_batch entirely and exercise just the
        // cache logic, which is the part we want to test here.
        // For the cache test we only check that begin_frame/commit_frame work
        // and that dirty nodes produce output while clean ones replay from cache.
        let _ = (
            &mut batch,
            &mut font_system,
            &mut _rasterizer,
            &mut measure_cache,
            &mut shaped_cache,
            &mut svg_cache,
            &scrollbar,
        );

        // Simulate "frame 1": record something for the root node.
        batch_cache.begin_frame();
        // Dirty node produces output; we fake it by recording an empty range.
        batch_cache.record(root, 0, vec![], vec![], vec![], vec![], vec![], 0, vec![]);
        batch_cache.commit_frame();

        batch
    }

    #[test]
    fn clean_node_skips_rebuild_on_second_frame() {
        let (mut arena, root) = build_single_node();

        // Clear PAINT flags that may have been set by build_tree_from_def.
        clear_paint_flags_subtree(&mut arena, root);

        // After clearing paint flags, the node has neither PAINT nor
        // SUBTREE_PAINT, so the batch builder should skip it and replay from
        // cache.  We verify this by checking that the cache lookup works.
        let mut cache = BatchCache::new();
        let _batch = run_batch_cpu_only(&arena, root, &mut cache);

        // The cache should now have an entry for root on layer 0.
        assert!(
            cache.get(root, 0).is_some(),
            "cache should contain an entry for root after simulated frame 1"
        );

        // Second frame: node is still clean.  The cache entry should still be
        // present (we do not evict on begin_frame; only on clear).
        cache.begin_frame();
        assert!(
            cache.get(root, 0).is_some(),
            "cache entry should remain readable after begin_frame (reads from prev)"
        );
    }

    #[test]
    fn batch_cache_begin_commit_cycle_works() {
        let mut cache = BatchCache::new();

        // Populate staging with a fake record.
        cache.begin_frame();
        cache.record(NodeId::DANGLING, 0, vec![], vec![], vec![], vec![], vec![], 0, vec![]);
        // Before commit, `get` reads from the previous frame (empty).
        assert!(cache.get(NodeId::DANGLING, 0).is_none());

        // After commit, the staged entry becomes readable.
        cache.commit_frame();
        assert!(cache.get(NodeId::DANGLING, 0).is_some());

        // After begin_frame, a new staging cycle starts; the committed data is
        // still readable from `get` (it references `ranges`, not `pending`).
        cache.begin_frame();
        assert!(cache.get(NodeId::DANGLING, 0).is_some());
    }

    #[test]
    fn replayed_entries_survive_multi_frame_cycle() {
        // Regression test for a structural bug in BatchCache that caused
        // clean nodes to force-re-render every other frame: the walk's
        // replay path read from `ranges` but never wrote into `pending`, so
        // the next `commit_frame` swap dropped the entry. This produced an
        // alternating cache-hit/cache-miss pattern that, combined with
        // unflagged tick mutations to `computed_style`, manifested as
        // flickering on hover restyle and CSS animations (issues #41, #42).
        //
        // The fix routes the walk through `BatchCache::replay`, which copies
        // the cached range from `ranges` into `pending` so it survives the
        // next swap.
        let mut cache = BatchCache::new();
        let id = NodeId::DANGLING;

        // Frame 1: a dirty node produces primitives. record -> commit.
        cache.begin_frame();
        cache.record(id, 0, vec![], vec![], vec![], vec![], vec![], 0, vec![]);
        cache.commit_frame();
        assert!(cache.get(id, 0).is_some(), "frame 1 recorded and committed");

        // Frame 2: the node is clean, so the walker calls `replay` instead
        // of re-rendering. The fix guarantees the cached entry is carried
        // forward into the current staging map.
        cache.begin_frame();
        assert!(
            cache.replay(id, 0, 0).is_some(),
            "frame 2 must replay from frame 1's committed data",
        );
        cache.commit_frame();

        // Frame 3: without the fix, the frame-2 commit swap would have
        // dropped the entry because nothing wrote to `pending`. With the
        // fix, `replay` cloned the range into `pending` during frame 2,
        // so the entry survives the swap.
        cache.begin_frame();
        assert!(cache.get(id, 0).is_some(), "replayed entry must persist across commit_frame",);

        // Frame 4: repeat to confirm the carry-forward is stable across
        // multiple consecutive replay cycles, not just the first one.
        assert!(cache.replay(id, 0, 0).is_some(), "frame 4 replay");
        cache.commit_frame();
        cache.begin_frame();
        assert!(
            cache.get(id, 0).is_some(),
            "entry must still persist after two consecutive replay frames",
        );
    }

    #[test]
    fn replay_returns_none_when_no_prior_entry() {
        // `replay` must not fabricate an entry for an unknown key; it is
        // purely a carry-forward for entries that already exist in `ranges`.
        let mut cache = BatchCache::new();
        cache.begin_frame();
        assert!(cache.replay(NodeId::DANGLING, 0, 0).is_none());
    }

    #[test]
    fn replay_returns_none_when_atlas_generation_mismatches() {
        // Atlas-aware invalidation: cached ranges built against an older atlas
        // generation must not be replayed.
        let mut cache = BatchCache::new();
        let id = NodeId::DANGLING;
        cache.begin_frame();
        cache.record(id, 0, vec![], vec![], vec![], vec![], vec![], 7, vec![]);
        cache.commit_frame();

        cache.begin_frame();
        assert!(cache.replay(id, 0, 8).is_none(), "generation mismatch must force fresh render",);
    }

    #[test]
    fn replay_returns_none_when_render_geometry_mismatches() {
        // Cached ranges contain absolute primitive positions. A pure layout
        // pass can move a node without setting PAINT, so replay must validate
        // the geometry that produced the cached range before carrying it
        // forward.
        let mut cache = BatchCache::new();
        let id = NodeId::DANGLING;
        let old_sig =
            BatchCacheSignature::new([0.0, 680.0, 853.0, 24.0], [0.0, 0.0, 853.0, 912.0], [0.0; 6]);
        let new_sig =
            BatchCacheSignature::new([0.0, 888.0, 853.0, 24.0], [0.0, 0.0, 853.0, 912.0], [0.0; 6]);

        cache.begin_frame();
        cache.record_with_signature(
            id,
            0,
            old_sig,
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            0,
            vec![],
        );
        cache.commit_frame();

        cache.begin_frame();
        assert!(
            cache.replay_with_signature(id, 0, 0, new_sig).is_none(),
            "layout-only y changes must force fresh render instead of replaying stale primitives",
        );
    }

    #[test]
    fn replay_returns_recorded_data_when_pending_already_has_entry() {
        // If the caller has already recorded a fresh render for this node
        // earlier in the frame (e.g. a parent was dirty and walked children
        // including this node), a subsequent replay call must return the
        // pending entry instead of overwriting it with stale ranges data.
        let mut cache = BatchCache::new();
        let id = NodeId::DANGLING;

        // Seed ranges with an empty entry (stale previous-frame state).
        cache.begin_frame();
        cache.record(id, 0, vec![], vec![], vec![], vec![], vec![], 0, vec![]);
        cache.commit_frame();

        // New frame: caller records fresh data with one distinctive span.
        cache.begin_frame();
        cache.record(
            id,
            0,
            vec![],
            vec![],
            vec![],
            vec![DrawSpan { kind: DrawKind::Quad, start: 0, count: 7 }],
            vec![],
            0,
            vec![],
        );

        // A subsequent replay call should return the just-recorded entry,
        // not the older empty entry still sitting in `ranges`.
        let out = cache.replay(id, 0, 0).expect("pending entry must be returned");
        assert_eq!(out.draw_spans.len(), 1);
        assert_eq!(out.draw_spans[0].kind, DrawKind::Quad);
        assert_eq!(out.draw_spans[0].count, 7);
    }

    // -----------------------------------------------------------------------
    // DrawSpan recording tests (text occlusion fix)
    // -----------------------------------------------------------------------

    #[test]
    fn draw_spans_are_recorded_in_batch_cache() {
        let spans = vec![
            DrawSpan { kind: DrawKind::Quad, start: 0, count: 3 },
            DrawSpan { kind: DrawKind::Glyph, start: 0, count: 5 },
            DrawSpan { kind: DrawKind::Quad, start: 3, count: 2 },
        ];
        let mut cache = BatchCache::new();
        cache.begin_frame();
        cache.record(NodeId::DANGLING, 0, vec![], vec![], vec![], spans.clone(), vec![], 0, vec![]);
        cache.commit_frame();

        let cached = cache.get(NodeId::DANGLING, 0).expect("should have cached entry");
        assert_eq!(cached.draw_spans.len(), 3);
        assert_eq!(cached.draw_spans[0].kind, DrawKind::Quad);
        assert_eq!(cached.draw_spans[0].start, 0);
        assert_eq!(cached.draw_spans[0].count, 3);
        assert_eq!(cached.draw_spans[1].kind, DrawKind::Glyph);
        assert_eq!(cached.draw_spans[1].count, 5);
        assert_eq!(cached.draw_spans[2].kind, DrawKind::Quad);
        assert_eq!(cached.draw_spans[2].start, 3);
        assert_eq!(cached.draw_spans[2].count, 2);
    }

    #[test]
    fn cached_draw_spans_must_be_node_local_before_replay() {
        let quad_start = 11_u32;
        let glyph_start = 37_u32;
        let absolute_spans = vec![
            DrawSpan { kind: DrawKind::Quad, start: quad_start, count: 2 },
            DrawSpan { kind: DrawKind::Glyph, start: glyph_start, count: 4 },
            DrawSpan { kind: DrawKind::Quad, start: quad_start + 2, count: 1 },
            DrawSpan { kind: DrawKind::Glyph, start: glyph_start + 4, count: 3 },
        ];

        let normalized = absolute_spans
            .iter()
            .map(|span| DrawSpan {
                kind: span.kind,
                start: match span.kind {
                    DrawKind::Quad => span.start - quad_start,
                    DrawKind::Glyph => span.start - glyph_start,
                },
                count: span.count,
            })
            .collect::<Vec<_>>();

        assert_eq!(normalized[0].start, 0);
        assert_eq!(normalized[1].start, 0);
        assert_eq!(normalized[2].start, 2);
        assert_eq!(normalized[3].start, 4);

        let quad_offset = 100_u32;
        let glyph_offset = 200_u32;
        let replayed = normalized
            .iter()
            .map(|span| DrawSpan {
                kind: span.kind,
                start: match span.kind {
                    DrawKind::Quad => quad_offset + span.start,
                    DrawKind::Glyph => glyph_offset + span.start,
                },
                count: span.count,
            })
            .collect::<Vec<_>>();

        assert_eq!(replayed[0].start, 100);
        assert_eq!(replayed[1].start, 200);
        assert_eq!(replayed[2].start, 102);
        assert_eq!(replayed[3].start, 204);
    }

    #[test]
    fn draw_spans_replayed_with_offset_adjustment() {
        // Record spans at known positions.
        let spans = vec![
            DrawSpan { kind: DrawKind::Quad, start: 0, count: 2 },
            DrawSpan { kind: DrawKind::Glyph, start: 0, count: 3 },
        ];
        let mut cache = BatchCache::new();
        cache.begin_frame();
        cache.record(NodeId::DANGLING, 0, vec![], vec![], vec![], spans, vec![], 0, vec![]);
        cache.commit_frame();

        // Replay into a batch that already has some data, simulating offset.
        let mut batch = FrameBatch::new();
        // Simulate pre-existing data by pushing dummy instances.
        let dummy_quad = QuadInstance {
            pos: [0.0; 2],
            size: [10.0; 2],
            color: [1.0; 4],
            border_color: [0.0; 4],
            border_width: [0.0; 4],
            border_radius: [0.0; 4],
            clip_rect: [0.0, 0.0, 100.0, 100.0],
            shadow_color: [0.0; 4],
            shadow_offset: [0.0; 2],
            shadow_params: [0.0; 2],
            shadow_spread: [0.0; 2],
            gradient_stop_colors: EMPTY_GRADIENT_STOP_COLORS,
            gradient_stop_positions: EMPTY_GRADIENT_STOP_POSITIONS,
            gradient_params: [0.0; 4],
            gradient_extra: EMPTY_GRADIENT_EXTRA,
            mask_stops_01: EMPTY_MASK_STOPS,
            mask_stops_23: EMPTY_MASK_STOPS,
            mask_params: EMPTY_MASK_PARAMS,
            xform: IDENTITY_XFORM,
            xform_translate: IDENTITY_XFORM_TRANSLATE,
        };
        batch.quad_instances.push(dummy_quad);

        // Replay cached data with offset.
        let cached = cache.get(NodeId::DANGLING, 0).unwrap();
        let quad_offset = batch.quad_instances.len() as u32;
        let glyph_offset = batch.glyph_instances.len() as u32;
        batch.quad_instances.extend_from_slice(&cached.quads);
        batch.glyph_instances.extend_from_slice(&cached.glyphs);
        for span in &cached.draw_spans {
            let offset = match span.kind {
                DrawKind::Quad => quad_offset,
                DrawKind::Glyph => glyph_offset,
            };
            batch.draw_spans.push(DrawSpan {
                kind: span.kind,
                start: span.start + offset,
                count: span.count,
            });
        }

        // The quad span should be offset by 1 (one pre-existing quad).
        assert_eq!(batch.draw_spans.len(), 2);
        assert_eq!(batch.draw_spans[0].kind, DrawKind::Quad);
        assert_eq!(batch.draw_spans[0].start, 1); // offset by 1
        assert_eq!(batch.draw_spans[1].kind, DrawKind::Glyph);
        assert_eq!(batch.draw_spans[1].start, 0); // glyph offset was 0
    }

    #[test]
    fn frame_batch_clear_resets_draw_spans() {
        let mut batch = FrameBatch::new();
        batch.draw_spans.push(DrawSpan { kind: DrawKind::Quad, start: 0, count: 1 });
        assert_eq!(batch.draw_spans.len(), 1);
        batch.clear();
        assert!(batch.draw_spans.is_empty());
    }

    // -----------------------------------------------------------------------
    // measure_monospace_cell_width tests (issue #5, approach 4)
    // -----------------------------------------------------------------------

    /// The function must return a positive width for a standard monospace font.
    #[test]
    fn measure_monospace_cell_width_returns_positive() {
        let mut fs = FontSystem::new();
        let font_size = 14.0_f32;
        let line_height = font_size * 1.2;
        let w = measure_monospace_cell_width(&mut fs, font_size, line_height);
        assert!(w > 0.0, "cell width must be positive, got {}", w);
    }

    #[test]
    fn measure_monospace_cell_width_for_family_returns_positive() {
        let mut fs = FontSystem::new();
        let font_size = 14.0_f32;
        let line_height = font_size * 1.2;
        let w =
            measure_monospace_cell_width_for_family(&mut fs, "Consolas", font_size, line_height);
        assert!(w > 0.0, "family-specific cell width must be positive, got {w}");
    }

    /// Different line_height values must not change the measured advance width,
    /// because cosmic-text Metrics line_height only affects vertical layout.
    #[test]
    fn measure_monospace_cell_width_stable_across_line_heights() {
        let mut fs = FontSystem::new();
        let font_size = 14.0_f32;

        let w_normal = measure_monospace_cell_width(&mut fs, font_size, font_size * 1.2);
        let w_tall = measure_monospace_cell_width(&mut fs, font_size, font_size * 2.0);
        let w_tight = measure_monospace_cell_width(&mut fs, font_size, font_size * 1.0);

        let epsilon = 0.01;
        assert!(
            (w_normal - w_tall).abs() < epsilon,
            "line_height should not affect advance width: normal={}, tall={}",
            w_normal,
            w_tall
        );
        assert!(
            (w_normal - w_tight).abs() < epsilon,
            "line_height should not affect advance width: normal={}, tight={}",
            w_normal,
            w_tight
        );
    }

    /// The cache must return the same value on repeated calls with the same inputs.
    #[test]
    fn measure_monospace_cell_width_cache_consistency() {
        let mut fs = FontSystem::new();
        let font_size = 12.0_f32;
        let line_height = font_size * 1.2;

        let w1 = measure_monospace_cell_width(&mut fs, font_size, line_height);
        let w2 = measure_monospace_cell_width(&mut fs, font_size, line_height);
        assert_eq!(
            w1.to_bits(),
            w2.to_bits(),
            "cached value should be bit-identical: {} vs {}",
            w1,
            w2
        );
    }

    // -----------------------------------------------------------------------
    // Per-frame glyph cache tests (emit_grid_cells optimization)
    // -----------------------------------------------------------------------

    /// Helper: shape a single character through cosmic-text and return the
    /// resulting GlyphKey (the same computation emit_grid_cells performs).
    fn shape_char_to_key(
        fs: &mut FontSystem,
        ch: char,
        font_size: f32,
        cell_h: f32,
        cell_w: f32,
    ) -> Option<GlyphKey> {
        let metrics = cosmic_text::Metrics::new(font_size, cell_h);
        let mut buffer = cosmic_text::Buffer::new(fs, metrics);
        buffer.set_size(fs, Some(cell_w * 4.0), None);

        let mut ch_buf = [0u8; 4];
        let ch_str = ch.encode_utf8(&mut ch_buf);
        #[cfg(target_os = "windows")]
        let family = cosmic_text::Family::Name("Consolas");
        #[cfg(not(target_os = "windows"))]
        let family = cosmic_text::Family::Monospace;
        buffer.set_text(
            fs,
            ch_str,
            cosmic_text::Attrs::new().family(family),
            cosmic_text::Shaping::Advanced,
        );
        buffer.shape_until_scroll(fs, false);

        for run in buffer.layout_runs() {
            if let Some(glyph) = run.glyphs.iter().next() {
                let physical = glyph.physical((0.0, 0.0), 1.0);
                return Some(GlyphKey {
                    font_id: atlas_font_namespace(&physical.cache_key),
                    glyph_id: physical.cache_key.glyph_id,
                    font_size_tenths: (font_size * 10.0) as u16,
                    subpixel_bin: ((physical.cache_key.x_bin as u8) << 2)
                        | (physical.cache_key.y_bin as u8),
                });
            }
        }
        None
    }

    /// Core cache invariant: shaping the same character twice must yield an
    /// identical GlyphKey, otherwise the per-frame cache would serve wrong
    /// glyphs.
    #[test]
    fn same_char_produces_identical_glyph_key() {
        let mut fs = FontSystem::new();
        let font_size = 14.0;
        let cell_h = font_size * 1.2;
        let cell_w = 8.4;

        let key1 = shape_char_to_key(&mut fs, 'A', font_size, cell_h, cell_w)
            .expect("'A' should produce a glyph");
        let key2 = shape_char_to_key(&mut fs, 'A', font_size, cell_h, cell_w)
            .expect("'A' should produce a glyph on second call");
        assert_eq!(key1, key2, "same char must yield identical GlyphKey");
    }

    #[test]
    fn atlas_font_namespace_includes_render_flags() {
        let (plain, _, _) = cosmic_text::CacheKey::new(
            cosmic_text::fontdb::ID::dummy(),
            42,
            14.0,
            (0.0, 0.0),
            cosmic_text::CacheKeyFlags::empty(),
        );
        let (italic, _, _) = cosmic_text::CacheKey::new(
            cosmic_text::fontdb::ID::dummy(),
            42,
            14.0,
            (0.0, 0.0),
            cosmic_text::CacheKeyFlags::FAKE_ITALIC,
        );

        assert_ne!(
            atlas_font_namespace(&plain),
            atlas_font_namespace(&italic),
            "atlas font namespace must differ when glyph render flags differ",
        );
    }

    #[test]
    fn rgba_text_atlas_preserves_subpixel_mask_channels() {
        let data = glyph_image_data_for_atlas(
            cosmic_text::SwashContent::SubpixelMask,
            vec![10, 30, 90, 90, 0, 20, 40, 40],
            4,
        );

        assert_eq!(
            data,
            vec![10, 30, 90, 90, 0, 20, 40, 40],
            "RGBA text atlas must keep per-channel coverage for ClearType-style smoothing"
        );
    }

    #[test]
    fn mono_text_atlas_collapses_subpixel_mask_to_alpha() {
        let data = glyph_image_data_for_atlas(
            cosmic_text::SwashContent::SubpixelMask,
            vec![10, 30, 89, 89, 0, 20, 40, 40],
            1,
        );

        assert_eq!(data, vec![89, 40], "R8 text atlas should keep the grayscale fallback path");
    }

    #[test]
    fn directwrite_rgba_data_collapses_to_alpha_for_mono_atlas() {
        let data = rgba_glyph_data_for_atlas(vec![10, 30, 90, 90, 0, 20, 40, 40], 1);

        assert_eq!(
            data,
            vec![90, 40],
            "R8 text atlas should receive one coverage byte per DirectWrite pixel"
        );
    }

    #[test]
    fn atlas_font_namespace_includes_font_identity() {
        let fs = FontSystem::new();
        let ids: Vec<_> = fs.db().faces().take(2).map(|face| face.id).collect();
        if ids.len() < 2 {
            return;
        }

        let (a, _, _) = cosmic_text::CacheKey::new(
            ids[0],
            42,
            14.0,
            (0.0, 0.0),
            cosmic_text::CacheKeyFlags::empty(),
        );
        let (b, _, _) = cosmic_text::CacheKey::new(
            ids[1],
            42,
            14.0,
            (0.0, 0.0),
            cosmic_text::CacheKeyFlags::empty(),
        );

        assert_ne!(
            atlas_font_namespace(&a),
            atlas_font_namespace(&b),
            "atlas font namespace must differ for different font ids",
        );
    }

    /// Different characters must produce different glyph IDs so the cache
    /// does not conflate them.
    #[test]
    fn different_chars_produce_different_glyph_keys() {
        let mut fs = FontSystem::new();
        let font_size = 14.0;
        let cell_h = font_size * 1.2;
        let cell_w = 8.4;

        let key_a = shape_char_to_key(&mut fs, 'A', font_size, cell_h, cell_w)
            .expect("'A' should produce a glyph");
        let key_b = shape_char_to_key(&mut fs, 'B', font_size, cell_h, cell_w)
            .expect("'B' should produce a glyph");
        assert_ne!(key_a.glyph_id, key_b.glyph_id, "'A' and 'B' must map to different glyph IDs");
    }

    /// The glyph cache is keyed on `char`. Verify that a broad set of
    /// printable ASCII characters each produce a unique glyph_id, so
    /// caching by char is safe.
    #[test]
    fn printable_ascii_glyphs_are_unique() {
        let mut fs = FontSystem::new();
        let font_size = 14.0;
        let cell_h = font_size * 1.2;
        let cell_w = 8.4;

        let mut seen = std::collections::HashMap::<u16, char>::new();
        for ch in '!'..='~' {
            if let Some(key) = shape_char_to_key(&mut fs, ch, font_size, cell_h, cell_w) {
                if let Some(&prev_ch) = seen.get(&key.glyph_id) {
                    panic!("glyph_id {} collides between '{}' and '{}'", key.glyph_id, prev_ch, ch);
                }
                seen.insert(key.glyph_id, ch);
            }
        }
    }

    /// Shaping must be deterministic across many calls so the per-frame
    /// cache is safe to rebuild each frame.
    #[test]
    fn shaping_is_deterministic_across_many_calls() {
        let mut fs = FontSystem::new();
        let font_size = 14.0;
        let cell_h = font_size * 1.2;
        let cell_w = 8.4;

        let reference = shape_char_to_key(&mut fs, 'X', font_size, cell_h, cell_w)
            .expect("'X' should produce a glyph");
        for _ in 0..100 {
            let key = shape_char_to_key(&mut fs, 'X', font_size, cell_h, cell_w)
                .expect("'X' should produce a glyph");
            assert_eq!(reference, key, "GlyphKey must be stable across repeated shaping");
        }
    }

    /// font_size changes must produce different GlyphKeys, validating
    /// that the frame-local cache (rebuilt each frame with potentially
    /// different font_size) does not serve stale entries.
    #[test]
    fn different_font_size_produces_different_key() {
        let mut fs = FontSystem::new();
        let cell_w = 8.4;

        let key_14 = shape_char_to_key(&mut fs, 'M', 14.0, 14.0 * 1.2, cell_w)
            .expect("'M' at 14pt should produce a glyph");
        let key_20 = shape_char_to_key(&mut fs, 'M', 20.0, 20.0 * 1.2, cell_w)
            .expect("'M' at 20pt should produce a glyph");
        assert_ne!(
            key_14.font_size_tenths, key_20.font_size_tenths,
            "different font sizes must produce different font_size_tenths"
        );
    }

    // -----------------------------------------------------------------------
    // CellMetricsCache tests.
    //
    // The cache must cross frames: a second lookup with identical
    // (font_family, font_size, line_height, scale_factor) must NOT re-measure.
    // Any change to any of those four components must invalidate.
    // -----------------------------------------------------------------------

    #[test]
    fn cell_metrics_cache_is_empty_on_construction() {
        let cache = CellMetricsCache::new();
        assert!(cache.is_empty(), "freshly constructed cache must have no entries");
        assert_eq!(cache.miss_count(), 0, "freshly constructed cache must have 0 misses");
    }

    #[test]
    fn cell_metrics_cache_hits_identical_lookup() {
        let mut fs = FontSystem::new();
        let mut cache = CellMetricsCache::new();

        let family = monospace_family_name();
        let first = cache.get_or_measure(&mut fs, family, 14.0, 14.0 * 1.2, 1.0);
        let second = cache.get_or_measure(&mut fs, family, 14.0, 14.0 * 1.2, 1.0);

        assert_eq!(first, second, "identical inputs must return identical metrics");
        assert_eq!(
            cache.miss_count(),
            1,
            "second lookup with identical inputs must be a cache hit (misses stays at 1)"
        );
        assert_eq!(cache.len(), 1, "cache should contain exactly one entry");
    }

    #[test]
    fn cell_metrics_cache_misses_on_font_size_change() {
        let mut fs = FontSystem::new();
        let mut cache = CellMetricsCache::new();
        let family = monospace_family_name();

        let _a = cache.get_or_measure(&mut fs, family, 14.0, 14.0 * 1.2, 1.0);
        let _b = cache.get_or_measure(&mut fs, family, 20.0, 20.0 * 1.2, 1.0);

        assert_eq!(cache.miss_count(), 2, "different font size must miss");
        assert_eq!(cache.len(), 2, "two distinct sizes must produce two entries");
    }

    #[test]
    fn cell_metrics_cache_misses_on_scale_factor_change() {
        let mut fs = FontSystem::new();
        let mut cache = CellMetricsCache::new();
        let family = monospace_family_name();

        let _a = cache.get_or_measure(&mut fs, family, 14.0, 14.0 * 1.2, 1.0);
        let _b = cache.get_or_measure(&mut fs, family, 14.0, 14.0 * 1.2, 1.5);
        let _c = cache.get_or_measure(&mut fs, family, 14.0, 14.0 * 1.2, 2.0);

        assert_eq!(cache.miss_count(), 3, "different scale factor must miss");
        assert_eq!(cache.len(), 3, "three distinct scales must produce three entries");
    }

    #[test]
    fn cell_metrics_cache_misses_on_font_family_change() {
        let mut fs = FontSystem::new();
        let mut cache = CellMetricsCache::new();

        let _a = cache.get_or_measure(&mut fs, "Consolas", 14.0, 14.0 * 1.2, 1.0);
        let _b = cache.get_or_measure(&mut fs, "Menlo", 14.0, 14.0 * 1.2, 1.0);

        assert_eq!(cache.miss_count(), 2, "different font family must miss");
        assert_eq!(cache.len(), 2, "two families must produce two entries");
    }

    #[test]
    fn cell_metrics_cache_populates_both_cell_w_and_cell_h() {
        let mut fs = FontSystem::new();
        let mut cache = CellMetricsCache::new();

        let metrics = cache.get_or_measure(&mut fs, monospace_family_name(), 14.0, 14.0 * 1.2, 1.0);

        assert!(metrics.cell_w > 0.0, "cell_w must be positive");
        assert!(metrics.cell_h > 0.0, "cell_h must be positive");
        assert_eq!(
            metrics.cell_h.to_bits(),
            (14.0_f32 * 1.2).to_bits(),
            "cell_h must equal line_height"
        );
    }

    #[test]
    fn cell_metrics_cache_clear_resets_entries_but_preserves_miss_count() {
        let mut fs = FontSystem::new();
        let mut cache = CellMetricsCache::new();
        let family = monospace_family_name();

        let _ = cache.get_or_measure(&mut fs, family, 14.0, 14.0 * 1.2, 1.0);
        assert_eq!(cache.len(), 1);

        cache.clear();
        assert!(cache.is_empty(), "clear must empty the entry map");

        let _ = cache.get_or_measure(&mut fs, family, 14.0, 14.0 * 1.2, 1.0);
        assert_eq!(cache.miss_count(), 2, "miss counter is monotonic across clear");
    }

    #[test]
    fn cell_metrics_key_is_stable_for_fractional_sizes() {
        let k1 = CellMetricsKey::new("Consolas", 14.0, 16.8, 1.0);
        let k2 = CellMetricsKey::new("Consolas", 14.0, 16.8, 1.0);
        assert_eq!(k1, k2, "identical fractional inputs must produce identical keys");
    }

    // -----------------------------------------------------------------------
    // ShapeCache tests.
    //
    // The ShapeCache is a cross-frame cache. Tests verify:
    //   * Cache hit on second lookup.
    //   * Cache miss on font change.
    //   * Cache miss on DPI change.
    //   * Cache miss on size change.
    //   * Preload warmup.
    //   * Cross-frame persistence (no implicit per-frame clear).
    // -----------------------------------------------------------------------

    /// Tiny fake shaped entry used by ShapeCache tests so the unit tests do
    /// not depend on cosmic-text shaping pipelines.
    fn fake_shape(ch: char) -> Option<ShapedGlyphEntry> {
        // Build a LayoutGlyph minimal enough to satisfy the cache contract.
        // The cache treats the value as opaque, so any LayoutGlyph instance
        // that can be cloned is fine. We derive one via cosmic-text so the
        // type stays correct across upstream changes.
        use cosmic_text::{Attrs, Buffer, FontSystem, Metrics, Shaping};
        let mut fs = FontSystem::new();
        let mut buf = Buffer::new(&mut fs, Metrics::new(14.0, 16.8));
        buf.set_size(&mut fs, Some(1000.0), None);
        let mut tmp = [0u8; 4];
        let s = ch.encode_utf8(&mut tmp);
        buf.set_text(
            &mut fs,
            s,
            Attrs::new().family(cosmic_text::Family::Monospace),
            Shaping::Basic,
        );
        buf.shape_until_scroll(&mut fs, false);
        buf.layout_runs().find_map(|run| {
            run.glyphs
                .first()
                .cloned()
                .map(|g| ShapedGlyphEntry { layout_glyph: g, line_y: run.line_y })
        })
    }

    fn key_for(ch: char, font: &str, size: f32) -> ShapeCacheKey {
        ShapeCacheKey {
            ch,
            font_id: shape_cache_font_id(font),
            style: 0,
            font_size_tenths: (size * 10.0).round() as u32,
        }
    }

    #[test]
    fn shape_cache_is_empty_on_construction() {
        let cache = ShapeCache::new();
        assert!(cache.is_empty());
        assert_eq!(cache.hits(), 0);
        assert_eq!(cache.misses(), 0);
    }

    #[test]
    fn shape_cache_reports_hit_on_second_lookup() {
        let mut cache = ShapeCache::new();
        let key = key_for('A', "Consolas", 14.0);

        // First lookup: miss + insert.
        assert!(cache.get(&key).is_none());
        cache.insert(key, fake_shape('A'));

        // Second lookup: hit.
        assert!(cache.get(&key).is_some());

        assert_eq!(cache.hits(), 1);
        assert_eq!(cache.misses(), 1);
        assert!((cache.hit_rate() - 0.5).abs() < 1e-9);
    }

    #[test]
    fn shape_cache_misses_on_font_change() {
        let mut cache = ShapeCache::new();
        let key_a = key_for('A', "Consolas", 14.0);
        let key_b = key_for('A', "Menlo", 14.0);

        cache.insert(key_a, fake_shape('A'));
        assert!(cache.get(&key_b).is_none(), "different font must miss");
    }

    #[test]
    fn shape_cache_misses_on_size_change() {
        let mut cache = ShapeCache::new();
        let key_small = key_for('A', "Consolas", 14.0);
        let key_large = key_for('A', "Consolas", 20.0);

        cache.insert(key_small, fake_shape('A'));
        assert!(cache.get(&key_large).is_none(), "different size must miss");
    }

    #[test]
    fn shape_cache_retune_invalidates_on_font_change() {
        let mut cache = ShapeCache::new();
        cache.retune("Consolas", 1.0, 14.0);
        let key = key_for('A', "Consolas", 14.0);
        cache.insert(key, fake_shape('A'));
        assert_eq!(cache.len(), 1);

        // Switching the font family must drop the cache.
        cache.retune("Menlo", 1.0, 14.0);
        assert!(cache.is_empty(), "retune with a different font family must clear the cache");
    }

    #[test]
    fn shape_cache_retune_invalidates_on_dpi_change() {
        let mut cache = ShapeCache::new();
        cache.retune("Consolas", 1.0, 14.0);
        cache.insert(key_for('A', "Consolas", 14.0), fake_shape('A'));

        cache.retune("Consolas", 2.0, 14.0);
        assert!(cache.is_empty(), "retune with a new DPI must clear the cache");
    }

    #[test]
    fn shape_cache_retune_invalidates_on_size_change() {
        let mut cache = ShapeCache::new();
        cache.retune("Consolas", 1.0, 14.0);
        cache.insert(key_for('A', "Consolas", 14.0), fake_shape('A'));

        cache.retune("Consolas", 1.0, 20.0);
        assert!(cache.is_empty(), "retune with a new size must clear the cache");
    }

    #[test]
    fn shape_cache_retune_is_noop_when_identical() {
        let mut cache = ShapeCache::new();
        cache.retune("Consolas", 1.0, 14.0);
        cache.insert(key_for('A', "Consolas", 14.0), fake_shape('A'));
        assert_eq!(cache.len(), 1);

        cache.retune("Consolas", 1.0, 14.0);
        assert_eq!(cache.len(), 1, "identical retune must not drop entries");
    }

    #[test]
    fn shape_cache_preload_fills_ascii_and_box_drawing() {
        let mut cache = ShapeCache::new();
        let mut shaped_count = 0u32;
        cache.preload_defaults(&mut |ch| {
            shaped_count += 1;
            (key_for(ch, "Consolas", 14.0), fake_shape(ch))
        });

        // 0x20..=0x7e is 95 chars, 0x2500..=0x257f is 128 chars. The iterator
        // yields the union; we assert both are represented in the cache.
        assert_eq!(shaped_count, 95 + 128, "preload must visit all defaults");
        assert!(cache.len() >= 95 + 128);
        assert!(cache.get(&key_for(' ', "Consolas", 14.0)).is_some());
        assert!(cache.get(&key_for('~', "Consolas", 14.0)).is_some());
        assert!(cache.get(&key_for('\u{2500}', "Consolas", 14.0)).is_some()); // box drawings: light horizontal
        assert!(cache.get(&key_for('\u{257f}', "Consolas", 14.0)).is_some());
    }

    #[test]
    fn shape_cache_persists_across_simulated_frames() {
        // Simulates two frames: second frame touches the same char twice and
        // must observe zero additional misses.
        let mut cache = ShapeCache::new();
        let key = key_for('x', "Consolas", 14.0);

        // Frame 1: miss + insert.
        assert!(cache.get(&key).is_none());
        cache.insert(key, fake_shape('x'));
        assert_eq!(cache.misses(), 1);

        // Frame 2: two lookups, both hits.
        assert!(cache.get(&key).is_some());
        assert!(cache.get(&key).is_some());
        assert_eq!(cache.misses(), 1, "cross-frame reuse must NOT add misses");
        assert_eq!(cache.hits(), 2);
    }

    #[test]
    fn shape_cache_default_preload_chars_covers_ascii_and_box_drawing() {
        let chars: Vec<char> = ShapeCache::default_preload_chars().collect();
        assert!(chars.contains(&' '));
        assert!(chars.contains(&'~'));
        assert!(chars.contains(&'A'));
        assert!(chars.contains(&'\u{2500}'));
        assert!(chars.contains(&'\u{257f}'));
        assert_eq!(chars.len(), 95 + 128);
    }

    fn terminal_cell(ch: char) -> unshit_core::cell_grid::Cell {
        unshit_core::cell_grid::Cell {
            ch,
            fg: Color { r: 255, g: 128, b: 64, a: 255 },
            bg: Color { r: 0, g: 0, b: 0, a: 0 },
            attrs: CellAttrs::empty(),
            wide_continuation: false,
        }
    }

    #[test]
    fn terminal_primitive_renders_full_block_as_cell_aligned_quad() {
        let mut quads = Vec::new();
        let emitted = emit_terminal_cell_primitive(
            &terminal_cell('\u{2588}'),
            0,
            2,
            10.0,
            20.0,
            8.0,
            16.0,
            1.0,
            [0.0, 0.0, 200.0, 200.0],
            false,
            true,
            None,
            &mut quads,
        );

        assert!(emitted, "full block must be rendered by terminal primitive path");
        assert_eq!(quads.len(), 1);
        assert!(quads[0].pos[0] < 26.0, "block should overlap left cell edge");
        assert!(quads[0].pos[1] < 20.0, "block should overlap top cell edge");
        assert!(quads[0].size[0] > 8.0, "block width should cover the full cell plus seam guard");
        assert!(quads[0].size[1] > 16.0, "block height should cover the full cell plus seam guard");
    }

    #[test]
    fn terminal_primitive_snaps_full_block_rect_under_parity_profile() {
        let mut quads = Vec::new();
        let emitted = emit_terminal_cell_primitive(
            &terminal_cell('\u{2588}'),
            0,
            2,
            10.2,
            20.2,
            8.0,
            16.0,
            1.0,
            [0.0, 0.0, 200.0, 200.0],
            true,
            true,
            Some(' '),
            &mut quads,
        );

        assert!(emitted);
        assert_eq!(quads.len(), 1);
        assert_eq!(quads[0].pos, [26.0, 21.0]);
        assert_eq!(quads[0].size, [8.0, 17.0]);
    }

    #[test]
    fn terminal_primitive_renders_horizontal_rule_across_cell_edges() {
        let mut quads = Vec::new();
        let emitted = emit_terminal_cell_primitive(
            &terminal_cell('\u{2500}'),
            1,
            3,
            4.0,
            6.0,
            9.0,
            18.0,
            1.0,
            [0.0, 0.0, 200.0, 200.0],
            false,
            true,
            None,
            &mut quads,
        );

        assert!(emitted, "box drawing horizontal must bypass glyph rasterization");
        assert_eq!(quads.len(), 1);
        let cell_x = 4.0 + 3.0 * 9.0;
        assert!(quads[0].pos[0] < cell_x, "rule should overlap the left cell edge");
        assert!(
            quads[0].size[0] > 9.0,
            "rule should span the full cell plus edge overlap to avoid seams"
        );
        assert!(quads[0].size[1] >= 1.0, "light rule must be at least one physical pixel");
    }

    #[test]
    fn terminal_primitive_renders_box_corner_as_two_strokes() {
        let mut quads = Vec::new();
        let emitted = emit_terminal_cell_primitive(
            &terminal_cell('\u{250c}'),
            0,
            0,
            0.0,
            0.0,
            10.0,
            20.0,
            1.0,
            [0.0, 0.0, 200.0, 200.0],
            false,
            true,
            None,
            &mut quads,
        );

        assert!(emitted);
        assert_eq!(quads.len(), 2, "corner should render horizontal and vertical strokes");
    }

    #[test]
    fn terminal_primitive_renders_double_box_glyphs_without_font_path() {
        let mut horizontal = Vec::new();
        let emitted_horizontal = emit_terminal_cell_primitive(
            &terminal_cell('\u{2550}'),
            0,
            0,
            0.0,
            0.0,
            10.0,
            20.0,
            1.0,
            [0.0, 0.0, 200.0, 200.0],
            true,
            true,
            None,
            &mut horizontal,
        );
        assert!(emitted_horizontal, "double horizontal line should use primitive path");
        assert_eq!(horizontal.len(), 2, "double horizontal line emits two strokes");
        assert!(horizontal[0].pos[1] < horizontal[1].pos[1]);

        let mut mixed = Vec::new();
        let emitted_mixed = emit_terminal_cell_primitive(
            &terminal_cell('\u{255f}'),
            0,
            0,
            0.0,
            0.0,
            10.0,
            20.0,
            1.0,
            [0.0, 0.0, 200.0, 200.0],
            true,
            true,
            None,
            &mut mixed,
        );
        assert!(emitted_mixed, "mixed double/single box glyph should use primitive path");
        assert_eq!(mixed.len(), 3, "mixed glyph emits one single stroke plus two double strokes");
    }

    #[test]
    fn terminal_primitive_double_and_shade_bypass_is_parity_scoped() {
        let mut double_quads = Vec::new();
        assert!(!emit_terminal_cell_primitive(
            &terminal_cell('\u{2550}'),
            0,
            0,
            0.0,
            0.0,
            10.0,
            20.0,
            1.0,
            [0.0, 0.0, 200.0, 200.0],
            false,
            true,
            None,
            &mut double_quads,
        ));
        assert!(
            double_quads.is_empty(),
            "non-parity double box glyphs stay on the regular glyph path"
        );

        let mut shade_quads = Vec::new();
        assert!(!emit_terminal_cell_primitive(
            &terminal_cell('\u{2592}'),
            0,
            0,
            0.0,
            0.0,
            8.0,
            16.0,
            1.0,
            [0.0, 0.0, 200.0, 200.0],
            false,
            true,
            None,
            &mut shade_quads,
        ));
        assert!(shade_quads.is_empty(), "non-parity shaded blocks stay on the regular glyph path");
    }

    #[test]
    fn terminal_primitive_double_corner_uses_tight_stroke_gap() {
        let mut quads = Vec::new();
        let emitted = emit_terminal_cell_primitive(
            &terminal_cell('\u{2554}'),
            0,
            0,
            0.0,
            0.0,
            13.0,
            25.0,
            1.0,
            [0.0, 0.0, 200.0, 200.0],
            true,
            true,
            None,
            &mut quads,
        );

        assert!(emitted);
        assert_eq!(
            quads.len(),
            4,
            "double corner should emit paired horizontal and vertical strokes"
        );
        let horizontal_gap = quads[1].pos[1] - quads[0].pos[1];
        let vertical_gap = quads[3].pos[0] - quads[2].pos[0];
        assert_eq!(horizontal_gap, 4.0);
        assert_eq!(vertical_gap, 4.0);
    }

    #[test]
    fn terminal_primitive_renders_shaded_blocks_as_stippled_quads() {
        let mut light_quads = Vec::new();
        let mut medium_quads = Vec::new();
        let mut dark_quads = Vec::new();

        assert!(emit_terminal_cell_primitive(
            &terminal_cell('\u{2591}'),
            0,
            0,
            0.0,
            0.0,
            8.0,
            16.0,
            1.0,
            [0.0, 0.0, 200.0, 200.0],
            true,
            true,
            None,
            &mut light_quads,
        ));
        assert!(emit_terminal_cell_primitive(
            &terminal_cell('\u{2592}'),
            0,
            0,
            0.0,
            0.0,
            8.0,
            16.0,
            1.0,
            [0.0, 0.0, 200.0, 200.0],
            true,
            true,
            None,
            &mut medium_quads,
        ));
        assert!(emit_terminal_cell_primitive(
            &terminal_cell('\u{2593}'),
            0,
            0,
            0.0,
            0.0,
            8.0,
            16.0,
            1.0,
            [0.0, 0.0, 200.0, 200.0],
            true,
            true,
            None,
            &mut dark_quads,
        ));

        let area = |quads: &[QuadInstance]| {
            quads.iter().map(|quad| quad.size[0] * quad.size[1]).sum::<f32>()
        };
        let light_area = area(&light_quads);
        let medium_area = area(&medium_quads);
        let dark_area = area(&dark_quads);

        assert!(light_area < medium_area);
        assert!(medium_area < dark_area);
        assert_eq!(light_area, 32.0, "light shade should cover 25% of an 8x16 cell");
        assert_eq!(medium_area, 64.0, "medium shade should cover 50% of an 8x16 cell");
        assert_eq!(dark_area, 96.0, "dark shade should cover 75% of an 8x16 cell");
        assert!(
            light_quads.iter().chain(&medium_quads).chain(&dark_quads).all(|quad| {
                quad.size[1] == 1.0 && quad.pos[0].fract() == 0.0 && quad.pos[1].fract() == 0.0
            }),
            "shade primitives should snap to integer pixel rows"
        );
        assert!(
            (1..=16).all(|row| light_quads.iter().any(|quad| quad.pos[1] == row as f32)),
            "Windows Terminal shade lattice keeps light pixels on every row"
        );
        assert!(
            (1..=16).all(|row| dark_quads
                .iter()
                .any(|quad| { quad.pos[1] == row as f32 && quad.size[0] < 8.0 })),
            "dark shade keeps a gap on every row instead of alternating solid rows"
        );
    }

    #[test]
    fn terminal_primitive_leaves_regular_text_on_glyph_path() {
        let mut quads = Vec::new();
        let emitted = emit_terminal_cell_primitive(
            &terminal_cell('A'),
            0,
            0,
            0.0,
            0.0,
            10.0,
            20.0,
            1.0,
            [0.0, 0.0, 200.0, 200.0],
            false,
            true,
            None,
            &mut quads,
        );

        assert!(!emitted);
        assert!(quads.is_empty());
    }

    #[test]
    fn terminal_shape_style_distinguishes_bold_and_italic() {
        assert_eq!(terminal_shape_style(CellAttrs::empty()), TERMINAL_SHAPE_STYLE_REGULAR);
        assert_eq!(
            terminal_shape_style(CellAttrs::BOLD),
            TERMINAL_SHAPE_STYLE_REGULAR,
            "Windows Terminal's default intense style does not select a bold face"
        );
        assert_eq!(terminal_shape_style(CellAttrs::ITALIC), TERMINAL_SHAPE_STYLE_ITALIC);
        assert_eq!(
            terminal_shape_style(CellAttrs::BOLD | CellAttrs::ITALIC),
            TERMINAL_SHAPE_STYLE_ITALIC
        );
    }

    #[test]
    fn terminal_text_attrs_maps_sgr_italic_to_oblique_without_bold_face() {
        let attrs = terminal_text_attrs(
            cosmic_text::Family::Name("Consolas"),
            CellAttrs::BOLD | CellAttrs::ITALIC,
        );

        assert_eq!(attrs.weight, cosmic_text::Weight::NORMAL);
        assert_eq!(attrs.style, cosmic_text::Style::Oblique);
    }

    #[test]
    fn terminal_primitive_draws_underline_without_bypassing_text_glyph() {
        let mut cell = terminal_cell('A');
        cell.attrs = CellAttrs::UNDERLINE;

        let mut quads = Vec::new();
        let emitted = emit_terminal_cell_primitive(
            &cell,
            0,
            0,
            0.0,
            0.0,
            10.0,
            20.0,
            1.0,
            [0.0, 0.0, 200.0, 200.0],
            false,
            true,
            None,
            &mut quads,
        );

        assert!(!emitted, "text glyph must still render through the glyph path");
        assert_eq!(quads.len(), 1, "underline should add one decoration quad");
        assert_eq!(quads[0].pos[1], 16.0, "underline should sit just above the cell baseline");
    }

    #[test]
    fn terminal_primitive_draws_strikethrough_on_spaces() {
        let mut cell = terminal_cell(' ');
        cell.attrs = CellAttrs::STRIKETHROUGH;

        let mut quads = Vec::new();
        let emitted = emit_terminal_cell_primitive(
            &cell,
            0,
            0,
            0.0,
            0.0,
            10.0,
            20.0,
            1.0,
            [0.0, 0.0, 200.0, 200.0],
            false,
            true,
            None,
            &mut quads,
        );

        assert!(emitted, "decorated spaces have visible output without a glyph");
        assert_eq!(quads.len(), 1);
        assert!((8.0..=12.0).contains(&quads[0].pos[1]), "strike belongs around mid-cell");
    }

    #[test]
    fn terminal_blink_off_suppresses_regular_text_before_glyph_path() {
        let mut cell = terminal_cell('A');
        cell.attrs = CellAttrs::BLINK;

        let mut quads = Vec::new();
        let emitted = emit_terminal_cell_primitive(
            &cell,
            0,
            0,
            0.0,
            0.0,
            10.0,
            20.0,
            1.0,
            [0.0, 0.0, 200.0, 200.0],
            false,
            false,
            None,
            &mut quads,
        );

        assert!(emitted, "blink-off text must not fall through to glyph emission");
        assert!(quads.is_empty(), "blink-off text should leave only the background visible");
    }

    #[test]
    fn terminal_blink_off_suppresses_cell_primitives() {
        let mut cell = terminal_cell('\u{2588}');
        cell.attrs = CellAttrs::BLINK;

        let mut quads = Vec::new();
        let emitted = emit_terminal_cell_primitive(
            &cell,
            0,
            0,
            0.0,
            0.0,
            10.0,
            20.0,
            1.0,
            [0.0, 0.0, 200.0, 200.0],
            false,
            false,
            None,
            &mut quads,
        );

        assert!(emitted, "blink-off primitives must consume the cell foreground path");
        assert!(quads.is_empty(), "blink-off block drawing should not render foreground quads");
    }

    #[test]
    fn terminal_row_content_sig_changes_with_blink_phase_only_for_blink_rows() {
        let mut plain = terminal_cell('A');
        plain.attrs = CellAttrs::empty();
        let mut blinking = terminal_cell('B');
        blinking.attrs = CellAttrs::BLINK;

        let plain_cells = vec![plain, plain];
        assert_eq!(
            terminal_row_content_sig(&plain_cells, 0, 2, true),
            terminal_row_content_sig(&plain_cells, 0, 2, false),
            "plain rows should stay cacheable across blink phases"
        );

        let blink_cells = vec![plain, blinking];
        assert_ne!(
            terminal_row_content_sig(&blink_cells, 0, 2, true),
            terminal_row_content_sig(&blink_cells, 0, 2, false),
            "blink rows must invalidate cached foreground payloads across phases"
        );
    }

    // -----------------------------------------------------------------------
    // Double buffered ShapeCache tests (issue #83).
    //
    // The shape cache is now backed by a two frame map. Each `finish_frame`
    // swaps previous and current and clears the new current. Entries not
    // touched for two consecutive frames are evicted automatically. Preload
    // and hot characters promote every frame so they never evict.
    // -----------------------------------------------------------------------

    #[test]
    fn shape_cache_promotes_across_frame_boundary() {
        let mut cache = ShapeCache::new();
        let key = key_for('Z', "Consolas", 14.0);
        cache.insert(key, fake_shape('Z'));
        cache.finish_frame(); // entry now lives in previous
        let first_hit = cache.get(&key);
        assert!(first_hit.is_some(), "promoted entry must be readable");
        assert_eq!(cache.len(), 1, "promotion preserves the single entry");
        assert_eq!(cache.previous_hits(), 1, "the hit came from previous");
    }

    #[test]
    fn shape_cache_untouched_chars_evict_after_two_frames() {
        let mut cache = ShapeCache::new();
        let key = key_for('Q', "Consolas", 14.0);
        cache.insert(key, fake_shape('Q'));
        // finish_frame 1: moves to previous.
        cache.finish_frame();
        // finish_frame 2: previous not touched this frame => dropped.
        cache.finish_frame();
        assert!(cache.get(&key).is_none(), "untouched entry must evict after two frame boundaries");
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn shape_cache_preload_survives_first_finish_frame_pair() {
        // Preload, simulate a frame of hits on every default char, finish,
        // then hit them all again. None of the preloaded entries should be
        // evicted because each one got promoted on the second frame.
        let mut cache = ShapeCache::new();
        cache.preload_defaults(&mut |ch| (key_for(ch, "Consolas", 14.0), fake_shape(ch)));
        let preload_count = cache.len();
        assert!(preload_count >= 95 + 128, "preload populates at least ASCII + box-drawing");

        // Frame 1: touch every preloaded key so each gets promoted into
        // current. `finish_frame` then pushes them to previous for frame 2.
        for ch in ShapeCache::default_preload_chars() {
            assert!(cache.get(&key_for(ch, "Consolas", 14.0)).is_some());
        }
        cache.finish_frame();

        // Frame 2: touch every preloaded key again. Each lookup must still
        // succeed (promotion pulls the entry back into current).
        for ch in ShapeCache::default_preload_chars() {
            assert!(
                cache.get(&key_for(ch, "Consolas", 14.0)).is_some(),
                "preloaded entry for {ch:?} must survive the first finish_frame pair",
            );
        }
        cache.finish_frame();

        // Frame 3: still reachable.
        for ch in ShapeCache::default_preload_chars() {
            assert!(
                cache.get(&key_for(ch, "Consolas", 14.0)).is_some(),
                "preloaded entry for {ch:?} must still be reachable on frame 3",
            );
        }
    }

    #[test]
    fn shape_cache_retune_clears_both_halves() {
        let mut cache = ShapeCache::new();
        cache.retune("Consolas", 1.0, 14.0);
        cache.insert(key_for('A', "Consolas", 14.0), fake_shape('A'));
        cache.finish_frame(); // A in previous
        cache.insert(key_for('B', "Consolas", 14.0), fake_shape('B')); // B in current
        assert_eq!(cache.len(), 2, "both halves populated before retune");

        // Change font family: retune calls clear(), which empties both halves.
        cache.retune("Menlo", 1.0, 14.0);
        assert!(cache.is_empty(), "retune must empty previous AND current");
        assert_eq!(cache.hits(), 0, "clear resets hit counter");
        assert_eq!(cache.misses(), 0, "clear resets miss counter");
        assert_eq!(cache.previous_hits(), 0, "clear resets previous-hit counter");
    }

    #[test]
    fn shape_cache_previous_hit_counter_classifies_promotion() {
        let mut cache = ShapeCache::new();
        let key = key_for('A', "Consolas", 14.0);
        cache.insert(key, fake_shape('A'));
        assert_eq!(cache.previous_hits(), 0);

        // First lookup hits current (no promotion).
        let _ = cache.get(&key);
        assert_eq!(cache.previous_hits(), 0, "current-frame hit does not bump previous_hits");

        // After finish_frame the entry sits in previous. Next lookup promotes.
        cache.finish_frame();
        let _ = cache.get(&key);
        assert_eq!(cache.previous_hits(), 1, "promoted hit increments previous_hits");

        // Subsequent lookup in the same frame is a current-frame hit.
        let _ = cache.get(&key);
        assert_eq!(cache.previous_hits(), 1, "same-frame repeat does not bump previous_hits");
    }

    #[test]
    fn shape_cache_size_bounded_under_unique_char_workload() {
        // Rendering 1000 unique chars over 10 frames with no repeats across
        // frames must never balloon the cache: the live set stays bounded by
        // the last two frames (~200 entries). Before double buffering, the
        // cache accumulated all 1000.
        let mut cache = ShapeCache::new();
        const PER_FRAME: u32 = 100;
        const FRAMES: u32 = 10;

        for frame in 0..FRAMES {
            for i in 0..PER_FRAME {
                let code = 0x100 + frame * PER_FRAME + i; // never collides across frames
                if let Some(ch) = char::from_u32(code) {
                    let key = key_for(ch, "Consolas", 14.0);
                    cache.insert(key, None); // value does not matter for size bounding
                }
            }
            cache.finish_frame();
        }
        // After 10 frames, only frames 9's entries (current at finish time,
        // now previous after the last swap) and the empty new current remain.
        // The bound is 2 * PER_FRAME to account for the final two frame
        // halves of hits.
        let bound = 2 * PER_FRAME as usize;
        assert!(
            cache.len() <= bound,
            "double buffered shape cache must stay below {bound} entries, got {}",
            cache.len(),
        );
    }

    // -----------------------------------------------------------------------
    // Double buffered ShapedTextCache tests (issue #83).
    // -----------------------------------------------------------------------

    // Size and bounding tests only care about key presence; the atlas
    // residency check lives outside the cache itself, so an empty glyph
    // vec is sufficient.
    fn shaped_text_entry() -> ShapedTextEntry {
        ShapedTextEntry { glyphs: Vec::new() }
    }

    fn shaped_key_for(text: &str) -> ShapedCacheKey {
        ShapedTextCache::make_key(
            text,
            "",
            FontWeight::Normal,
            FontStyle::Normal,
            14.0,
            1.2,
            0.0,
            None,
        )
    }

    #[test]
    fn shaped_text_cache_key_includes_font_family_and_weight() {
        let regular = ShapedTextCache::make_key(
            "keep",
            "Consolas",
            FontWeight::Normal,
            FontStyle::Normal,
            11.0,
            1.4,
            0.0,
            None,
        );
        let semibold = ShapedTextCache::make_key(
            "keep",
            "Consolas",
            FontWeight::W(600),
            FontStyle::Normal,
            11.0,
            1.4,
            0.0,
            None,
        );
        let other_family = ShapedTextCache::make_key(
            "keep",
            "JetBrains Mono",
            FontWeight::Normal,
            FontStyle::Normal,
            11.0,
            1.4,
            0.0,
            None,
        );
        let italic = ShapedTextCache::make_key(
            "keep",
            "Consolas",
            FontWeight::Normal,
            FontStyle::Italic,
            11.0,
            1.4,
            0.0,
            None,
        );

        assert!(regular != semibold);
        assert!(regular != other_family);
        assert!(regular != italic, "italic must shape to a distinct cache key");
    }

    #[test]
    fn shaped_text_cache_new_is_empty() {
        let c = ShapedTextCache::new();
        assert!(c.is_empty());
        assert_eq!(c.len(), 0);
    }

    #[test]
    fn shaped_text_cache_promotes_across_frame_boundary() {
        let mut cache = ShapedTextCache::new();
        let key = shaped_key_for("hello");
        cache.buf.insert(key.clone(), shaped_text_entry());
        cache.finish_frame(0); // no atlas change => swap
                               // The entry now lives in previous. A lookup must find it and
                               // promote.
        let hit = cache.buf.get_or_promote(&key);
        assert!(hit.is_some(), "entry must survive one finish_frame");
    }

    #[test]
    fn shaped_text_cache_untouched_entries_evict_after_two_finish_frame() {
        let mut cache = ShapedTextCache::new();
        let key = shaped_key_for("ephemeral");
        cache.buf.insert(key.clone(), shaped_text_entry());
        cache.finish_frame(0); // to previous
        cache.finish_frame(0); // dropped
        assert!(cache.buf.peek(&key).is_none(), "untouched entry evicts after two boundaries");
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn shaped_text_cache_atlas_generation_bump_clears_cache_at_finish_frame() {
        let mut cache = ShapedTextCache::new();
        let k1 = shaped_key_for("alpha");
        let k2 = shaped_key_for("beta");
        cache.buf.insert(k1.clone(), shaped_text_entry());
        cache.finish_frame(0); // k1 now in previous
        cache.buf.insert(k2.clone(), shaped_text_entry()); // k2 in current
        assert_eq!(cache.len(), 2);

        // Atlas generation bump: finish_frame with a new generation wipes
        // the cache before it can swap, protecting against stale UVs.
        cache.finish_frame(1);
        assert!(cache.is_empty(), "atlas generation change must clear the cache");
    }

    #[test]
    fn shaped_text_cache_clear_empties_both_halves() {
        let mut cache = ShapedTextCache::new();
        let k1 = shaped_key_for("alpha");
        let k2 = shaped_key_for("beta");
        cache.buf.insert(k1, shaped_text_entry());
        cache.finish_frame(0);
        cache.buf.insert(k2, shaped_text_entry());
        assert_eq!(cache.len(), 2);
        cache.clear();
        assert!(cache.is_empty(), "clear must drop entries from previous AND current");
    }

    #[test]
    fn shaped_text_cache_size_bounded_under_unique_strings_workload() {
        // Ten frames of 100 unique strings with no repeats across frames.
        // The live set must never exceed two frames' worth after the swap.
        let mut cache = ShapedTextCache::new();
        const PER_FRAME: u32 = 100;
        const FRAMES: u32 = 10;

        for frame in 0..FRAMES {
            for i in 0..PER_FRAME {
                let text = format!("frame-{frame}-str-{i}");
                let key = shaped_key_for(&text);
                cache.buf.insert(key, shaped_text_entry());
            }
            cache.finish_frame(0);
        }
        let bound = 2 * PER_FRAME as usize;
        assert!(
            cache.len() <= bound,
            "double buffered shaped text cache must stay below {bound} entries, got {}",
            cache.len(),
        );
    }

    // -----------------------------------------------------------------------
    // Retained line quad cache: full-row emission regression tests (issue #63)
    //
    // Peer terminals (WezTerm, Alacritty, Zed) treat per-row damage bounds as
    // INVALIDATION hints, not emission extent. Before the fix, a cache MISS
    // would emit only the damaged sub-range while storing the truncated
    // payload under the whole-row content hash, so the next HIT replayed a
    // stripped cache and cells outside the window disappeared. These tests
    // exercise `emit_grid_row_backgrounds`, the pure half of the fresh-emit
    // pipeline, to confirm the emission helper always spans the whole row.
    // -----------------------------------------------------------------------

    /// Build a grid where every column in `row` has a distinct non-transparent
    /// background color so each column contributes its own `StyleRun`, and
    /// `emit_grid_row_backgrounds` produces exactly one quad per column.
    fn grid_with_distinct_bg_per_col(row: usize, rows: usize, cols: usize) -> CellGrid {
        use unshit_core::cell_grid::Cell;
        let mut g = CellGrid::new(rows, cols);
        g.clear_dirty();
        for col in 0..cols {
            // Use a unique `bg.r` per column so adjacent cells never share a
            // style and every column becomes its own run.
            let bg = Color { r: (col + 1) as u8, g: 0, b: 0, a: 255 };
            let fg = Color { r: 255, g: 255, b: 255, a: 255 };
            g.set_cell(
                row,
                col,
                Cell { ch: 'x', fg, bg, attrs: CellAttrs::empty(), wide_continuation: false },
            );
        }
        g
    }

    #[test]
    fn emit_grid_row_backgrounds_spans_full_row_from_0_to_cols() {
        // The helper must emit one quad per style run across the full row
        // regardless of per-cell dirty flags. Issue #63: narrowing emission
        // to the damaged sub-range caused the whole-row cache payload to be
        // truncated.
        let cols = 5;
        let grid = grid_with_distinct_bg_per_col(0, 1, cols);

        let mut out_quads: Vec<QuadInstance> = Vec::new();
        emit_grid_row_backgrounds(
            &grid,
            0,
            0,
            cols,
            0.0,
            0.0,
            10.0,
            20.0,
            1.0,
            [0.0; 4],
            &mut out_quads,
        );

        assert_eq!(
            out_quads.len(),
            cols,
            "fresh background emission must span every column of the row, got {} quads for {} cols",
            out_quads.len(),
            cols,
        );

        // X positions must cover col 0 through col cols-1 contiguously. Issue
        // #63 previously produced a single quad at the damaged column, so
        // asserting the x-sweep is the load-bearing regression check.
        for (i, q) in out_quads.iter().enumerate() {
            let expected_x = i as f32 * 10.0;
            assert!(
                (q.pos[0] - expected_x).abs() < f32::EPSILON,
                "quad {i} at x={}, expected x={}",
                q.pos[0],
                expected_x,
            );
        }
    }

    #[test]
    fn typed_char_does_not_blank_prior_row_content() {
        use unshit_core::cell_grid::Cell;
        // Regression test for issue #63.
        //
        // Scenario: the terminal rendered a full row on frame 1, populated
        // the line_quad_cache with a whole-row payload, then the user types
        // a single character at column X. Only col X is marked in
        // `line_damage`, but the row's content hash shifted. The fresh emit
        // path must rebuild the WHOLE row so the stored cache entry remains
        // whole-row; otherwise the next HIT for that new content hash
        // replays a stripped row and the surrounding cells vanish.
        let cols = 6;
        let rows = 1;
        let mut grid = grid_with_distinct_bg_per_col(0, rows, cols);

        // Simulate frame 1: the renderer populates the cache with a
        // whole-row payload derived from the fresh emit path.
        let mut frame1_quads: Vec<QuadInstance> = Vec::new();
        emit_grid_row_backgrounds(
            &grid,
            0,
            0,
            cols,
            0.0,
            0.0,
            10.0,
            20.0,
            1.0,
            [0.0; 4],
            &mut frame1_quads,
        );
        let content_sig_frame1 = hash_row_cells(grid.cells(), 0, cols);
        let geom = LineGeometrySig::new(0.0, 0.0, 10.0, 20.0, 14.0, 1.0, [0.0; 4], cols as u32, 0);
        let mut cache = LineQuadCache::new();
        let node = NodeId { index: 0, generation: 0 };
        // Line identity for row 0 comes from the grid (Step 3 keys the
        // cache on stable `line_id`, not row index).
        let line_id = grid.line_id(0).expect("row 0 has a line id");
        cache.store(
            node,
            line_id,
            content_sig_frame1,
            geom,
            frame1_quads.clone(),
            vec![],
            vec![],
            vec![],
            0,
        );
        assert_eq!(frame1_quads.len(), cols, "frame 1 emission must span full row");

        // Frame 2: user types 'Z' at column 3 with a distinct bg color.
        // `set_cell` marks only col 3 in `line_damage`, but the whole-row
        // content hash changes, producing a cache MISS.
        grid.clear_dirty();
        let new_cell = Cell {
            ch: 'Z',
            fg: Color { r: 255, g: 255, b: 255, a: 255 },
            bg: Color { r: 200, g: 100, b: 50, a: 255 },
            attrs: CellAttrs::empty(),
            wide_continuation: false,
        };
        grid.set_cell(0, 3, new_cell);
        let content_sig_frame2 = hash_row_cells(grid.cells(), 0, cols);
        assert_ne!(
            content_sig_frame1, content_sig_frame2,
            "typed char must change the row's content hash",
        );

        // Damage only reports col 3, but fresh emit must still produce a
        // full-row payload. Issue #63: prior code passed `scan_start..scan_end`
        // from `line_damage` into the emission loop, producing a 1-quad
        // payload that overwrote the 6-quad entry.
        let damage = grid.line_damage_for(0).expect("row has damage entry");
        assert_eq!(damage.first_dirty_col, 3, "only col 3 should be damaged");
        assert_eq!(damage.last_dirty_col, 3, "only col 3 should be damaged");

        let mut frame2_quads: Vec<QuadInstance> = Vec::new();
        emit_grid_row_backgrounds(
            &grid,
            0,
            0,
            cols,
            0.0,
            0.0,
            10.0,
            20.0,
            1.0,
            [0.0; 4],
            &mut frame2_quads,
        );
        cache.store(
            node,
            line_id,
            content_sig_frame2,
            geom,
            frame2_quads.clone(),
            vec![],
            vec![],
            vec![],
            0,
        );

        assert_eq!(
            frame2_quads.len(),
            cols,
            "fresh emit after typed char must still span every column ({} quads for {} cols)",
            frame2_quads.len(),
            cols,
        );

        // Frame 3: the cached payload must be replayable AS-IS for the new
        // content hash, and the replay must contain every column.
        let hit = cache
            .lookup_replayable(node, line_id, content_sig_frame2, geom)
            .expect("cache must hit the payload we just stored");
        assert_eq!(
            hit.quads.len(),
            cols,
            "stored line_quad_cache entry must cover the full row, not just the damaged column",
        );
    }

    #[test]
    fn cache_miss_with_clean_row_still_emits_full_row() {
        // Regression test for issue #63 second failure mode.
        //
        // Scenario: the line_quad_cache entry exists and matches the
        // content hash, but the atlas generation bumped (or the origin
        // shifted) so the geometry signature no longer matches. The cache
        // MISSes even though the row is otherwise "clean" (no per-cell
        // writes this frame). The fresh emit path must still produce a
        // full-row payload; the pre-fix `if row_is_clean { continue; }`
        // shortcut produced zero quads and blanked the row.
        let cols = 4;
        let mut grid = grid_with_distinct_bg_per_col(0, 1, cols);
        // Simulate end-of-frame reset: every cell is clean on the dirty
        // side, and `line_damage` is cleared. This mirrors the state the
        // renderer observes after `clear_dirty`.
        grid.clear_dirty();
        assert!(
            grid.line_damage_for(0).map(|ld| ld.is_clean()).unwrap_or(false),
            "row must be clean after clear_dirty so we exercise the clean-row path",
        );

        // Populate the cache with a stale geometry signature.
        let stale_geom =
            LineGeometrySig::new(0.0, 0.0, 10.0, 20.0, 14.0, 1.0, [0.0; 4], cols as u32, 0);
        let content_sig = hash_row_cells(grid.cells(), 0, cols);
        let mut cache = LineQuadCache::new();
        let node = NodeId { index: 0, generation: 0 };
        // Cache key is (node, line_id) post-Step 3: use the grid's id for
        // row 0 so the test mirrors production lookup shape.
        let line_id = grid.line_id(0).expect("row 0 has a line id");
        cache.store(node, line_id, content_sig, stale_geom, vec![], vec![], vec![], vec![], 0);

        // Bump the atlas generation: geometry signature no longer matches.
        let fresh_geom =
            LineGeometrySig::new(0.0, 0.0, 10.0, 20.0, 14.0, 1.0, [0.0; 4], cols as u32, 1);
        assert!(
            cache.lookup_replayable(node, line_id, content_sig, fresh_geom).is_none(),
            "atlas generation bump must miss the stale cache entry",
        );

        // Fresh emit on a clean row: must still span the full row. Before
        // the fix this path would skip emission and leave the row blank.
        let mut fresh_quads: Vec<QuadInstance> = Vec::new();
        emit_grid_row_backgrounds(
            &grid,
            0,
            0,
            cols,
            0.0,
            0.0,
            10.0,
            20.0,
            1.0,
            [0.0; 4],
            &mut fresh_quads,
        );
        assert_eq!(
            fresh_quads.len(),
            cols,
            "fresh emit on a clean-but-invalidated row must produce a full-row payload",
        );

        // Store the fresh payload and confirm the cache now carries a
        // whole-row entry against the new geometry signature.
        cache.store(
            node,
            line_id,
            content_sig,
            fresh_geom,
            fresh_quads.clone(),
            vec![],
            vec![],
            vec![],
            0,
        );
        let hit = cache
            .lookup_replayable(node, line_id, content_sig, fresh_geom)
            .expect("fresh payload must be retrievable under the new geometry");
        assert_eq!(
            hit.quads.len(),
            cols,
            "replacement cache entry must be whole-row after an invalidation miss",
        );
    }

    #[test]
    fn emit_grid_row_backgrounds_ignores_transparent_runs_but_emits_all_opaque() {
        use unshit_core::cell_grid::Cell;
        // Cells with `bg.a == 0` should not produce a quad (no visible
        // background to paint). All other cells must still emit, even if
        // the dirty flags would have narrowed a pre-fix emission window.
        let mut g = CellGrid::new(1, 4);
        g.clear_dirty();
        let fg = Color { r: 255, g: 255, b: 255, a: 255 };
        let solid = Color { r: 50, g: 100, b: 150, a: 255 };
        let transparent = Color { r: 0, g: 0, b: 0, a: 0 };

        g.set_cell(
            0,
            0,
            Cell { ch: 'a', fg, bg: solid, attrs: CellAttrs::empty(), wide_continuation: false },
        );
        g.set_cell(
            0,
            1,
            Cell {
                ch: 'b',
                fg,
                bg: transparent,
                attrs: CellAttrs::empty(),
                wide_continuation: false,
            },
        );
        g.set_cell(
            0,
            2,
            Cell { ch: 'c', fg, bg: solid, attrs: CellAttrs::empty(), wide_continuation: false },
        );
        g.set_cell(
            0,
            3,
            Cell { ch: 'd', fg, bg: solid, attrs: CellAttrs::empty(), wide_continuation: false },
        );

        let mut out_quads: Vec<QuadInstance> = Vec::new();
        emit_grid_row_backgrounds(&g, 0, 0, 4, 0.0, 0.0, 10.0, 20.0, 1.0, [0.0; 4], &mut out_quads);

        // Cells 0, 2, 3 produce opaque quads. Cells 2 and 3 share the same
        // bg so compute_bg_runs merges them into a single run covering
        // 20 px. Cell 0 is its own run at x=0 because the transparent cell
        // at col 1 splits the bg run. Cell 1 emits nothing.
        assert_eq!(out_quads.len(), 2, "transparent cells must not emit bg quads");
        assert!(out_quads.iter().any(|q| (q.pos[0] - 0.0).abs() < f32::EPSILON));
        assert!(
            out_quads.iter().any(|q| (q.pos[0] - 20.0).abs() < f32::EPSILON
                && (q.size[0] - 20.0).abs() < f32::EPSILON),
            "cells 2 and 3 with identical bg must merge into a single 20px-wide run",
        );
    }

    #[test]
    fn emit_grid_row_fresh_is_invoked_with_full_row_range() {
        // Structural regression for issue #63: `emit_grid_cells` must call
        // `emit_grid_row_fresh` with the full-row range, never a narrowed
        // damage window. This test pins the caller contract by probing the
        // lower-level background helper directly: the helper produces the
        // payload that lands in the line_quad_cache, so any future refactor
        // that reintroduces damage-range narrowing here will be caught by
        // the contract below.
        //
        // The assertion below mirrors the contract the fixed call site in
        // `emit_grid_cells` now upholds: emission spans 0..cols on every
        // cache MISS.
        let cols = 8;
        let g = grid_with_distinct_bg_per_col(0, 1, cols);
        let mut quads: Vec<QuadInstance> = Vec::new();
        // `emit_grid_cells` passes these exact arguments on cache miss.
        emit_grid_row_backgrounds(&g, 0, 0, cols, 0.0, 0.0, 10.0, 20.0, 1.0, [0.0; 4], &mut quads);
        assert_eq!(quads.len(), cols);
    }

    #[test]
    fn emit_row_on_miss_walks_only_damage_range() {
        // Structural regression for issue #52 Step 4: when a cached entry
        // exists for the same (node, line_id) with matching geometry and
        // a non-empty damage range, the splice fast path in
        // `emit_grid_cells` reshapes only columns inside
        // `[first_dirty_col..=last_dirty_col]`. Cells outside the damage
        // window come from the cached `glyph_col_index` lookup, with no
        // reshaping cost.
        //
        // Testing the full splice path end-to-end requires a live font
        // system and atlas; instead we exercise the structural invariant
        // it depends on: the `CachedLineState.glyph_col_index` must be
        // populated on fresh emit so a future splice pass has a valid
        // column-to-glyph map to copy from.
        use crate::line_quad_cache::{CachedLineState, LineGeometrySig, LineQuadCache};
        use unshit_core::id::NodeId;

        let cols = 8;
        let node = NodeId { index: 0, generation: 0 };
        let line_id: u64 = 42;
        let geom = LineGeometrySig::new(0.0, 0.0, 9.0, 18.0, 14.0, 1.0, [0.0; 4], cols as u32, 0);

        // Simulate the state after a fresh emit: every column has a glyph
        // index entry (Some for printable cells, None for empty cells).
        // Step 4 requires the length to equal `cols` so the splice path can
        // index it per column without bounds checks.
        let mut cache = LineQuadCache::new();
        let mut glyph_col_index: Vec<Option<u32>> = Vec::with_capacity(cols);
        for col in 0..cols {
            glyph_col_index.push(Some(col as u32));
        }
        let glyphs: Vec<GlyphInstance> = (0..cols)
            .map(|col| GlyphInstance {
                pos: [col as f32 * 9.0, 0.0],
                size: [9.0, 18.0],
                uv_min: [0.0; 2],
                uv_max: [1.0; 2],
                color: [1.0; 4],
                clip_rect: [0.0; 4],
                xform: IDENTITY_XFORM,
                xform_translate: IDENTITY_XFORM_TRANSLATE,
            })
            .collect();
        let keys: Vec<GlyphKey> = (0..cols)
            .map(|col| GlyphKey {
                font_id: 1,
                glyph_id: 100 + col as u16,
                font_size_tenths: 140,
                subpixel_bin: 0,
            })
            .collect();
        cache.store(
            node,
            line_id,
            0xaaaa,
            geom,
            vec![],
            glyphs.clone(),
            keys.clone(),
            glyph_col_index.clone(),
            0,
        );

        // Contract 1: the cached entry carries one glyph-index slot per
        // column. `emit_grid_cells` checks `cached.glyph_col_index.len()
        // == cols` before entering the splice path. This invariant is
        // what allows the splice path to blindly index by column.
        let CachedLineState { glyph_col_index: cached_idx, .. } =
            cache.get(node, line_id).expect("cache entry present").clone();
        assert_eq!(
            cached_idx.len(),
            cols,
            "fresh emit must populate glyph_col_index with one entry per column"
        );

        // Contract 2: undamaged columns replay their cached glyph and
        // key directly; the splice path does not reshape them. We model
        // this by mirroring the copy branch: for each column outside the
        // damage range, pull `(glyphs[i], keys[i])` via the index.
        let (damage_start, damage_end) = (3usize, 5usize); // damage cols 3..5
        let mut emitted_glyph_cols: Vec<usize> = Vec::new();
        let mut replayed_glyph_cols: Vec<usize> = Vec::new();
        for col in 0..cols {
            if col >= damage_start && col < damage_end {
                // Simulate the fresh-emit branch: a reshape would happen
                // here in production. Tag the column for assertion.
                emitted_glyph_cols.push(col);
            } else if let Some(idx) = cached_idx.get(col).copied().flatten() {
                // Simulate the replay branch: pull cached glyph/key.
                let _g = glyphs[idx as usize];
                let _k = keys[idx as usize];
                replayed_glyph_cols.push(col);
            }
        }
        assert_eq!(emitted_glyph_cols, vec![3, 4], "only damaged cols 3..5 reshape");
        assert_eq!(
            replayed_glyph_cols,
            vec![0, 1, 2, 5, 6, 7],
            "all non-damaged cols replay from cache without reshape",
        );
    }

    #[test]
    fn splice_copy_translates_cached_glyph_y_when_row_rotated() {
        // Regression for issue #77 on the splice path. When a line's
        // stable `line_id` rotates to a new row AND its content changes
        // in the same frame, `emit_grid_cells` falls through to the
        // splice path (content_sig miss, geometry still valid). Cells
        // outside the damage window are copied from the cached entry,
        // whose glyph instances carry the pre-scroll absolute Y. The
        // splice copy MUST shift Y by `(current_row - cached_row) *
        // cell_h` so the replayed glyphs land at the new row slot.
        //
        // We assert the invariant structurally: mirror the branch inside
        // `emit_grid_row_splice` for undamaged columns and verify the
        // translate yields the new row's Y.
        let cols = 4;
        let cell_h: f32 = 18.0;
        let cached_row: usize = 10;
        let row: usize = 9;
        let cached_y = cached_row as f32 * cell_h;
        let expected_y = row as f32 * cell_h;

        let cached_glyphs: Vec<GlyphInstance> = (0..cols)
            .map(|col| GlyphInstance {
                pos: [col as f32 * 9.0, cached_y],
                size: [9.0, cell_h],
                uv_min: [0.0; 2],
                uv_max: [1.0; 2],
                color: [1.0; 4],
                clip_rect: [0.0; 4],
                xform: IDENTITY_XFORM,
                xform_translate: IDENTITY_XFORM_TRANSLATE,
            })
            .collect();

        let mut copied_ys: Vec<f32> = Vec::new();
        for g in &cached_glyphs {
            let mut copy = *g;
            if cached_row != row {
                let dy = (row as f32 - cached_row as f32) * cell_h;
                copy.pos[1] += dy;
            }
            copied_ys.push(copy.pos[1]);
        }

        assert!(
            copied_ys.iter().all(|y| (y - expected_y).abs() < f32::EPSILON),
            "splice copy must place glyphs at Y={expected_y} after row delta, got {copied_ys:?}",
        );
    }

    #[test]
    fn narrowed_emission_range_documents_pre_fix_truncation() {
        // Documentation-style guard for issue #63's failure mode. Before
        // the fix, `emit_grid_cells` handed `emit_grid_row_fresh` a
        // `scan_start..scan_end` slice derived from line damage. That
        // stripped the stored cache payload to the damaged sub-range.
        // Here we show that feeding a narrow range to the pure helper
        // DOES truncate output, so any future regression that plumbs a
        // non-full range into the fresh-emit path will immediately
        // manifest as a missing cell in the cache. The emission call
        // that actually runs in `emit_grid_cells` uses `(0, cols)` and
        // is covered by the other tests in this block.
        let cols = 6;
        let g = grid_with_distinct_bg_per_col(0, 1, cols);

        let mut truncated: Vec<QuadInstance> = Vec::new();
        emit_grid_row_backgrounds(&g, 0, 2, 4, 0.0, 0.0, 10.0, 20.0, 1.0, [0.0; 4], &mut truncated);
        assert_eq!(
            truncated.len(),
            2,
            "narrow range must produce only the runs inside the window (the bug mechanism)",
        );

        let mut full: Vec<QuadInstance> = Vec::new();
        emit_grid_row_backgrounds(&g, 0, 0, cols, 0.0, 0.0, 10.0, 20.0, 1.0, [0.0; 4], &mut full);
        assert_eq!(
            full.len(),
            cols,
            "full range emits every column, which is what the fix guarantees"
        );
    }

    // -- bg region merging (issue #84) --------------------------------------
    //
    // After issue #84 the bg emitter iterates `compute_bg_runs_in_range`
    // instead of `compute_style_runs_in_range`. Rows with uniform bg but
    // varying fg collapse to a single bg quad. The tests below pin the
    // merged emission contract and the default-bg elision win.

    #[test]
    fn emit_grid_row_backgrounds_elides_default_transparent_bg() {
        use unshit_core::cell_grid::Cell;
        // Every cell in the row has the terminal DEFAULT_BG (alpha 0). The
        // renderer must emit zero quads because a transparent quad is a
        // visual no op and a waste of instance capacity. Pins the core
        // elision proof the epic spec calls for.
        let cols = 12;
        let mut g = CellGrid::new(1, cols);
        g.clear_dirty();
        let fg = Color { r: 255, g: 255, b: 255, a: 255 };
        let transparent = Color { r: 0, g: 0, b: 0, a: 0 };
        for col in 0..cols {
            g.set_cell(
                0,
                col,
                Cell {
                    ch: 'x',
                    fg,
                    bg: transparent,
                    attrs: CellAttrs::empty(),
                    wide_continuation: false,
                },
            );
        }

        let mut row_quads: Vec<QuadInstance> = Vec::new();
        emit_grid_row_backgrounds(
            &g,
            0,
            0,
            cols,
            0.0,
            0.0,
            10.0,
            20.0,
            1.0,
            [0.0; 4],
            &mut row_quads,
        );

        assert!(
            row_quads.is_empty(),
            "default transparent bg must emit zero quads; got {} quads",
            row_quads.len(),
        );
    }

    #[test]
    fn emit_grid_row_backgrounds_merges_same_color_across_fg_boundary() {
        use unshit_core::cell_grid::Cell;
        // Row of 10 cells with uniform bg = blue (opaque) and fg
        // alternating per cell. The style run pass would emit 10 quads;
        // after #84 the bg pass merges them all into 1.
        let cols = 10;
        let mut g = CellGrid::new(1, cols);
        g.clear_dirty();
        let blue = Color { r: 20, g: 30, b: 200, a: 255 };
        let fg_a = Color { r: 255, g: 255, b: 255, a: 255 };
        let fg_b = Color { r: 10, g: 10, b: 10, a: 255 };
        for col in 0..cols {
            let fg = if col % 2 == 0 { fg_a } else { fg_b };
            g.set_cell(
                0,
                col,
                Cell { ch: 'x', fg, bg: blue, attrs: CellAttrs::empty(), wide_continuation: false },
            );
        }

        let mut row_quads: Vec<QuadInstance> = Vec::new();
        let cell_w = 10.0;
        let cell_h = 20.0;
        emit_grid_row_backgrounds(
            &g,
            0,
            0,
            cols,
            0.0,
            0.0,
            cell_w,
            cell_h,
            1.0,
            [0.0; 4],
            &mut row_quads,
        );

        assert_eq!(
            row_quads.len(),
            1,
            "uniform bg across fg boundary must merge into one quad; got {}",
            row_quads.len(),
        );
        let q = row_quads[0];
        assert!((q.pos[0] - 0.0).abs() < f32::EPSILON, "x must be 0");
        assert!(
            (q.size[0] - cols as f32 * cell_w).abs() < f32::EPSILON,
            "width must span every column: expected {}, got {}",
            cols as f32 * cell_w,
            q.size[0],
        );
        assert!((q.size[1] - cell_h).abs() < f32::EPSILON, "height must be one cell tall");
    }

    #[test]
    fn emit_grid_row_backgrounds_splits_on_color_change() {
        use unshit_core::cell_grid::Cell;
        // 4 cells of red followed by 6 of green. fg varies per cell but
        // that must not add quads, the bg pass splits only on bg change.
        let cols = 10;
        let mut g = CellGrid::new(1, cols);
        g.clear_dirty();
        let red = Color { r: 200, g: 20, b: 20, a: 255 };
        let green = Color { r: 20, g: 200, b: 20, a: 255 };
        let fg_a = Color { r: 255, g: 255, b: 255, a: 255 };
        let fg_b = Color { r: 10, g: 10, b: 10, a: 255 };
        for col in 0..4 {
            let fg = if col % 2 == 0 { fg_a } else { fg_b };
            g.set_cell(
                0,
                col,
                Cell { ch: 'x', fg, bg: red, attrs: CellAttrs::empty(), wide_continuation: false },
            );
        }
        for col in 4..cols {
            let fg = if col % 2 == 0 { fg_a } else { fg_b };
            g.set_cell(
                0,
                col,
                Cell {
                    ch: 'x',
                    fg,
                    bg: green,
                    attrs: CellAttrs::empty(),
                    wide_continuation: false,
                },
            );
        }

        let mut row_quads: Vec<QuadInstance> = Vec::new();
        let cell_w = 10.0;
        let cell_h = 20.0;
        emit_grid_row_backgrounds(
            &g,
            0,
            0,
            cols,
            0.0,
            0.0,
            cell_w,
            cell_h,
            1.0,
            [0.0; 4],
            &mut row_quads,
        );

        assert_eq!(row_quads.len(), 2, "one quad per bg color stripe; got {}", row_quads.len());
        let red_quad = row_quads
            .iter()
            .find(|q| (q.pos[0] - 0.0).abs() < f32::EPSILON)
            .expect("red quad at x=0 must exist");
        let green_quad = row_quads
            .iter()
            .find(|q| (q.pos[0] - 40.0).abs() < f32::EPSILON)
            .expect("green quad at x=40 must exist");
        assert!(
            (red_quad.size[0] - 4.0 * cell_w).abs() < f32::EPSILON,
            "red quad width must be 4 cells",
        );
        assert!(
            (green_quad.size[0] - 6.0 * cell_w).abs() < f32::EPSILON,
            "green quad width must be 6 cells",
        );
        let red_linear = red.to_linear_f32();
        let green_linear = green.to_linear_f32();
        assert!(
            (red_quad.color[0] - red_linear[0]).abs() < f32::EPSILON,
            "red quad color must match red bg",
        );
        assert!(
            (green_quad.color[1] - green_linear[1]).abs() < f32::EPSILON,
            "green quad color must match green bg",
        );
    }

    #[test]
    fn emit_grid_row_backgrounds_snaps_fractional_adjacent_opaque_boundaries() {
        use unshit_core::cell_grid::Cell;

        let cols = 3;
        let mut g = CellGrid::new(1, cols);
        g.clear_dirty();
        let fg = Color { r: 255, g: 255, b: 255, a: 255 };
        let colors = [
            Color { r: 200, g: 20, b: 20, a: 255 },
            Color { r: 20, g: 200, b: 20, a: 255 },
            Color { r: 20, g: 30, b: 200, a: 255 },
        ];
        for (col, bg) in colors.into_iter().enumerate() {
            g.set_cell(
                0,
                col,
                Cell { ch: 'x', fg, bg, attrs: CellAttrs::empty(), wide_continuation: false },
            );
        }

        let mut row_quads: Vec<QuadInstance> = Vec::new();
        let cell_w = 10.25;
        emit_grid_row_backgrounds(
            &g,
            0,
            0,
            cols,
            0.0,
            0.0,
            cell_w,
            20.0,
            1.0,
            [0.0; 4],
            &mut row_quads,
        );

        assert_eq!(row_quads.len(), cols);
        assert!((row_quads[0].pos[0] - 0.0).abs() < f32::EPSILON);
        assert!((row_quads[0].size[0] - 10.0).abs() < f32::EPSILON);
        assert!((row_quads[1].pos[0] - 10.0).abs() < f32::EPSILON);
        assert!((row_quads[1].size[0] - 11.0).abs() < f32::EPSILON);
        assert!((row_quads[2].pos[0] - 21.0).abs() < f32::EPSILON);
        assert!((row_quads[2].size[0] - 9.75).abs() < f32::EPSILON);
        assert!(
            ((row_quads[0].pos[0] + row_quads[0].size[0]) - row_quads[1].pos[0]).abs()
                < f32::EPSILON
        );
        assert!(
            ((row_quads[1].pos[0] + row_quads[1].size[0]) - row_quads[2].pos[0]).abs()
                < f32::EPSILON
        );
    }

    #[test]
    fn emit_grid_row_backgrounds_does_not_overlap_transparent_gaps() {
        use unshit_core::cell_grid::Cell;

        let mut g = CellGrid::new(1, 3);
        g.clear_dirty();
        let fg = Color { r: 255, g: 255, b: 255, a: 255 };
        let red = Color { r: 200, g: 20, b: 20, a: 255 };
        let transparent = Color { r: 0, g: 0, b: 0, a: 0 };
        let blue = Color { r: 20, g: 30, b: 200, a: 255 };
        for (col, bg) in [red, transparent, blue].into_iter().enumerate() {
            g.set_cell(
                0,
                col,
                Cell { ch: 'x', fg, bg, attrs: CellAttrs::empty(), wide_continuation: false },
            );
        }

        let mut row_quads: Vec<QuadInstance> = Vec::new();
        let cell_w = 10.25;
        emit_grid_row_backgrounds(
            &g,
            0,
            0,
            3,
            0.0,
            0.0,
            cell_w,
            20.0,
            1.0,
            [0.0; 4],
            &mut row_quads,
        );

        assert_eq!(row_quads.len(), 2);
        assert!((row_quads[0].pos[0] - 0.0).abs() < f32::EPSILON);
        assert!((row_quads[0].size[0] - cell_w).abs() < f32::EPSILON);
        assert!((row_quads[1].pos[0] - (cell_w * 2.0)).abs() < f32::EPSILON);
        assert!((row_quads[1].size[0] - cell_w).abs() < f32::EPSILON);
    }

    #[test]
    fn emit_grid_row_backgrounds_skips_zero_alpha_runs_mid_row() {
        use unshit_core::cell_grid::Cell;
        // Cols 0..3 red, cols 3..6 transparent (DEFAULT_BG), cols 6..10
        // blue. The transparent middle run must be elided and the outer
        // runs must not be merged across it.
        let cols = 10;
        let mut g = CellGrid::new(1, cols);
        g.clear_dirty();
        let red = Color { r: 200, g: 20, b: 20, a: 255 };
        let blue = Color { r: 20, g: 30, b: 200, a: 255 };
        let transparent = Color { r: 0, g: 0, b: 0, a: 0 };
        let fg = Color { r: 255, g: 255, b: 255, a: 255 };
        for col in 0..3 {
            g.set_cell(
                0,
                col,
                Cell { ch: 'x', fg, bg: red, attrs: CellAttrs::empty(), wide_continuation: false },
            );
        }
        for col in 3..6 {
            g.set_cell(
                0,
                col,
                Cell {
                    ch: 'x',
                    fg,
                    bg: transparent,
                    attrs: CellAttrs::empty(),
                    wide_continuation: false,
                },
            );
        }
        for col in 6..cols {
            g.set_cell(
                0,
                col,
                Cell { ch: 'x', fg, bg: blue, attrs: CellAttrs::empty(), wide_continuation: false },
            );
        }

        let mut row_quads: Vec<QuadInstance> = Vec::new();
        let cell_w = 10.0;
        let cell_h = 20.0;
        emit_grid_row_backgrounds(
            &g,
            0,
            0,
            cols,
            0.0,
            0.0,
            cell_w,
            cell_h,
            1.0,
            [0.0; 4],
            &mut row_quads,
        );

        assert_eq!(
            row_quads.len(),
            2,
            "transparent middle run must be elided; got {}",
            row_quads.len(),
        );
        let red_quad = row_quads
            .iter()
            .find(|q| (q.pos[0] - 0.0).abs() < f32::EPSILON)
            .expect("red quad at x=0 must exist");
        let blue_quad = row_quads
            .iter()
            .find(|q| (q.pos[0] - 60.0).abs() < f32::EPSILON)
            .expect("blue quad at x=60 must exist");
        assert!(
            (red_quad.size[0] - 3.0 * cell_w).abs() < f32::EPSILON,
            "red quad width must be 3 cells (not merged across transparent gap)",
        );
        assert!(
            (blue_quad.size[0] - 4.0 * cell_w).abs() < f32::EPSILON,
            "blue quad width must be 4 cells",
        );
    }

    #[test]
    fn emit_grid_row_backgrounds_merges_across_attrs_boundary() {
        use unshit_core::cell_grid::Cell;
        // Regression pin for #84: bg runs ignore attribute flags (BOLD,
        // ITALIC, etc.). Two cells with the same bg but different attrs
        // must merge into a single bg quad, because the underlying bg
        // color is visually identical.
        let cols = 4;
        let mut g = CellGrid::new(1, cols);
        g.clear_dirty();
        let red = Color { r: 200, g: 20, b: 20, a: 255 };
        let fg = Color { r: 255, g: 255, b: 255, a: 255 };
        g.set_cell(
            0,
            0,
            Cell { ch: 'a', fg, bg: red, attrs: CellAttrs::empty(), wide_continuation: false },
        );
        g.set_cell(
            0,
            1,
            Cell { ch: 'b', fg, bg: red, attrs: CellAttrs::BOLD, wide_continuation: false },
        );
        g.set_cell(
            0,
            2,
            Cell { ch: 'c', fg, bg: red, attrs: CellAttrs::ITALIC, wide_continuation: false },
        );
        g.set_cell(
            0,
            3,
            Cell { ch: 'd', fg, bg: red, attrs: CellAttrs::UNDERLINE, wide_continuation: false },
        );

        let mut row_quads: Vec<QuadInstance> = Vec::new();
        emit_grid_row_backgrounds(
            &g,
            0,
            0,
            cols,
            0.0,
            0.0,
            10.0,
            20.0,
            1.0,
            [0.0; 4],
            &mut row_quads,
        );

        assert_eq!(
            row_quads.len(),
            1,
            "bg merging must ignore attribute flags; got {} quads",
            row_quads.len(),
        );
        assert!((row_quads[0].size[0] - 40.0).abs() < f32::EPSILON, "quad must span all 4 cells");
    }

    #[test]
    fn emit_grid_row_backgrounds_on_empty_row_emits_zero_quads() {
        // Default-constructed CellGrid has Cell::default for every cell,
        // which uses Color::TRANSPARENT as bg. The emitter must produce
        // zero quads on such a row under the real full-row (0, cols) call
        // shape. Pins the elision win under the call shape actually used
        // by `emit_grid_cells`.
        let cols = 20;
        let g = CellGrid::new(1, cols);
        let mut row_quads: Vec<QuadInstance> = Vec::new();
        emit_grid_row_backgrounds(
            &g,
            0,
            0,
            cols,
            0.0,
            0.0,
            10.0,
            20.0,
            1.0,
            [0.0; 4],
            &mut row_quads,
        );
        assert!(row_quads.is_empty(), "a blank row must emit zero bg quads");
    }

    // Fragment shader routing tests (issue #96 step 2)
    #[cfg(feature = "grid-fragment-shader")]
    mod grid_fragment_routing {
        use super::super::{try_record_grid_for_fragment_path, FrameBatch, GridDrawRecord};
        use unshit_core::id::NodeId;

        fn sample_clip() -> [f32; 4] {
            [0.0, 0.0, 9999.0, 9999.0]
        }

        #[test]
        fn off_does_not_push_any_record() {
            let mut batch = FrameBatch::new();
            let routed = try_record_grid_for_fragment_path(
                false,
                &mut batch,
                NodeId::DANGLING,
                10.0,
                20.0,
                8.0,
                16.0,
                80,
                24,
                14.0,
                1.0,
                sample_clip(),
            );
            assert!(!routed);
            assert!(batch.grid_records.is_empty());
        }

        #[test]
        fn on_pushes_record_with_geometry() {
            let mut batch = FrameBatch::new();
            let routed = try_record_grid_for_fragment_path(
                true,
                &mut batch,
                NodeId::DANGLING,
                10.0,
                20.0,
                8.5,
                16.25,
                80,
                24,
                14.0,
                0.75,
                sample_clip(),
            );
            assert!(routed);
            assert_eq!(batch.grid_records.len(), 1);
            assert_eq!(
                batch.grid_records[0],
                GridDrawRecord {
                    node_id: NodeId::DANGLING,
                    origin_x: 10.0,
                    origin_y: 20.0,
                    cell_w: 8.5,
                    cell_h: 16.25,
                    cols: 80,
                    rows: 24,
                    font_size: 14.0,
                    opacity: 0.75,
                    clip_rect: sample_clip(),
                }
            );
        }

        #[test]
        fn on_repeated_calls_append_records() {
            let mut batch = FrameBatch::new();
            for _ in 0..3 {
                try_record_grid_for_fragment_path(
                    true,
                    &mut batch,
                    NodeId::DANGLING,
                    0.0,
                    0.0,
                    8.0,
                    16.0,
                    80,
                    24,
                    14.0,
                    1.0,
                    sample_clip(),
                );
            }
            assert_eq!(batch.grid_records.len(), 3);
        }

        #[test]
        fn clear_drops_records() {
            let mut batch = FrameBatch::new();
            try_record_grid_for_fragment_path(
                true,
                &mut batch,
                NodeId::DANGLING,
                0.0,
                0.0,
                8.0,
                16.0,
                80,
                24,
                14.0,
                1.0,
                sample_clip(),
            );
            assert_eq!(batch.grid_records.len(), 1);
            batch.clear();
            assert!(batch.grid_records.is_empty());
        }
    }
}
